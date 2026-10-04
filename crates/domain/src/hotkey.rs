//! Global hotkey bindings (SPEC-HK): key combinations, actions and their
//! text forms as stored in the config file, e.g. `Ctrl+Alt+Up = brightness+`.

use std::fmt;
use std::str::FromStr;

use crate::{ControlKey, DomainError, MonitorId, validate_preset_name};

/// A key that can be bound, independent of any OS key-code numbering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    /// `A`-`Z`.
    Letter(char),
    /// `0`-`9` on the main keyboard.
    Digit(u8),
    /// `F1`-`F24`.
    Function(u8),
    /// `Num0`-`Num9`.
    Numpad(u8),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    Space,
    Plus,
    Minus,
    Comma,
    Period,
    NumAdd,
    NumSubtract,
    NumMultiply,
    NumDivide,
}

const NAMED_KEYS: [(Key, &str); 19] = [
    (Key::Up, "Up"),
    (Key::Down, "Down"),
    (Key::Left, "Left"),
    (Key::Right, "Right"),
    (Key::PageUp, "PageUp"),
    (Key::PageDown, "PageDown"),
    (Key::Home, "Home"),
    (Key::End, "End"),
    (Key::Insert, "Insert"),
    (Key::Delete, "Delete"),
    (Key::Space, "Space"),
    (Key::Plus, "Plus"),
    (Key::Minus, "Minus"),
    (Key::Comma, "Comma"),
    (Key::Period, "Period"),
    (Key::NumAdd, "NumAdd"),
    (Key::NumSubtract, "NumSubtract"),
    (Key::NumMultiply, "NumMultiply"),
    (Key::NumDivide, "NumDivide"),
];

impl Key {
    pub fn is_function_key(self) -> bool {
        matches!(self, Self::Function(_))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(letter) => write!(f, "{letter}"),
            Self::Digit(digit) => write!(f, "{digit}"),
            Self::Function(number) => write!(f, "F{number}"),
            Self::Numpad(digit) => write!(f, "Num{digit}"),
            named => {
                let name = NAMED_KEYS
                    .iter()
                    .find(|(key, _)| key == named)
                    .map(|(_, name)| *name)
                    .unwrap_or("?");
                f.write_str(name)
            }
        }
    }
}

impl FromStr for Key {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || DomainError::InvalidHotkey(format!("unknown key '{text}'"));
        let mut chars = text.chars();
        if let (Some(single), None) = (chars.next(), chars.next()) {
            return match single.to_ascii_uppercase() {
                letter @ 'A'..='Z' => Ok(Self::Letter(letter)),
                digit @ '0'..='9' => Ok(Self::Digit(digit as u8 - b'0')),
                _ => Err(invalid()),
            };
        }
        let number = |prefix: &str, range: std::ops::RangeInclusive<u8>| {
            text.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .and_then(|_| text[prefix.len()..].parse::<u8>().ok())
                .filter(|number| range.contains(number))
        };
        if let Some(number) = number("F", 1..=24) {
            return Ok(Self::Function(number));
        }
        if let Some(digit) = number("Num", 0..=9) {
            return Ok(Self::Numpad(digit));
        }
        NAMED_KEYS
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(text))
            .map(|(key, _)| *key)
            .ok_or_else(invalid)
    }
}

/// Modifiers plus one key, e.g. `Ctrl+Alt+Up`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

impl KeyCombo {
    /// SPEC-HK-2: a combination needs Ctrl, Alt or Win unless it is a function
    /// key. Shift alone is not enough, because `Shift+A` would take over typing.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.ctrl || self.alt || self.win || self.key.is_function_key() {
            Ok(())
        } else {
            Err(DomainError::InvalidHotkey(format!(
                "{self} needs Ctrl, Alt or Win (only function keys may be used alone)"
            )))
        }
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.win, "Win+"),
        ] {
            if held {
                f.write_str(name)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl FromStr for KeyCombo {
    type Err = DomainError;

    /// Parses `Ctrl+Alt+Up`; names are case-insensitive and modifiers may come
    /// in any order. The result is validated.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key_name = parts.pop().unwrap_or_default();
        if key_name.is_empty() {
            return Err(DomainError::InvalidHotkey(format!(
                "'{text}' has no key; write the + key as Plus"
            )));
        }
        let mut combo = KeyCombo {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: key_name.parse()?,
        };
        for modifier in parts {
            let slot = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut combo.ctrl,
                "alt" => &mut combo.alt,
                "shift" => &mut combo.shift,
                "win" | "windows" => &mut combo.win,
                _ => {
                    return Err(DomainError::InvalidHotkey(format!(
                        "unknown modifier '{modifier}' in '{text}'"
                    )));
                }
            };
            *slot = true;
        }
        combo.validate()?;
        Ok(combo)
    }
}

/// What a hotkey does (SPEC-HK-1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotkeyAction {
    ApplyPreset(String),
    NextPreset,
    PreviousPreset,
    /// Raise or lower brightness, contrast or volume by its step size.
    Step {
        control: ControlKey,
        up: bool,
    },
    SetInput(u32),
    TogglePower,
}

/// Controls that `+`/`-` hotkeys can step.
pub const STEPPABLE_CONTROLS: [ControlKey; 3] = [
    ControlKey::Brightness,
    ControlKey::Contrast,
    ControlKey::Volume,
];

