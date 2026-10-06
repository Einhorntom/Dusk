//! Default Settings window: sidebar, cards and light/dark theme modeled on `mockups/settings.html`.

use std::ffi::c_void;

use dusk_domain::{AppSettings, ControlKey, ControlReading, ControlValue};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FillRect, GetDC,
    GetStockObject, GetTextExtentPoint32W, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, LineTo,
    MapWindowPoints, MoveToEx, NULL_BRUSH, PS_SOLID, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE,
    RedrawWindow, ReleaseDC, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, SetScrollInfo, SetWindowTheme, TBM_SETPOS,
    TBM_SETRANGE, TRACKBAR_CLASSW,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CreateWindowExW, DestroyWindow, GetClientRect, GetDlgCtrlID, GetScrollInfo, GetWindowRect,
    GetWindowTextW, HMENU, IDC_ARROW, KillTimer, LoadCursorW, MoveWindow, RegisterClassW, SB_VERT,
    SCROLLINFO, SIF_ALL, SIF_PAGE, SIF_POS, SIF_RANGE, SW_ERASE, SW_HIDE, SW_INVALIDATE,
    SW_SCROLLCHILDREN, SW_SHOW, ScrollWindowEx, SendMessageW as send_message_raw, SetTimer,
    ShowWindow, WINDOW_STYLE, WM_SETFONT, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_EX_COMPOSITED,
    WS_EX_CONTROLPARENT, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{BOOL, PCWSTR, w};

mod builder;
mod hotkeys_page;
mod ids;
mod monitors;
pub(crate) mod osd;
mod paint;
mod presets;
mod settings_pages;
use builder::*;
use hotkeys_page::*;
pub(crate) use monitors::*;
pub(crate) use paint::*;
use presets::*;
pub(crate) use settings_pages::*;

use dusk_domain::{HotkeyBinding, STEPPABLE_CONTROLS};
use dusk_ui_model::text::{
    apply_report_message, control_description, control_title, enum_label, exchange_path,
    hotkey_action_label, hotkey_description, import_summary_message, parse_entry_value,
    preset_entry_description, snap_debounce_ms, snap_revert_seconds,
};
use ids::{Control, EntryButton, ROW_LIMIT, RowButton};

use super::{
    DEBOUNCE_ID, MONITOR_ID, REFRESH_ID, SETTINGS_TIMER, STATUS_ID, WindowContext, set_status,
    wide_null,
};

const CONTENT_CLASS: PCWSTR = w!("DuskSettingsContent");
const EM_SETLIMITTEXT: u32 = 0x00C5;
const SS_NOTIFY: u32 = 0x100;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const EN_KILLFOCUS: u32 = 0x0200;
const WM_GETDLGCODE: u32 = 0x0087;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const EM_SETSEL: u32 = 0x00B1;
const TBM_GETRANGEMIN: u32 = 0x0401;
const TBM_GETRANGEMAX: u32 = 0x0402;
const SS_NOPREFIX: u32 = 0x80;
const SS_RIGHT: u32 = 0x02;
const TBS_NOTICKS: u32 = 0x10;
const TBM_GETPOS: u32 = 0x0400;
const WM_VSCROLL: u32 = 0x0115;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_COMMAND: u32 = 0x0111;
const WM_HSCROLL: u32 = 0x0114;
const WM_DRAWITEM: u32 = 0x002B;
const WM_SIZE: u32 = 0x0005;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Monitors,
    Presets,
    Hotkeys,
    Safety,
    General,
}

const PAGES: [(Page, &str, &str); 5] = [
    (Page::Monitors, "\u{E7F4}", "Monitors"),
    (Page::Presets, "\u{E8FD}", "Presets"),
    (Page::Hotkeys, "\u{E765}", "Hotkeys"),
    (Page::Safety, "\u{EA18}", "Safety & writes"),
    (Page::General, "\u{E713}", "General"),
];

const CONTROL_ORDER: [ControlKey; 9] = [
    ControlKey::Brightness,
    ControlKey::Contrast,
    ControlKey::Volume,
    ControlKey::Input,
    ControlKey::ColorPreset,
    ControlKey::GainRed,
    ControlKey::GainGreen,
    ControlKey::GainBlue,
    ControlKey::Power,
];

