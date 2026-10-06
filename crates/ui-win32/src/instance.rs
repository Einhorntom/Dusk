//! One `duskd` per pipe name and session. A second start hands over to the
//! running one (which shows Settings) instead of adding a second tray icon,
//! a second set of hotkeys and a second write rate limiter.

use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, HWND_BROADCAST, MB_ICONERROR, MB_OK, MessageBoxW,
    PostMessageW, RegisterWindowMessageW,
};
use windows::core::PCWSTR;

use crate::wide_null;

/// Held for the life of the process; the claim ends when it is dropped.
pub struct InstanceGuard {
    _mutex: OwnedHandle,
}

pub enum Claim {
    First(InstanceGuard),
    AlreadyRunning,
}

/// Claims `key` (the daemon's pipe name) for this session.
pub fn claim_instance(key: &str) -> Result<Claim, String> {
    let name = wide_null(&format!(r"Local\Dusk-{}", sanitize(key)));
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
        .map_err(|error| format!("cannot create the single-instance mutex: {error}"))?;
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let mutex = unsafe { OwnedHandle::from_raw_handle(handle.0 as RawHandle) };
    Ok(if existed {
        Claim::AlreadyRunning
    } else {
        Claim::First(InstanceGuard { _mutex: mutex })
    })
}

/// Asks the running instance for `key` to show its Settings window.
pub fn request_show_settings(key: &str) {
    unsafe {
        // Lets the running instance bring its window to the front.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        if let Err(error) = PostMessageW(
            Some(HWND_BROADCAST),
            show_settings_message(key),
            WPARAM(0),
            LPARAM(0),
        ) {
            log::warn!("cannot reach the running Dusk: {error}");
        }
    }
}

/// The window message that asks the instance for `key` to show Settings.
pub(crate) fn show_settings_message(key: &str) -> u32 {
    let name = wide_null(&format!("Dusk.ShowSettings.{}", sanitize(key)));
    unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
}

/// A windowless process has no console, so startup failures are shown.
pub fn show_startup_error(message: &str) {
    let text = wide_null(&format!("Dusk could not start.\n\n{message}"));
    let title = wide_null("Dusk");
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Kernel object names cannot contain backslashes after the namespace.
fn sanitize(key: &str) -> String {
    key.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_valid_object_names() {
        assert_eq!(sanitize(r"\\.\pipe\dusk-v0"), "--.-pipe-dusk-v0");
        assert_eq!(sanitize("dusk test 1"), "dusk-test-1");
    }

    #[test]
    fn a_key_can_be_claimed_once_until_released() {
        let key = format!(r"\\.\pipe\dusk-instance-test-{}", std::process::id());
        let first = claim_instance(&key).unwrap();
        assert!(matches!(first, Claim::First(_)));
        assert!(matches!(
            claim_instance(&key).unwrap(),
            Claim::AlreadyRunning
        ));
        drop(first);
        assert!(matches!(claim_instance(&key).unwrap(), Claim::First(_)));
    }

    #[test]
    fn each_key_has_its_own_show_settings_message() {
        let user = show_settings_message(r"\\.\pipe\dusk-v0");
        let test = show_settings_message(r"\\.\pipe\dusk-smoke-1");
        assert!(user >= 0xC000 && test >= 0xC000);
        assert_ne!(user, test);
        assert_eq!(user, show_settings_message(r"\\.\pipe\dusk-v0"));
    }
}
