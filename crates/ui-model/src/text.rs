//! Labels, messages and input rules shared by the Settings presentations.
//! Everything here is pure so it can be unit-tested without a window.

use std::path::{Path, PathBuf};

use dispcontrol_app::{ApplyReport, EntryStatus, HotkeyOutcome, ImportSummary};
use dispcontrol_domain::{
    ControlKey, ControlValue, HotkeyAction, HotkeyBinding, Preset, PresetEntry, STEPPABLE_CONTROLS,
};

/// File name used by the Settings Export/Import buttons, next to the config file.
pub const PRESET_EXCHANGE_FILE: &str = "dispcontrol-presets.toml";

pub fn control_title(key: ControlKey) -> &'static str {
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

pub fn control_description(key: ControlKey) -> Option<&'static str> {
    match key {
        ControlKey::Input => Some("Changing input asks for confirmation (see Safety & writes)."),
        ControlKey::Power => {
            Some("Sends a power mode to the display. Wake it with its own button.")
        }
        _ => None,
    }
}

/// Human name for an enumerated control value; unknown values show as raw hex.
pub fn enum_label(key: ControlKey, value: u32) -> String {
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
        (ControlKey::ColorPreset, 0x0C) => Some("User 2"),
        (ControlKey::ColorPreset, 0x0D) => Some("User 3"),
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

pub fn value_label(key: ControlKey, value: ControlValue) -> String {
    match value {
        ControlValue::Normalized(value) => format!("{value}%"),
        ControlValue::Enum(value) => enum_label(key, value),
    }
}

/// The revert timer is off (0) or at least 5 seconds.
pub fn snap_revert_seconds(position: u32) -> u32 {
    if (1..5).contains(&position) {
        5
    } else {
        position
    }
}

/// The quiet period moves in 50 ms steps within 150-2000 ms.
pub fn snap_debounce_ms(position: u32) -> u32 {
    (position.saturating_add(25) / 50 * 50).clamp(150, 2000)
}

/// Parses an edited preset value, keeping the entry's kind (percent or enum).
pub fn parse_entry_value(text: &str, current: ControlValue) -> Result<ControlValue, &'static str> {
    let number = text
        .trim()
        .parse::<u32>()
        .map_err(|_| "Enter a whole number.")?;
    match current {
        ControlValue::Normalized(_) if number > 100 => Err("Enter a percentage from 0 to 100."),
        ControlValue::Normalized(_) => Ok(ControlValue::Normalized(number)),
        ControlValue::Enum(_) => Ok(ControlValue::Enum(number)),
    }
}

/// Secondary line for a preset entry in the editor.
pub fn preset_entry_description(preset: &Preset, entry: &PresetEntry) -> String {
    let mut description = format!(
        "{} \u{2014} saved as {}",
        entry.monitor.as_str(),
        value_label(entry.control, entry.value)
    );
    if preset.is_entry_inert(entry) {
        description.push_str(" (not applied: the colour preset has its own gains)");
    }
    description
}

pub fn apply_report_message(report: &ApplyReport) -> String {
    let mut message = format!(
        "Applied '{}': {} changed, {} already set, {} skipped, {} failed.",
        report.preset,
        report.applied(),
        report.unchanged(),
        report.skipped(),
        report.failed()
    );
    if let Some(reason) = report
        .outcomes
        .iter()
        .find_map(|outcome| match &outcome.status {
            EntryStatus::Failed(reason) => Some(reason),
            _ => None,
        })
    {
        message.push_str(&format!(" First failure: {reason}"));
    }
    message
}

pub fn import_summary_message(path: &Path, summary: &ImportSummary) -> String {
    format!(
        "Imported {}: {} added, {} updated.",
        path.display(),
        summary.added,
        summary.updated
    )
}