#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
    bg: u32,
    card: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
    accent_text: u32,
    ctl: u32,
    ctl_border: u32,
    hover: u32,
    sel: u32,
    track: u32,
}

const fn rgb(value: u32) -> u32 {
    ((value & 0xff) << 16) | (value & 0xff00) | ((value >> 16) & 0xff)
}

impl Theme {
    const LIGHT: Theme = Theme {
        dark: false,
        bg: rgb(0xf3f3f3),
        card: rgb(0xfbfbfb),
        border: rgb(0xe5e5e5),
        text: rgb(0x1b1b1b),
        muted: rgb(0x616161),
        accent: rgb(0x0067c0),
        accent_text: rgb(0xffffff),
        ctl: rgb(0xfdfdfd),
        ctl_border: rgb(0xd0d0d0),
        hover: rgb(0xebebeb),
        sel: rgb(0xe6e6e6),
        track: rgb(0x8a8a8a),
    };
    const DARK: Theme = Theme {
        dark: true,
        bg: rgb(0x202020),
        card: rgb(0x2b2b2b),
        border: rgb(0x3a3a3a),
        text: rgb(0xf3f3f3),
        muted: rgb(0xb0b0b0),
        accent: rgb(0x60cdff),
        accent_text: rgb(0x000000),
        ctl: rgb(0x333333),
        ctl_border: rgb(0x4a4a4a),
        hover: rgb(0x333333),
        sel: rgb(0x383838),
        track: rgb(0x9a9a9a),
    };
}

#[derive(Clone, Copy)]
struct Brushes {
    bg: HBRUSH,
    card: HBRUSH,
}

#[derive(Clone, Copy)]
struct Fonts {
    body: HFONT,
    title: HFONT,
    section: HFONT,
    small: HFONT,
    icon: HFONT,
}

#[derive(Clone, Copy)]
enum FontKind {
    Body,
    Title,
    Section,
    Small,
}

struct Card {
    rect: RECT,
    separators: Vec<i32>,
}

struct ValueEdit {
    edit: HWND,
    label: HWND,
    slider: HWND,
}

struct RowControl {
    key: ControlKey,
    slider: HWND,
    value: HWND,
    combo: HWND,
}

pub(crate) struct State {
    theme: Theme,
    brushes: Option<Brushes>,
    fonts: Option<Fonts>,
    dpi: i32,
    page: Page,
    content: HWND,
    nav: Vec<HWND>,
    hint: HWND,
    children: Vec<HWND>,
    muted: Vec<HWND>,
    editing: Option<ValueEdit>,
    on_card: Vec<HWND>,
    cards: Vec<Card>,
    chips: Vec<(RECT, String)>,
    rows: Vec<RowControl>,
    delay_value: HWND,
    revert_value: HWND,
    step_values: [HWND; 3],
    pending_settings: Option<AppSettings>,
    preset_name: String,
    editing_preset: Option<String>,
    /// Whether the Run entry exists, read when the General page is built.
    starts_with_windows: bool,
    scroll_y: i32,
    total_height: i32,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            theme: Theme::LIGHT,
            brushes: None,
            fonts: None,
            dpi: 96,
            page: Page::Monitors,
            content: HWND::default(),
            nav: Vec::new(),
            hint: HWND::default(),
            children: Vec::new(),
            muted: Vec::new(),
            editing: None,
            on_card: Vec::new(),
            cards: Vec::new(),
            chips: Vec::new(),
            rows: Vec::new(),
            delay_value: HWND::default(),
            revert_value: HWND::default(),
            step_values: [HWND::default(); 3],
            pending_settings: None,
            preset_name: String::new(),
            editing_preset: None,
            starts_with_windows: false,
            scroll_y: 0,
            total_height: 0,
        }
    }

    fn px(&self, value: i32) -> i32 {
        value * self.dpi / 96
    }
}

fn send(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { send_message_raw(window, message, Some(wparam), Some(lparam)) }
}

pub(crate) fn system_uses_dark() -> bool {
    let mut data: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast::<c_void>()),
            Some(&mut size),
        )
    };
    result.is_ok() && data == 0
}

pub(crate) fn register_content_class(instance: HINSTANCE) -> Result<(), String> {
    let class = WNDCLASSW {
        lpfnWndProc: Some(content_proc),
        hInstance: instance,
        lpszClassName: CONTENT_CLASS,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("RegisterClassW failed for the content panel".into());
    }
    Ok(())
}

