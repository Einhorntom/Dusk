#![forbid(unsafe_code)]

//! The Settings window's state. Anything that talks to a monitor is split
//! in two so the window never waits on DDC/CI (SPEC-MON-4): a `fetch_*`
//! function that runs on a worker thread against the `Api`, and an
//! `apply_*` / `show_*` method that installs its result on the UI thread.
//! The blocking methods (`refresh`, `select_monitor`, `apply_preset`, ...)
//! combine the two for tests and non-UI callers.

use std::sync::Arc;

pub mod text;

use dusk_app::{Api, ApplyReport, HotkeyOutcome, ImportSummary, UseCaseError};
use dusk_domain::{
    AppSettings, ControlKey, ControlReading, ControlValue, HotkeyAction, HotkeyBinding, KeyCombo,
    Monitor, MonitorId, Preset, PresetEntry, is_user_color_preset, preset_names_equal,
};

const CONTROLS: [ControlKey; 9] = [
    ControlKey::Brightness,
    ControlKey::Contrast,
    ControlKey::ColorPreset,
    ControlKey::GainRed,
    ControlKey::GainGreen,
    ControlKey::GainBlue,
    ControlKey::Input,
    ControlKey::Volume,
    ControlKey::Power,
];

/// Everything the window shows, read off the UI thread by `fetch_overview`.
pub struct Overview {
    monitors: Vec<Monitor>,
    selected: Option<MonitorId>,
    settings: AppSettings,
    controls: Vec<ControlReading>,
    presets: Vec<Preset>,
    matching_preset: Option<String>,
}

/// Reads the monitors, settings, the selected monitor's controls (keeping
/// `selected` if it is still connected) and the presets.
pub fn fetch_overview(
    api: &dyn Api,
    selected: Option<MonitorId>,
) -> Result<Overview, UseCaseError> {
    let monitors = api.list_monitors()?;
    let selected = selected
        .filter(|id| monitors.iter().any(|monitor| &monitor.id == id))
        .or_else(|| monitors.first().map(|monitor| monitor.id.clone()));
    let settings = api.settings()?;
    let controls = match &selected {
        Some(monitor) => read_controls(api, monitor)?,
        None => Vec::new(),
    };
    Ok(Overview {
        monitors,
        selected,
        settings,
        controls,
        presets: api.list_presets()?,
        matching_preset: fetch_matching_preset(api),
    })
}

