//! Default Settings window: sidebar, cards and light/dark theme modeled on `mockups/settings.html`.

use std::ffi::c_void;

use dispcontrol_domain::{AppSettings, ControlKey, ControlReading, ControlValue};
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
    ShowWindow, WINDOW_STYLE, WM_SETFONT, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_CONTROLPARENT, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{BOOL, PCWSTR, w};

use super::{
    DEBOUNCE_ID, MONITOR_ID, REFRESH_ID, SETTINGS_TIMER, SLIDER_TIMER, STATUS_ID, WindowContext,
    set_status, wide_null,
};

const CONTENT_CLASS: PCWSTR = w!("DispcontrolSettingsContent");
const NAV_BASE_ID: u16 = 300;
const SLIDER_BASE_ID: u16 = 1000;
const VALUE_BASE_ID: u16 = 1100;
const COMBO_BASE_ID: u16 = 1200;
const REVERT_SLIDER_ID: u16 = 1501;
const CONFIRM_TOGGLE_ID: u16 = 2000;
const SS_NOTIFY: u32 = 0x100;
const EDIT_ID: u16 = 1900;
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
const WM_NCCREATE: u32 = 0x0081;
const WM_SIZE: u32 = 0x0005;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Monitors,
    Safety,
    General,
}

const PAGES: [(Page, &str, &str); 3] = [
    (Page::Monitors, "\u{E7F4}", "Monitors"),
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
    pending_settings: Option<AppSettings>,
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
            pending_settings: None,
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

fn system_uses_dark() -> bool {
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
    let context_ptr = (context as *mut WindowContext).cast::<c_void>();
    let content = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
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
            Some(context_ptr),
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
                Some(HMENU((NAV_BASE_ID as usize + index) as *mut c_void)),
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
    let brushes = context.modern.brushes?;
    let theme = context.modern.theme;
    unsafe {
        SetBkColor(hdc, COLORREF(theme.bg));
        SetTextColor(
            hdc,
            COLORREF(if child == context.status {
                theme.text
            } else {
                theme.muted
            }),
        );
    }
    Some(LRESULT(brushes.bg.0 as isize))
}

fn selected_page_index(state: &State) -> usize {
    PAGES
        .iter()
        .position(|(page, _, _)| *page == state.page)
        .unwrap_or(0)
}

pub(crate) fn refresh(context: &mut WindowContext) {
    match context.model.refresh() {
        Ok(()) => {
            context.modern.pending_settings = None;
            set_status(
                context,
                "Ready. Changes are sent to the monitor after you stop adjusting.",
            );
        }
        Err(error) => set_status(context, &error.to_string()),
    }
    rebuild(context);
}

fn current_settings(context: &WindowContext) -> AppSettings {
    context
        .modern
        .pending_settings
        .clone()
        .unwrap_or_else(|| context.model.settings().clone())
}

fn control_title(key: ControlKey) -> &'static str {
    match key {
        ControlKey::Brightness => "Brightness",
        ControlKey::Contrast => "Contrast",
        ControlKey::Volume => "Volume",
        ControlKey::Input => "Input source",
        ControlKey::ColorPreset => "Color preset",
        ControlKey::GainRed => "Red gain",
        ControlKey::GainGreen => "Green gain",
        ControlKey::GainBlue => "Blue gain",
        ControlKey::Power => "Power",
    }
}

fn control_description(key: ControlKey) -> Option<&'static str> {
    match key {
        ControlKey::Input => Some("Changing input asks for confirmation (see Safety & writes)."),
        ControlKey::Power => {
            Some("Sends a power mode to the display. Wake it with its own button.")
        }
        _ => None,
    }
}

fn enum_label(key: ControlKey, value: u32) -> String {
    let named = match (key, value) {
        (ControlKey::Input, 0x0F) => Some("DisplayPort 1"),
        (ControlKey::Input, 0x10) => Some("DisplayPort 2"),
        (ControlKey::Input, 0x11) => Some("HDMI 1"),
        (ControlKey::Input, 0x12) => Some("HDMI 2"),
        (ControlKey::Input, 0x1B) => Some("USB-C"),
        (ControlKey::Input, 0x31) => Some("USB-C"),
        (ControlKey::ColorPreset, 0x01) => Some("sRGB"),
        (ControlKey::ColorPreset, 0x02) => Some("Native"),
        (ControlKey::ColorPreset, 0x03) => Some("4000 K"),
        (ControlKey::ColorPreset, 0x04) => Some("5000 K"),
        (ControlKey::ColorPreset, 0x05) => Some("6500 K"),
        (ControlKey::ColorPreset, 0x06) => Some("7500 K"),
        (ControlKey::ColorPreset, 0x07) => Some("8200 K"),
        (ControlKey::ColorPreset, 0x08) => Some("9300 K"),
        (ControlKey::ColorPreset, 0x09) => Some("10000 K"),
        (ControlKey::ColorPreset, 0x0B) => Some("User 1"),
        (ControlKey::Power, 0x01) => Some("On"),
        (ControlKey::Power, 0x02) => Some("Standby"),
        (ControlKey::Power, 0x03) => Some("Suspend"),
        (ControlKey::Power, 0x04) => Some("Off (soft)"),
        (ControlKey::Power, 0x05) => Some("Off (hard)"),
        _ => None,
    };
    match (named, key) {
        (Some(name), ControlKey::Input) => format!("{name} (raw-0x{value:02X})"),
        (Some(name), _) => name.to_owned(),
        (None, _) => format!("raw-0x{value:02X}"),
    }
}

