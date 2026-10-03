#![forbid(unsafe_code)]

use std::sync::Arc;

use dispcontrol_app::{Api, UseCaseError};
use dispcontrol_domain::{
    AppSettings, ControlKey, ControlReading, ControlValue, Monitor, MonitorId,
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
