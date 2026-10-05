use std::ffi::c_void;
use std::sync::Arc;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows::Win32::UI::Controls::{TBM_SETPOS, TBM_SETRANGE, TBS_AUTOTICKS, TRACKBAR_CLASSW};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DestroyMenu, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_NUMBER, GetCursorPos,
    GetMessageW, GetWindowTextW, HMENU, IDI_APPLICATION, IsDialogMessageW, KillTimer, LoadIconW,
    MF_STRING, MSG, PostQuitMessage, RegisterClassW, SC_MINIMIZE, SW_HIDE, SW_SHOW,
    SendMessageW as send_message_raw, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DRAWITEM, WM_ERASEBKGND, WM_HSCROLL, WM_NCCREATE, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND,
    WM_TIMER, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{BOOL, PCWSTR, w};

use dispcontrol_app::Api;
use dispcontrol_domain::{AppSettings, ControlReading, ControlValue};
use dispcontrol_ui_model::MonitorSettingsModel;

const WINDOW_CLASS: PCWSTR = w!("DispcontrolSettingsWindow");
const MONITOR_ID: u16 = 100;
const CONTROL_ID: u16 = 101;
const VALUE_ID: u16 = 102;
const SLIDER_ID: u16 = 103;
const APPLY_ID: u16 = 104;
const REFRESH_ID: u16 = 105;
const DEBOUNCE_ID: u16 = 106;
const CONFIRM_ID: u16 = 107;
const REVERT_ID: u16 = 108;
const SAVE_SETTINGS_ID: u16 = 110;
const STATUS_ID: u16 = 111;
const ENUM_VALUE_ID: u16 = 112;
const SLIDER_TIMER: usize = 1;
const SETTINGS_TIMER: usize = 2;
const TBM_GETPOS: u32 = 0x0400;
const TRAY_MESSAGE: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const MENU_SETTINGS: usize = 200;
const MENU_QUIT: usize = 201;

mod hotkeys;
mod keys;
mod modern;
mod prompts;

pub use prompts::DesktopInputPrompter;

const WM_HOTKEY: u32 = 0x0312;

struct WindowContext {
    model: MonitorSettingsModel,
    native_ui: bool,
    modern: modern::State,
    monitor_combo: HWND,
    control_combo: HWND,
    value_edit: HWND,
    slider: HWND,
    enum_combo: HWND,
    debounce_edit: HWND,
    confirm_check: HWND,
    revert_edit: HWND,
    status: HWND,
    window: HWND,
    tray_icon: Option<NOTIFYICONDATAW>,
    hotkeys: hotkeys::Registrar,
    hotkey_runner: hotkeys::Runner,
}

pub fn run(api: Arc<dyn Api>) -> Result<(), String> {
    run_with_native_ui(api, false)
}

pub fn run_with_native_ui(api: Arc<dyn Api>, native_ui: bool) -> Result<(), String> {
    run_with_options(api, native_ui, false)
}