// ---------------------------------------------------------------- page building

struct Builder<'a> {
    context: &'a mut WindowContext,
    x: i32,
    width: i32,
    y: i32,
    card_top: Option<i32>,
    separators: Vec<i32>,
    first_row: bool,
}

impl Builder<'_> {
    fn px(&self, value: i32) -> i32 {
        self.context.modern.px(value)
    }

    fn fonts(&self) -> Fonts {
        self.context
            .modern
            .fonts
            .expect("fonts are created at startup")
    }

    fn font(&self, kind: FontKind) -> HFONT {
        let fonts = self.fonts();
        match kind {
            FontKind::Body => fonts.body,
            FontKind::Title => fonts.title,
            FontKind::Section => fonts.section,
            FontKind::Small => fonts.small,
        }
    }

    fn text_width(&self, kind: FontKind, text: &str) -> i32 {
        let font = self.font(kind);
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            let hdc = GetDC(Some(self.context.modern.content));
            let old = SelectObject(hdc, HGDIOBJ(font.0));
            let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
            SelectObject(hdc, old);
            ReleaseDC(Some(self.context.modern.content), hdc);
        }
        size.cx
    }

    fn create(
        &mut self,
        class: PCWSTR,
        text: &str,
        id: u16,
        style: u32,
        rect: (i32, i32, i32, i32),
        font: FontKind,
    ) -> HWND {
        let wide = wide_null(text);
        let (x, y, width, height) = rect;
        let window = unsafe {
            CreateWindowExW(
                Default::default(),
                class,
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                x,
                y - self.context.modern.scroll_y,
                width,
                height,
                Some(self.context.modern.content),
                Some(HMENU(id as usize as *mut c_void)),
                None,
                None,
            )
        }
        .unwrap_or_default();
        if self.card_top.is_some() {
            self.context.modern.on_card.push(window);
        }
        send(
            window,
            WM_SETFONT,
            WPARAM(self.font(font).0 as usize),
            LPARAM(1),
        );
        self.context.modern.children.push(window);
        window
    }

    fn label(&mut self, text: &str, rect: (i32, i32, i32, i32), font: FontKind, muted: bool) {
        let window = self.create(w!("STATIC"), text, 0, SS_NOPREFIX, rect, font);
        if muted {
            self.context.modern.muted.push(window);
        }
    }

    fn heading(&mut self, text: &str) {
        let rect = (self.x, self.y + self.px(4), self.width, self.px(38));
        self.label(text, rect, FontKind::Title, false);
        self.y += self.px(56);
    }

    fn section(&mut self, text: &str) {
        self.y += self.px(14);
        let rect = (self.x, self.y, self.width, self.px(22));
        self.label(text, rect, FontKind::Section, false);
        self.y += self.px(30);
    }

    fn card_begin(&mut self) {
        self.card_top = Some(self.y);
        self.separators.clear();
        self.first_row = true;
    }

    fn card_end(&mut self) {
        if let Some(top) = self.card_top.take() {
            let rect = RECT {
                left: self.x,
                top,
                right: self.x + self.width,
                bottom: self.y,
            };
            self.context.modern.cards.push(Card {
                rect,
                separators: std::mem::take(&mut self.separators),
            });
            self.y += self.px(6);
        }
    }

    /// Lays out a title/description pair and returns the row's (top, height).
    fn row(&mut self, title: &str, description: Option<&str>, control_width: i32) -> (i32, i32) {
        let top = self.y;
        if self.card_top.is_some() && !self.first_row {
            self.separators.push(top);
        }
        self.first_row = false;
        let label_width = self.width - self.px(48) - control_width;
        let lines = description.map_or(0, |text| {
            let text_width = self.text_width(FontKind::Small, text);
            (text_width / label_width.max(1) + 1).min(3)
        });
        let height = if lines == 0 {
            self.px(56)
        } else {
            self.px(30) + lines * self.px(16) + self.px(18)
        };
        let title_top = if lines == 0 {
            top + (height - self.px(20)) / 2
        } else {
            top + self.px(12)
        };
        self.label(
            title,
            (self.x + self.px(16), title_top, label_width, self.px(20)),
            FontKind::Body,
            false,
        );
        if let Some(text) = description {
            self.label(
                text,
                (
                    self.x + self.px(16),
                    title_top + self.px(22),
                    label_width,
                    lines * self.px(16),
                ),
                FontKind::Small,
                true,
            );
        }
        self.y += height;
        (top, height)
    }

    fn control_x(&self, control_width: i32) -> i32 {
        self.x + self.width - self.px(16) - control_width
    }

    #[allow(clippy::too_many_arguments)]
    fn slider_row(
        &mut self,
        title: &str,
        description: Option<&str>,
        id: u16,
        value_id: u16,
        range: (u32, u32),
        position: u32,
        value_text: &str,
    ) -> (HWND, HWND) {
        let value_width = self.px(64);
        let slider_width = self.px(200);
        let (top, height) = self.row(title, description, slider_width + value_width + self.px(16));
        let slider_x = self.control_x(slider_width + value_width + self.px(16));
        let slider = self.create(
            TRACKBAR_CLASSW,
            "",
            id,
            TBS_NOTICKS | WS_TABSTOP.0,
            (
                slider_x,
                top + (height - self.px(30)) / 2,
                slider_width,
                self.px(30),
            ),
            FontKind::Body,
        );
        send(
            slider,
            TBM_SETRANGE,
            WPARAM(1),
            LPARAM(((range.1 as isize) << 16) | range.0 as isize),
        );
        send(slider, TBM_SETPOS, WPARAM(1), LPARAM(position as isize));
        let value = self.create(
            w!("STATIC"),
            value_text,
            value_id,
            SS_NOPREFIX | SS_RIGHT | SS_NOTIFY,
            (
                self.control_x(value_width),
                top + (height - self.px(20)) / 2,
                value_width,
                self.px(20),
            ),
            FontKind::Body,
        );
        self.context.modern.muted.push(value);
        self.theme_control(slider, w!("DarkMode_Explorer"));
        (slider, value)
    }

    fn combo_row(
        &mut self,
        title: &str,
        description: Option<&str>,
        id: u16,
        items: &[String],
        selected: Option<usize>,
    ) -> HWND {
        let combo_width = self.px(280);
        let (top, height) = self.row(title, description, combo_width);
        let combo = self.create(
            w!("COMBOBOX"),
            "",
            id,
            (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32) as u32 | WS_TABSTOP.0,
            (
                self.control_x(combo_width),
                top + (height - self.px(30)) / 2,
                combo_width,
                self.px(300),
            ),
            FontKind::Body,
        );
        for item in items {
            let wide = wide_null(item);
            send(
                combo,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(wide.as_ptr() as isize),
            );
        }
        if let Some(index) = selected {
            send(combo, CB_SETCURSEL, WPARAM(index), LPARAM(0));
        }
        self.theme_control(combo, w!("DarkMode_CFD"));
        combo
    }

    fn theme_control(&self, window: HWND, dark_theme: PCWSTR) {
        if self.context.modern.theme.dark {
            unsafe {
                let _ = SetWindowTheme(window, dark_theme, PCWSTR::null());
            }
        }
    }

    fn button(&mut self, id: u16, text: &str, x: i32, y: i32, width: i32) {
        self.create(
            w!("BUTTON"),
            text,
            id,
            WS_TABSTOP.0 | BS_OWNERDRAW as u32,
            (x, y, width, self.px(32)),
            FontKind::Body,
        );
    }
}

