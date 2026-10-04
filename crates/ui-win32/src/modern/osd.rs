//! On-screen indicator (SPEC-HK-6): a small topmost window near the bottom of
//! the screen under the mouse, showing a control and its value for 1.5 s. It
//! never takes focus and lets clicks through.

use std::cell::RefCell;

use dispcontrol_ui_model::text::Indicator;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_LEFT, DT_RIGHT, DeleteObject, EndPaint, FillRect,
    GetMonitorInfoW, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, PAINTSTRUCT,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, HWND_TOPMOST, KillTimer, LWA_ALPHA,
    RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer,
    SetWindowPos, ShowWindow, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use super::{Fonts, Theme, create_fonts, draw_text, round_rect, system_uses_dark};

const HIDE_TIMER: usize = 1;
const VISIBLE_MS: u32 = 1500;

#[derive(Default)]
struct State {
    window: Option<HWND>,
    fonts: Option<Fonts>,
    indicator: Option<Indicator>,
}

thread_local! {
    // The indicator lives on the UI thread, like every other window.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn px(value: i32) -> i32 {
    value * unsafe { GetDpiForSystem() } as i32 / 96
}

/// Shows (or updates) the indicator and restarts its 1.5 s timer.
pub(crate) fn show(indicator: &Indicator) {
    let Some(window) = STATE.with_borrow_mut(|state| {
        state.indicator = Some(indicator.clone());
        if state.window.is_none() {
            state.window = create_window();
            state.fonts = Some(create_fonts(unsafe { GetDpiForSystem() } as i32));
        }
        state.window
    }) else {
        return;
    };
    let width = px(320);
    let height = px(if indicator.level.is_some() { 84 } else { 60 });
    let mut cursor = POINT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(monitor, &mut info);
        let work = info.rcWork;
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.bottom - height - px(56);
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(window), None, true);
        let _ = KillTimer(Some(window), HIDE_TIMER);
        SetTimer(Some(window), HIDE_TIMER, VISIBLE_MS, None);
    }
}

fn create_window() -> Option<HWND> {
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.ok()?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: w!("DispcontrolIndicator"),
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            w!("DispcontrolIndicator"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), 245, LWA_ALPHA);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of_val(&corner) as u32,
        );
        Some(window)
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER if wparam.0 == HIDE_TIMER => {
            unsafe {
                let _ = KillTimer(Some(window), HIDE_TIMER);
                let _ = ShowWindow(window, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint(window);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn paint(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(window, &mut paint) };
    STATE.with_borrow(|state| {
        let (Some(indicator), Some(fonts)) = (&state.indicator, state.fonts) else {
            return;
        };
        let theme = if system_uses_dark() {
            Theme::DARK
        } else {
            Theme::LIGHT
        };
        let mut client = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut client);
            let background = CreateSolidBrush(COLORREF(theme.card));
            FillRect(hdc, &client, background);
            let _ = DeleteObject(HGDIOBJ(background.0));
        }
        let pad = px(18);
        let text_row = RECT {
            left: client.left + pad,
            top: client.top + px(12),
            right: client.right - pad,
            bottom: client.top + px(48),
        };
        draw_text(
            hdc,
            &indicator.title,
            text_row,
            fonts.section,
            theme.text,
            DT_LEFT,
        );
        draw_text(
            hdc,
            &indicator.value,
            text_row,
            fonts.section,
            theme.text,
            DT_RIGHT,
        );
        if let Some(level) = indicator.level {
            let track = RECT {
                left: client.left + pad,
                top: client.bottom - px(28),
                right: client.right - pad,
                bottom: client.bottom - px(22),
            };
            let radius = track.bottom - track.top;
            round_rect(hdc, &track, radius, Some(theme.track), Some(theme.track));
            let filled = RECT {
                right: track.left + (track.right - track.left) * level.min(100) as i32 / 100,
                ..track
            };
            if filled.right > filled.left {
                round_rect(hdc, &filled, radius, Some(theme.accent), Some(theme.accent));
            }
        }
    });
    unsafe {
        let _ = EndPaint(window, &paint);
    }
}
