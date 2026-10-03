#![forbid(unsafe_code)]

use std::sync::Arc;

use dispcontrol_app::{Api, ApplyReport, ImportSummary, UseCaseError};
use dispcontrol_domain::{
    AppSettings, ControlKey, ControlReading, ControlValue, Monitor, MonitorId, Preset, PresetEntry,
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

pub struct MonitorSettingsModel {
    api: Arc<dyn Api>,
    monitors: Vec<Monitor>,
    selected_monitor: Option<MonitorId>,
    selected_control: Option<ControlKey>,
    available_controls: Vec<ControlReading>,
    settings: AppSettings,
    presets: Vec<Preset>,
    matching_preset: Option<String>,
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
            last_error: None,
        }
    }

    pub fn refresh(&mut self) -> Result<(), UseCaseError> {
        self.monitors = self.api.list_monitors()?;
        if !self
            .monitors
            .iter()
            .any(|monitor| Some(&monitor.id) == self.selected_monitor.as_ref())
        {
            self.selected_monitor = self.monitors.first().map(|monitor| monitor.id.clone());
        }
        self.settings = self.api.settings()?;
        self.refresh_controls()
    }

    pub fn select_monitor(&mut self, id: &MonitorId) -> Result<(), UseCaseError> {
        if !self.monitors.iter().any(|monitor| &monitor.id == id) {
            return Err(UseCaseError::SettingsInvalid(
                "selected monitor is no longer available",
            ));
        }
        self.selected_monitor = Some(id.clone());
        self.selected_control = None;
        self.refresh_controls()
    }

    pub fn select_control(&mut self, control: ControlKey) {
        self.selected_control = Some(control);
    }

    pub fn set_selected_value(&mut self, value: ControlValue) -> Result<bool, UseCaseError> {
        let monitor = self
            .selected_monitor
            .as_ref()
            .ok_or(UseCaseError::SettingsInvalid("no monitor is selected"))?;
        let control = self
            .selected_control
            .ok_or(UseCaseError::SettingsInvalid("no control is selected"))?;
        let changed = self.api.set(monitor, control, value)?;
        // Monitors are often busy or report the old value right after a
        // write, so trust the accepted write instead of reading it back.
        if let Some(existing) = self
            .available_controls
            .iter_mut()
            .find(|existing| existing.capability.key == control)
        {
            existing.value = value;
        }
        Ok(changed)
    }

    pub fn adjust_selected_value(&self, value: ControlValue) -> Result<(), UseCaseError> {
        let monitor = self
            .selected_monitor
            .as_ref()
            .ok_or(UseCaseError::SettingsInvalid("no monitor is selected"))?;
        let control = self
            .selected_control
            .ok_or(UseCaseError::SettingsInvalid("no control is selected"))?;
        self.api.adjust(monitor, control, value)
    }

    pub fn flush_pending_adjustments(
        &mut self,
    ) -> Result<(usize, Option<std::time::Duration>), UseCaseError> {
        let (committed, next_wake) = self.api.flush_pending_adjustments()?;
        if committed > 0 {
            self.refresh_selected_control()?;
        }
        Ok((committed, next_wake))
    }

    pub fn update_settings(&mut self, settings: AppSettings) -> Result<(), UseCaseError> {
        self.api.update_settings(settings.clone())?;
        self.settings = settings;
        Ok(())
    }

    pub fn refresh_presets(&mut self) -> Result<(), UseCaseError> {
        self.presets = self.api.list_presets()?;
        self.matching_preset = self.api.matching_preset().unwrap_or(None);
        Ok(())
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn matching_preset(&self) -> Option<&str> {
        self.matching_preset.as_deref()
    }

    pub fn apply_preset(&mut self, name: &str) -> Result<ApplyReport, UseCaseError> {
        let report = self.api.apply_preset(name)?;
        self.after_preset_change()?;
        Ok(report)
    }

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

    pub fn delete_preset(&mut self, name: &str) -> Result<(), UseCaseError> {
        self.api.delete_preset(name)?;
        self.refresh_presets()
    }

    pub fn rename_preset(&mut self, name: &str, new_name: &str) -> Result<(), UseCaseError> {
        self.api.rename_preset(name, new_name)?;
        self.refresh_presets()
    }

    pub fn move_preset(&mut self, name: &str, offset: i32) -> Result<(), UseCaseError> {
        self.api.move_preset(name, offset)?;
        self.refresh_presets()
    }

    pub fn set_preset_entry(&mut self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError> {
        self.api.set_preset_entry(name, entry)?;
        self.refresh_presets()
    }

    pub fn remove_preset_entry(
        &mut self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        self.api.remove_preset_entry(name, monitor, control)?;
        self.refresh_presets()
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
        self.refresh_presets()?;
        Ok(summary)
    }

    pub fn presets_location(&self) -> String {
        self.api.presets_location()
    }

    fn after_preset_change(&mut self) -> Result<(), UseCaseError> {
        self.refresh_controls()?;
        self.matching_preset = self.api.matching_preset().unwrap_or(None);
        Ok(())
    }

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

    fn refresh_controls(&mut self) -> Result<(), UseCaseError> {
        self.available_controls.clear();
        let Some(monitor) = self.selected_monitor.as_ref() else {
            self.selected_control = None;
            return Ok(());
        };

        for control in CONTROLS {
            match self.api.read(monitor, control) {
                Ok(reading) => self.available_controls.push(reading),
                Err(UseCaseError::UnsupportedControl(_)) => {}
                Err(error) => return Err(error),
            }
        }
        if !self.available_controls.is_empty()
            && !self
                .available_controls
                .iter()
                .any(|reading| Some(reading.capability.key) == self.selected_control)
        {
            self.selected_control = Some(self.available_controls[0].capability.key);
        }
        Ok(())
    }

    fn refresh_selected_control(&mut self) -> Result<(), UseCaseError> {
        let (Some(monitor), Some(control)) =
            (self.selected_monitor.as_ref(), self.selected_control)
        else {
            return Ok(());
        };
        match self.api.read(monitor, control) {
            Ok(reading) => {
                if let Some(existing) = self
                    .available_controls
                    .iter_mut()
                    .find(|existing| existing.capability.key == control)
                {
                    *existing = reading;
                } else {
                    self.available_controls.push(reading);
                }
            }
            Err(UseCaseError::UnsupportedControl(_)) => {
                self.available_controls
                    .retain(|reading| reading.capability.key != control);
                self.selected_control = self
                    .available_controls
                    .first()
                    .map(|reading| reading.capability.key);
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
}
