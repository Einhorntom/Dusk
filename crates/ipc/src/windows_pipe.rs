use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, NAMED_PIPE_MODE, PIPE_READMODE_BYTE,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::{HRESULT, PCWSTR};

use crate::{IpcError, MAX_MESSAGE_SIZE};

const PIPE_NAME: &str = r"\\.\pipe\dusk-v0";
/// Overrides the pipe name for the daemon and the CLI alike, so a test or a
/// second instance does not talk to the user's running daemon.
pub const PIPE_ENV: &str = "DUSK_PIPE";

fn pipe_name() -> String {
    match std::env::var(PIPE_ENV) {
        Ok(name) if !name.trim().is_empty() => format!(r"\\.\pipe\{}", name.trim()),
        _ => PIPE_NAME.to_owned(),
    }
}
const PIPE_INSTANCE_LIMIT: u32 = 16;

pub fn serve_forever(
    handler: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static,
) -> Result<(), IpcError> {
    serve_forever_on(&pipe_name(), handler)
}

/// Serves `handler` on the named pipe `name` (tests use a private name).
pub(crate) fn serve_forever_on(
    name: &str,
    handler: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static,
) -> Result<(), IpcError> {
    let handler = Arc::new(handler);
    let encoded_name = wide_null(name);
    loop {
        let raw_pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(encoded_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                NAMED_PIPE_MODE(PIPE_TYPE_BYTE.0 | PIPE_READMODE_BYTE.0 | PIPE_WAIT.0),
                PIPE_INSTANCE_LIMIT,
                MAX_MESSAGE_SIZE as u32 + 4,
                MAX_MESSAGE_SIZE as u32 + 4,
                0,
                None,
            )
        };
        if raw_pipe == INVALID_HANDLE_VALUE {
            return Err(IpcError::Io(std::io::Error::last_os_error()));
        }
        let pipe_handle = raw_pipe.0 as isize;
        let handle = unsafe { OwnedHandle::from_raw_handle(raw_pipe.0 as RawHandle) };
        let mut pipe = File::from(handle);
        let connect_result = unsafe { ConnectNamedPipe(HANDLE(pipe_handle as _), None) };
        if let Err(error) = connect_result
            && error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0)
        {
            eprintln!("warning: ConnectNamedPipe failed: {error}");
            drop(pipe);
            continue;
        }
        let handler = handler.clone();
        thread::Builder::new()
            .name("dusk-ipc-client".into())
            .spawn(move || {
                let result = serve_connection(&mut pipe, handler.as_ref());
                if let Err(error) = unsafe { DisconnectNamedPipe(HANDLE(pipe_handle as _)) } {
                    eprintln!("warning: DisconnectNamedPipe failed: {error}");
                }
                drop(pipe);
                if let Err(error) = result {
                    eprintln!("warning: named-pipe request failed: {error}");
                }
            })
            .map_err(|error| IpcError::Io(std::io::Error::other(error)))?;
    }
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
