//! The Monitors page: the selected monitor's controls, sliders, typed values
//! and enum lists.

use super::*;

pub(super) fn build_monitors(builder: &mut Builder<'_>) {
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

pub(super) fn add_control_row(builder: &mut Builder<'_>, index: usize, reading: &ControlReading) {
    let key = reading.capability.key;
    let title = control_title(key);
    let is_gain = matches!(
        key,
        ControlKey::GainRed | ControlKey::GainGreen | ControlKey::GainBlue
    );
    // Gains do nothing in a factory colour preset: dim them and say why.
    let unused_note = builder
        .context
        .model
        .gains_unused_in()
        .filter(|_| is_gain)
        .map(dusk_ui_model::text::gains_unused_note);
    let description = unused_note.as_deref().or(control_description(key));
    match reading.value {
        ControlValue::Normalized(value) => {
            let (slider, value_label) = builder.slider_row(
                title,
                description,
                Control::Slider(index).id(),
                Control::ValueLabel(index).id(),
                (0, 100),
                value,
                &format!("{value}%"),
            );
            if unused_note.is_some() {
                unsafe {
                    let _ =
                        windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(slider, false);
                }
                builder.context.modern.muted.push(value_label);
            }
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
                Control::Combo(index).id(),
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

/// Shows a stepped value at once on the Monitors page (SPEC-WR-1).
pub(crate) fn show_control_value(context: &mut WindowContext, control: ControlKey) {
    let Some(ControlValue::Normalized(value)) = context
        .model
        .controls()
        .iter()
        .find(|reading| reading.capability.key == control)
        .map(|reading| reading.value)
    else {
        return;
    };
    if let Some(row) = context.modern.rows.iter().find(|row| row.key == control) {
        set_slider(row.slider, value);
        set_window_text(row.value, &format!("{value}%"));
    }
}

/// Moves a slider without redrawing it at once (see lib.rs on painting
/// while the context is borrowed); it is redrawn by the next paint.
fn set_slider(slider: HWND, position: u32) {
    send(slider, TBM_SETPOS, WPARAM(0), LPARAM(position as isize));
    unsafe {
        let _ = InvalidateRect(Some(slider), None, true);
    }
}

pub(super) fn begin_value_edit(context: &mut WindowContext, id: u16) {
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
            Some(HMENU(Control::InlineEdit.id() as usize as *mut c_void)),
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

pub(super) fn finish_value_edit(context: &mut WindowContext) {
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
        set_slider(editing.slider, value);
        on_hscroll(context, editing.slider);
    }
}

// Enter commits and Escape cancels by moving focus away from the edit box.
pub(super) unsafe extern "system" fn edit_subclass(
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

pub(super) fn select_monitor(context: &mut WindowContext) {
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
    let monitor = match context.model.begin_select_monitor(&id) {
        Ok(monitor) => monitor,
        Err(error) => {
            set_status(context, &error.to_string());
            return;
        }
    };
    set_status(context, "Reading the monitor...");
    rebuild(context);
    crate::tasks::run(
        context,
        move |api| {
            let controls = dusk_ui_model::read_controls(api, &monitor);
            (monitor, controls)
        },
        |context, (monitor, controls)| match controls {
            Ok(controls) => {
                if context.model.apply_controls(&monitor, controls) {
                    set_status(context, "Ready.");
                    rebuild(context);
                }
            }
            Err(error) => set_status(context, &error.to_string()),
        },
    );
}

pub(super) fn apply_enum(context: &mut WindowContext, id: u16) {
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
    let Ok((monitor, control)) = context.model.selected_target() else {
        return;
    };
    let value = ControlValue::Enum(value);
    set_status(context, "Writing to the monitor...");
    crate::tasks::run(
        context,
        move |api| {
            // Input changes ask for confirmation here, off the UI thread.
            let result = api.set(&monitor, control, value);
            (monitor, result)
        },
        move |context, (monitor, result)| match result {
            // Monitors often report the old value for a moment after a
            // write, so a successful write keeps the user's choice.
            Ok(changed) => {
                context.model.show_written_value(&monitor, control, value);
                if control == ControlKey::ColorPreset {
                    rebuild(context);
                }
                set_status(
                    context,
                    if changed {
                        "Monitor setting applied."
                    } else {
                        "Value is unchanged; no monitor write was sent."
                    },
                );
            }
            Err(error) => {
                set_status(context, &error.to_string());
                show_actual_enum(context, control);
            }
        },
    );
}

/// Puts an enum row's list back on the value the model holds.
pub(super) fn show_actual_enum(context: &WindowContext, key: ControlKey) {
    let Some(combo) = context
        .modern
        .rows
        .iter()
        .find(|row| row.key == key)
        .map(|row| row.combo)
    else {
        return;
    };
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
    let control = Control::from_id(id);
    if let Some(Control::Slider(_)) = control {
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
        }
    } else if control == Some(Control::RevertSlider) {
        let seconds = snap_revert_seconds(position);
        if seconds != position {
            set_slider(slider, seconds);
        }
        set_window_text(context.modern.revert_value, &format!("{seconds} s"));
        let mut settings = current_settings(context);
        settings.input_revert_seconds = seconds;
        queue_settings_save(context, settings);
    } else if let Some(Control::StepSlider(index)) = control {
        let step = position.clamp(1, 25);
        set_window_text(context.modern.step_values[index], &format!("{step}%"));
        let mut settings = current_settings(context);
        match STEPPABLE_CONTROLS[index] {
            ControlKey::Brightness => settings.brightness_step = step,
            ControlKey::Contrast => settings.contrast_step = step,
            _ => settings.volume_step = step,
        }
        queue_settings_save(context, settings);
    } else if id == DEBOUNCE_ID {
        let milliseconds = snap_debounce_ms(position);
        set_window_text(context.modern.delay_value, &format!("{milliseconds} ms"));
        let mut settings = current_settings(context);
        settings.debounce_ms = milliseconds;
        queue_settings_save(context, settings);
    }
}