/// Runs the tray app. With `start_hidden`, the Settings window stays hidden
/// until opened from the tray (e.g. when started with Windows).
pub fn run_with_options(
    api: Arc<dyn Api>,
    native_ui: bool,
    start_hidden: bool,
) -> Result<(), String> {
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map_err(|error| error.to_string())?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("RegisterClassW failed".into());
    }
    if !native_ui {
        modern::register_content_class(HINSTANCE(instance.0))?;
    }
    let (window_width, window_height) = if native_ui {
        (760, 640)
    } else {
        modern::initial_window_size()
    };

    let mut context = Box::new(WindowContext {
        model: MonitorSettingsModel::new(api),
        native_ui,
        modern: modern::State::new(),
        monitor_combo: HWND::default(),
        control_combo: HWND::default(),
        value_edit: HWND::default(),
        slider: HWND::default(),
        enum_combo: HWND::default(),
        debounce_edit: HWND::default(),
        confirm_check: HWND::default(),
        revert_edit: HWND::default(),
        status: HWND::default(),
        window: HWND::default(),
        tray_icon: None,
        hotkeys: hotkeys::Registrar::default(),
        hotkey_runner: hotkeys::Runner::default(),
    });
    let context_ptr = (&mut *context) as *mut WindowContext;
    let window = unsafe {
        CreateWindowExW(
            Default::default(),
            WINDOW_CLASS,
            w!("dispcontrol — Settings"),
            if start_hidden {
                WS_OVERLAPPEDWINDOW
            } else {
                WS_OVERLAPPEDWINDOW | WS_VISIBLE
            },
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            window_width,
            window_height,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            Some(context_ptr.cast::<c_void>()),
        )
    }
    .map_err(|error| format!("CreateWindowExW failed: {error}"))?;
    context.window = window;
    unsafe {
        if native_ui {
            let dark_mode = BOOL(1);
            let _ = DwmSetWindowAttribute(
                window,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark_mode as *const BOOL).cast(),
                std::mem::size_of::<BOOL>() as u32,
            );
        }
        if !start_hidden || context.tray_icon.is_none() {
            // Without a tray icon the window is the only way back in.
            let _ = ShowWindow(window, SW_SHOW);
        }
    }

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe {
            if !native_ui && IsDialogMessageW(window, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe {
            &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW)
        };
        unsafe {
            SetWindowLongPtrW(
                window,
                windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                create.lpCreateParams as isize,
            );
        }
    }
    let context_pointer = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            window,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
        ) as *mut WindowContext
    };

    if message == WM_CREATE && !context_pointer.is_null() {
        let context = unsafe { &mut *context_pointer };
        context.window = window;
        if let Err(error) = create_controls(context) {
            eprintln!("could not create Settings controls: {error}");
            unsafe { PostQuitMessage(1) };
        } else {
            // Without a taskbar (e.g. Explorer not running) the app still
            // works; the window stays visible instead.
            if let Err(error) = add_tray_icon(context) {
                eprintln!("warning: could not add tray icon: {error}");
            }
            refresh(context);
            hotkeys::register(context);
        }
        return LRESULT(0);
    }
    if !context_pointer.is_null() {
        let context = unsafe { &mut *context_pointer };
        if message == TRAY_MESSAGE {
            match lparam.0 as u32 {
                windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP
                | windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDBLCLK => {
                    show_settings(context);
                    return LRESULT(0);
                }
                windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP => {
                    show_tray_menu(context);
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        if message == WM_HOTKEY {
            hotkeys::on_hotkey(context, wparam.0 as i32);
            return LRESULT(0);
        }
        if message == hotkeys::FINISHED_MESSAGE {
            hotkeys::on_hotkey_finished(context, lparam);
            return LRESULT(0);
        }
        if message == hotkeys::RECORDING_MESSAGE {
            hotkeys::on_recording(context, wparam.0 != 0);
            return LRESULT(0);
        }
        if message == WM_CLOSE {
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
            return LRESULT(0);
        }
        if message == WM_SYSCOMMAND && (wparam.0 as u32 & 0xfff0) == SC_MINIMIZE {
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
            return LRESULT(0);
        }
        if message == WM_COMMAND {
            if context.native_ui {
                handle_command(context, wparam, lparam);
            } else {
                modern::handle_command(context, wparam);
            }
            return LRESULT(0);
        }
        if !context.native_ui {
            match message {
                WM_SIZE => {
                    modern::on_size(context);
                    return LRESULT(0);
                }
                WM_ERASEBKGND => {
                    modern::paint_main_background(
                        context,
                        windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void),
                    );
                    return LRESULT(1);
                }
                WM_CTLCOLORSTATIC => {
                    if let Some(result) = modern::main_ctl_color(
                        context,
                        windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void),
                        HWND(lparam.0 as *mut c_void),
                    ) {
                        return result;
                    }
                }
                WM_DRAWITEM => {
                    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
                    modern::draw_item(context, item);
                    return LRESULT(1);
                }
                WM_SETTINGCHANGE => {
                    modern::on_theme_change(context);
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        if message == WM_HSCROLL {
            if !context.native_ui {
                modern::on_hscroll(context, HWND(lparam.0 as *mut c_void));
                return LRESULT(0);
            }
            let position =
                unsafe { SendMessageW(context.slider, TBM_GETPOS, WPARAM(0), LPARAM(0)).0 as u32 };
            set_text(context.value_edit, &position.to_string());
            if let Err(error) = context
                .model
                .adjust_selected_value(ControlValue::Normalized(position))
            {
                set_status(context, &error.to_string());
                return LRESULT(0);
            }
            unsafe {
                let _ = KillTimer(Some(window), SLIDER_TIMER);
                if SetTimer(
                    Some(window),
                    SLIDER_TIMER,
                    context.model.settings().debounce_ms,
                    None,
                ) == 0
                {
                    set_status(
                        context,
                        "Could not start the slider debounce timer; no write was sent.",
                    );
                }
            }
            return LRESULT(0);
        }
        if message == WM_TIMER && wparam.0 == SLIDER_TIMER {
            unsafe {
                let _ = KillTimer(Some(window), SLIDER_TIMER);
            }
            commit_slider(context);
            return LRESULT(0);
        }
        if message == WM_TIMER && wparam.0 == SETTINGS_TIMER {
            modern::save_pending_settings(context);
            return LRESULT(0);
        }
    }
    if message == WM_DESTROY {
        if !context_pointer.is_null() {
            let context = unsafe { &mut *context_pointer };
            context.hotkeys.unregister_all(window);
            if let Some(icon) = context.tray_icon.take() {
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_DELETE, &icon);
                }
            }
        }
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(window, message, wparam, lparam)
    }
}

fn create_controls(context: &mut WindowContext) -> Result<(), String> {
    if context.native_ui {
        return create_native_controls(context);
    }
    modern::create_controls(context)
}

fn create_native_controls(context: &mut WindowContext) -> Result<(), String> {
    context.monitor_combo = control(
        context.window,
        w!("COMBOBOX"),
        MONITOR_ID,
        160,
        34,
        540,
        CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32,
    )?;
    context.control_combo = control(
        context.window,
        w!("COMBOBOX"),
        CONTROL_ID,
        160,
        80,
        260,
        CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32,
    )?;
    context.value_edit = control(
        context.window,
        w!("EDIT"),
        VALUE_ID,
        440,
        80,
        100,
        ES_AUTOHSCROLL | ES_NUMBER | WS_TABSTOP.0 as i32,
    )?;
    context.enum_combo = control(
        context.window,
        w!("COMBOBOX"),
        ENUM_VALUE_ID,
        440,
        80,
        200,
        CBS_DROPDOWNLIST | WS_TABSTOP.0 as i32,
    )?;
    context.slider = control(
        context.window,
        TRACKBAR_CLASSW,
        SLIDER_ID,
        160,
        122,
        500,
        TBS_AUTOTICKS as i32 | WS_TABSTOP.0 as i32,
    )?;
    context.debounce_edit = control(
        context.window,
        w!("EDIT"),
        DEBOUNCE_ID,
        210,
        330,
        100,
        ES_AUTOHSCROLL | ES_NUMBER | WS_TABSTOP.0 as i32,
    )?;
    context.confirm_check = control(
        context.window,
        w!("BUTTON"),
        CONFIRM_ID,
        24,
        382,
        400,
        BS_AUTOCHECKBOX | WS_TABSTOP.0 as i32,
    )?;
    set_text(
        context.confirm_check,
        "Confirm input changes away from the active input",
    );
    context.revert_edit = control(
        context.window,
        w!("EDIT"),
        REVERT_ID,
        210,
        422,
        100,
        ES_AUTOHSCROLL | ES_NUMBER | WS_TABSTOP.0 as i32,
    )?;
    context.status = control(context.window, w!("STATIC"), STATUS_ID, 24, 530, 680, 0)?;

    label(context.window, "Monitor", 24, 36, 120, 28)?;
    label(context.window, "Control", 24, 82, 120, 28)?;
    label(context.window, "Value", 440, 58, 120, 20)?;
    label(
        context.window,
        "Adjust (quiet-period write)",
        24,
        124,
        250,
        24,
    )?;
    label(
        context.window,
        "Safety and write settings",
        24,
        290,
        360,
        30,
    )?;
    label(context.window, "Quiet period (ms)", 24, 332, 180, 28)?;
    label(
        context.window,
        "Revert timer (sec; 0 disables)",
        24,
        422,
        180,
        28,
    )?;
    button(context.window, REFRESH_ID, "Refresh", 560, 78, 90, 32)?;
    button(context.window, APPLY_ID, "Apply value", 560, 118, 120, 34)?;
    button(
        context.window,
        SAVE_SETTINGS_ID,
        "Save settings",
        24,
        492,
        140,
        34,
    )?;

    unsafe {
        SendMessageW(
            context.slider,
            TBM_SETRANGE,
            WPARAM(1),
            LPARAM(100isize << 16),
        );
        SendMessageW(context.confirm_check, BM_SETCHECK, WPARAM(1), LPARAM(0));
    }
    set_text(context.revert_edit, "10");
    Ok(())
}

fn add_tray_icon(context: &mut WindowContext) -> Result<(), String> {
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }
        .map_err(|error| format!("LoadIconW failed: {error}"))?;
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: context.window,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY_MESSAGE,
        hIcon: icon,
        ..Default::default()
    };
    let tooltip: Vec<u16> = "dispcontrol - Monitor Settings"
        .encode_utf16()
        .take(data.szTip.len() - 1)
        .collect();
    data.szTip[..tooltip.len()].copy_from_slice(&tooltip);
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        return Err("Shell_NotifyIconW(NIM_ADD) failed".into());
    }
    context.tray_icon = Some(data);
    Ok(())
}

