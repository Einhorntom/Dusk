//! The Safety & writes and General pages, and saving their settings.

use super::*;

pub(super) fn current_settings(context: &WindowContext) -> AppSettings {
    context
        .modern
        .pending_settings
        .clone()
        .unwrap_or_else(|| context.model.settings().clone())
}

pub(super) fn build_safety(builder: &mut Builder<'_>) {
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
        Control::ConfirmToggle.id(),
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
        Control::RevertSlider.id(),
        Control::RevertValue.id(),
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
        Control::DebounceValue.id(),
        (150, 2000),
        settings.debounce_ms,
        &format!("{} ms", settings.debounce_ms),
    );
    builder.context.modern.delay_value = delay_value;
    builder.card_end();
}

pub(super) fn build_general(builder: &mut Builder<'_>) {
    builder.heading("General");
    builder.card_begin();
    builder.row(
        "Theme",
        Some("Follows your Windows app mode (light or dark)."),
        0,
    );
    builder.row("Settings", Some("Changes are saved automatically."), 0);
    let toggle_width = builder.px(44);
    let (top, height) = builder.row(
        "Diagnostic log",
        Some("Records warnings and errors in %LOCALAPPDATA%\\Dusk\\logs to help track down problems."),
        toggle_width,
    );
    builder.create(
        w!("BUTTON"),
        "",
        Control::LogToggle.id(),
        WS_TABSTOP.0 | BS_OWNERDRAW as u32,
        (
            builder.control_x(toggle_width),
            top + (height - builder.px(24)) / 2,
            toggle_width,
            builder.px(24),
        ),
        FontKind::Body,
    );
    builder.card_end();
    builder.section("About");
    builder.card_begin();
    builder.row(
        concat!("Dusk ", env!("CARGO_PKG_VERSION")),
        Some("Runs from the tray. Close or minimize hides the window; use Quit in the tray menu to exit."),
        0,
    );
    builder.card_end();
}

pub(super) fn queue_settings_save(context: &mut WindowContext, settings: AppSettings) {
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
