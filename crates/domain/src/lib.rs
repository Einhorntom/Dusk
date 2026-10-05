#![forbid(unsafe_code)]

use std::fmt;

mod hotkey;

pub use hotkey::{HotkeyAction, HotkeyBinding, Key, KeyCombo, STEPPABLE_CONTROLS};

mod write_policy;

pub use write_policy::{
    DueTarget, MAX_WRITES_PER_MINUTE, MIN_WRITE_INTERVAL, RateLimiter, WriteBuffer,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MonitorId(String);

impl MonitorId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainError::EmptyMonitorId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ControlKey {
    Brightness,
    Contrast,
    ColorPreset,
    GainRed,
    GainGreen,
    GainBlue,
    Input,
    Volume,
    Power,
}

impl ControlKey {
    pub const ALL: [ControlKey; 9] = [
        Self::Brightness,
        Self::Contrast,
        Self::ColorPreset,
        Self::GainRed,
        Self::GainGreen,
        Self::GainBlue,
        Self::Input,
        Self::Volume,
        Self::Power,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brightness => "brightness",
            Self::Contrast => "contrast",
            Self::ColorPreset => "color-preset",
            Self::GainRed => "gain-red",
            Self::GainGreen => "gain-green",
            Self::GainBlue => "gain-blue",
            Self::Input => "input",
            Self::Volume => "volume",
            Self::Power => "power",
        }
    }

    pub fn vcp_code(self) -> u8 {
        match self {
            Self::Brightness => 0x10,
            Self::Contrast => 0x12,
            Self::ColorPreset => 0x14,
            Self::GainRed => 0x16,
            Self::GainGreen => 0x18,
            Self::GainBlue => 0x1A,
            Self::Input => 0x60,
            Self::Volume => 0x62,
            Self::Power => 0xD6,
        }
    }

    pub fn is_numeric(self) -> bool {
        !matches!(self, Self::Input | Self::ColorPreset | Self::Power)
    }
}

