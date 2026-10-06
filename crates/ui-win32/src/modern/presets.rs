//! The Presets page: the list, the detail editor, saving the current setup,
//! export/import, and the row actions.

use super::*;

/// Re-checks which preset matches the monitors, off the UI thread.
pub(super) fn refresh_matching_preset(context: &mut WindowContext) {
    crate::tasks::run(
        context,
        dusk_ui_model::fetch_matching_preset,
        |context, matching| {
            if context.model.matching_preset() != matching.as_deref() {
                context.model.set_matching_preset(matching);
                if context.modern.page == Page::Presets {
                    rebuild(context);
                }
            }
        },
    );
}

pub(super) fn build_presets(builder: &mut Builder<'_>) {
    use windows::Win32::UI::WindowsAndMessaging::{ES_AUTOHSCROLL, WS_BORDER};
    builder.heading("Presets");
    let presets: Vec<_> = builder.context.model.presets().to_vec();
    let matching = builder.context.model.matching_preset().map(str::to_owned);
    if let Some(name) = builder.context.modern.editing_preset.clone() {
        if let Some(preset) = presets
            .iter()
            .find(|preset| dusk_domain::preset_names_equal(&preset.name, &name))
        {
            build_preset_detail(builder, preset);
            return;
        }
        builder.context.modern.editing_preset = None;
    }

    builder.section("Saved presets");
    builder.card_begin();
    if presets.is_empty() {
        builder.row(
            "No presets yet",
            Some("Set the monitor the way you like it, then save it below."),
            0,
        );
    }
    let widths = [60, 52, 68, 32, 32, 60];
    let gap = builder.px(6);
    let total: i32 = widths.iter().map(|w| builder.px(*w)).sum::<i32>() + gap * 5;
    for (index, preset) in presets.iter().enumerate().take(ROW_LIMIT) {
        let current = matching
            .as_deref()
            .is_some_and(|name| dusk_domain::preset_names_equal(name, &preset.name));
        let title = if current {
            format!("{}  (current)", preset.name)
        } else {
            preset.name.clone()
        };
        let description = format!("{} saved settings", preset.entries.len());
        let (top, height) = builder.row(&title, Some(&description), total);
        let mut x = builder.control_x(total);
        let y = top + (height - builder.px(32)) / 2;
        let buttons = [
            (RowButton::Apply, "Apply"),
            (RowButton::Edit, "Edit"),
            (RowButton::Rename, "Rename"),
            (RowButton::Up, "\u{25B2}"),
            (RowButton::Down, "\u{25BC}"),
            (RowButton::Delete, "Delete"),
        ];
        for ((button, text), width) in buttons.iter().zip(widths) {
            let width = builder.px(width);
            let id = Control::PresetRow(*button, index).id();
            builder.button(id, text, x, y, width);
            x += width + gap;
        }
    }
    builder.card_end();

    builder.section("Save current settings as a preset");
    builder.card_begin();
    let name_width = builder.px(280);
    let (top, height) = builder.row(
        "Preset name",
        Some("To rename a preset, type the new name here and press Rename on it."),
        name_width,
    );
    let draft = builder.context.modern.preset_name.clone();
    let edit = builder.create(
        w!("EDIT"),
        &draft,
        Control::PresetName.id(),
        WS_TABSTOP.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        (
            builder.control_x(name_width),
            top + (height - builder.px(30)) / 2,
            name_width,
            builder.px(30),
        ),
        FontKind::Body,
    );
    send(edit, EM_SETLIMITTEXT, WPARAM(40), LPARAM(0));
    let button_width = builder.px(170);
    let (top, height) = builder.row(
        "Save",
        Some("Stores brightness, contrast, colour and volume. Input source is stored only if you choose to."),
        button_width * 2 + gap,
    );
    let y = top + (height - builder.px(32)) / 2;
    let x = builder.control_x(button_width * 2 + gap);
    builder.button(Control::PresetSave.id(), "Save current", x, y, button_width);
    builder.button(
        Control::PresetSaveInput.id(),
        "Save incl. input source",
        x + button_width + gap,
        y,
        button_width,
    );
    builder.card_end();

    builder.section("Export, import and file location");
    builder.card_begin();
    let location = builder.context.model.presets_location();
    let folder = exchange_path(&location);
    let width = builder.px(110);
    let total = width * 3 + gap * 2;
    let (top, height) = builder.row(
        "Text file",
        Some(&format!(
            "Presets live in {location}. Export and import use {}.",
            folder.display()
        )),
        total,
    );
    let y = top + (height - builder.px(32)) / 2;
    let mut x = builder.control_x(total);
    for (id, text) in [
        (Control::PresetExport, "Export"),
        (Control::PresetImport, "Import"),
        (Control::PresetFolder, "Open folder"),
    ] {
        builder.button(id.id(), text, x, y, width);
        x += width + gap;
    }
    builder.card_end();
}