pub(crate) fn initial_window_size() -> (i32, i32) {
    let scale = unsafe { GetDpiForSystem() } as i32;
    (960 * scale / 96, 720 * scale / 96)
}

fn apply_theme(context: &mut WindowContext) {
    let theme = if system_uses_dark() {
        Theme::DARK
    } else {
        Theme::LIGHT
    };
    context.modern.theme = theme;
    if let Some(old) = context.modern.brushes.take() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(old.bg.0));
            let _ = DeleteObject(HGDIOBJ(old.card.0));
        }
    }
    context.modern.brushes = Some(Brushes {
        bg: unsafe { CreateSolidBrush(COLORREF(theme.bg)) },
        card: unsafe { CreateSolidBrush(COLORREF(theme.card)) },
    });
    let dark = BOOL::from(theme.dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            context.window,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

fn create_fonts(dpi: i32) -> Fonts {
    let make = |height: i32, weight: i32, face: PCWSTR| unsafe {
        CreateFontW(
            -(height * dpi / 96),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
            windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY,
            0,
            face,
        )
    };
    Fonts {
        body: make(14, 400, w!("Segoe UI")),
        title: make(26, 600, w!("Segoe UI")),
        section: make(14, 600, w!("Segoe UI")),
        small: make(12, 400, w!("Segoe UI")),
        icon: make(16, 400, w!("Segoe MDL2 Assets")),
    }
}

pub(crate) fn create_controls(context: &mut WindowContext) -> Result<(), String> {
    context.modern.dpi = unsafe { GetDpiForSystem() } as i32;
    context.modern.fonts = Some(create_fonts(context.modern.dpi));
    apply_theme(context);

    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map_err(|error| error.to_string())?;
    // The content window shares the main window's `Shared` (see lib.rs).
    let shared_ptr = context.shared.cast::<c_void>();
    let content = unsafe {
        CreateWindowExW(
            // Double-buffered: a rebuilt page appears at once, not control by control.
            WS_EX_CONTROLPARENT | WS_EX_COMPOSITED,
            CONTENT_CLASS,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_VSCROLL,
            0,
            0,
            10,
            10,
            Some(context.window),
            None,
            Some(HINSTANCE(instance.0)),
            Some(shared_ptr),
        )
    }
    .map_err(|error| format!("creating content panel: {error}"))?;
    context.modern.content = content;

    let fonts = context.modern.fonts.expect("fonts were just created");
    for (index, (_, _, title)) in PAGES.iter().enumerate() {
        let text = wide_null(title);
        let nav = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("BUTTON"),
                PCWSTR(text.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                10,
                10,
                Some(context.window),
                Some(HMENU(Control::Nav(index).id() as usize as *mut c_void)),
                None,
                None,
            )
        }
        .map_err(|error| format!("creating navigation item: {error}"))?;
        context.modern.nav.push(nav);
    }
    context.status = create_static(context.window, STATUS_ID, 0, fonts.small);
    context.modern.hint = create_static(context.window, 0, SS_RIGHT, fonts.small);
    set_window_text(
        context.modern.hint,
        "Close or minimize to send to the tray. Use Quit in the tray menu to exit.",
    );
    Ok(())
}

fn create_static(parent: HWND, id: u16, extra_style: u32, font: HFONT) -> HWND {
    let window = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_NOPREFIX | extra_style),
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as usize as *mut c_void)),
            None,
            None,
        )
    }
    .unwrap_or_default();
    send(window, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    window
}

fn set_window_text(window: HWND, text: &str) {
    let text = wide_null(text);
    unsafe {
        let _ =
            windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(window, PCWSTR(text.as_ptr()));
    }
}