impl fmt::Display for ControlKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ControlKey {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "brightness" => Ok(Self::Brightness),
            "contrast" => Ok(Self::Contrast),
            "color-preset" => Ok(Self::ColorPreset),
            "gain-red" => Ok(Self::GainRed),
            "gain-green" => Ok(Self::GainGreen),
            "gain-blue" => Ok(Self::GainBlue),
            "input" => Ok(Self::Input),
            "volume" => Ok(Self::Volume),
            "power" => Ok(Self::Power),
            _ => Err(DomainError::UnknownControl(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Monitor {
    pub id: MonitorId,
    pub name: String,
    pub unstable_id: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlCapability {
    pub key: ControlKey,
    pub native_min: u32,
    pub native_max: u32,
    pub enum_values: Vec<u32>,
    /// Writes go to the monitor's non-volatile memory, so commits follow the
    /// write-rate limits (SPEC-WR-6). False for the built-in display (SPEC-PNL-4).
    pub rate_limited: bool,
}

impl ControlCapability {
    pub fn normalized_to_native(&self, value: u32) -> Result<u32, DomainError> {
        if !self.key.is_numeric() || self.native_min > self.native_max {
            return Err(DomainError::InvalidNumericCapability(self.key));
        }
        if value > 100 {
            return Err(DomainError::NormalizedValueOutOfRange(value));
        }
        let span = u64::from(self.native_max - self.native_min);
        let scaled = (u64::from(value) * span + 50) / 100;
        Ok(self.native_min + scaled as u32)
    }

    pub fn native_to_normalized(&self, value: u32) -> Result<u32, DomainError> {
        if !self.key.is_numeric() || self.native_min > self.native_max {
            return Err(DomainError::InvalidNumericCapability(self.key));
        }
        if value < self.native_min || value > self.native_max {
            return Err(DomainError::NativeValueOutOfRange {
                value,
                min: self.native_min,
                max: self.native_max,
            });
        }
        let span = u64::from(self.native_max - self.native_min);
        if span == 0 {
            return Ok(0);
        }
        let offset = u64::from(value - self.native_min);
        Ok(((offset * 100 + span / 2) / span) as u32)
    }

    pub fn validate_enum(&self, value: u32) -> Result<(), DomainError> {
        if self.key.is_numeric() {
            return Err(DomainError::InvalidEnumCapability(self.key));
        }
        if self.enum_values.contains(&value) {
            Ok(())
        } else {
            Err(DomainError::EnumValueUnavailable {
                control: self.key,
                value,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlValue {
    Normalized(u32),
    Enum(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlReading {
    pub capability: ControlCapability,
    pub value: ControlValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetEntry {
    pub monitor: MonitorId,
    pub control: ControlKey,
    pub value: ControlValue,
}

impl PresetEntry {
    /// Numeric controls need a 0-100 normalized value; others need an enum value.
    pub fn validate(&self) -> Result<(), DomainError> {
        match (self.control.is_numeric(), self.value) {
            (true, ControlValue::Normalized(value)) if value <= 100 => Ok(()),
            (true, ControlValue::Normalized(value)) => {
                Err(DomainError::NormalizedValueOutOfRange(value))
            }
            (false, ControlValue::Enum(_)) => Ok(()),
            _ => Err(DomainError::InvalidNumericCapability(self.control)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preset {
    pub name: String,
    pub entries: Vec<PresetEntry>,
}

/// MCCS colour preset values 0x0B-0x0D are the user profiles; the others
/// (sRGB, native, colour temperatures) are fixed modes with their own gains.
pub fn is_user_color_preset(value: u32) -> bool {
    (0x0B..=0x0D).contains(&value)
}

impl Preset {
    /// RGB gains only belong to a user colour profile, and writing one makes the
    /// monitor switch to that profile. A gain entry is therefore inert when the
    /// preset selects a fixed colour preset for the same monitor.
    pub fn is_entry_inert(&self, entry: &PresetEntry) -> bool {
        matches!(
            entry.control,
            ControlKey::GainRed | ControlKey::GainGreen | ControlKey::GainBlue
        ) && self.entries.iter().any(|other| {
            other.monitor == entry.monitor
                && other.control == ControlKey::ColorPreset
                && matches!(other.value, ControlValue::Enum(value) if !is_user_color_preset(value))
        })
    }
}

pub const PRESET_NAME_MAX_CHARS: usize = 40;

/// Returns the trimmed name if it satisfies SPEC-PRE-1 (1-40 characters).
pub fn validate_preset_name(name: &str) -> Result<String, DomainError> {
    let name = name.trim();
    let length = name.chars().count();
    if length == 0 || length > PRESET_NAME_MAX_CHARS {
        return Err(DomainError::InvalidPresetName);
    }
    Ok(name.to_owned())
}

pub fn preset_names_equal(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

impl ControlKey {
    /// Fixed apply order from SPEC-PRE-4.
    pub fn preset_apply_rank(self) -> u8 {
        match self {
            Self::Power => 0,
            Self::Input => 1,
            Self::ColorPreset => 2,
            Self::GainRed | Self::GainGreen | Self::GainBlue => 3,
            Self::Brightness => 4,
            Self::Contrast => 5,
            Self::Volume => 6,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppSettings {
    pub debounce_ms: u32,
    pub confirm_input_change: bool,
    pub input_revert_seconds: u32,
    pub live_preview: bool,
    /// Percent per `brightness+`/`-` hotkey press (SPEC-HK-3).
    pub brightness_step: u32,
    pub contrast_step: u32,
    pub volume_step: u32,
    /// Show the on-screen indicator after a hotkey (SPEC-HK-6).
    pub show_osd: bool,
}

/// Allowed hotkey step sizes, in percent.
pub const STEP_RANGE: std::ops::RangeInclusive<u32> = 1..=25;

/// Allowed quiet period before a buffered write (SPEC-WR-2).
pub const DEBOUNCE_RANGE: std::ops::RangeInclusive<u32> = 150..=2000;
/// Allowed input revert timer; 0 turns it off (SPEC-IN).
pub const REVERT_RANGE: std::ops::RangeInclusive<u32> = 5..=60;

impl AppSettings {
    /// The settings rules, shared by the use cases and the settings file.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !DEBOUNCE_RANGE.contains(&self.debounce_ms) {
            return Err("debounce must be between 150 and 2000 milliseconds");
        }
        if self.input_revert_seconds != 0 && !REVERT_RANGE.contains(&self.input_revert_seconds) {
            return Err("input revert must be 0 or between 5 and 60 seconds");
        }
        if [self.brightness_step, self.contrast_step, self.volume_step]
            .iter()
            .any(|step| !STEP_RANGE.contains(step))
        {
            return Err("hotkey steps must be between 1 and 25 percent");
        }
        Ok(())
    }

    /// Step size for a steppable control, or `None` for other controls.
    pub fn step_for(&self, control: ControlKey) -> Option<u32> {
        match control {
            ControlKey::Brightness => Some(self.brightness_step),
            ControlKey::Contrast => Some(self.contrast_step),
            ControlKey::Volume => Some(self.volume_step),
            _ => None,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            debounce_ms: 400,
            confirm_input_change: true,
            input_revert_seconds: 10,
            live_preview: false,
            brightness_step: 5,
            contrast_step: 5,
            volume_step: 5,
            show_osd: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    EmptyMonitorId,
    UnknownControl(String),
    NormalizedValueOutOfRange(u32),
    NativeValueOutOfRange { value: u32, min: u32, max: u32 },
    InvalidNumericCapability(ControlKey),
    InvalidEnumCapability(ControlKey),
    EnumValueUnavailable { control: ControlKey, value: u32 },
    InvalidPresetName,
    InvalidHotkey(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMonitorId => f.write_str("monitor ID cannot be empty"),
            Self::UnknownControl(value) => write!(f, "unknown control: {value}"),
            Self::NormalizedValueOutOfRange(value) => {
                write!(f, "normalized value {value} is outside 0..=100")
            }
            Self::NativeValueOutOfRange { value, min, max } => {
                write!(f, "native value {value} is outside {min}..={max}")
            }
            Self::InvalidNumericCapability(key) => {
                write!(f, "{key} has invalid numeric capability data")
            }
            Self::InvalidEnumCapability(key) => {
                write!(f, "{key} is not an enum control")
            }
            Self::EnumValueUnavailable { control, value } => {
                write!(f, "enum value {value} is not available for {control}")
            }
            Self::InvalidPresetName => {
                f.write_str("preset name must be between 1 and 40 characters")
            }
            Self::InvalidHotkey(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_controls_round_trip_normalized_values() {
        let capability = ControlCapability {
            key: ControlKey::Brightness,
            native_min: 20,
            native_max: 100,
            enum_values: vec![],
            rate_limited: true,
        };
        for value in 0..=100 {
            let native = capability.normalized_to_native(value).unwrap();
            assert!(
                capability
                    .native_to_normalized(native)
                    .unwrap()
                    .abs_diff(value)
                    <= 1
            );
        }
    }

    #[test]
    fn enum_capability_rejects_unreported_values() {
        let capability = ControlCapability {
            key: ControlKey::Input,
            native_min: 0,
            native_max: 0,
            enum_values: vec![0x11, 0x0f, 0x31],
            rate_limited: true,
        };
        assert!(capability.validate_enum(0x31).is_ok());
        assert!(capability.validate_enum(0x99).is_err());
    }

    #[test]
    fn preset_names_are_trimmed_and_length_checked() {
        assert_eq!(validate_preset_name("  Day  ").unwrap(), "Day");
        assert!(validate_preset_name("   ").is_err());
        assert!(validate_preset_name(&"x".repeat(41)).is_err());
        assert!(validate_preset_name(&"x".repeat(40)).is_ok());
        assert!(preset_names_equal("Day", "dAY"));
    }

    #[test]
    fn gains_are_inert_only_under_a_fixed_colour_preset() {
        let monitor = MonitorId::new("m").unwrap();
        let entry = |control, value| PresetEntry {
            monitor: monitor.clone(),
            control,
            value,
        };
        let gain = entry(ControlKey::GainRed, ControlValue::Normalized(40));
        let mut preset = Preset {
            name: "p".into(),
            entries: vec![
                entry(ControlKey::ColorPreset, ControlValue::Enum(0x06)),
                gain.clone(),
            ],
        };
        assert!(preset.is_entry_inert(&gain));
        assert!(!preset.is_entry_inert(&preset.entries[0].clone()));
        preset.entries[0].value = ControlValue::Enum(0x0B);
        assert!(!preset.is_entry_inert(&gain));
        preset.entries.remove(0);
        assert!(!preset.is_entry_inert(&gain));
    }

    #[test]
    fn settings_rules_accept_the_defaults_and_reject_each_bad_value() {
        assert_eq!(AppSettings::default().validate(), Ok(()));
        let bad = [
            AppSettings {
                debounce_ms: 149,
                ..AppSettings::default()
            },
            AppSettings {
                debounce_ms: 2001,
                ..AppSettings::default()
            },
            AppSettings {
                input_revert_seconds: 4,
                ..AppSettings::default()
            },
            AppSettings {
                volume_step: 0,
                ..AppSettings::default()
            },
            AppSettings {
                brightness_step: 26,
                ..AppSettings::default()
            },
        ];
        for settings in bad {
            assert!(settings.validate().is_err(), "{settings:?}");
        }
        let off = AppSettings {
            input_revert_seconds: 0,
            ..AppSettings::default()
        };
        assert_eq!(off.validate(), Ok(()));
    }

    #[test]
    fn preset_apply_order_follows_the_spec() {
        let mut keys = [
            ControlKey::Volume,
            ControlKey::Contrast,
            ControlKey::Brightness,
            ControlKey::GainRed,
            ControlKey::ColorPreset,
            ControlKey::Input,
            ControlKey::Power,
        ];
        keys.sort_by_key(|key| key.preset_apply_rank());
        assert_eq!(keys[0], ControlKey::Power);
        assert_eq!(keys[1], ControlKey::Input);
        assert_eq!(keys[2], ControlKey::ColorPreset);
        assert_eq!(keys[3], ControlKey::GainRed);
        assert_eq!(keys[6], ControlKey::Volume);
    }
}
