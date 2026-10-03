//! Labels, messages and input rules shared by the Settings presentations.
//! Everything here is pure so it can be unit-tested without a window.

use std::path::{Path, PathBuf};

use dispcontrol_app::{ApplyReport, EntryStatus, ImportSummary};
use dispcontrol_domain::{ControlKey, ControlValue, Preset, PresetEntry};

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
