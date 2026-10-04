//! Mapping between domain keys and Windows virtual-key codes and
//! `RegisterHotKey` modifier flags. Pure, so it is unit-tested.

use dispcontrol_domain::{Key, KeyCombo};

pub(crate) const MOD_ALT: u32 = 0x0001;
pub(crate) const MOD_CONTROL: u32 = 0x0002;
pub(crate) const MOD_SHIFT: u32 = 0x0004;
pub(crate) const MOD_WIN: u32 = 0x0008;
/// Without it, holding the keys sends `WM_HOTKEY` repeatedly.
pub(crate) const MOD_NOREPEAT: u32 = 0x4000;

const NAMED: [(Key, u16); 19] = [
    (Key::Up, 0x26),
    (Key::Down, 0x28),
    (Key::Left, 0x25),
    (Key::Right, 0x27),
    (Key::PageUp, 0x21),
    (Key::PageDown, 0x22),
    (Key::Home, 0x24),
    (Key::End, 0x23),
    (Key::Insert, 0x2D),
    (Key::Delete, 0x2E),
    (Key::Space, 0x20),
    (Key::Plus, 0xBB),
    (Key::Minus, 0xBD),
    (Key::Comma, 0xBC),
    (Key::Period, 0xBE),
    (Key::NumAdd, 0x6B),
    (Key::NumSubtract, 0x6D),
    (Key::NumMultiply, 0x6A),
    (Key::NumDivide, 0x6F),
];

pub(crate) fn virtual_key(key: Key) -> u16 {
    match key {
        Key::Letter(letter) => letter as u16,
        Key::Digit(digit) => 0x30 + u16::from(digit),
        Key::Function(number) => 0x6F + u16::from(number),
        Key::Numpad(digit) => 0x60 + u16::from(digit),
        named => NAMED
            .iter()
            .find(|(key, _)| *key == named)
            .map(|(_, code)| *code)
            .unwrap_or(0),
    }
}

/// The bindable key for a virtual-key code, or `None` for modifiers and
/// keys that cannot be bound.
pub(crate) fn key_from_virtual(code: u16) -> Option<Key> {
    match code {
        0x41..=0x5A => Some(Key::Letter(char::from(code as u8))),
        0x30..=0x39 => Some(Key::Digit((code - 0x30) as u8)),
        0x70..=0x87 => Some(Key::Function((code - 0x6F) as u8)),
        0x60..=0x69 => Some(Key::Numpad((code - 0x60) as u8)),
        _ => NAMED
            .iter()
            .find(|(_, named)| *named == code)
            .map(|(key, _)| *key),
    }
}

/// `RegisterHotKey` modifier flags for a combination.
pub(crate) fn modifier_flags(combo: &KeyCombo, repeats: bool) -> u32 {
    let mut flags = if repeats { 0 } else { MOD_NOREPEAT };
    for (held, flag) in [
        (combo.ctrl, MOD_CONTROL),
        (combo.alt, MOD_ALT),
        (combo.shift, MOD_SHIFT),
        (combo.win, MOD_WIN),
    ] {
        if held {
            flags |= flag;
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bindable_key_maps_to_a_distinct_code_and_back() {
        let mut keys: Vec<Key> = NAMED.iter().map(|(key, _)| *key).collect();
        keys.extend(('A'..='Z').map(Key::Letter));
        keys.extend((0..=9).map(Key::Digit));
        keys.extend((0..=9).map(Key::Numpad));
        keys.extend((1..=24).map(Key::Function));
        let mut codes = std::collections::HashSet::new();
        for key in keys {
            let code = virtual_key(key);
            assert!(codes.insert(code), "{key} shares code {code:#x}");
            assert_eq!(key_from_virtual(code), Some(key));
        }
        assert_eq!(virtual_key(Key::Function(1)), 0x70);
        assert_eq!(virtual_key(Key::Letter('B')), 0x42);
    }

    #[test]
    fn modifier_keys_and_unknown_codes_are_not_bindable() {
        for code in [0x10, 0x11, 0x12, 0x5B, 0x1B, 0x09, 0x0D] {
            assert_eq!(key_from_virtual(code), None, "{code:#x}");
        }
    }

    #[test]
    fn only_repeating_actions_omit_the_no_repeat_flag() {
        let combo: KeyCombo = "Ctrl+Shift+Win+Up".parse().unwrap();
        assert_eq!(
            modifier_flags(&combo, true),
            MOD_CONTROL | MOD_SHIFT | MOD_WIN
        );
        let f9: KeyCombo = "Alt+F9".parse().unwrap();
        assert_eq!(modifier_flags(&f9, false), MOD_ALT | MOD_NOREPEAT);
    }
}
