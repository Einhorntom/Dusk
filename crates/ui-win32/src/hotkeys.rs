//! Global hotkey registration (SPEC-HK), `WM_HOTKEY` handling, and the
//! key-recording field used by the Hotkeys page. Humble: the behaviour lives
//! in `app::hotkeys` and `ui-model`.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use dusk_app::{Api, HotkeyOutcome};
use dusk_domain::{HotkeyBinding, KeyCombo};
use dusk_ui_model::text::{Indicator, hotkey_action_label, hotkey_indicator};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, IsWindowVisible, KillTimer, MSG, PostMessageW, SetTimer, SetWindowTextW,
    WM_APP, WM_CHAR, WM_GETDLGCODE, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFOCUS, WM_SYSCHAR,
    WM_SYSKEYDOWN,
};
use windows::core::{HRESULT, PCWSTR};

use super::{SLIDER_TIMER, WindowContext, set_status, wide_null};
use crate::keys;
use crate::modern::{self, osd};

/// Posted to the main window when the key-recording field gains (`wparam` 1)
/// or loses (0) focus, so registered hotkeys do not swallow the keys.
pub(crate) const RECORDING_MESSAGE: u32 = WM_APP + 2;
/// Posted by the hotkey worker with a boxed `Finished` in `lparam`.
pub(crate) const FINISHED_MESSAGE: u32 = WM_APP + 3;

const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

/// The combinations registered with Windows and the ones that failed.
#[derive(Default)]
pub(crate) struct Registrar {
    registered: Vec<(i32, KeyCombo)>,
    failures: Vec<(KeyCombo, String)>,
}

impl Registrar {
    /// Replaces all registrations with `bindings`; returns the new failures.
    pub(crate) fn register_all(
        &mut self,
        window: HWND,
        bindings: &[HotkeyBinding],
    ) -> &[(KeyCombo, String)] {
        self.unregister_all(window);
        self.failures.clear();
        for (index, binding) in bindings.iter().enumerate() {
            let id = index as i32 + 1;
            let flags = keys::modifier_flags(&binding.keys, binding.action.repeats());
            let code = u32::from(keys::virtual_key(binding.keys.key));
            match unsafe { RegisterHotKey(Some(window), id, HOT_KEY_MODIFIERS(flags), code) } {
                Ok(()) => self.registered.push((id, binding.keys)),
                Err(error) => {
                    let reason =
                        if error.code() == HRESULT::from_win32(ERROR_HOTKEY_ALREADY_REGISTERED) {
                            "the keys are used by another app or by Windows".to_owned()
                        } else {
                            error.message()
                        };
                    self.failures.push((binding.keys, reason));
                }
            }
        }
        &self.failures
    }

    pub(crate) fn unregister_all(&mut self, window: HWND) {
        for (id, _) in self.registered.drain(..) {
            unsafe {
                let _ = UnregisterHotKey(Some(window), id);
            }
        }
    }

    pub(crate) fn keys_for(&self, id: i32) -> Option<KeyCombo> {
        self.registered
            .iter()
            .find(|(registered, _)| *registered == id)
            .map(|(_, keys)| *keys)
    }

    pub(crate) fn failure(&self, keys: &KeyCombo) -> Option<&str> {
        self.failures
            .iter()
            .find(|(failed, _)| failed == keys)
            .map(|(_, reason)| reason.as_str())
    }
}

/// Registers the stored hotkeys and reports combinations that are taken
/// (SPEC-HK-5); the app keeps running either way.
pub(crate) fn register(context: &mut WindowContext) {
    if let Err(error) = context.model.refresh_hotkeys() {
        set_status(context, &format!("Could not load hotkeys: {error}"));
        return;
    }
    let bindings = context.model.hotkeys().to_vec();
    let failures = context
        .hotkeys
        .register_all(context.window, &bindings)
        .to_vec();
    if let Some((keys, reason)) = failures.first() {
        let action = bindings
            .iter()
            .find(|binding| binding.keys == *keys)
            .map(|binding| hotkey_action_label(&binding.action))
            .unwrap_or_default();
        let more = match failures.len() {
            1 => String::new(),
            count => format!(" ({} more on the Hotkeys page)", count - 1),
        };
        let message = format!("Hotkey {keys} ({action}) is not active: {reason}.{more}");
        set_status(context, &message);
        super::notify_tray(context, "Hotkey not available", &message);
    }
}

