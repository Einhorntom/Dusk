//! Ports: what the use cases need from the outside world (monitors,
//! storage, prompts, time). Adapters implement them.

use crate::*;
use std::time::{Duration, Instant};

use dusk_domain::{AppSettings, ControlCapability, ControlKey, Monitor, MonitorId, Preset};

pub trait PresetRepository: Send + Sync {
    fn load_presets(&self) -> Result<Vec<Preset>, BackendError>;
    fn save_presets(&self, presets: &[Preset]) -> Result<(), BackendError>;
    /// Encodes presets as a standalone, human-editable text document.
    fn export_text(&self, _presets: &[Preset]) -> Result<String, BackendError> {
        Err(BackendError::Failed(
            "preset export is not supported".into(),
        ))
    }
    /// Parses and validates a document produced by `export_text`.
    fn parse_text(&self, _text: &str) -> Result<Vec<Preset>, BackendError> {
        Err(BackendError::Failed(
            "preset import is not supported".into(),
        ))
    }
    /// Human-readable storage location, if any.
    fn location(&self) -> String {
        String::new()
    }
}

pub trait MonitorBackend: Send + Sync {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError>;
    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError>;
    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError>;
}

pub trait SettingsRepository: Send + Sync {
    fn load(&self) -> Result<AppSettings, BackendError>;
    fn save(&self, settings: &AppSettings) -> Result<(), BackendError>;
}

pub trait InputChangePrompter: Send + Sync {
    fn confirm_input_change(&self, monitor: &Monitor, from: u32, to: u32) -> bool;
    fn confirm_keep_input(
        &self,
        monitor: &Monitor,
        previous_input: u32,
        new_input: u32,
        timeout_seconds: u32,
    ) -> Result<bool, String>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}
