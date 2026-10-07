//! Start with Windows (PRD "Settings", SPEC-UI-12). The zip version uses the
//! per-user Run entry that starts `duskd --background` at sign-in; the
//! packaged (Microsoft Store) version cannot, because Windows keeps a
//! package's registry writes private, so it uses the package's startup task
//! (`DuskStartup` in its manifest). Either way Windows holds the setting, so
//! Settings and Task Manager's Startup apps never disagree.

use std::path::Path;

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::package::is_packaged;
use crate::wide_null;

/// The startup task's `TaskId` in the package manifest.
const STARTUP_TASK: &str = "DuskStartup";

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Dusk";

/// Whether Dusk starts at sign-in.
pub(crate) fn is_enabled() -> bool {
    if is_packaged() {
        return startup_task()
            .and_then(|task| task.State())
            .is_ok_and(|state| {
                state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
            });
    }
    read(RUN_KEY, VALUE_NAME).is_some()
}

/// Turns starting at sign-in on (for the running `duskd.exe`) or off.
pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    if is_packaged() {
        return set_startup_task(enabled);
    }
    if enabled {
        let exe =
            std::env::current_exe().map_err(|error| format!("cannot find duskd.exe: {error}"))?;
        write(RUN_KEY, VALUE_NAME, &command_for(&exe))
    } else {
        delete(RUN_KEY, VALUE_NAME)
    }
}

fn startup_task() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK))?.join()
}

fn set_startup_task(enabled: bool) -> Result<(), String> {
    let task = startup_task().map_err(|error| format!("cannot reach the startup task: {error}"))?;
    if !enabled {
        return task
            .Disable()
            .map_err(|error| format!("could not turn off Start with Windows: {error}"));
    }
    let state = task
        .RequestEnableAsync()
        .and_then(|request| request.join())
        .map_err(|error| format!("could not turn on Start with Windows: {error}"))?;
    match state {
        StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Ok(()),
        StartupTaskState::DisabledByUser => Err(
            "Windows keeps Dusk from starting because it was turned off in Task Manager > Startup apps; turn it on there."
                .into(),
        ),
        _ => Err("Starting at sign-in is turned off by a policy on this PC.".into()),
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
