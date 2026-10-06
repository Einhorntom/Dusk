//! Every child-window ID of the default Settings window, and the one decoder
//! that turns a `WM_COMMAND`/`WM_HSCROLL` ID back into the control it names.
//!
//! Repeated controls (one per monitor control, preset or preset entry) get a
//! block of `ROW_LIMIT` IDs. The tests check that every ID decodes back to its
//! own control and that no two controls share an ID, so a new block cannot
//! silently overlap an existing one.

/// Most rows a repeated control can have.
pub(crate) const ROW_LIMIT: usize = 99;

const NAV_BASE: u16 = 300;
const SLIDER_BASE: u16 = 1000;
const VALUE_LABEL_BASE: u16 = 1100;
const COMBO_BASE: u16 = 1200;
const REVERT_SLIDER: u16 = 1501;
const REVERT_VALUE: u16 = 1601;
const INLINE_EDIT: u16 = 1900;
const CONFIRM_TOGGLE: u16 = 2000;
const LOG_TOGGLE: u16 = 2001;
const AUTOSTART_TOGGLE: u16 = 2002;
const PRESET_NAME: u16 = 2050;
const PRESET_SAVE: u16 = 2051;
const PRESET_SAVE_INPUT: u16 = 2052;
const PRESET_APPLY_BASE: u16 = 2100;
const PRESET_RENAME_BASE: u16 = 2200;
const PRESET_UP_BASE: u16 = 2300;
const PRESET_DOWN_BASE: u16 = 2400;
const PRESET_DELETE_BASE: u16 = 2500;
const ENTRY_VALUE_BASE: u16 = 2600;
const ENTRY_SET_BASE: u16 = 2700;
const ENTRY_REMOVE_BASE: u16 = 2800;
const PRESET_EDIT_BASE: u16 = 2900;
const PRESET_BACK: u16 = 3001;
const PRESET_EXPORT: u16 = 3002;
const PRESET_IMPORT: u16 = 3003;
const PRESET_FOLDER: u16 = 3004;
const HOTKEY_REMOVE_BASE: u16 = 3100;
const HOTKEY_KEYS: u16 = 3201;
const HOTKEY_ACTION: u16 = 3202;
const HOTKEY_MONITOR: u16 = 3203;
const HOTKEY_ADD: u16 = 3204;
const OSD_TOGGLE: u16 = 3205;
const STEP_SLIDER_BASE: u16 = 3300;
const STEP_VALUE_BASE: u16 = 3400;
/// The debounce slider reuses the shared `DEBOUNCE_ID`; its label sits 100 above.
const DEBOUNCE_VALUE_OFFSET: u16 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RowButton {
    Apply,
    Edit,
    Rename,
    Up,
    Down,
    Delete,
}

impl RowButton {
    pub(crate) const ALL: [RowButton; 6] = [
        Self::Apply,
        Self::Edit,
        Self::Rename,
        Self::Up,
        Self::Down,
        Self::Delete,
    ];