fn rebuild(context: &mut WindowContext) {
    if context.modern.content.0.is_null() {
        return;
    }
    for _ in 0..2 {
        let before = client_width(context.modern.content);
        build_page(context, before);
        let max_scroll =
            (context.modern.total_height - client_height(context.modern.content)).max(0);
        context.modern.scroll_y = context.modern.scroll_y.min(max_scroll);
        update_scrollbar(context);
        if client_width(context.modern.content) == before {
            break;
        }
    }
    unsafe {
        let _ = RedrawWindow(
            Some(context.modern.content),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
        for nav in &context.modern.nav {
            let _ = InvalidateRect(Some(*nav), None, true);
        }
    }
}

fn client_width(window: HWND) -> i32 {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut rect);
    }
    rect.right
}

fn client_height(window: HWND) -> i32 {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut rect);
    }
    rect.bottom
}

fn build_page(context: &mut WindowContext, client_w: i32) {
    for child in context.modern.children.drain(..) {
        unsafe {
            let _ = DestroyWindow(child);
        }
    }
    context.modern.muted.clear();
    context.modern.editing = None;
    context.modern.on_card.clear();
    context.modern.cards.clear();
    context.modern.chips.clear();
    context.modern.rows.clear();
    context.modern.delay_value = HWND::default();
    context.modern.revert_value = HWND::default();

    let page = context.modern.page;
    let x = context.modern.px(18);
    let width = (client_w - x - context.modern.px(28)).max(context.modern.px(300));
    let mut builder = Builder {
        context,
        x,
        width,
        y: 0,
        card_top: None,
        separators: Vec::new(),
        first_row: true,
    };
    match page {
        Page::Monitors => build_monitors(&mut builder),
        Page::Safety => build_safety(&mut builder),
        Page::General => build_general(&mut builder),
    }
    let end = builder.y + builder.px(24);
    builder.context.modern.total_height = end;
}

