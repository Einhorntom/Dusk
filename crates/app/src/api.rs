//! The application API, split by concern; `Api` is all of it. The
//! daemon serves it over IPC and the Settings window uses it directly.

use crate::*;
use std::time::Duration;

use dusk_domain::{
    AppSettings, ControlKey, ControlReading, ControlValue, HotkeyBinding, KeyCombo, Monitor,
    MonitorId, Preset, PresetEntry,
};

/// Discovering monitors and reading or changing their controls (SPEC-CTL, SPEC-WR).
pub trait ControlApi: Send + Sync {
    fn list_monitors(&self) -> Result<Vec<Monitor>, UseCaseError>;
    fn read(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<ControlReading, UseCaseError>;
    fn set(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<bool, UseCaseError>;
    fn adjust(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<(), UseCaseError>;
    fn flush_pending_adjustments(&self) -> Result<(usize, Option<Duration>), UseCaseError>;
}

/// Application settings (SPEC-DAT).
pub trait SettingsApi: Send + Sync {
    fn settings(&self) -> Result<AppSettings, UseCaseError>;
    fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError>;
}

/// Named monitor configurations (SPEC-PRE).
pub trait PresetApi: Send + Sync {
    fn list_presets(&self) -> Result<Vec<Preset>, UseCaseError>;
    fn capture_preset(
        &self,
        name: &str,
        monitor: &MonitorId,
        include_input: bool,
    ) -> Result<Preset, UseCaseError>;
    fn delete_preset(&self, name: &str) -> Result<(), UseCaseError>;
    fn rename_preset(&self, name: &str, new_name: &str) -> Result<(), UseCaseError>;
    fn move_preset(&self, name: &str, offset: i32) -> Result<(), UseCaseError>;
    fn apply_preset(&self, name: &str) -> Result<ApplyReport, UseCaseError>;
    fn cycle_preset(&self, forward: bool) -> Result<ApplyReport, UseCaseError>;
    fn matching_preset(&self) -> Result<Option<String>, UseCaseError>;
    fn set_preset_entry(&self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError>;
    fn remove_preset_entry(
        &self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError>;
    fn export_presets(&self) -> Result<String, UseCaseError>;
    fn import_presets(&self, text: &str, replace: bool) -> Result<ImportSummary, UseCaseError>;
    fn presets_location(&self) -> String;
}

/// Global hotkey bindings and running them (SPEC-HK).
pub trait HotkeyApi: Send + Sync {
    fn list_hotkeys(&self) -> Result<Vec<HotkeyBinding>, UseCaseError>;
    fn save_hotkey(&self, binding: HotkeyBinding) -> Result<(), UseCaseError>;
    fn remove_hotkey(&self, keys: &KeyCombo) -> Result<(), UseCaseError>;
    fn hotkeys_using_preset(&self, name: &str) -> Result<Vec<KeyCombo>, UseCaseError>;
    fn run_hotkey(&self, keys: &KeyCombo) -> Result<HotkeyOutcome, UseCaseError>;
}

/// Everything a full client such as the Settings window uses. Clients that
/// need less depend on the narrower traits.
pub trait Api: ControlApi + SettingsApi + PresetApi + HotkeyApi {}

impl<T: ControlApi + SettingsApi + PresetApi + HotkeyApi> Api for T {}
