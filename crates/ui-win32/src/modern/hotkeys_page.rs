//! The Hotkeys page: the bindings, the Add form, step sizes and the indicator.

use super::*;

pub(super) fn build_hotkeys(builder: &mut Builder<'_>) {
    use windows::Win32::UI::WindowsAndMessaging::{ES_AUTOHSCROLL, WS_BORDER};
    builder.heading("Hotkeys");
    let bindings: Vec<HotkeyBinding> = builder.context.model.hotkeys().to_vec();
    let settings = current_settings(builder.context);

    builder.section("Configured hotkeys");
    builder.card_begin();
    if bindings.is_empty() {
        builder.row(
            "No hotkeys yet",
            Some("Add one below. Hotkeys work while the Settings window is closed."),
            0,
        );
    }
    let remove_width = builder.px(90);
    for (index, binding) in bindings.iter().enumerate().take(ROW_LIMIT) {
        let failure = builder
            .context
            .hotkeys
            .failure(&binding.keys)
            .map(str::to_owned);
        let description = hotkey_description(binding, failure.as_deref());
        let (top, height) =
            builder.row(&binding.keys.to_string(), Some(&description), remove_width);
        let x = builder.control_x(remove_width);
        builder.button(
            Control::HotkeyRemove(index).id(),
            "Remove",
            x,
            top + (height - builder.px(32)) / 2,
            remove_width,
        );
    }
    builder.card_end();

    builder.section("Add a hotkey");
    builder.card_begin();
    let field_width = builder.px(280);
    let (top, height) = builder.row(
        "Keys",
        Some("Click here and press the combination, e.g. Ctrl+Alt+Up. Use Ctrl, Alt or Win, or a function key alone. Backspace clears."),
        field_width,
    );
    let keys = builder.create(
        w!("EDIT"),
        "",
        Control::HotkeyKeys.id(),
        WS_TABSTOP.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        (
            builder.control_x(field_width),
            top + (height - builder.px(30)) / 2,
            field_width,
            builder.px(30),
        ),
        FontKind::Body,
    );
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(
            keys,
            Some(crate::hotkeys::recording_subclass),
            2,
            0,
        );
    }
    let actions: Vec<String> = builder
        .context
        .model
        .hotkey_choices()
        .iter()
        .map(hotkey_action_label)
        .collect();
    builder.combo_row(
        "Action",
        None,
        Control::HotkeyAction.id(),
        &actions,
        Some(0),
    );
    let mut monitors = vec!["All monitors".to_owned()];
    monitors.extend(
        builder
            .context
            .model
            .monitors()
            .iter()
            .map(|monitor| monitor.name.clone()),
    );
    builder.combo_row(
        "Monitor",
        Some("Preset actions always use the monitors stored in the preset."),
        Control::HotkeyMonitor.id(),
        &monitors,
        Some(0),
    );
    let add_width = builder.px(120);
    let (top, height) = builder.row("", None, add_width);
    let x = builder.control_x(add_width);
    builder.button(
        Control::HotkeyAdd.id(),
        "Add hotkey",
        x,
        top + (height - builder.px(32)) / 2,
        add_width,
    );
    builder.card_end();

    builder.section("Steps and indicator");
    builder.card_begin();
    for (index, control) in STEPPABLE_CONTROLS.iter().enumerate() {
        let step = settings.step_for(*control).unwrap_or(5);
        let (_, value) = builder.slider_row(
            &format!("{} step", control_title(*control)),
            None,
            Control::StepSlider(index).id(),
            Control::StepValue(index).id(),
            (1, 25),
            step,
            &format!("{step}%"),
        );
        builder.context.modern.step_values[index] = value;
    }
    let toggle_width = builder.px(44);
    let (top, height) = builder.row(
        "Show on-screen indicator",
        Some("Briefly shows the control and its new value after a hotkey."),
        toggle_width,
    );
    builder.create(
        w!("BUTTON"),
        "",
        Control::OsdToggle.id(),
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
}

pub(super) fn add_hotkey(context: &mut WindowContext) {
    let binding = context.model.hotkey_from_form(
        &read_child_text(context, Control::HotkeyKeys).unwrap_or_default(),
        combo_selection(context, Control::HotkeyAction),
        combo_selection(context, Control::HotkeyMonitor),
    );
    let binding = match binding {
        Ok(binding) => binding,
        Err(message) => {
            set_status(context, &message);
            return;
        }
    };
    let keys = binding.keys;
    let replaced = context
        .model
        .hotkeys()
        .iter()
        .any(|binding| binding.keys == keys);
    let label = hotkey_action_label(&binding.action);
    match context.model.save_hotkey(binding) {
        Ok(()) => {
            crate::hotkeys::register(context);
            let message = match context.hotkeys.failure(&keys) {
                Some(reason) => format!("Saved {keys} ({label}), but it is not active: {reason}."),
                None if replaced => format!("{keys} now does: {label}."),
                None => format!("Added {keys}: {label}."),
            };
            set_status(context, &message);
            rebuild(context);
        }
        Err(error) => set_status(context, &error.to_string()),
    }
}

pub(super) fn remove_hotkey(context: &mut WindowContext, index: usize) {
    let Some(keys) = context
        .model
        .hotkeys()
        .get(index)
        .map(|binding| binding.keys)
    else {
        return;
    };
    match context.model.remove_hotkey(&keys) {
        Ok(()) => {
            crate::hotkeys::register(context);
            set_status(context, &format!("Removed hotkey {keys}."));
            rebuild(context);
        }
        Err(error) => set_status(context, &error.to_string()),
    }
}