fn build_monitors(builder: &mut Builder<'_>) {
    builder.heading("Monitors");
    let monitors: Vec<_> = builder.context.model.monitors().to_vec();
    if monitors.is_empty() {
        builder.card_begin();
        builder.row(
            "No controllable monitor found",
            Some("Check the cable and that DDC/CI is enabled in the monitor's on-screen menu."),
            builder.px(140),
        );
        builder.card_end();
        let y = builder.y + builder.px(8);
        let button_x = builder.x;
        builder.button(REFRESH_ID, "Refresh monitors", button_x, y, builder.px(150));
        builder.y = y + builder.px(40);
        return;
    }

    let selected_id = builder.context.model.selected_monitor().cloned();
    let selected_index = monitors
        .iter()
        .position(|monitor| Some(&monitor.id) == selected_id.as_ref());
    let names: Vec<String> = monitors
        .iter()
        .map(|monitor| format!("{} ({})", monitor.name, monitor.id))
        .collect();
    builder.combo_row("Selected monitor", None, MONITOR_ID, &names, selected_index);
    builder.y += builder.px(8);

    let selected = selected_index.and_then(|index| monitors.get(index));
    builder.card_begin();
    builder.row(
        selected.map_or("Monitor", |monitor| monitor.name.as_str()),
        selected.map(|monitor| match monitor.unstable_id {
            true => "ID may change when the monitor is reconnected",
            false => "Connected over DDC/CI",
        }),
        0,
    );
    let controls: Vec<ControlReading> = CONTROL_ORDER
        .iter()
        .filter_map(|key| {
            builder
                .context
                .model
                .controls()
                .iter()
                .find(|reading| reading.capability.key == *key)
                .cloned()
        })
        .collect();
    let caption = "Supported controls (discovered from the monitor)";
    let caption_rect = (
        builder.x + builder.px(16),
        builder.y - builder.px(2),
        builder.width - builder.px(32),
        builder.px(18),
    );
    builder.separators.push(builder.y);
    builder.y += builder.px(10);
    builder.label(
        caption,
        (caption_rect.0, builder.y, caption_rect.2, caption_rect.3),
        FontKind::Small,
        true,
    );
    builder.y += builder.px(24);
    let mut chip_x = builder.x + builder.px(16);
    let right_limit = builder.x + builder.width - builder.px(16);
    for reading in &controls {
        let text = control_title(reading.capability.key);
        let chip_width = builder.text_width(FontKind::Small, text) + builder.px(20);
        if chip_x + chip_width > right_limit {
            chip_x = builder.x + builder.px(16);
            builder.y += builder.px(30);
        }
        let rect = RECT {
            left: chip_x,
            top: builder.y,
            right: chip_x + chip_width,
            bottom: builder.y + builder.px(24),
        };
        builder.context.modern.chips.push((rect, text.to_owned()));
        chip_x += chip_width + builder.px(6);
    }
    builder.y += builder.px(40);
    builder.card_end();

    builder.section("Current settings");
    builder.card_begin();
    for (index, reading) in controls.iter().enumerate() {
        add_control_row(builder, index, reading);
    }
    if controls.is_empty() {
        builder.row("No supported controls were reported", None, 0);
    }
    builder.card_end();

    let y = builder.y + builder.px(8);
    let button_x = builder.x;
    builder.button(
        REFRESH_ID,
        "Re-read from monitor",
        button_x,
        y,
        builder.px(170),
    );
    let note = format!(
        "Changes are sent to the monitor {} ms after you stop adjusting.",
        current_settings(builder.context).debounce_ms
    );
    builder.label(
        &note,
        (
            button_x + builder.px(186),
            y + builder.px(6),
            builder.width - builder.px(186),
            builder.px(20),
        ),
        FontKind::Small,
        true,
    );
    builder.y = y + builder.px(40);
}

fn add_control_row(builder: &mut Builder<'_>, index: usize, reading: &ControlReading) {
    let key = reading.capability.key;
    let title = control_title(key);
    let description = control_description(key);
    match reading.value {
        ControlValue::Normalized(value) => {
            let (slider, value_label) = builder.slider_row(
                title,
                description,
                SLIDER_BASE_ID + index as u16,
                VALUE_BASE_ID + index as u16,
                (0, 100),
                value,
                &format!("{value}%"),
            );
            builder.context.modern.rows.push(RowControl {
                key,
                slider,
                value: value_label,
                combo: HWND::default(),
            });
        }
        ControlValue::Enum(value) => {
            let items: Vec<String> = reading
                .capability
                .enum_values
                .iter()
                .map(|candidate| enum_label(key, *candidate))
                .collect();
            let selected = reading
                .capability
                .enum_values
                .iter()
                .position(|candidate| *candidate == value);
            let combo = builder.combo_row(
                title,
                description,
                COMBO_BASE_ID + index as u16,
                &items,
                selected,
            );
            builder.context.modern.rows.push(RowControl {
                key,
                slider: HWND::default(),
                value: HWND::default(),
                combo,
            });
        }
    }
}