pub(crate) fn on_size(context: &mut WindowContext) {
    if context.modern.content.0.is_null() {
        return;
    }
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(context.window, &mut client);
    }
    let state = &context.modern;
    let nav_width = state.px(230);
    let footer = state.px(36);
    unsafe {
        let _ = MoveWindow(
            state.content,
            nav_width,
            0,
            client.right - nav_width,
            client.bottom - footer,
            true,
        );
        for (index, nav) in state.nav.iter().enumerate() {
            let _ = MoveWindow(
                *nav,
                state.px(10),
                state.px(8) + index as i32 * state.px(42),
                nav_width - state.px(20),
                state.px(40),
                true,
            );
        }
        let top = client.bottom - footer + state.px(9);
        let half = client.right / 2;
        let _ = MoveWindow(
            context.status,
            state.px(28),
            top,
            half - state.px(28),
            state.px(20),
            true,
        );
        let _ = MoveWindow(
            state.hint,
            half,
            top,
            client.right - half - state.px(28),
            state.px(20),
            true,
        );
    }
    rebuild(context);
}

pub(crate) fn on_theme_change(context: &mut WindowContext) {
    let new_dark = system_uses_dark();
    if new_dark == context.modern.theme.dark || context.modern.content.0.is_null() {
        return;
    }
    apply_theme(context);
    for nav in &context.modern.nav {
        unsafe {
            let _ = InvalidateRect(Some(*nav), None, true);
        }
    }
    unsafe {
        let _ = InvalidateRect(Some(context.window), None, true);
    }
    rebuild(context);
}

pub(crate) fn paint_main_background(context: &WindowContext, hdc: HDC) {
    let state = &context.modern;
    let Some(brushes) = state.brushes else {
        return;
    };
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(context.window, &mut client);
        FillRect(hdc, &client, brushes.bg);
        let pen = CreatePen(PS_SOLID, 1, COLORREF(state.theme.border));
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        let y = client.bottom - state.px(36);
        let _ = MoveToEx(hdc, 0, y, None);
        let _ = LineTo(hdc, client.right, y);
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

pub(crate) fn main_ctl_color(context: &WindowContext, hdc: HDC, child: HWND) -> Option<LRESULT> {
    color_snapshot(context)?.color(context.window, hdc, child)
}

fn selected_page_index(state: &State) -> usize {
    PAGES
        .iter()
        .position(|(page, _, _)| *page == state.page)
        .unwrap_or(0)
}

pub(crate) fn refresh(context: &mut WindowContext) {
    set_status(context, "Reading monitors...");
    let selected = context.model.selected_monitor().cloned();
    crate::tasks::run(
        context,
        move |api| dusk_ui_model::fetch_overview(api, selected),
        |context, overview| {
            match overview {
                Ok(overview) => {
                    context.model.apply_overview(overview);
                    context.modern.pending_settings = None;
                    set_status(
                        context,
                        "Ready. Changes are sent to the monitor after you stop adjusting.",
                    );
                }
                Err(error) => set_status(context, &error.to_string()),
            }
            rebuild(context);
        },
    );
}

// ---------------------------------------------------------------- scrolling

fn update_scrollbar(context: &WindowContext) {
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: context.modern.total_height.max(1) - 1,
        nPage: client_height(context.modern.content).max(0) as u32,
        nPos: context.modern.scroll_y,
        nTrackPos: 0,
    };
    unsafe {
        SetScrollInfo(context.modern.content, SB_VERT, &info, true);
    }
}

fn scroll_to(context: &mut WindowContext, target: i32) {
    let max_scroll = (context.modern.total_height - client_height(context.modern.content)).max(0);
    let target = target.clamp(0, max_scroll);
    let delta = context.modern.scroll_y - target;
    if delta == 0 {
        return;
    }
    context.modern.scroll_y = target;
    unsafe {
        let _ = ScrollWindowEx(
            context.modern.content,
            0,
            delta,
            None,
            None,
            None,
            None,
            SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
        );
    }
    update_scrollbar(context);
}

fn on_vscroll(context: &mut WindowContext, wparam: WPARAM) {
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        ..Default::default()
    };
    unsafe {
        let _ = GetScrollInfo(context.modern.content, SB_VERT, &mut info);
    }
    let line = context.modern.px(40);
    let page = info.nPage as i32;
    let target = match (wparam.0 & 0xffff) as u32 {
        0 => context.modern.scroll_y - line,
        1 => context.modern.scroll_y + line,
        2 => context.modern.scroll_y - page,
        3 => context.modern.scroll_y + page,
        5 | 4 => info.nTrackPos,
        6 => 0,
        7 => i32::MAX,
        _ => return,
    };
    scroll_to(context, target);
}

// ---------------------------------------------------------------- commands