pub(super) fn build_preset_detail(builder: &mut Builder<'_>, preset: &dusk_domain::Preset) {
    use windows::Win32::UI::WindowsAndMessaging::{ES_AUTOHSCROLL, ES_NUMBER, ES_RIGHT, WS_BORDER};
    let gap = builder.px(6);
    builder.section(&format!("Settings stored in '{}'", preset.name));
    builder.card_begin();
    let back_width = builder.px(100);
    let (top, height) = builder.row(
        "Back to presets",
        Some("Change a value, then press Set. Applying the preset writes only values that differ."),
        back_width,
    );
    let x = builder.control_x(back_width);
    builder.button(
        Control::PresetBack.id(),
        "Back",
        x,
        top + (height - builder.px(32)) / 2,
        back_width,
    );
    if preset.entries.is_empty() {
        builder.row("This preset has no settings", None, 0);
    }
    let edit_width = builder.px(70);
    let button_width = builder.px(72);
    let total = edit_width + button_width * 2 + gap * 2;
    for (index, entry) in preset.entries.iter().enumerate().take(ROW_LIMIT) {
        let text = match entry.value {
            ControlValue::Normalized(value) | ControlValue::Enum(value) => value.to_string(),
        };
        let description = preset_entry_description(preset, entry);
        let (top, height) = builder.row(control_title(entry.control), Some(&description), total);
        let x = builder.control_x(total);
        let edit_height = builder.px(30);
        let y = top + (height - edit_height) / 2;
        builder.create(
            w!("EDIT"),
            &text,
            Control::EntryValue(index).id(),
            WS_TABSTOP.0 | WS_BORDER.0 | ES_NUMBER as u32 | ES_RIGHT as u32 | ES_AUTOHSCROLL as u32,
            (x, y, edit_width, edit_height),
            FontKind::Body,
        );
        let y = top + (height - builder.px(32)) / 2;
        builder.button(
            Control::Entry(EntryButton::Set, index).id(),
            "Set",
            x + edit_width + gap,
            y,
            button_width,
        );
        builder.button(
            Control::Entry(EntryButton::Remove, index).id(),
            "Remove",
            x + edit_width + button_width + gap * 2,
            y,
            button_width,
        );
    }
    builder.card_end();
}

pub(super) fn read_preset_name(context: &WindowContext) -> Option<String> {
    let edit =
        context.modern.children.iter().copied().find(
            |window| unsafe { GetDlgCtrlID(*window) } == i32::from(Control::PresetName.id()),
        )?;
    let mut buffer = [0u16; 64];
    let length = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
    Some(String::from_utf16_lossy(&buffer[..length]))
}

pub(super) fn save_preset(context: &mut WindowContext, include_input: bool) {
    let name = read_preset_name(context).unwrap_or_default();
    let Some(monitor) = context.model.selected_monitor().cloned() else {
        set_status(context, "Select a monitor first.");
        return;
    };
    set_status(context, "Reading the monitor...");
    crate::tasks::run(
        context,
        move |api| {
            let preset = api.capture_preset(&name, &monitor, include_input);
            let matching = dusk_ui_model::fetch_matching_preset(api);
            (preset, matching)
        },
        |context, (preset, matching)| match preset {
            Ok(preset) => {
                context.modern.preset_name.clear();
                let _ = context.model.reload_presets();
                context.model.set_matching_preset(matching);
                set_status(
                    context,
                    &format!(
                        "Saved preset '{}' with {} settings.",
                        preset.name,
                        preset.entries.len()
                    ),
                );
                rebuild(context);
            }
            Err(error) => set_status(context, &error.to_string()),
        },
    );
}

pub(super) fn read_dialog_text(context: &WindowContext, id: u16) -> Option<String> {
    let edit = context
        .modern
        .children
        .iter()
        .copied()
        .find(|window| unsafe { GetDlgCtrlID(*window) } == i32::from(id))?;
    let mut buffer = [0u16; 16];
    let length = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
    Some(String::from_utf16_lossy(&buffer[..length]))
}

pub(super) fn preset_entry_action(context: &mut WindowContext, button: EntryButton, index: usize) {
    let Some(name) = context.modern.editing_preset.clone() else {
        return;
    };
    let Some(entry) = context
        .model
        .presets()
        .iter()
        .find(|preset| dusk_domain::preset_names_equal(&preset.name, &name))
        .and_then(|preset| preset.entries.get(index).cloned())
    else {
        return;
    };
    let result = match button {
        EntryButton::Set => {
            let text =
                read_dialog_text(context, Control::EntryValue(index).id()).unwrap_or_default();
            let value = match parse_entry_value(&text, entry.value) {
                Ok(value) => value,
                Err(message) => {
                    set_status(context, message);
                    return;
                }
            };
            let result = context
                .model
                .set_preset_entry(&name, dusk_domain::PresetEntry { value, ..entry });
            if result.is_ok() {
                set_status(context, &format!("Updated '{name}'."));
            }
            result
        }
        EntryButton::Remove => {
            let result = context
                .model
                .remove_preset_entry(&name, &entry.monitor, entry.control);
            if result.is_ok() {
                set_status(context, &format!("Removed a setting from '{name}'."));
            }
            result
        }
    };
    match result {
        Ok(()) => rebuild(context),
        Err(error) => set_status(context, &error.to_string()),
    }
}