fn build_safety(builder: &mut Builder<'_>) {
    builder.heading("Safety & writes");
    let settings = current_settings(builder.context);

    builder.section("Input switching");
    builder.card_begin();
    let toggle_width = builder.px(44);
    let (top, height) = builder.row(
        "Confirm before changing input",
        Some(
            "Monitors that act as a USB hub disconnect keyboard and mouse when the input changes.",
        ),
        toggle_width,
    );
    builder.create(
        w!("BUTTON"),
        "",
        CONFIRM_TOGGLE_ID,
        WS_TABSTOP.0 | BS_OWNERDRAW as u32,
        (
            builder.control_x(toggle_width),
            top + (height - builder.px(24)) / 2,
            toggle_width,
            builder.px(24),
        ),
        FontKind::Body,
    );
    let (_, revert_value) = builder.slider_row(
        "Automatic revert timeout",
        Some("Restore the previous input unless you choose Keep. 0 disables."),
        REVERT_SLIDER_ID,
        REVERT_SLIDER_ID + 100,
        (0, 60),
        settings.input_revert_seconds,
        &format!("{} s", settings.input_revert_seconds),
    );
    builder.context.modern.revert_value = revert_value;
    builder.card_end();

    builder.section("Protecting monitor memory");
    builder.card_begin();
    let (_, delay_value) = builder.slider_row(
        "Write delay",
        Some("Changes are sent once, this long after you stop adjusting."),
        DEBOUNCE_ID,
        DEBOUNCE_ID + 100,
        (150, 2000),
        settings.debounce_ms,
        &format!("{} ms", settings.debounce_ms),
    );
    builder.context.modern.delay_value = delay_value;
    builder.card_end();
}

fn build_general(builder: &mut Builder<'_>) {
    builder.heading("General");
    builder.card_begin();
    builder.row(
        "Theme",
        Some("Follows your Windows app mode (light or dark)."),
        0,
    );
    builder.row("Settings", Some("Changes are saved automatically."), 0);
    builder.card_end();
    builder.section("About");
    builder.card_begin();
    builder.row(
        concat!("dispcontrol ", env!("CARGO_PKG_VERSION")),
        Some("Runs from the tray. Close or minimize hides the window; use Quit in the tray menu to exit."),
        0,
    );
    builder.card_end();
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

// ---------------------------------------------------------------- content window

unsafe extern "system" fn content_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CREATESTRUCTW, DefWindowProcW, GWLP_USERDATA, GetParent, GetWindowLongPtrW,
        SetWindowLongPtrW,
    };
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut WindowContext;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => {
            paint_content(unsafe { &*pointer }, window, HDC(wparam.0 as *mut c_void));
            LRESULT(1)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT => {
            let context = unsafe { &*pointer };
            ctl_color(
                context,
                HDC(wparam.0 as *mut c_void),
                HWND(lparam.0 as *mut c_void),
            )
            .unwrap_or_else(|| unsafe { DefWindowProcW(window, message, wparam, lparam) })
        }
        WM_COMMAND | WM_HSCROLL | WM_DRAWITEM => {
            let parent = unsafe { GetParent(window) }.unwrap_or_default();
            send(parent, message, wparam, lparam)
        }
        WM_VSCROLL => {
            on_vscroll(unsafe { &mut *pointer }, wparam);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let context = unsafe { &mut *pointer };
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            let step = context.modern.px(60) * delta / 120;
            let target = context.modern.scroll_y - step;
            scroll_to(context, target);
            LRESULT(0)
        }
        WM_SIZE => LRESULT(0),
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn ctl_color(context: &WindowContext, hdc: HDC, child: HWND) -> Option<LRESULT> {
    let brushes = context.modern.brushes?;
    let theme = context.modern.theme;
    let on_card = context.modern.on_card.contains(&child);
    let (background, brush) = if on_card {
        (theme.card, brushes.card)
    } else {
        (theme.bg, brushes.bg)
    };
    let muted = context.modern.muted.contains(&child);
    unsafe {
        SetBkColor(hdc, COLORREF(background));
        SetTextColor(hdc, COLORREF(if muted { theme.muted } else { theme.text }));
    }
    Some(LRESULT(brush.0 as isize))
}

fn round_rect(hdc: HDC, rect: &RECT, radius: i32, fill: Option<u32>, border: Option<u32>) {
    unsafe {
        let pen = match border {
            Some(color) => CreatePen(PS_SOLID, 1, COLORREF(color)),
            None => CreatePen(PS_SOLID, 0, COLORREF(fill.unwrap_or(0))),
        };
        let brush = match fill {
            Some(color) => CreateSolidBrush(COLORREF(color)),
            None => HBRUSH(GetStockObject(NULL_BRUSH).0),
        };
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        if fill.is_some() {
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
    }
}

fn paint_content(context: &WindowContext, window: HWND, hdc: HDC) {
    let state = &context.modern;
    let (Some(brushes), Some(fonts)) = (state.brushes, state.fonts) else {
        return;
    };
    let theme = state.theme;
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut client);
        FillRect(hdc, &client, brushes.bg);
    }
    let offset = state.scroll_y;
    for card in &state.cards {
        let rect = RECT {
            left: card.rect.left,
            top: card.rect.top - offset,
            right: card.rect.right,
            bottom: card.rect.bottom - offset,
        };
        round_rect(
            hdc,
            &rect,
            state.px(8),
            Some(theme.card),
            Some(theme.border),
        );
        unsafe {
            let pen = CreatePen(PS_SOLID, 1, COLORREF(theme.border));
            let old = SelectObject(hdc, HGDIOBJ(pen.0));
            for separator in &card.separators {
                let y = separator - offset;
                let _ = MoveToEx(hdc, rect.left + 1, y, None);
                let _ = LineTo(hdc, rect.right - 1, y);
            }
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }
    for (rect, text) in &state.chips {
        let mut shifted = RECT {
            left: rect.left,
            top: rect.top - offset,
            right: rect.right,
            bottom: rect.bottom - offset,
        };
        round_rect(hdc, &shifted, state.px(24), Some(theme.sel), None);
        unsafe {
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(theme.text));
            let old = SelectObject(hdc, HGDIOBJ(fonts.small.0));
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut wide,
                &mut shifted,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            SelectObject(hdc, old);
        }
    }
}