pub(crate) fn handle_command(context: &mut WindowContext, wparam: WPARAM) {
    let id = (wparam.0 & 0xffff) as u16;
    let notification = ((wparam.0 >> 16) & 0xffff) as u32;
    if id == REFRESH_ID {
        refresh(context);
        return;
    }
    if id == MONITOR_ID {
        if notification == CBN_SELCHANGE {
            select_monitor(context);
        }
        return;
    }
    let Some(control) = Control::from_id(id) else {
        return;
    };
    let clicked = notification == 0;
    match control {
        Control::Nav(index) if index < PAGES.len() => {
            let page = PAGES[index].0;
            if page != context.modern.page {
                context.modern.page = page;
                context.modern.scroll_y = 0;
                if matches!(page, Page::Presets | Page::Hotkeys) {
                    let _ = context.model.reload_presets();
                    refresh_matching_preset(context);
                }
                if page == Page::Hotkeys {
                    let _ = context.model.refresh_hotkeys();
                }
                rebuild(context);
            }
        }
        Control::PresetSave if clicked => save_preset(context, false),
        Control::PresetSaveInput if clicked => save_preset(context, true),
        Control::PresetBack
        | Control::PresetExport
        | Control::PresetImport
        | Control::PresetFolder
            if clicked =>
        {
            preset_file_action(context, control)
        }
        Control::Entry(button, index) if clicked => preset_entry_action(context, button, index),
        Control::PresetRow(button, index) if clicked => preset_row_action(context, button, index),
        Control::ValueLabel(_) if clicked => begin_value_edit(context, id),
        Control::InlineEdit if notification == EN_KILLFOCUS => finish_value_edit(context),
        Control::Combo(_) if notification == CBN_SELCHANGE => apply_enum(context, id),
        Control::HotkeyRemove(index) if clicked => remove_hotkey(context, index),
        Control::HotkeyAdd if clicked => add_hotkey(context),
        Control::AutostartToggle => {
            let wanted = !context.modern.starts_with_windows;
            match crate::autostart::set_enabled(wanted) {
                Ok(()) => {
                    context.modern.starts_with_windows = wanted;
                    set_status(
                        context,
                        if wanted {
                            "Dusk will start in the tray when you sign in."
                        } else {
                            "Dusk will no longer start when you sign in."
                        },
                    );
                }
                Err(error) => set_status(context, &error),
            }
            if let Some(window) = context.modern.children.iter().find(|window| {
                let id = unsafe { GetDlgCtrlID(**window) };
                id == i32::from(control.id())
            }) {
                unsafe {
                    let _ = InvalidateRect(Some(*window), None, true);
                }
            }
        }
        Control::ConfirmToggle | Control::OsdToggle | Control::LogToggle => {
            let mut settings = current_settings(context);
            match control {
                Control::OsdToggle => settings.show_osd = !settings.show_osd,
                Control::LogToggle => settings.diagnostic_log = !settings.diagnostic_log,
                _ => settings.confirm_input_change = !settings.confirm_input_change,
            }
            queue_settings_save(context, settings);
            if let Some(window) = context.modern.children.iter().find(|window| {
                let id = unsafe { GetDlgCtrlID(**window) };
                id == i32::from(control.id())
            }) {
                unsafe {
                    let _ = InvalidateRect(Some(*window), None, true);
                }
            }
        }
        _ => {}
    }
}

fn read_child_text(context: &WindowContext, control: Control) -> Option<String> {
    let window = context
        .modern
        .children
        .iter()
        .copied()
        .find(|window| unsafe { GetDlgCtrlID(*window) } == i32::from(control.id()))?;
    let mut buffer = [0u16; 128];
    let length = unsafe { GetWindowTextW(window, &mut buffer) }.max(0) as usize;
    Some(String::from_utf16_lossy(&buffer[..length]))
}

fn combo_selection(context: &WindowContext, control: Control) -> Option<usize> {
    let combo = context
        .modern
        .children
        .iter()
        .copied()
        .find(|window| unsafe { GetDlgCtrlID(*window) } == i32::from(control.id()))?;
    usize::try_from(send(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0).ok()
}

/// Rebuilds the current page from the model without re-reading monitors.
pub(crate) fn redraw(context: &mut WindowContext) {
    rebuild(context);
}
