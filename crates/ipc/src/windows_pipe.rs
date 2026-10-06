use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, NAMED_PIPE_MODE, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::{HRESULT, PCWSTR};

use crate::{IpcError, MAX_MESSAGE_SIZE};

const PIPE_NAME: &str = r"\\.\pipe\dusk-v0";
/// Overrides the pipe name for the daemon and the CLI alike, so a test or a
/// second instance does not talk to the user's running daemon.
pub const PIPE_ENV: &str = "DUSK_PIPE";

/// The full pipe path, e.g. `\\.\pipe\dusk-v0`.
pub fn pipe_name() -> String {
    match std::env::var(PIPE_ENV) {
        Ok(name) if !name.trim().is_empty() => format!(r"\\.\pipe\{}", name.trim()),
        _ => PIPE_NAME.to_owned(),
    }
}
const PIPE_INSTANCE_LIMIT: u32 = 16;

/// The daemon's end of the pipe. Binding claims the name before any client
/// can connect: it fails if any other process (another `duskd`, or a program
/// impersonating it) already serves that name, and remote clients are
/// rejected.
pub struct PipeServer {
    name: Vec<u16>,
    /// The first instance, created by `bind` and not yet connected.
    first: Option<HANDLE>,
}

// SAFETY: the handle is owned by the server and used by one thread at a time.
unsafe impl Send for PipeServer {}

impl PipeServer {
    pub fn bind() -> Result<Self, IpcError> {
        Self::bind_on(&pipe_name())
    }

    /// Binds the named pipe `name` (tests use a private name).
    pub(crate) fn bind_on(name: &str) -> Result<Self, IpcError> {
        let name = wide_null(name);
        let first = create_instance(&name, true)?;
        Ok(Self {
            name,
            first: Some(first),
        })
    }

    /// Answers each request with `handler`, one thread per connection.
    pub fn serve_forever(
        mut self,
        handler: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static,
    ) -> Result<(), IpcError> {
        let handler = Arc::new(handler);
        loop {
            let raw_pipe = match self.first.take() {
                Some(first) => first,
                None => create_instance(&self.name, false)?,
            };
            serve_instance(raw_pipe, &handler)?;
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        if let Some(first) = self.first.take() {
            drop(unsafe { OwnedHandle::from_raw_handle(first.0 as RawHandle) });
        }
    }
}

fn create_instance(name: &[u16], first: bool) -> Result<HANDLE, IpcError> {
    let open_mode = if first {
        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        PIPE_ACCESS_DUPLEX
    };
    let raw_pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            NAMED_PIPE_MODE(
                PIPE_TYPE_BYTE.0
                    | PIPE_READMODE_BYTE.0
                    | PIPE_WAIT.0
                    | PIPE_REJECT_REMOTE_CLIENTS.0,
            ),
            PIPE_INSTANCE_LIMIT,
            MAX_MESSAGE_SIZE as u32 + 4,
            MAX_MESSAGE_SIZE as u32 + 4,
            0,
            None,
        )
    };
    if raw_pipe == INVALID_HANDLE_VALUE {
        if first && unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            return Err(IpcError::PipeInUse);
        }
        return Err(IpcError::Io(std::io::Error::last_os_error()));
    }
    Ok(raw_pipe)
}

/// Waits for a client on `raw_pipe` and answers it on its own thread.
fn serve_instance(
    raw_pipe: HANDLE,
    handler: &Arc<impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static>,
) -> Result<(), IpcError> {
    let pipe_handle = raw_pipe.0 as isize;
    let handle = unsafe { OwnedHandle::from_raw_handle(raw_pipe.0 as RawHandle) };
    let mut pipe = File::from(handle);
    let connect_result = unsafe { ConnectNamedPipe(HANDLE(pipe_handle as _), None) };
    if let Err(error) = connect_result
        && error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0)
    {
        log::warn!("ConnectNamedPipe failed: {error}");
        drop(pipe);
        return Ok(());
    }
    let handler = handler.clone();
    thread::Builder::new()
        .name("dusk-ipc-client".into())
        .spawn(move || {
            let result = serve_connection(&mut pipe, handler.as_ref());
            if let Err(error) = unsafe { DisconnectNamedPipe(HANDLE(pipe_handle as _)) } {
                log::warn!("DisconnectNamedPipe failed: {error}");
            }
            drop(pipe);
            if let Err(error) = result {
                log::warn!("named-pipe request failed: {error}");
            }
        })
        .map_err(|error| IpcError::Io(std::io::Error::other(error)))?;
    Ok(())
}