// ---------------------------------------------------------------- owner-drawn controls

pub(crate) fn draw_item(context: &WindowContext, item: &DRAWITEMSTRUCT) {
    let state = &context.modern;
    let (Some(_), Some(fonts)) = (state.brushes, state.fonts) else {
        return;
    };
    let theme = state.theme;
    let hdc = item.hDC;
    let rect = item.rcItem;
    let id = item.CtlID as u16;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;

    if (NAV_BASE_ID..NAV_BASE_ID + PAGES.len() as u16).contains(&id) {
        let index = (id - NAV_BASE_ID) as usize;
        let active = index == selected_page_index(state);
        unsafe {
            FillRect(
                hdc,
                &rect,
                match context.modern.brushes {
                    Some(brushes) => brushes.bg,
                    None => return,
                },
            );
        }
        if active || pressed {
            let inner = RECT {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            };
            round_rect(
                hdc,
                &inner,
                state.px(8),
                Some(if pressed && !active {
                    theme.hover
                } else {
                    theme.sel
                }),
                None,
            );
        }
        if active {
            let bar = RECT {
                left: rect.left,
                top: rect.top + state.px(10),
                right: rect.left + state.px(3),
                bottom: rect.bottom - state.px(10),
            };
            round_rect(hdc, &bar, state.px(3), Some(theme.accent), None);
        }
        let (_, glyph, title) = PAGES[index];
        draw_text(
            hdc,
            glyph,
            RECT {
                left: rect.left + state.px(12),
                ..rect
            },
            fonts.icon,
            theme.text,
            DT_LEFT,
        );
        draw_text(
            hdc,
            title,
            RECT {
                left: rect.left + state.px(44),
                ..rect
            },
            fonts.body,
            theme.text,
            DT_LEFT,
        );
    } else if id == CONFIRM_TOGGLE_ID {
        let on = current_settings(context).confirm_input_change;
        unsafe {
            FillRect(
                hdc,
                &rect,
                context.modern.brushes.map_or(HBRUSH::default(), |b| b.card),
            );
        }
        let track = RECT {
            left: rect.left + state.px(2),
            top: rect.top + state.px(2),
            right: rect.right - state.px(2),
            bottom: rect.bottom - state.px(2),
        };
        let radius = track.bottom - track.top;
        if on {
            round_rect(hdc, &track, radius, Some(theme.accent), Some(theme.accent));
        } else {
            round_rect(hdc, &track, radius, None, Some(theme.track));
        }
        let knob = state.px(12);
        let knob_x = if on {
            track.right - state.px(5) - knob
        } else {
            track.left + state.px(5)
        };
        let knob_y = track.top + (track.bottom - track.top - knob) / 2;
        let knob_rect = RECT {
            left: knob_x,
            top: knob_y,
            right: knob_x + knob,
            bottom: knob_y + knob,
        };
        let knob_color = if on { theme.accent_text } else { theme.track };
        round_rect(hdc, &knob_rect, knob, Some(knob_color), Some(knob_color));
        if focused {
            unsafe {
                let _ = DrawFocusRect(hdc, &rect);
            }
        }
    } else {
        unsafe {
            FillRect(
                hdc,
                &rect,
                context.modern.brushes.map_or(HBRUSH::default(), |b| b.bg),
            );
        }
        let background = if pressed { theme.hover } else { theme.ctl };
        round_rect(
            hdc,
            &rect,
            state.px(8),
            Some(background),
            Some(theme.ctl_border),
        );
        let mut text = [0u16; 128];
        let length = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(item.hwndItem, &mut text)
        } as usize;
        draw_text(
            hdc,
            &String::from_utf16_lossy(&text[..length]),
            rect,
            fonts.body,
            theme.text,
            DT_CENTER,
        );
        if focused {
            unsafe {
                let _ = DrawFocusRect(hdc, &rect);
            }
        }
    }
}

fn draw_text(
    hdc: HDC,
    text: &str,
    mut rect: RECT,
    font: HFONT,
    color: u32,
    align: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(color));
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        DrawTextW(
            hdc,
            &mut wide,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(hdc, old);
    }
}

// ---------------------------------------------------------------- commands

