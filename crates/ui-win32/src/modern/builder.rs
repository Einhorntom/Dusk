//! Building a Settings page: the `Builder` that lays out cards and rows, and
//! `rebuild`, which recreates the current page's controls.

use super::*;

pub(super) struct Builder<'a> {
    pub(super) context: &'a mut WindowContext,
    pub(super) x: i32,
    pub(super) width: i32,
    pub(super) y: i32,
    pub(super) card_top: Option<i32>,
    pub(super) separators: Vec<i32>,
    pub(super) first_row: bool,
}

impl Builder<'_> {
    pub(super) fn px(&self, value: i32) -> i32 {
        self.context.modern.px(value)
    }

    pub(super) fn fonts(&self) -> Fonts {
        self.context
            .modern
            .fonts
            .expect("fonts are created at startup")
    }

    pub(super) fn font(&self, kind: FontKind) -> HFONT {
        let fonts = self.fonts();
        match kind {
            FontKind::Body => fonts.body,
            FontKind::Title => fonts.title,
            FontKind::Section => fonts.section,
            FontKind::Small => fonts.small,
        }
    }

    pub(super) fn text_width(&self, kind: FontKind, text: &str) -> i32 {
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

    pub(super) fn create(
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

    pub(super) fn label(
        &mut self,
        text: &str,
        rect: (i32, i32, i32, i32),
        font: FontKind,
        muted: bool,
    ) {
        let window = self.create(w!("STATIC"), text, 0, SS_NOPREFIX, rect, font);
        if muted {
            self.context.modern.muted.push(window);
        }
    }

    pub(super) fn heading(&mut self, text: &str) {
        let rect = (self.x, self.y + self.px(4), self.width, self.px(38));
        self.label(text, rect, FontKind::Title, false);
        self.y += self.px(56);
    }

    pub(super) fn section(&mut self, text: &str) {
        self.y += self.px(14);
        let rect = (self.x, self.y, self.width, self.px(22));
        self.label(text, rect, FontKind::Section, false);
        self.y += self.px(30);
    }

    pub(super) fn card_begin(&mut self) {
        self.card_top = Some(self.y);
        self.separators.clear();
        self.first_row = true;
    }

    pub(super) fn card_end(&mut self) {
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
    pub(super) fn row(
        &mut self,
        title: &str,
        description: Option<&str>,
        control_width: i32,
    ) -> (i32, i32) {
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

    pub(super) fn control_x(&self, control_width: i32) -> i32 {
        self.x + self.width - self.px(16) - control_width
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn slider_row(
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

    pub(super) fn combo_row(
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

    pub(super) fn theme_control(&self, window: HWND, dark_theme: PCWSTR) {
        if self.context.modern.theme.dark {
            unsafe {
                let _ = SetWindowTheme(window, dark_theme, PCWSTR::null());
            }
        }
    }

    pub(super) fn button(&mut self, id: u16, text: &str, x: i32, y: i32, width: i32) {
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

pub(super) fn rebuild(context: &mut WindowContext) {
    if context.modern.content.0.is_null() {
        return;
    }
    const WM_SETREDRAW: u32 = 0x000B;
    send(context.modern.content, WM_SETREDRAW, WPARAM(0), LPARAM(0));
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
    send(context.modern.content, WM_SETREDRAW, WPARAM(1), LPARAM(0));
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

pub(super) fn client_width(window: HWND) -> i32 {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut rect);
    }
    rect.right
}

pub(super) fn client_height(window: HWND) -> i32 {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut rect);
    }
    rect.bottom
}

pub(super) fn build_page(context: &mut WindowContext, client_w: i32) {
    if let Some(name) = read_preset_name(context) {
        context.modern.preset_name = name;
    }
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
        Page::Presets => build_presets(&mut builder),
        Page::Hotkeys => build_hotkeys(&mut builder),
        Page::Safety => build_safety(&mut builder),
        Page::General => build_general(&mut builder),
    }
    let end = builder.y + builder.px(24);
    builder.context.modern.total_height = end;
}