/// Shows a tray notification, for problems the user must see even while the
/// Settings window is hidden (SPEC-HK-5).
fn notify_tray(context: &WindowContext, title: &str, text: &str) {
    let Some(mut data) = context.tray_icon else {
        return;
    };
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_WARNING;
    let title: Vec<u16> = title
        .encode_utf16()
        .take(data.szInfoTitle.len() - 1)
        .collect();
    data.szInfoTitle[..title.len()].copy_from_slice(&title);
    let text: Vec<u16> = text.encode_utf16().take(data.szInfo.len() - 1).collect();
    data.szInfo[..text.len()].copy_from_slice(&text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Redraws the visible view from the model after a change made elsewhere.
fn refresh_view(context: &mut WindowContext) {
    if context.native_ui {
        update_control_view(context);
    } else {
        modern::redraw(context);
    }
}

fn show_settings(context: &WindowContext) {
    unsafe {
        let _ = ShowWindow(context.window, SW_SHOW);
        let _ = SetForegroundWindow(context.window);
    }
}

fn show_tray_menu(context: &WindowContext) {
    let menu = match unsafe { CreatePopupMenu() } {
        Ok(menu) => menu,
        Err(error) => {
            set_status(context, &format!("Could not open tray menu: {error}"));
            return;
        }
    };
    let settings_label: Vec<u16> = "Open Settings\0".encode_utf16().collect();
    let quit_label: Vec<u16> = "Quit\0".encode_utf16().collect();
    let mut point = POINT::default();
    let menu_result = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            MENU_SETTINGS,
            PCWSTR(settings_label.as_ptr()),
        )
        .and_then(|()| AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit_label.as_ptr())))
        .and_then(|()| GetCursorPos(&mut point))
    };
    if let Err(error) = menu_result {
        unsafe {
            let _ = DestroyMenu(menu);
        }
        set_status(context, &format!("Could not build tray menu: {error}"));
        return;
    }

    unsafe {
        let _ = SetForegroundWindow(context.window);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            context.window,
            None,
        );
        let _ = DestroyMenu(menu);
        match selected.0 as usize {
            MENU_SETTINGS => show_settings(context),
            MENU_QUIT => {
                if let Err(error) = DestroyWindow(context.window) {
                    set_status(context, &format!("Could not quit: {error}"));
                }
            }
            _ => {}
        }
    }
}

