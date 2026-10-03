#![forbid(unsafe_code)]

use std::fmt;

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
pub struct AppSettings {
    pub debounce_ms: u32,
    pub confirm_input_change: bool,
    pub input_revert_seconds: u32,
    pub live_preview: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            debounce_ms: 400,
            confirm_input_change: true,
            input_revert_seconds: 10,
            live_preview: false,
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
        };
        assert!(capability.validate_enum(0x31).is_ok());
        assert!(capability.validate_enum(0x99).is_err());
    }
}