/// Where the Settings Export/Import buttons read and write: next to the
/// presets' storage file, or the working directory if it has no folder.
pub fn exchange_path(location: &str) -> PathBuf {
    Path::new(location)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(PRESET_EXCHANGE_FILE))
        .unwrap_or_else(|| PathBuf::from(PRESET_EXCHANGE_FILE))
}

/// What a hotkey does, as shown in the Hotkeys list and its Action menu.
pub fn hotkey_action_label(action: &HotkeyAction) -> String {
    match action {
        HotkeyAction::ApplyPreset(name) => format!("Apply preset \u{201C}{name}\u{201D}"),
        HotkeyAction::NextPreset => "Next preset".into(),
        HotkeyAction::PreviousPreset => "Previous preset".into(),
        HotkeyAction::Step { control, up } => format!(
            "{} {}",
            control_title(*control),
            if *up { "up" } else { "down" }
        ),
        HotkeyAction::SetInput(value) => {
            format!("Switch input to {}", enum_label(ControlKey::Input, *value))
        }
        HotkeyAction::TogglePower => "Turn display off or on".into(),
    }
}

/// Secondary line for a configured hotkey: action, target and, when the
/// combination could not be registered, why (SPEC-HK-5).
pub fn hotkey_description(binding: &HotkeyBinding, failure: Option<&str>) -> String {
    let mut text = hotkey_action_label(&binding.action);
    let per_monitor = binding.action.uses_monitor();
    match &binding.monitor {
        Some(monitor) if per_monitor => text.push_str(&format!(" on {monitor}")),
        None if per_monitor => text.push_str(" on all monitors"),
        _ => {}
    }
    if let Some(reason) = failure {
        text.push_str(&format!(" \u{2014} not active: {reason}"));
    }
    text
}

/// The actions offered when adding a hotkey: steps, preset cycling, each
/// preset, each input the selected monitor offers, and power.
pub fn hotkey_action_choices(presets: &[Preset], inputs: &[u32]) -> Vec<HotkeyAction> {
    let mut choices = Vec::new();
    for control in STEPPABLE_CONTROLS {
        choices.push(HotkeyAction::Step { control, up: true });
        choices.push(HotkeyAction::Step { control, up: false });
    }
    choices.push(HotkeyAction::NextPreset);
    choices.push(HotkeyAction::PreviousPreset);
    choices.extend(
        presets
            .iter()
            .map(|preset| HotkeyAction::ApplyPreset(preset.name.clone())),
    );
    choices.extend(inputs.iter().map(|value| HotkeyAction::SetInput(*value)));
    choices.push(HotkeyAction::TogglePower);
    choices
}

/// Content of the on-screen indicator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Indicator {
    pub title: String,
    pub value: String,
    /// Percent level for a bar, for stepped controls.
    pub level: Option<u32>,
}