/// The supported controls of `monitor`, in display order.
pub fn read_controls(
    api: &dyn Api,
    monitor: &MonitorId,
) -> Result<Vec<ControlReading>, UseCaseError> {
    let mut readings = Vec::new();
    for control in CONTROLS {
        match api.read(monitor, control) {
            Ok(reading) => readings.push(reading),
            Err(UseCaseError::UnsupportedControl(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(readings)
}

/// The preset matching the monitors' current state; reads every entry.
pub fn fetch_matching_preset(api: &dyn Api) -> Option<String> {
    api.matching_preset().unwrap_or(None)
}

/// What changed after a preset or a non-step hotkey ran.
pub struct AfterChange {
    monitor: Option<MonitorId>,
    controls: Result<Vec<ControlReading>, UseCaseError>,
    matching_preset: Option<String>,
}

/// Re-reads `monitor`'s controls and the matching preset.
pub fn fetch_after_change(api: &dyn Api, monitor: Option<MonitorId>) -> AfterChange {
    let controls = match &monitor {
        Some(monitor) => read_controls(api, monitor),
        None => Ok(Vec::new()),
    };
    AfterChange {
        monitor,
        controls,
        matching_preset: fetch_matching_preset(api),
    }
}

pub struct MonitorSettingsModel {
    api: Arc<dyn Api>,
    monitors: Vec<Monitor>,
    selected_monitor: Option<MonitorId>,
    selected_control: Option<ControlKey>,
    available_controls: Vec<ControlReading>,
    settings: AppSettings,
    presets: Vec<Preset>,
    matching_preset: Option<String>,
    hotkeys: Vec<HotkeyBinding>,
    last_error: Option<String>,
}

impl MonitorSettingsModel {
    pub fn new(api: Arc<dyn Api>) -> Self {
        Self {
            api,
            monitors: Vec::new(),
            selected_monitor: None,
            selected_control: None,
            available_controls: Vec::new(),
            settings: AppSettings::default(),
            presets: Vec::new(),
            matching_preset: None,
            hotkeys: Vec::new(),
            last_error: None,
        }
    }

    // ------------------------------------------------- monitors and values

    /// Blocking: `fetch_overview` + `apply_overview`.
    pub fn refresh(&mut self) -> Result<(), UseCaseError> {
        let overview = fetch_overview(self.api.as_ref(), self.selected_monitor.clone())?;
        self.apply_overview(overview);
        Ok(())
    }

    pub fn apply_overview(&mut self, overview: Overview) {
        self.monitors = overview.monitors;
        self.selected_monitor = overview.selected;
        self.settings = overview.settings;
        self.presets = overview.presets;
        self.matching_preset = overview.matching_preset;
        self.install_controls(overview.controls);
    }

    /// Blocking: `begin_select_monitor` + `read_controls` + `apply_controls`.
    pub fn select_monitor(&mut self, id: &MonitorId) -> Result<(), UseCaseError> {
        let monitor = self.begin_select_monitor(id)?;
        let controls = read_controls(self.api.as_ref(), &monitor)?;
        self.apply_controls(&monitor, controls);
        Ok(())
    }

    /// Selects `id` and clears its controls until they are read.
    pub fn begin_select_monitor(&mut self, id: &MonitorId) -> Result<MonitorId, UseCaseError> {
        if !self.monitors.iter().any(|monitor| &monitor.id == id) {
            return Err(UseCaseError::SettingsInvalid(
                "selected monitor is no longer available",
            ));
        }
        self.selected_monitor = Some(id.clone());
        self.selected_control = None;
        self.available_controls.clear();
        Ok(id.clone())
    }

    /// Installs `monitor`'s controls, unless another monitor was selected
    /// meanwhile. Returns whether they were installed.
    pub fn apply_controls(&mut self, monitor: &MonitorId, controls: Vec<ControlReading>) -> bool {
        if self.selected_monitor.as_ref() != Some(monitor) {
            return false;
        }
        self.install_controls(controls);
        true
    }

    pub fn select_control(&mut self, control: ControlKey) {
        self.selected_control = Some(control);
    }

    /// The selected monitor and control, for a write.
    pub fn selected_target(&self) -> Result<(MonitorId, ControlKey), UseCaseError> {
        let monitor = self
            .selected_monitor
            .clone()
            .ok_or(UseCaseError::SettingsInvalid("no monitor is selected"))?;
        let control = self
            .selected_control
            .ok_or(UseCaseError::SettingsInvalid("no control is selected"))?;
        Ok((monitor, control))
    }

    /// Blocking: `selected_target` + `Api::set` + `show_written_value`.
    pub fn set_selected_value(&mut self, value: ControlValue) -> Result<bool, UseCaseError> {
        let (monitor, control) = self.selected_target()?;
        let changed = self.api.set(&monitor, control, value)?;
        self.show_written_value(&monitor, control, value);
        Ok(changed)
    }

    /// Monitors are often busy or report the old value right after a
    /// write, so an accepted write is shown without reading it back.
    pub fn show_written_value(
        &mut self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) {
        if self.selected_monitor.as_ref() != Some(monitor) {
            return;
        }
        if let Some(existing) = self
            .available_controls
            .iter_mut()
            .find(|existing| existing.capability.key == control)
        {
            existing.value = value;
        }
    }

    /// Buffers a slider value (SPEC-WR-2): `app` writes it after the quiet
    /// period. Shown at once, like a hotkey step. No monitor I/O.
    pub fn adjust_selected_value(&mut self, value: ControlValue) -> Result<(), UseCaseError> {
        let (monitor, control) = self.selected_target()?;
        self.api.adjust(&monitor, control, value)?;
        self.show_written_value(&monitor, control, value);
        Ok(())
    }

    pub fn update_settings(&mut self, settings: AppSettings) -> Result<(), UseCaseError> {
        self.api.update_settings(settings.clone())?;
        self.settings = settings;
        Ok(())
    }

    // ------------------------------------------------------------- presets

    /// Blocking: the list plus `fetch_matching_preset`.
    pub fn refresh_presets(&mut self) -> Result<(), UseCaseError> {
        self.reload_presets()?;
        self.matching_preset = fetch_matching_preset(self.api.as_ref());
        Ok(())
    }

    /// Reloads the preset list from the settings file (no monitor I/O).
    /// The matching marker is kept if its preset still exists.
    pub fn reload_presets(&mut self) -> Result<(), UseCaseError> {
        self.presets = self.api.list_presets()?;
        if let Some(matching) = &self.matching_preset
            && !self
                .presets
                .iter()
                .any(|preset| preset_names_equal(&preset.name, matching))
        {
            self.matching_preset = None;
        }
        Ok(())
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn matching_preset(&self) -> Option<&str> {
        self.matching_preset.as_deref()
    }

    pub fn set_matching_preset(&mut self, matching: Option<String>) {
        self.matching_preset = matching;
    }

    /// Blocking: `Api::apply_preset` + `fetch_after_change` + `apply_after_change`.
    pub fn apply_preset(&mut self, name: &str) -> Result<ApplyReport, UseCaseError> {
        let report = self.api.apply_preset(name)?;
        let after = fetch_after_change(self.api.as_ref(), self.selected_monitor.clone());
        self.apply_after_change(after)?;
        Ok(report)
    }

    /// Installs re-read values and the matching preset after a change.
    pub fn apply_after_change(&mut self, after: AfterChange) -> Result<(), UseCaseError> {
        self.matching_preset = after.matching_preset;
        let controls = after.controls?;
        if let Some(monitor) = &after.monitor {
            self.apply_controls(monitor, controls);
        }
        Ok(())
    }

    /// Blocking: captures the selected monitor, then reloads the presets.
    pub fn save_current_as_preset(
        &mut self,
        name: &str,
        include_input: bool,
    ) -> Result<Preset, UseCaseError> {
        let monitor = self
            .selected_monitor
            .clone()
            .ok_or(UseCaseError::SettingsInvalid("no monitor is selected"))?;
        let preset = self.api.capture_preset(name, &monitor, include_input)?;
        self.refresh_presets()?;
        Ok(preset)
    }

    /// Also removes hotkeys that apply the preset; ask first if
    /// `hotkeys_using_preset` is not empty (SPEC-PRE-5).
    pub fn delete_preset(&mut self, name: &str) -> Result<(), UseCaseError> {
        self.api.delete_preset(name)?;
        self.reload_presets()?;
        self.refresh_hotkeys()
    }

    pub fn rename_preset(&mut self, name: &str, new_name: &str) -> Result<(), UseCaseError> {
        self.api.rename_preset(name, new_name)?;
        if self
            .matching_preset
            .as_deref()
            .is_some_and(|matching| preset_names_equal(matching, name))
        {
            self.matching_preset = Some(new_name.trim().to_owned());
        }
        self.reload_presets()?;
        self.refresh_hotkeys()
    }

    pub fn move_preset(&mut self, name: &str, offset: i32) -> Result<(), UseCaseError> {
        self.api.move_preset(name, offset)?;
        self.reload_presets()
    }

    pub fn set_preset_entry(&mut self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError> {
        self.api.set_preset_entry(name, entry)?;
        self.reload_presets()
    }

    pub fn remove_preset_entry(
        &mut self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        self.api.remove_preset_entry(name, monitor, control)?;
        self.reload_presets()
    }

    pub fn export_presets(&self) -> Result<String, UseCaseError> {
        self.api.export_presets()
    }

    pub fn import_presets(
        &mut self,
        text: &str,
        replace: bool,
    ) -> Result<ImportSummary, UseCaseError> {
        let summary = self.api.import_presets(text, replace)?;
        self.reload_presets()?;
        Ok(summary)
    }

    pub fn presets_location(&self) -> String {
        self.api.presets_location()
    }

    // ------------------------------------------------------------- hotkeys

    pub fn refresh_hotkeys(&mut self) -> Result<(), UseCaseError> {
        self.hotkeys = self.api.list_hotkeys()?;
        Ok(())
    }

    pub fn hotkeys(&self) -> &[HotkeyBinding] {
        &self.hotkeys
    }

    pub fn save_hotkey(&mut self, binding: HotkeyBinding) -> Result<(), UseCaseError> {
        self.api.save_hotkey(binding)?;
        self.refresh_hotkeys()
    }

    pub fn remove_hotkey(&mut self, keys: &KeyCombo) -> Result<(), UseCaseError> {
        self.api.remove_hotkey(keys)?;
        self.refresh_hotkeys()
    }

    /// The Add form's Action entries: steps, preset actions, the inputs the
    /// selected monitor offers, and power.
    pub fn hotkey_choices(&self) -> Vec<HotkeyAction> {
        let inputs: Vec<u32> = self
            .available_controls
            .iter()
            .find(|reading| reading.capability.key == ControlKey::Input)
            .map(|reading| reading.capability.enum_values.clone())
            .unwrap_or_default();
        text::hotkey_action_choices(&self.presets, &inputs)
    }

    /// Turns the Add form into a binding: the recorded keys, the chosen
    /// entry of `hotkey_choices`, and a monitor menu whose first entry is
    /// "All monitors". Returns a message for the user when it is incomplete.
    pub fn hotkey_from_form(
        &self,
        keys: &str,
        action: Option<usize>,
        monitor: Option<usize>,
    ) -> Result<HotkeyBinding, String> {
        if keys.trim().is_empty() {
            return Err("Click the Keys box and press the combination you want to use.".into());
        }
        let keys: KeyCombo = keys.parse().map_err(|error| format!("{error}"))?;
        let action = action
            .and_then(|index| self.hotkey_choices().into_iter().nth(index))
            .ok_or_else(|| "Choose what the hotkey does.".to_owned())?;
        let monitor = match monitor {
            Some(index) if index > 0 && action.uses_monitor() => self
                .monitors
                .get(index - 1)
                .map(|monitor| monitor.id.clone()),
            _ => None,
        };
        Ok(HotkeyBinding {
            keys,
            action,
            monitor,
        })
    }

    pub fn hotkeys_using_preset(&self, name: &str) -> Result<Vec<KeyCombo>, UseCaseError> {
        self.api.hotkeys_using_preset(name)
    }

    /// Blocking: runs a pressed hotkey and updates the shown values. The UI
    /// runs `Api::run_hotkey` on a worker and calls `show_hotkey_outcome`.
    pub fn run_hotkey(&mut self, keys: &KeyCombo) -> Result<HotkeyOutcome, UseCaseError> {
        let outcome = self.api.run_hotkey(keys)?;
        if self.show_hotkey_outcome(&outcome) {
            let after = fetch_after_change(self.api.as_ref(), self.selected_monitor.clone());
            // A monitor that was just switched off or away may not answer.
            let _ = self.apply_after_change(after);
        }
        Ok(outcome)
    }

    /// The application API, for calls made off the UI thread.
    pub fn api(&self) -> Arc<dyn Api> {
        self.api.clone()
    }

    /// Shows a hotkey's result without monitor I/O: a step updates the
    /// selected monitor's value at once (SPEC-WR-1). Returns true when the
    /// values must be re-read (`fetch_after_change`), after other actions.
    pub fn show_hotkey_outcome(&mut self, outcome: &HotkeyOutcome) -> bool {
        match outcome {
            HotkeyOutcome::Stepped { control, values } => {
                let selected = values
                    .iter()
                    .find(|(monitor, _)| Some(monitor) == self.selected_monitor.as_ref());
                if let Some((_, value)) = selected
                    && let Some(reading) = self
                        .available_controls
                        .iter_mut()
                        .find(|reading| reading.capability.key == *control)
                {
                    reading.value = ControlValue::Normalized(*value);
                }
                false
            }
            _ => true,
        }
    }

    // ---------------------------------------------------------------- view

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn selected_monitor(&self) -> Option<&MonitorId> {
        self.selected_monitor.as_ref()
    }

    pub fn selected_control(&self) -> Option<ControlKey> {
        self.selected_control
    }

    pub fn controls(&self) -> &[ControlReading] {
        &self.available_controls
    }

    /// The selected monitor's colour preset when it is a factory mode (e.g.
    /// 7500 K). The red/green/blue gains belong to the user profiles, so in
    /// such a mode they are not in effect, and the monitor may still report
    /// a user profile's values.
    pub fn gains_unused_in(&self) -> Option<u32> {
        self.available_controls
            .iter()
            .find(|reading| reading.capability.key == ControlKey::ColorPreset)
            .and_then(|reading| match reading.value {
                ControlValue::Enum(mode) if !is_user_color_preset(mode) => Some(mode),
                _ => None,
            })
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn report_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Installs readings and keeps the selected control if still present.
    fn install_controls(&mut self, controls: Vec<ControlReading>) {
        self.available_controls = controls;
        if self.selected_monitor.is_none() {
            self.selected_control = None;
            return;
        }
        if !self.available_controls.is_empty()
            && !self
                .available_controls
                .iter()
                .any(|reading| Some(reading.capability.key) == self.selected_control)
        {
            self.selected_control = Some(self.available_controls[0].capability.key);
        }
    }
}