impl HotkeyAction {
    /// Holding the keys repeats the action only for steps (SPEC-HK-4); the
    /// others run once per press.
    pub fn repeats(&self) -> bool {
        matches!(self, Self::Step { .. })
    }

    pub fn preset_name(&self) -> Option<&str> {
        match self {
            Self::ApplyPreset(name) => Some(name),
            _ => None,
        }
    }
}

impl fmt::Display for HotkeyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApplyPreset(name) => write!(f, "preset:{name}"),
            Self::NextPreset => f.write_str("preset-next"),
            Self::PreviousPreset => f.write_str("preset-prev"),
            Self::Step { control, up } => write!(f, "{control}{}", if *up { '+' } else { '-' }),
            Self::SetInput(value) => write!(f, "input:0x{value:02X}"),
            Self::TogglePower => f.write_str("power-toggle"),
        }
    }
}

impl FromStr for HotkeyAction {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let invalid = |message: String| DomainError::InvalidHotkey(message);
        if let Some(name) = text.strip_prefix("preset:") {
            return Ok(Self::ApplyPreset(validate_preset_name(name)?));
        }
        if let Some(value) = text.strip_prefix("input:") {
            let value = value.trim();
            let parsed = match value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
            {
                Some(hex) => u32::from_str_radix(hex, 16),
                None => value.parse(),
            };
            return parsed
                .map(Self::SetInput)
                .map_err(|_| invalid(format!("invalid input value '{value}'")));
        }
        match text {
            "preset-next" => return Ok(Self::NextPreset),
            "preset-prev" => return Ok(Self::PreviousPreset),
            "power-toggle" => return Ok(Self::TogglePower),
            "schedule-toggle" => {
                return Err(invalid(
                    "schedule-toggle is available once schedules exist (v2)".into(),
                ));
            }
            _ => {}
        }
        let (name, up) = match (text.strip_suffix('+'), text.strip_suffix('-')) {
            (Some(name), _) => (name, true),
            (_, Some(name)) => (name, false),
            _ => return Err(invalid(format!("unknown hotkey action '{text}'"))),
        };
        let control: ControlKey = name.parse()?;
        if !STEPPABLE_CONTROLS.contains(&control) {
            return Err(invalid(format!("{control} cannot be stepped with +/-")));
        }
        Ok(Self::Step { control, up })
    }
}

/// One configured hotkey. Without a monitor it acts on every connected
/// monitor that supports the action (SPEC-HK-2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotkeyBinding {
    pub keys: KeyCombo,
    pub action: HotkeyAction,
    pub monitor: Option<MonitorId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_combos_round_trip_in_canonical_order() {
        let combo: KeyCombo = "shift+ctrl+ALT+up".parse().unwrap();
        assert_eq!(combo.to_string(), "Ctrl+Alt+Shift+Up");
        for text in [
            "Ctrl+Alt+B",
            "Win+7",
            "F9",
            "Shift+F12",
            "Ctrl+Num5",
            "Alt+PageDown",
            "Ctrl+Plus",
            "Ctrl+NumSubtract",
        ] {
            let combo: KeyCombo = text.parse().unwrap();
            assert_eq!(combo.to_string(), text);
        }
    }

    #[test]
    fn combos_need_a_real_modifier_unless_they_use_a_function_key() {
        assert!("Up".parse::<KeyCombo>().is_err());
        assert!("Shift+A".parse::<KeyCombo>().is_err());
        assert!("F5".parse::<KeyCombo>().is_ok());
        assert!("Ctrl+".parse::<KeyCombo>().is_err());
        assert!("Hyper+A".parse::<KeyCombo>().is_err());
        assert!("Ctrl+F25".parse::<KeyCombo>().is_err());
        assert!("Ctrl+Num10".parse::<KeyCombo>().is_err());
        assert!("Ctrl+Escape".parse::<KeyCombo>().is_err());
    }

    #[test]
    fn actions_round_trip() {
        for text in [
            "preset:Night mode",
            "preset-next",
            "preset-prev",
            "brightness+",
            "contrast-",
            "volume+",
            "input:0x11",
            "power-toggle",
        ] {
            let action: HotkeyAction = text.parse().unwrap();
            assert_eq!(action.to_string(), text);
        }
        assert_eq!(
            "input:49".parse::<HotkeyAction>().unwrap(),
            HotkeyAction::SetInput(0x31)
        );
    }

    #[test]
    fn invalid_actions_are_rejected_with_a_reason() {
        assert!("preset:".parse::<HotkeyAction>().is_err());
        assert!("color-preset+".parse::<HotkeyAction>().is_err());
        assert!("sharpness+".parse::<HotkeyAction>().is_err());
        assert!("input:HDMI".parse::<HotkeyAction>().is_err());
        let schedule = "schedule-toggle".parse::<HotkeyAction>().unwrap_err();
        assert!(schedule.to_string().contains("v2"));
    }

    #[test]
    fn only_steps_repeat_while_held() {
        assert!("brightness+".parse::<HotkeyAction>().unwrap().repeats());
        assert!(!"preset-next".parse::<HotkeyAction>().unwrap().repeats());
        assert!(!"input:0x11".parse::<HotkeyAction>().unwrap().repeats());
    }
}
