//! Errors of the ports (`BackendError`) and the use cases (`UseCaseError`).

use std::error::Error;
use std::fmt;

use dusk_domain::{ControlKey, DomainError, MonitorId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    NotFound(String),
    NotResponding(String),
    Failed(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) | Self::NotResponding(message) | Self::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

impl Error for BackendError {}

#[derive(Debug)]
pub enum UseCaseError {
    Backend(BackendError),
    Domain(DomainError),
    UnsupportedControl(ControlKey),
    InputChangeDeclined,
    InputKeepPromptFailed(String),
    InputChangeReverted,
    /// The settings break a rule (e.g. a value out of range).
    SettingsInvalid(&'static str),
    /// A UI action needs a selected monitor or control.
    NoSelection(&'static str),
    PresetNotFound(String),
    PresetExists(String),
    /// Cycling presets needs at least one.
    NoPresets,
    /// Capturing found no control of the monitor that a preset can hold.
    NothingToCapture(MonitorId),
    PresetEntryNotFound {
        preset: String,
        monitor: MonitorId,
        control: ControlKey,
    },
    HotkeyNotFound(String),
    /// A bug, e.g. a lock poisoned by a panic; not caused by the user or a monitor.
    Internal(&'static str),
}

impl fmt::Display for UseCaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => write!(f, "{error}"),
            Self::Domain(error) => write!(f, "{error}"),
            Self::UnsupportedControl(control) => write!(f, "monitor does not support {control}"),
            Self::InputChangeDeclined => f.write_str("input change was not confirmed"),
            Self::InputKeepPromptFailed(error) => {
                write!(f, "could not show input keep prompt: {error}")
            }
            Self::InputChangeReverted => {
                f.write_str("input change was reverted because it was not kept")
            }
            Self::SettingsInvalid(message) | Self::NoSelection(message) => f.write_str(message),
            Self::PresetNotFound(name) => write!(f, "preset not found: {name}"),
            Self::PresetExists(name) => write!(f, "a preset named {name} already exists"),
            Self::NoPresets => f.write_str("no presets are defined"),
            Self::NothingToCapture(monitor) => write!(
                f,
                "monitor {monitor} has no controls that a preset can save"
            ),
            Self::PresetEntryNotFound {
                preset,
                monitor,
                control,
            } => write!(f, "preset {preset} has no {control} entry for {monitor}"),
            Self::HotkeyNotFound(keys) => write!(f, "no hotkey is bound to {keys}"),
            Self::Internal(what) => write!(f, "internal error: {what}"),
        }
    }
}

impl Error for UseCaseError {}

impl From<BackendError> for UseCaseError {
    fn from(value: BackendError) -> Self {
        Self::Backend(value)
    }
}

impl From<DomainError> for UseCaseError {
    fn from(value: DomainError) -> Self {
        Self::Domain(value)
    }
}