pub(super) fn preset_file_action(context: &mut WindowContext, control: Control) {
    let path = exchange_path(&context.model.presets_location());
    match control {
        Control::PresetBack => {
            context.modern.editing_preset = None;
            context.modern.scroll_y = 0;
            rebuild(context);
        }
        Control::PresetExport => {
            let message = match context.model.export_presets() {
                Ok(text) => match std::fs::write(&path, text) {
                    Ok(()) => format!("Exported presets to {}.", path.display()),
                    Err(error) => format!("Could not write {}: {error}", path.display()),
                },
                Err(error) => error.to_string(),
            };
            set_status(context, &message);
        }
        Control::PresetImport => {
            let message = match std::fs::read_to_string(&path) {
                Ok(text) => match context.model.import_presets(&text, false) {
                    Ok(summary) => {
                        rebuild(context);
                        import_summary_message(&path, &summary)
                    }
                    Err(error) => error.to_string(),
                },
                Err(error) => format!("Could not read {}: {error}", path.display()),
            };
            set_status(context, &message);
        }
        Control::PresetFolder => {
            use windows::Win32::UI::Shell::ShellExecuteW;
            use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
            if let Some(dir) = path.parent() {
                let dir = wide_null(&dir.to_string_lossy());
                crate::defer(context, move |_| unsafe {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR(dir.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                });
            }
        }
        _ => {}
    }
}

pub(super) fn preset_row_action(context: &mut WindowContext, button: RowButton, index: usize) {
    let Some(name) = context
        .model
        .presets()
        .get(index)
        .map(|preset| preset.name.clone())
    else {
        return;
    };
    let result = match button {
        RowButton::Apply => {
            set_status(context, &format!("Applying '{name}'..."));
            let monitor = context.model.selected_monitor().cloned();
            crate::tasks::run(
                context,
                move |api| {
                    let report = api.apply_preset(&name);
                    let after = dusk_ui_model::fetch_after_change(api, monitor);
                    (report, after)
                },
                |context, (report, after)| {
                    let _ = context.model.apply_after_change(after);
                    match report {
                        Ok(report) => set_status(context, &apply_report_message(&report)),
                        Err(error) => set_status(context, &error.to_string()),
                    }
                    rebuild(context);
                },
            );
            return;
        }
        RowButton::Rename => {
            let new_name = read_preset_name(context).unwrap_or_default();
            let result = context.model.rename_preset(&name, &new_name);
            if result.is_ok() {
                context.modern.preset_name.clear();
                set_status(
                    context,
                    &format!("Renamed '{name}' to '{}'.", new_name.trim()),
                );
            }
            result
        }
        RowButton::Edit => {
            context.modern.editing_preset = Some(name);
            context.modern.scroll_y = 0;
            Ok(())
        }
        RowButton::Up => context.model.move_preset(&name, -1),
        RowButton::Down => context.model.move_preset(&name, 1),
        RowButton::Delete => {
            crate::defer(context, move |shared| {
                delete_preset_after_asking(shared, name)
            });
            return;
        }
    };
    match result {
        Ok(()) => rebuild(context),
        Err(error) => set_status(context, &error.to_string()),
    }
}

/// Asks before deleting a preset that hotkeys use (they are deleted with
/// it), then deletes it. A deferred step: the message box runs a modal loop.
pub(super) fn delete_preset_after_asking(shared: &crate::Shared, name: String) {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO, MessageBoxW,
    };
    let (window, keys) = {
        let context = shared.context().borrow();
        let keys = context
            .model
            .hotkeys_using_preset(&name)
            .unwrap_or_default();
        (context.window, keys)
    };
    if !keys.is_empty() {
        let list: Vec<String> = keys.iter().map(ToString::to_string).collect();
        let text = wide_null(&format!(
            "The preset '{name}' is used by these hotkeys: {}.\n\nDelete the preset and these hotkeys?",
            list.join(", ")
        ));
        let title = wide_null("Delete preset");
        let confirmed = unsafe {
            MessageBoxW(
                Some(window),
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) == IDYES
        };
        if !confirmed {
            return;
        }
    }
    crate::update(shared, |context| match context.model.delete_preset(&name) {
        Ok(()) => {
            set_status(context, &format!("Deleted preset '{name}'."));
            rebuild(context);
        }
        Err(error) => set_status(context, &error.to_string()),
    });
}