/// A hotkey run on the worker thread, sent back to the UI thread.
struct Finished {
    label: String,
    result: Result<HotkeyOutcome, String>,
}

/// Runs hotkey actions on one worker thread, in press order, so a slow or
/// hung monitor never blocks the window, tray or later key presses
/// (SPEC-MON-4). Results come back as `FINISHED_MESSAGE`.
#[derive(Default)]
pub(crate) struct Runner {
    jobs: Option<Sender<(KeyCombo, String)>>,
}

impl Runner {
    fn submit(&mut self, window: HWND, api: &Arc<dyn Api>, keys: KeyCombo, label: String) {
        let jobs = self
            .jobs
            .get_or_insert_with(|| start_worker(window, api.clone()));
        if let Err(mpsc::SendError(job)) = jobs.send((keys, label)) {
            // The worker stopped; start a new one for this and later presses.
            let jobs = self.jobs.insert(start_worker(window, api.clone()));
            let _ = jobs.send(job);
        }
    }
}

fn start_worker(window: HWND, api: Arc<dyn Api>) -> Sender<(KeyCombo, String)> {
    let (sender, jobs) = mpsc::channel::<(KeyCombo, String)>();
    // HWND is not Send; the worker only posts messages to it.
    let window = window.0 as isize;
    let spawned = std::thread::Builder::new()
        .name("dusk-hotkeys".into())
        .spawn(move || {
            while let Ok((keys, label)) = jobs.recv() {
                let result = api.run_hotkey(&keys).map_err(|error| error.to_string());
                let finished = Box::into_raw(Box::new(Finished { label, result }));
                let posted = unsafe {
                    PostMessageW(
                        Some(HWND(window as *mut c_void)),
                        FINISHED_MESSAGE,
                        WPARAM(0),
                        LPARAM(finished as isize),
                    )
                };
                if posted.is_err() {
                    // The window is gone; nobody will take the result.
                    drop(unsafe { Box::from_raw(finished) });
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("could not start the hotkey worker: {error}");
    }
    sender
}

pub(crate) fn on_hotkey(context: &mut WindowContext, id: i32) {
    let Some(keys) = context.hotkeys.keys_for(id) else {
        return;
    };
    let label = context
        .model
        .hotkeys()
        .iter()
        .find(|binding| binding.keys == keys)
        .map(|binding| hotkey_action_label(&binding.action))
        .unwrap_or_else(|| keys.to_string());
    let api = context.model.api();
    context
        .hotkey_runner
        .submit(context.window, &api, keys, label);
}

/// Shows a finished hotkey: updates values, starts the commit timer for
/// steps, and shows the on-screen indicator.
pub(crate) fn on_hotkey_finished(context: &mut WindowContext, lparam: LPARAM) {
    // SAFETY: only the hotkey worker posts this message, with a leaked Box.
    let finished = unsafe { Box::from_raw(lparam.0 as *mut Finished) };
    let show_indicator = context.model.settings().show_osd;
    match finished.result {
        Ok(outcome) => {
            context.model.apply_hotkey_outcome(&outcome);
            if let HotkeyOutcome::Stepped { control, .. } = &outcome {
                start_commit_timer(context);
                if !context.native_ui {
                    modern::show_control_value(context, *control);
                }
            } else if unsafe { IsWindowVisible(context.window) }.as_bool() {
                super::refresh_view(context);
            }
            if show_indicator && let Some(indicator) = hotkey_indicator(&outcome) {
                osd::show(&indicator);
            }
        }
        Err(error) => {
            set_status(context, &format!("{}: {error}", finished.label));
            if show_indicator {
                osd::show(&Indicator {
                    title: finished.label,
                    value: "Failed".into(),
                    level: None,
                });
            }
        }
    }
}

/// Step targets are written after the quiet period, like slider moves.
fn start_commit_timer(context: &WindowContext) {
    unsafe {
        let _ = KillTimer(Some(context.window), SLIDER_TIMER);
        if SetTimer(
            Some(context.window),
            SLIDER_TIMER,
            context.model.settings().debounce_ms,
            None,
        ) == 0
        {
            set_status(
                context,
                "Could not start the write timer; the hotkey change was not sent.",
            );
        }
    }
}

/// Pauses hotkeys while the user records a combination, so pressing an
/// existing one is captured instead of run.
pub(crate) fn on_recording(context: &mut WindowContext, recording: bool) {
    if recording {
        context.hotkeys.unregister_all(context.window);
    } else {
        let bindings = context.model.hotkeys().to_vec();
        context.hotkeys.register_all(context.window, &bindings);
    }
}

/// Subclass for the key-recording field: shows the pressed combination,
/// e.g. `Ctrl+Alt+Up`, instead of typing. Backspace or Escape clears it.
pub(crate) unsafe extern "system" fn recording_subclass(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    const DLGC_WANTALLKEYS: isize = 0x4;
    const VK_TAB: usize = 0x09;
    const VK_BACK: usize = 0x08;
    const VK_ESCAPE: usize = 0x1B;
    match message {
        WM_GETDLGCODE => {
            // Let Tab move focus; take every other key.
            let tab = lparam.0 != 0 && {
                let message = unsafe { &*(lparam.0 as *const MSG) };
                message.message == WM_KEYDOWN && message.wParam.0 == VK_TAB
            };
            if tab {
                unsafe { DefSubclassProc(window, message, wparam, lparam) }
            } else {
                LRESULT(DLGC_WANTALLKEYS)
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let held = |code: i32| unsafe { GetKeyState(code) } < 0;
            let (ctrl, alt, shift) = (held(0x11), held(0x12), held(0x10));
            let win = held(0x5B) || held(0x5C);
            let plain = !(ctrl || alt || shift || win);
            if plain && (wparam.0 == VK_BACK || wparam.0 == VK_ESCAPE) {
                set_text(window, "");
            } else if let Some(key) = keys::key_from_virtual(wparam.0 as u16) {
                let combo = KeyCombo {
                    ctrl,
                    alt,
                    shift,
                    win,
                    key,
                };
                set_text(window, &combo.to_string());
            }
            LRESULT(0)
        }
        WM_CHAR | WM_SYSCHAR => LRESULT(0),
        WM_SETFOCUS | WM_KILLFOCUS => {
            let recording = usize::from(message == WM_SETFOCUS);
            unsafe {
                let root = GetAncestor(window, GA_ROOT);
                let _ = PostMessageW(Some(root), RECORDING_MESSAGE, WPARAM(recording), LPARAM(0));
                DefSubclassProc(window, message, wparam, lparam)
            }
        }
        _ => unsafe { DefSubclassProc(window, message, wparam, lparam) },
    }
}

fn set_text(window: HWND, text: &str) {
    let wide = wide_null(text);
    unsafe {
        let _ = SetWindowTextW(window, PCWSTR(wide.as_ptr()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    };
    use windows::core::w;

    fn message_window() -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .expect("message-only window")
    }

    fn binding(keys: &str) -> HotkeyBinding {
        HotkeyBinding {
            keys: keys.parse().unwrap(),
            action: "preset-next".parse().unwrap(),
            monitor: None,
        }
    }

    /// OS integration check: registers real system-wide hotkeys, so it needs
    /// an interactive desktop and is run explicitly.
    #[test]
    #[ignore = "registers system-wide hotkeys; needs a desktop session; run with --ignored"]
    fn a_taken_combination_is_reported_and_the_others_still_register() {
        let (other_app, ours) = (message_window(), message_window());
        let taken = binding("Ctrl+Alt+Shift+F23");
        let free = binding("Ctrl+Alt+Shift+F24");

        let mut blocker = Registrar::default();
        blocker.register_all(other_app, std::slice::from_ref(&taken));
        assert_eq!(
            blocker.failure(&taken.keys),
            None,
            "this session cannot register hotkeys, or another app holds the test keys"
        );
        let mut registrar = Registrar::default();
        let failures = registrar
            .register_all(ours, &[taken.clone(), free.clone()])
            .to_vec();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].1.contains("another app"));
        assert_eq!(registrar.keys_for(1), None);
        assert_eq!(registrar.keys_for(2), Some(free.keys));

        blocker.unregister_all(other_app);
        registrar.register_all(ours, std::slice::from_ref(&taken));
        assert!(registrar.failure(&taken.keys).is_none());
        registrar.unregister_all(ours);
        unsafe {
            let _ = DestroyWindow(other_app);
            let _ = DestroyWindow(ours);
        }
    }
}
