//! Start with Windows (PRD "Settings", SPEC-UI-6): the per-user Run entry
//! that starts `duskd --background` at sign-in. The entry itself is the
//! setting, so Settings, Task Manager's Startup apps and the registry never
//! disagree.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::PCWSTR;

use crate::wide_null;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Dusk";

/// Whether Dusk starts at sign-in.
pub(crate) fn is_enabled() -> bool {
    read(RUN_KEY, VALUE_NAME).is_some()
}

/// Turns starting at sign-in on (for the running `duskd.exe`) or off.
pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe =
            std::env::current_exe().map_err(|error| format!("cannot find duskd.exe: {error}"))?;
        write(RUN_KEY, VALUE_NAME, &command_for(&exe))
    } else {
        delete(RUN_KEY, VALUE_NAME)
    }
}

/// The command Windows runs at sign-in: quiet, straight to the tray.
fn command_for(exe: &Path) -> String {
    format!("\"{}\" --background", exe.display())
}

fn read(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide_null(key), wide_null(value));
    let mut size = 0u32;
    let query = |data: Option<*mut core::ffi::c_void>, size: &mut u32| unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            data,
            Some(size),
        )
    };
    if query(None, &mut size) != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    if query(Some(buffer.as_mut_ptr().cast()), &mut size) != ERROR_SUCCESS {
        return None;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn write(key: &str, value: &str, data: &str) -> Result<(), String> {
    let (key, value, data) = (wide_null(key), wide_null(value), wide_null(data));
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    check(result, "could not turn on Start with Windows")
}

fn delete(key: &str, value: &str) -> Result<(), String> {
    let (key, value) = (wide_null(key), wide_null(value));
    let result = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(result, "could not turn off Start with Windows")
}

fn check(result: WIN32_ERROR, what: &str) -> Result<(), String> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "{what}: {}",
            windows::core::Error::from(result.to_hresult()).message()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::RegDeleteTreeW;

    #[test]
    fn the_command_starts_this_exe_in_the_tray() {
        assert_eq!(
            command_for(Path::new(r"C:\Program Files\Dusk\duskd.exe")),
            r#""C:\Program Files\Dusk\duskd.exe" --background"#
        );
    }

    /// Uses a private key under HKCU\Software, never the real Run key.
    #[test]
    fn an_entry_is_written_read_and_removed() {
        let key = format!(r"Software\Dusk-autostart-test-{}", std::process::id());
        let command = command_for(Path::new(r"C:\Dusk\duskd.exe"));
        assert_eq!(read(&key, VALUE_NAME), None);
        write(&key, VALUE_NAME, &command).unwrap();
        assert_eq!(read(&key, VALUE_NAME).as_deref(), Some(command.as_str()));
        delete(&key, VALUE_NAME).unwrap();
        assert_eq!(read(&key, VALUE_NAME), None);
        // Removing it again is not an error.
        delete(&key, VALUE_NAME).unwrap();
        let wide = wide_null(&key);
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide.as_ptr()));
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(
                HKEY_CURRENT_USER,
                PCWSTR(wide.as_ptr()),
            );
        }
    }
}