    fn base(self) -> u16 {
        match self {
            Self::Apply => PRESET_APPLY_BASE,
            Self::Edit => PRESET_EDIT_BASE,
            Self::Rename => PRESET_RENAME_BASE,
            Self::Up => PRESET_UP_BASE,
            Self::Down => PRESET_DOWN_BASE,
            Self::Delete => PRESET_DELETE_BASE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EntryButton {
    Set,
    Remove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Control {
    Nav(usize),
    Slider(usize),
    ValueLabel(usize),
    Combo(usize),
    RevertSlider,
    RevertValue,
    DebounceValue,
    InlineEdit,
    ConfirmToggle,
    LogToggle,
    AutostartToggle,
    PresetName,
    PresetSave,
    PresetSaveInput,
    PresetRow(RowButton, usize),
    EntryValue(usize),
    Entry(EntryButton, usize),
    PresetBack,
    PresetExport,
    PresetImport,
    PresetFolder,
    HotkeyRemove(usize),
    HotkeyKeys,
    HotkeyAction,
    HotkeyMonitor,
    HotkeyAdd,
    OsdToggle,
    /// Hotkey step size for `STEPPABLE_CONTROLS[index]`.
    StepSlider(usize),
    StepValue(usize),
}

const FIXED: [(Control, u16); 19] = [
    (Control::RevertSlider, REVERT_SLIDER),
    (Control::RevertValue, REVERT_VALUE),
    (
        Control::DebounceValue,
        super::DEBOUNCE_ID + DEBOUNCE_VALUE_OFFSET,
    ),
    (Control::InlineEdit, INLINE_EDIT),
    (Control::ConfirmToggle, CONFIRM_TOGGLE),
    (Control::LogToggle, LOG_TOGGLE),
    (Control::AutostartToggle, AUTOSTART_TOGGLE),
    (Control::PresetName, PRESET_NAME),
    (Control::PresetSave, PRESET_SAVE),
    (Control::PresetSaveInput, PRESET_SAVE_INPUT),
    (Control::PresetBack, PRESET_BACK),
    (Control::PresetExport, PRESET_EXPORT),
    (Control::PresetImport, PRESET_IMPORT),
    (Control::PresetFolder, PRESET_FOLDER),
    (Control::HotkeyKeys, HOTKEY_KEYS),
    (Control::HotkeyAction, HOTKEY_ACTION),
    (Control::HotkeyMonitor, HOTKEY_MONITOR),
    (Control::HotkeyAdd, HOTKEY_ADD),
    (Control::OsdToggle, OSD_TOGGLE),
];

/// Builds the control for row `index` of an ID block.
type MakeControl = fn(usize) -> Control;

fn block(base: u16, index: usize) -> u16 {
    assert!(index < ROW_LIMIT, "row {index} exceeds the ID block");
    base + index as u16
}

impl Control {
    pub(crate) fn id(self) -> u16 {
        match self {
            Self::Nav(index) => block(NAV_BASE, index),
            Self::Slider(index) => block(SLIDER_BASE, index),
            Self::ValueLabel(index) => block(VALUE_LABEL_BASE, index),
            Self::Combo(index) => block(COMBO_BASE, index),
            Self::PresetRow(button, index) => block(button.base(), index),
            Self::EntryValue(index) => block(ENTRY_VALUE_BASE, index),
            Self::Entry(EntryButton::Set, index) => block(ENTRY_SET_BASE, index),
            Self::Entry(EntryButton::Remove, index) => block(ENTRY_REMOVE_BASE, index),
            Self::HotkeyRemove(index) => block(HOTKEY_REMOVE_BASE, index),
            Self::StepSlider(index) => block(STEP_SLIDER_BASE, index),
            Self::StepValue(index) => block(STEP_VALUE_BASE, index),
            fixed => {
                FIXED
                    .iter()
                    .find(|(control, _)| *control == fixed)
                    .expect("every fixed control has an ID")
                    .1
            }
        }
    }

    pub(crate) fn from_id(id: u16) -> Option<Self> {
        if let Some((control, _)) = FIXED.iter().find(|(_, fixed)| *fixed == id) {
            return Some(*control);
        }
        let row = |base: u16| {
            (base..base + ROW_LIMIT as u16)
                .contains(&id)
                .then(|| usize::from(id - base))
        };
        let blocks: [(u16, MakeControl); 10] = [
            (NAV_BASE, Control::Nav),
            (SLIDER_BASE, Control::Slider),
            (VALUE_LABEL_BASE, Control::ValueLabel),
            (COMBO_BASE, Control::Combo),
            (ENTRY_VALUE_BASE, Control::EntryValue),
            (ENTRY_SET_BASE, |index| {
                Control::Entry(EntryButton::Set, index)
            }),
            (ENTRY_REMOVE_BASE, |index| {
                Control::Entry(EntryButton::Remove, index)
            }),
            (HOTKEY_REMOVE_BASE, Control::HotkeyRemove),
            (STEP_SLIDER_BASE, Control::StepSlider),
            (STEP_VALUE_BASE, Control::StepValue),
        ];
        for (base, make) in blocks {
            if let Some(index) = row(base) {
                return Some(make(index));
            }
        }
        RowButton::ALL
            .iter()
            .find_map(|button| row(button.base()).map(|index| Control::PresetRow(*button, index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn every_control() -> Vec<Control> {
        let mut all: Vec<Control> = FIXED.iter().map(|(control, _)| *control).collect();
        for index in 0..ROW_LIMIT {
            all.extend([
                Control::Nav(index),
                Control::Slider(index),
                Control::ValueLabel(index),
                Control::Combo(index),
                Control::EntryValue(index),
                Control::Entry(EntryButton::Set, index),
                Control::Entry(EntryButton::Remove, index),
                Control::HotkeyRemove(index),
                Control::StepSlider(index),
                Control::StepValue(index),
            ]);
            all.extend(
                RowButton::ALL
                    .iter()
                    .map(|button| Control::PresetRow(*button, index)),
            );
        }
        all.dedup();
        all
    }

    #[test]
    fn every_id_decodes_back_to_its_own_control() {
        for control in every_control() {
            assert_eq!(Control::from_id(control.id()), Some(control), "{control:?}");
        }
    }

    #[test]
    fn no_two_controls_share_an_id_or_a_window_level_id() {
        let mut seen: HashMap<u16, Control> = HashMap::new();
        for control in every_control() {
            if let Some(previous) = seen.insert(control.id(), control) {
                assert_eq!(previous, control, "ID {} is used twice", control.id());
            }
        }
        let window_level = [
            super::super::MONITOR_ID,
            super::super::REFRESH_ID,
            super::super::DEBOUNCE_ID,
            super::super::STATUS_ID,
        ];
        for id in window_level {
            assert!(
                Control::from_id(id).is_none(),
                "window-level ID {id} collides"
            );
        }
    }

    #[test]
    fn decoding_is_unambiguous_across_the_whole_id_space() {
        for id in 0..=u16::MAX {
            if let Some(control) = Control::from_id(id) {
                assert_eq!(control.id(), id, "{id} decodes to {control:?}");
            }
        }
    }

    #[test]
    fn the_preset_edit_button_routes_to_the_preset_row() {
        // Regression: Edit IDs were once captured by the entry-button range.
        assert_eq!(
            Control::from_id(Control::PresetRow(RowButton::Edit, 3).id()),
            Some(Control::PresetRow(RowButton::Edit, 3))
        );
    }
}
