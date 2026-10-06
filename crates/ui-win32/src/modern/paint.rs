//! The scrolling content window and owner-drawn painting (cards, toggles,
//! the navigation).

use super::*;

pub(super) unsafe extern "system" fn content_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetParent};
    crate::remember_shared(window, message, lparam);
    // SAFETY: the content window is created with its parent's `Shared` pointer.
    let Some(shared) = (unsafe { crate::Shared::of(window) }) else {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    match message {
        // Handled by the main window, which borrows the context itself.
        WM_COMMAND | WM_HSCROLL | WM_DRAWITEM => {
            let parent = unsafe { GetParent(window) }.unwrap_or_default();
            send(parent, message, wparam, lparam)
        }
        WM_ERASEBKGND | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_VSCROLL
        | WM_MOUSEWHEEL => {
            crate::dispatch(
                shared,
                window,
                message,
                wparam,
                lparam,
                |context| match message {
                    WM_ERASEBKGND => {
                        paint_content(context, window, HDC(wparam.0 as *mut c_void));
                        Some(LRESULT(1))
                    }
                    WM_VSCROLL => {
                        on_vscroll(context, wparam);
                        Some(LRESULT(0))
                    }
                    WM_MOUSEWHEEL => {
                        let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
                        let step = context.modern.px(60) * delta / 120;
                        let target = context.modern.scroll_y - step;
                        scroll_to(context, target);
                        Some(LRESULT(0))
                    }
                    _ => ctl_color(
                        context,
                        HDC(wparam.0 as *mut c_void),
                        HWND(lparam.0 as *mut c_void),
                    ),
                },
            )
        }
        WM_SIZE => LRESULT(0),
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

pub(super) fn ctl_color(context: &WindowContext, hdc: HDC, child: HWND) -> Option<LRESULT> {
    color_snapshot(context)?.content_color(hdc, child)
}

/// What text and background colours depend on, copied out of the context
/// so a control that repaints while the context is borrowed (see lib.rs)
/// still gets the right colours.
#[derive(Clone)]
pub(crate) struct ColorSnapshot {
    theme: Theme,
    brushes: Brushes,
    on_card: Vec<HWND>,
    muted: Vec<HWND>,
    main: HWND,
    status: HWND,
}

pub(crate) fn color_snapshot(context: &WindowContext) -> Option<ColorSnapshot> {
    Some(ColorSnapshot {
        theme: context.modern.theme,
        brushes: context.modern.brushes?,
        on_card: context.modern.on_card.clone(),
        muted: context.modern.muted.clone(),
        main: context.window,
        status: context.status,
    })
}

impl ColorSnapshot {
    /// `WM_CTLCOLOR*` for a child of `window`.
    pub(crate) fn color(&self, window: HWND, hdc: HDC, child: HWND) -> Option<LRESULT> {
        if window == self.main {
            self.main_color(hdc, child)
        } else {
            self.content_color(hdc, child)
        }
    }

    fn content_color(&self, hdc: HDC, child: HWND) -> Option<LRESULT> {
        let theme = self.theme;
        let (background, brush) = if self.on_card.contains(&child) {
            (theme.card, self.brushes.card)
        } else {
            (theme.bg, self.brushes.bg)
        };
        let muted = self.muted.contains(&child);
        unsafe {
            SetBkColor(hdc, COLORREF(background));
            SetTextColor(hdc, COLORREF(if muted { theme.muted } else { theme.text }));
        }
        Some(LRESULT(brush.0 as isize))
    }

    fn main_color(&self, hdc: HDC, child: HWND) -> Option<LRESULT> {
        let theme = self.theme;
        unsafe {
            SetBkColor(hdc, COLORREF(theme.bg));
            SetTextColor(
                hdc,
                COLORREF(if child == self.status {
                    theme.text
                } else {
                    theme.muted
                }),
            );
        }
        Some(LRESULT(self.brushes.bg.0 as isize))
    }
}

pub(super) fn round_rect(
    hdc: HDC,
    rect: &RECT,
    radius: i32,
    fill: Option<u32>,
    border: Option<u32>,
) {
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

pub(super) fn paint_content(context: &WindowContext, window: HWND, hdc: HDC) {
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

    let control = Control::from_id(id);
    if let Some(Control::Nav(index)) = control
        && index < PAGES.len()
    {
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
    } else if let Some(
        toggle @ (Control::ConfirmToggle
        | Control::OsdToggle
        | Control::LogToggle
        | Control::AutostartToggle),
    ) = control
    {
        let settings = current_settings(context);
        let on = match toggle {
            Control::OsdToggle => settings.show_osd,
            Control::LogToggle => settings.diagnostic_log,
            Control::AutostartToggle => context.modern.starts_with_windows,
            _ => settings.confirm_input_change,
        };
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

pub(super) fn draw_text(
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