pub(crate) fn handle_command(context: &mut WindowContext, wparam: WPARAM) {
    let id = (wparam.0 & 0xffff) as u16;
    let notification = ((wparam.0 >> 16) & 0xffff) as u32;
    if (NAV_BASE_ID..NAV_BASE_ID + PAGES.len() as u16).contains(&id) {
        let page = PAGES[(id - NAV_BASE_ID) as usize].0;
        if page != context.modern.page {
            context.modern.page = page;
            context.modern.scroll_y = 0;
            rebuild(context);
        }
    } else if (VALUE_BASE_ID..VALUE_BASE_ID + 100).contains(&id) && notification == 0 {
        begin_value_edit(context, id);
    } else if id == EDIT_ID && notification == EN_KILLFOCUS {
        finish_value_edit(context);
    } else if id == REFRESH_ID {
        refresh(context);
    } else if id == MONITOR_ID && notification == CBN_SELCHANGE {
        select_monitor(context);
    } else if (COMBO_BASE_ID..COMBO_BASE_ID + 100).contains(&id) && notification == CBN_SELCHANGE {
        apply_enum(context, id);
    } else if id == CONFIRM_TOGGLE_ID {
        let mut settings = current_settings(context);
        settings.confirm_input_change = !settings.confirm_input_change;
        queue_settings_save(context, settings);
        if let Some(window) = context.modern.children.iter().find(|window| {
            let id = unsafe { GetDlgCtrlID(**window) };
            id == i32::from(CONFIRM_TOGGLE_ID)
        }) {
            unsafe {
                let _ = InvalidateRect(Some(*window), None, true);
            }
        }
    }
}

fn begin_value_edit(context: &mut WindowContext, id: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::Shell::SetWindowSubclass;
    use windows::Win32::UI::WindowsAndMessaging::{ES_AUTOHSCROLL, ES_NUMBER, ES_RIGHT, WS_BORDER};
    finish_value_edit(context);
    let Some(row) = context
        .modern
        .rows
        .iter()
        .find(|row| unsafe { GetDlgCtrlID(row.value) } == i32::from(id))
    else {
        return;
    };
    let (label, slider) = (row.value, row.slider);
    let content = context.modern.content;
    let mut rect = RECT::default();
    let mut points = [POINT::default(); 2];
    unsafe {
        let _ = GetWindowRect(label, &mut rect);
        points[0] = POINT {
            x: rect.left,
            y: rect.top,
        };
        points[1] = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        MapWindowPoints(None, Some(content), &mut points);
    }
    let pad = context.modern.px(4);
    let position = send(slider, TBM_GETPOS, WPARAM(0), LPARAM(0)).0;
    let text = wide_null(&position.to_string());
    let edit = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            PCWSTR(text.as_ptr()),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | WINDOW_STYLE((ES_NUMBER | ES_RIGHT | ES_AUTOHSCROLL) as u32),
            points[0].x,
            points[0].y - pad,
            points[1].x - points[0].x,
            points[1].y - points[0].y + 2 * pad,
            Some(content),
            Some(HMENU(EDIT_ID as usize as *mut c_void)),
            None,
            None,
        )
    }
    .unwrap_or_default();
    if edit.0.is_null() {
        return;
    }
    if let Some(fonts) = context.modern.fonts {
        send(edit, WM_SETFONT, WPARAM(fonts.body.0 as usize), LPARAM(1));
    }
    context.modern.children.push(edit);
    context.modern.on_card.push(edit);
    context.modern.editing = Some(ValueEdit {
        edit,
        label,
        slider,
    });
    unsafe {
        let _ = SetWindowSubclass(edit, Some(edit_subclass), 1, 0);
        let _ = ShowWindow(label, SW_HIDE);
        let _ = SetFocus(Some(edit));
    }
    send(edit, EM_SETSEL, WPARAM(0), LPARAM(-1));
}

fn finish_value_edit(context: &mut WindowContext) {
    let Some(editing) = context.modern.editing.take() else {
        return;
    };
    let mut buffer = [0u16; 16];
    let length = unsafe { GetWindowTextW(editing.edit, &mut buffer) }.max(0) as usize;
    let text = String::from_utf16_lossy(&buffer[..length]);
    context
        .modern
        .children
        .retain(|window| *window != editing.edit);
    unsafe {
        let _ = DestroyWindow(editing.edit);
        let _ = ShowWindow(editing.label, SW_SHOW);
    }
    if let Ok(value) = text.trim().parse::<u32>() {
        let min = send(editing.slider, TBM_GETRANGEMIN, WPARAM(0), LPARAM(0)).0 as u32;
        let max = send(editing.slider, TBM_GETRANGEMAX, WPARAM(0), LPARAM(0)).0 as u32;
        let value = value.clamp(min, max);
        send(
            editing.slider,
            TBM_SETPOS,
            WPARAM(1),
            LPARAM(value as isize),
        );
        on_hscroll(context, editing.slider);
    }
}

// Enter commits and Escape cancels by moving focus away from the edit box.
unsafe extern "system" fn edit_subclass(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::GetParent;
    const VK_RETURN: usize = 0x0D;
    const VK_ESCAPE: usize = 0x1B;
    match message {
        WM_GETDLGCODE => {
            let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
            LRESULT(result.0 | 0x4)
        }
        WM_KEYDOWN if wparam.0 == VK_RETURN || wparam.0 == VK_ESCAPE => {
            if wparam.0 == VK_ESCAPE {
                set_window_text(window, "");
            }
            if let Ok(parent) = unsafe { GetParent(window) } {
                unsafe {
                    let _ = SetFocus(Some(parent));
                }
            }
            LRESULT(0)
        }
        WM_CHAR if wparam.0 == VK_RETURN || wparam.0 == VK_ESCAPE => LRESULT(0),
        _ => unsafe { DefSubclassProc(window, message, wparam, lparam) },
    }
}