fn control(
    parent: HWND,
    class: PCWSTR,
    id: u16,
    x: i32,
    y: i32,
    width: i32,
    style: i32,
) -> Result<HWND, String> {
    let height = if id == SLIDER_ID || id == STATUS_ID {
        42
    } else {
        30
    };
    unsafe {
        CreateWindowExW(
            Default::default(),
            class,
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style as u32),
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as usize as *mut c_void)),
            None,
            None,
        )
        .map_err(|error| format!("creating control {id}: {error}"))
    }
}

fn label(
    parent: HWND,
    text: &'static str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    let label_text = text;
    let text = wide_null(label_text);
    unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            x,
            y,
            width,
            height,
            Some(parent),
            None,
            None,
            None,
        )
        .map(|_| ())
        .map_err(|error| format!("creating label {label_text}: {error}"))
    }
}

fn button(
    parent: HWND,
    id: u16,
    text: &'static str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    let button_text = text;
    let text = wide_null(button_text);
    unsafe {
        CreateWindowExW(
            Default::default(),
            w!("BUTTON"),
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as usize as *mut c_void)),
            None,
            None,
        )
        .map(|_| ())
        .map_err(|error| format!("creating button {button_text}: {error}"))
    }
}

fn handle_command(context: &mut WindowContext, wparam: WPARAM, _lparam: LPARAM) {
    let id = (wparam.0 & 0xffff) as u16;
    let notification = ((wparam.0 >> 16) & 0xffff) as u16;
    if id == REFRESH_ID {
        refresh(context);
    } else if id == MONITOR_ID && u32::from(notification) == CBN_SELCHANGE {
        let index =
            unsafe { SendMessageW(context.monitor_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 };
        if index >= 0
            && let Some(monitor_id) = context
                .model
                .monitors()
                .get(index as usize)
                .map(|monitor| monitor.id.clone())
        {
            if let Err(error) = context.model.select_monitor(&monitor_id) {
                set_status(context, &error.to_string());
            } else {
                populate_controls(context);
                update_control_view(context);
            }
        }
    } else if id == CONTROL_ID && u32::from(notification) == CBN_SELCHANGE {
        let index =
            unsafe { SendMessageW(context.control_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 };
        if index >= 0
            && let Some(reading) = context.model.controls().get(index as usize)
        {
            context.model.select_control(reading.capability.key);
            update_control_view(context);
        }
    } else if id == APPLY_ID {
        apply_value(context);
    } else if id == SAVE_SETTINGS_ID {
        save_settings(context);
    } else if id == ENUM_VALUE_ID && u32::from(notification) == CBN_SELCHANGE {
        apply_value(context);
    }
}

fn refresh(context: &mut WindowContext) {
    if !context.native_ui {
        modern::refresh(context);
        return;
    }
    match context.model.refresh() {
        Ok(()) => {
            populate_monitors(context);
            populate_controls(context);
            load_settings(context);
            update_control_view(context);
            set_status(
                context,
                "Ready. Slider changes are written after the quiet period.",
            );
        }
        Err(error) => set_status(context, &error.to_string()),
    }
}

fn populate_monitors(context: &WindowContext) {
    unsafe {
        SendMessageW(
            context.monitor_combo,
            windows::Win32::UI::WindowsAndMessaging::CB_RESETCONTENT,
            WPARAM(0),
            LPARAM(0),
        );
    }
    for monitor in context.model.monitors() {
        add_combo_item(
            context.monitor_combo,
            &format!("{} ({})", monitor.name, monitor.id),
        );
    }
    if !context.model.monitors().is_empty() {
        let selected = context
            .model
            .monitors()
            .iter()
            .position(|monitor| Some(&monitor.id) == context.model.selected_monitor())
            .unwrap_or(0);
        unsafe {
            SendMessageW(
                context.monitor_combo,
                CB_SETCURSEL,
                WPARAM(selected),
                LPARAM(0),
            );
        }
    }
}

fn populate_controls(context: &WindowContext) {
    unsafe {
        SendMessageW(
            context.control_combo,
            windows::Win32::UI::WindowsAndMessaging::CB_RESETCONTENT,
            WPARAM(0),
            LPARAM(0),
        );
    }
    for reading in context.model.controls() {
        add_combo_item(context.control_combo, reading.capability.key.as_str());
    }
    if let Some(selected) = context.model.selected_control()
        && let Some(index) = context
            .model
            .controls()
            .iter()
            .position(|reading| reading.capability.key == selected)
    {
        unsafe {
            SendMessageW(
                context.control_combo,
                CB_SETCURSEL,
                WPARAM(index),
                LPARAM(0),
            );
        }
    }
}

fn update_control_view(context: &WindowContext) {
    let Some(reading) = selected_reading(context) else {
        unsafe {
            let _ = ShowWindow(
                context.slider,
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
            let _ = ShowWindow(
                context.value_edit,
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
            let _ = ShowWindow(
                context.enum_combo,
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }
        return;
    };
    match reading.value {
        ControlValue::Normalized(value) => {
            unsafe {
                SendMessageW(
                    context.slider,
                    TBM_SETPOS,
                    WPARAM(1),
                    LPARAM(value as isize),
                );
                let _ = ShowWindow(context.slider, SW_SHOW);
                let _ = ShowWindow(
                    context.value_edit,
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
                let _ = ShowWindow(
                    context.enum_combo,
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
            }
            set_text(context.value_edit, &value.to_string());
        }
        ControlValue::Enum(value) => {
            unsafe {
                let _ = ShowWindow(
                    context.slider,
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
                let _ = ShowWindow(
                    context.value_edit,
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
                let _ = ShowWindow(context.enum_combo, SW_SHOW);
            }
            unsafe {
                SendMessageW(
                    context.enum_combo,
                    windows::Win32::UI::WindowsAndMessaging::CB_RESETCONTENT,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            for enum_value in &reading.capability.enum_values {
                add_combo_item(context.enum_combo, &format!("raw-{enum_value}"));
            }
            let selected = reading
                .capability
                .enum_values
                .iter()
                .position(|enum_value| *enum_value == value)
                .unwrap_or(0);
            unsafe {
                SendMessageW(
                    context.enum_combo,
                    CB_SETCURSEL,
                    WPARAM(selected),
                    LPARAM(0),
                );
            }
        }
    }
}

fn apply_value(context: &mut WindowContext) {
    let Some(reading) = selected_reading(context).cloned() else {
        set_status(context, "Select a monitor and a supported control first.");
        return;
    };
    let value = match reading.value {
        ControlValue::Normalized(_) => match read_number(context.value_edit) {
            Some(value) if value <= 100 => ControlValue::Normalized(value),
            _ => {
                set_status(context, "Numeric control value must be between 0 and 100.");
                return;
            }
        },
        ControlValue::Enum(_) => {
            let index =
                unsafe { SendMessageW(context.enum_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 };
            if index < 0 {
                set_status(context, "Choose an available enum value first.");
                return;
            }
            let Some(value) = reading.capability.enum_values.get(index as usize) else {
                set_status(context, "Selected enum value is no longer available.");
                return;
            };
            ControlValue::Enum(*value)
        }
    };
    match context.model.set_selected_value(value) {
        Ok(true) => {
            update_control_view(context);
            set_status(context, "Monitor setting applied.");
        }
        Ok(false) => set_status(context, "Value is unchanged; no monitor write was sent."),
        Err(error) => set_status(context, &error.to_string()),
    }
}

fn commit_slider(context: &mut WindowContext) {
    match context.model.flush_pending_adjustments() {
        Ok((committed, Some(next_wake))) => {
            if committed > 0 && context.native_ui {
                update_control_view(context);
            }
            let delay = next_wake.as_millis().clamp(1, u32::MAX as u128) as u32;
            if unsafe { SetTimer(Some(context.window), SLIDER_TIMER, delay, None) } == 0 {
                set_status(
                    context,
                    "A monitor change remains queued, but its rate-limit timer could not be started.",
                );
                return;
            }
            if committed > 0 {
                set_status(
                    context,
                    "Buffered monitor setting applied; another change is queued.",
                );
            } else {
                set_status(
                    context,
                    "Monitor setting queued until the write limit allows it.",
                );
            }
        }
        Ok((committed, None)) if committed > 0 => {
            if context.native_ui {
                update_control_view(context);
            }
            set_status(context, "Buffered monitor setting applied.");
        }
        Ok((_, None)) => set_status(context, "Value is unchanged; no monitor write was sent."),
        Err(error) => set_status(context, &error.to_string()),
    }
}

fn load_settings(context: &WindowContext) {
    let settings = context.model.settings();
    set_text(context.debounce_edit, &settings.debounce_ms.to_string());
    set_text(
        context.revert_edit,
        &settings.input_revert_seconds.to_string(),
    );
    unsafe {
        SendMessageW(
            context.confirm_check,
            BM_SETCHECK,
            WPARAM(usize::from(settings.confirm_input_change)),
            LPARAM(0),
        );
    }
}

fn save_settings(context: &mut WindowContext) {
    let Some(debounce_ms) = read_number(context.debounce_edit) else {
        set_status(context, "Quiet period must be a number.");
        return;
    };
    let confirm_input_change =
        unsafe { SendMessageW(context.confirm_check, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1 };
    let Some(input_revert_seconds) = read_number(context.revert_edit) else {
        set_status(context, "Revert timer must be a number.");
        return;
    };
    let settings = AppSettings {
        debounce_ms,
        confirm_input_change,
        input_revert_seconds,
        ..context.model.settings().clone()
    };
    match context.model.update_settings(settings) {
        Ok(()) => set_status(context, "Settings saved."),
        Err(error) => set_status(context, &error.to_string()),
    }
}

fn selected_reading(context: &WindowContext) -> Option<&ControlReading> {
    let control = context.model.selected_control()?;
    context
        .model
        .controls()
        .iter()
        .find(|reading| reading.capability.key == control)
}

fn add_combo_item(combo: HWND, text: &str) {
    let text = wide_null(text);
    unsafe {
        SendMessageW(
            combo,
            CB_ADDSTRING,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
        );
    }
}

fn set_status(context: &WindowContext, text: &str) {
    set_text(context.status, text);
}

fn set_text(window: HWND, text: &str) {
    let text = wide_null(text);
    if let Err(error) = unsafe { SetWindowTextW(window, PCWSTR(text.as_ptr())) } {
        eprintln!("could not update a Settings control: {error}");
    }
}

fn read_number(window: HWND) -> Option<u32> {
    let mut buffer = [0u16; 32];
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    String::from_utf16(&buffer[..length.max(0) as usize])
        .ok()?
        .parse()
        .ok()
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(non_snake_case)]
unsafe fn SendMessageW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { send_message_raw(hwnd, message, Some(wparam), Some(lparam)) }
}