/// What the on-screen indicator shows after a hotkey (SPEC-HK-6). Input
/// changes show none: their confirmation dialogs are the feedback.
pub fn hotkey_indicator(outcome: &HotkeyOutcome) -> Option<Indicator> {
    match outcome {
        HotkeyOutcome::Stepped { control, values } => values.first().map(|(_, value)| Indicator {
            title: control_title(*control).to_owned(),
            value: format!("{value}%"),
            level: Some(*value),
        }),
        HotkeyOutcome::Preset(report) => Some(Indicator {
            title: "Preset".into(),
            value: match report.failed() {
                0 => report.preset.clone(),
                failed => format!("{} ({failed} failed)", report.preset),
            },
            level: None,
        }),
        HotkeyOutcome::Input { .. } => None,
        HotkeyOutcome::Power { on, .. } => Some(Indicator {
            title: "Display".into(),
            value: if *on { "On" } else { "Off" }.into(),
            level: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispcontrol_app::EntryOutcome;
    use dispcontrol_domain::MonitorId;

    fn entry(control: ControlKey, value: ControlValue) -> PresetEntry {
        PresetEntry {
            monitor: MonitorId::new("L32p-30#0").unwrap(),
            control,
            value,
        }
    }

    #[test]
    fn every_control_has_a_title() {
        for key in [
            ControlKey::Brightness,
            ControlKey::Contrast,
            ControlKey::Volume,
            ControlKey::Input,
            ControlKey::ColorPreset,
            ControlKey::GainRed,
            ControlKey::GainGreen,
            ControlKey::GainBlue,
            ControlKey::Power,
        ] {
            assert!(!control_title(key).is_empty());
        }
        assert!(control_description(ControlKey::Input).is_some());
        assert!(control_description(ControlKey::Brightness).is_none());
    }

    #[test]
    fn enum_labels_name_known_values_and_show_raw_codes() {
        assert_eq!(enum_label(ControlKey::ColorPreset, 0x06), "7500 K");
        assert_eq!(enum_label(ControlKey::ColorPreset, 0x0B), "User 1");
        assert_eq!(enum_label(ControlKey::Input, 0x31), "USB-C (raw-0x31)");
        assert_eq!(enum_label(ControlKey::Input, 0x99), "raw-0x99");
        assert_eq!(enum_label(ControlKey::Power, 0x04), "Off (soft)");
        assert_eq!(
            value_label(ControlKey::Brightness, ControlValue::Normalized(65)),
            "65%"
        );
    }

    #[test]
    fn slider_positions_snap_to_allowed_settings() {
        assert_eq!(snap_revert_seconds(0), 0);
        assert_eq!(snap_revert_seconds(1), 5);
        assert_eq!(snap_revert_seconds(4), 5);
        assert_eq!(snap_revert_seconds(12), 12);
        assert_eq!(snap_debounce_ms(0), 150);
        assert_eq!(snap_debounce_ms(424), 400);
        assert_eq!(snap_debounce_ms(425), 450);
        assert_eq!(snap_debounce_ms(5000), 2000);
        assert_eq!(snap_debounce_ms(u32::MAX), 2000);
    }

    #[test]
    fn edited_preset_values_keep_their_kind_and_range() {
        let percent = ControlValue::Normalized(50);
        assert_eq!(
            parse_entry_value(" 70 ", percent),
            Ok(ControlValue::Normalized(70))
        );
        assert!(parse_entry_value("101", percent).is_err());
        assert!(parse_entry_value("-1", percent).is_err());
        assert!(parse_entry_value("", percent).is_err());
        assert_eq!(
            parse_entry_value("11", ControlValue::Enum(6)),
            Ok(ControlValue::Enum(11))
        );
    }

    #[test]
    fn entry_descriptions_show_the_saved_value_and_inert_gains() {
        let gain = entry(ControlKey::GainRed, ControlValue::Normalized(40));
        let mut preset = Preset {
            name: "Day".into(),
            entries: vec![
                entry(ControlKey::ColorPreset, ControlValue::Enum(0x06)),
                gain.clone(),
            ],
        };
        assert_eq!(
            preset_entry_description(&preset, &preset.entries[0].clone()),
            "L32p-30#0 \u{2014} saved as 7500 K"
        );
        assert!(preset_entry_description(&preset, &gain).contains("not applied"));
        preset.entries[0].value = ControlValue::Enum(0x0B);
        assert!(!preset_entry_description(&preset, &gain).contains("not applied"));
    }

    #[test]
    fn apply_messages_count_outcomes_and_quote_the_first_failure() {
        let brightness = entry(ControlKey::Brightness, ControlValue::Normalized(10));
        let report = ApplyReport {
            preset: "Night".into(),
            outcomes: vec![
                EntryOutcome {
                    entry: brightness.clone(),
                    status: EntryStatus::Applied,
                },
                EntryOutcome {
                    entry: brightness.clone(),
                    status: EntryStatus::Failed("DDC/CI error".into()),
                },
                EntryOutcome {
                    entry: brightness,
                    status: EntryStatus::Skipped("absent".into()),
                },
            ],
        };
        assert_eq!(
            apply_report_message(&report),
            "Applied 'Night': 1 changed, 0 already set, 1 skipped, 1 failed. \
             First failure: DDC/CI error"
        );
    }

    fn action(text: &str) -> HotkeyAction {
        text.parse().unwrap()
    }

    #[test]
    fn hotkey_labels_describe_action_target_and_registration() {
        assert_eq!(hotkey_action_label(&action("brightness+")), "Brightness up");
        assert_eq!(
            hotkey_action_label(&action("input:0x11")),
            "Switch input to HDMI 1 (raw-0x11)"
        );
        let binding = HotkeyBinding {
            keys: "Ctrl+Alt+Up".parse().unwrap(),
            action: action("volume-"),
            monitor: None,
        };
        assert_eq!(
            hotkey_description(&binding, None),
            "Volume down on all monitors"
        );
        let targeted = HotkeyBinding {
            monitor: Some(dispcontrol_domain::MonitorId::new("L32p-30#0").unwrap()),
            ..binding.clone()
        };
        assert_eq!(
            hotkey_description(&targeted, Some("used by another app")),
            "Volume down on L32p-30#0 \u{2014} not active: used by another app"
        );
        let preset = HotkeyBinding {
            action: action("preset:Night"),
            ..targeted
        };
        assert_eq!(
            hotkey_description(&preset, None),
            "Apply preset \u{201C}Night\u{201D}"
        );
    }

    #[test]
    fn action_choices_list_steps_presets_inputs_and_power() {
        let presets = vec![Preset {
            name: "Night".into(),
            entries: vec![],
        }];
        let choices = hotkey_action_choices(&presets, &[0x0F, 0x31]);
        let texts: Vec<String> = choices.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            vec![
                "brightness+",
                "brightness-",
                "contrast+",
                "contrast-",
                "volume+",
                "volume-",
                "preset-next",
                "preset-prev",
                "preset:Night",
                "input:0x0F",
                "input:0x31",
                "power-toggle",
            ]
        );
    }

    #[test]
    fn indicators_show_steps_presets_and_power_but_not_inputs() {
        let monitor = dispcontrol_domain::MonitorId::new("m").unwrap();
        let step = HotkeyOutcome::Stepped {
            control: ControlKey::Contrast,
            values: vec![(monitor, 35)],
        };
        assert_eq!(
            hotkey_indicator(&step),
            Some(Indicator {
                title: "Contrast".into(),
                value: "35%".into(),
                level: Some(35),
            })
        );
        let report = ApplyReport {
            preset: "Night".into(),
            outcomes: vec![],
        };
        assert_eq!(
            hotkey_indicator(&HotkeyOutcome::Preset(report))
                .unwrap()
                .value,
            "Night"
        );
        assert_eq!(
            hotkey_indicator(&HotkeyOutcome::Power {
                on: false,
                changed: 1
            })
            .unwrap()
            .value,
            "Off"
        );
        assert_eq!(
            hotkey_indicator(&HotkeyOutcome::Input {
                value: 0x11,
                changed: 1
            }),
            None
        );
    }

    #[test]
    fn exchange_file_sits_next_to_the_config_file() {
        assert_eq!(
            exchange_path(r"C:\Users\me\AppData\Roaming\dispcontrol\config.toml"),
            Path::new(r"C:\Users\me\AppData\Roaming\dispcontrol").join(PRESET_EXCHANGE_FILE)
        );
        assert_eq!(exchange_path(""), PathBuf::from(PRESET_EXCHANGE_FILE));
        assert_eq!(
            exchange_path("config.toml"),
            PathBuf::from(PRESET_EXCHANGE_FILE)
        );
        let summary = ImportSummary {
            added: 1,
            updated: 2,
            removed: 0,
        };
        assert_eq!(
            import_summary_message(Path::new("p.toml"), &summary),
            "Imported p.toml: 1 added, 2 updated."
        );
    }
}
