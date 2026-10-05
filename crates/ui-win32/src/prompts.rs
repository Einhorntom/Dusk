//! Input-change confirmation and the "Keep this input?" revert prompt
//! (SPEC-IN), implementing the application's `InputChangePrompter` port with
//! message boxes.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dispcontrol_app::InputChangePrompter;
use dispcontrol_domain::Monitor;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, IDNO, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
    MessageBoxW, SendMessageW, WM_COMMAND,
};
use windows::core::PCWSTR;

use crate::wide_null;

/// Asks with message boxes; safe to call from any thread.
pub struct DesktopInputPrompter;

impl InputChangePrompter for DesktopInputPrompter {
    fn confirm_input_change(&self, monitor: &Monitor, from: u32, to: u32) -> bool {
        let text = wide_null(&format!(
            "Changing this input may disconnect devices connected to the monitor's USB hub or leave the display without a picture. The previous input might not be restorable if the monitor stops responding to this PC.\n\nMonitor: {}\nInput: {} -> {}\n\nContinue?",
            monitor.name, from, to
        ));
        let title = wide_null("Confirm monitor input change");
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING,
            ) == IDYES
        }
    }

    fn confirm_keep_input(
        &self,
        monitor: &Monitor,
        previous_input: u32,
        new_input: u32,
        timeout_seconds: u32,
    ) -> Result<bool, String> {
        let title = wide_null(&format!("Keep input - {}", monitor.name));
        let content = wide_null(&format!(
            "Input changed from {previous_input} to {new_input}.\n\nSelect Yes to keep it or No to restore the previous input. It will revert automatically in {timeout_seconds} seconds."
        ));
        let owner_thread = unsafe { GetCurrentThreadId() };
        let completed = Arc::new(Mutex::new(false));
        let timer_completed = completed.clone();
        let timer_title = title.clone();
        let timer = thread::Builder::new()
            .name("dispcontrol-input-revert".into())
            .spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(timeout_seconds.into());
                loop {
                    let Ok(done) = timer_completed.lock() else {
                        return;
                    };
                    if *done {
                        return;
                    }
                    drop(done);
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }

                loop {
                    let Ok(mut done) = timer_completed.lock() else {
                        return;
                    };
                    if *done {
                        return;
                    }
                    if let Ok(window) = unsafe { FindWindowW(None, PCWSTR(timer_title.as_ptr())) }
                        && unsafe { GetWindowThreadProcessId(window, None) } == owner_thread
                    {
                        unsafe {
                            let _ = SendMessageW(
                                window,
                                WM_COMMAND,
                                Some(WPARAM(IDNO.0 as usize)),
                                Some(LPARAM(0)),
                            );
                        }
                        *done = true;
                        return;
                    }
                    drop(done);
                    thread::sleep(Duration::from_millis(50));
                }
            })
            .map_err(|error| format!("could not start input revert timer: {error}"))?;
        let response = unsafe {
            MessageBoxW(
                None,
                PCWSTR(content.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        };
        {
            let mut done = completed
                .lock()
                .map_err(|_| "input revert timer state is unavailable".to_owned())?;
            *done = true;
        }
        timer
            .join()
            .map_err(|_| "input revert timer stopped unexpectedly".to_owned())?;
        Ok(response == IDYES)
    }
}