fn serve_connection(pipe: &mut File, handler: &impl Fn(&[u8]) -> Vec<u8>) -> Result<(), IpcError> {
    let request = read_frame(pipe)?;
    let response = handler(&request);
    write_frame(pipe, &response)?;
    wait_for_client_close(pipe)
}

fn wait_for_client_close(pipe: &mut File) -> Result<(), IpcError> {
    let mut trailing_byte = [0u8; 1];
    match pipe.read(&mut trailing_byte) {
        Ok(0) => Ok(()),
        Ok(_) => Err(IpcError::Protocol(
            "client sent data after receiving the response".into(),
        )),
        Err(error) if matches!(error.raw_os_error(), Some(109 | 232 | 233)) => Ok(()),
        Err(error) => Err(io_context("wait for client close", error)),
    }
}

pub fn transact(payload: &[u8]) -> Result<Vec<u8>, IpcError> {
    transact_on(&pipe_name(), payload)
}

pub(crate) fn transact_on(name: &str, payload: &[u8]) -> Result<Vec<u8>, IpcError> {
    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(IpcError::Protocol("request exceeds the 1 MiB limit".into()));
    }
    let encoded_name = wide_null(name);
    let mut handle = None;
    for attempt in 0..10 {
        match open_pipe(&encoded_name) {
            Ok(opened) => {
                handle = Some(opened);
                break;
            }
            Err(IpcError::DaemonUnavailable) if attempt < 9 => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    let handle = handle.ok_or(IpcError::DaemonUnavailable)?;
    let owned = unsafe { OwnedHandle::from_raw_handle(handle.0 as RawHandle) };
    let mut pipe = File::from(owned);
    write_frame(&mut pipe, payload)?;
    read_frame(&mut pipe)
}

fn open_pipe(name: &[u16]) -> Result<HANDLE, IpcError> {
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|error| {
        if error.code() == HRESULT::from_win32(windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.0)
            || error.code() == HRESULT::from_win32(windows::Win32::Foundation::ERROR_PIPE_BUSY.0)
        {
            IpcError::DaemonUnavailable
        } else {
            IpcError::Io(std::io::Error::other(error.to_string()))
        }
    })
}

fn read_frame(pipe: &mut File) -> Result<Vec<u8>, IpcError> {
    let mut header = [0u8; 4];
    pipe.read_exact(&mut header)
        .map_err(|error| io_context("read frame header", error))?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_MESSAGE_SIZE {
        return Err(IpcError::Protocol("message exceeds the 1 MiB limit".into()));
    }
    let mut payload = vec![0; length];
    pipe.read_exact(&mut payload)
        .map_err(|error| io_context("read frame payload", error))?;
    Ok(payload)
}

fn write_frame(pipe: &mut File, payload: &[u8]) -> Result<(), IpcError> {
    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(IpcError::Protocol("message exceeds the 1 MiB limit".into()));
    }
    pipe.write_all(&(payload.len() as u32).to_le_bytes())
        .map_err(|error| io_context("write frame header", error))?;
    pipe.write_all(payload)
        .map_err(|error| io_context("write frame payload", error))?;
    pipe.flush()
        .map_err(|error| io_context("flush frame", error))
}

fn io_context(context: &str, error: std::io::Error) -> IpcError {
    IpcError::Io(std::io::Error::new(
        error.kind(),
        format!("{context}: {error}"),
    ))
}

fn wide_null(value: &str) -> Vec<u16> {
    Path::new(value)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