fn select_monitor(context: &mut WindowContext) {
    let Some(combo) = context
        .modern
        .children
        .iter()
        .copied()
        .find(|window| unsafe { GetDlgCtrlID(*window) } == i32::from(MONITOR_ID))
    else {
        return;
    };
    let index = send(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
    if index < 0 {
        return;
    }
    let Some(id) = context
        .model
        .monitors()
        .get(index as usize)
        .map(|monitor| monitor.id.clone())
    else {
        return;
    };
    match context.model.select_monitor(&id) {
        Ok(()) => rebuild(context),
        Err(error) => set_status(context, &error.to_string()),
    }
}

fn apply_enum(context: &mut WindowContext, id: u16) {
    let Some(row) = context
        .modern
        .rows
        .iter()
        .find(|row| unsafe { GetDlgCtrlID(row.combo) } == i32::from(id))
    else {
        return;
    };
    let (key, combo) = (row.key, row.combo);
    let index = send(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
    let value = context
        .model
        .controls()
        .iter()
        .find(|reading| reading.capability.key == key)
        .and_then(|reading| {
            reading
                .capability
                .enum_values
                .get(index.max(0) as usize)
                .copied()
        });
    let Some(value) = value.filter(|_| index >= 0) else {
        return;
    };
    context.model.select_control(key);
    let applied = match context.model.set_selected_value(ControlValue::Enum(value)) {
        Ok(true) => {
            set_status(context, "Monitor setting applied.");
            true
        }
        Ok(false) => {
            set_status(context, "Value is unchanged; no monitor write was sent.");
            true
        }
        Err(error) => {
            set_status(context, &error.to_string());
            false
        }
    };
    // Monitors often report the old value for a moment after a write, so a
    // successful write keeps the user's choice instead of snapping back.
    if applied {
        return;
    }
    let actual = context
        .model
        .controls()
        .iter()
        .find(|reading| reading.capability.key == key)
        .and_then(|reading| match reading.value {
            ControlValue::Enum(current) => reading
                .capability
                .enum_values
                .iter()
                .position(|candidate| *candidate == current),
            ControlValue::Normalized(_) => None,
        });
    if let Some(position) = actual {
        send(combo, CB_SETCURSEL, WPARAM(position), LPARAM(0));
    }
}

pub(crate) fn on_hscroll(context: &mut WindowContext, slider: HWND) {
    let id = unsafe { GetDlgCtrlID(slider) } as u16;
    let position = send(slider, TBM_GETPOS, WPARAM(0), LPARAM(0)).0 as u32;
    if (SLIDER_BASE_ID..SLIDER_BASE_ID + 100).contains(&id) {
        let Some(row) = context.modern.rows.iter().find(|row| row.slider == slider) else {
            return;
        };
        let (key, value_label) = (row.key, row.value);
        set_window_text(value_label, &format!("{position}%"));
        context.model.select_control(key);
        if let Err(error) = context
            .model
            .adjust_selected_value(ControlValue::Normalized(position))
        {
            set_status(context, &error.to_string());
            return;
        }
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
                    "Could not start the slider debounce timer; no write was sent.",
                );
            }
        }
    } else if id == REVERT_SLIDER_ID {
        let seconds = if (1..5).contains(&position) {
            5
        } else {
            position
        };
        if seconds != position {
            send(slider, TBM_SETPOS, WPARAM(1), LPARAM(seconds as isize));
        }
        set_window_text(context.modern.revert_value, &format!("{seconds} s"));
        let mut settings = current_settings(context);
        settings.input_revert_seconds = seconds;
        queue_settings_save(context, settings);
    } else if id == DEBOUNCE_ID {
        let milliseconds = ((position + 25) / 50 * 50).clamp(150, 2000);
        set_window_text(context.modern.delay_value, &format!("{milliseconds} ms"));
        let mut settings = current_settings(context);
        settings.debounce_ms = milliseconds;
        queue_settings_save(context, settings);
    }
}

fn queue_settings_save(context: &mut WindowContext, settings: AppSettings) {
    context.modern.pending_settings = Some(settings);
    unsafe {
        let _ = KillTimer(Some(context.window), SETTINGS_TIMER);
        if SetTimer(Some(context.window), SETTINGS_TIMER, 600, None) == 0 {
            set_status(context, "Could not schedule saving the settings.");
        }
    }
}

pub(crate) fn save_pending_settings(context: &mut WindowContext) {
    unsafe {
        let _ = KillTimer(Some(context.window), SETTINGS_TIMER);
    }
    let Some(settings) = context.modern.pending_settings.take() else {
        return;
    };
    match context.model.update_settings(settings) {
        Ok(()) => set_status(context, "Settings saved."),
        Err(error) => {
            set_status(context, &error.to_string());
            rebuild(context);
        }
    }
}
