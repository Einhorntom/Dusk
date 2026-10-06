#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use dusk_domain::{AppSettings, ControlKey, MonitorId};

mod api;
mod control;
mod errors;
mod hotkeys;
mod monitor_ids;
mod ports;
mod presets;

pub use api::{Api, ControlApi, HotkeyApi, PresetApi, SettingsApi};
pub use control::KNOWN_VALUE_TTL;
pub use errors::{BackendError, UseCaseError};
pub use ports::{Clock, InputChangePrompter, MonitorBackend, PresetRepository, SettingsRepository};
pub use presets::{ApplyReport, COLOR_MODE_SETTLE, EntryOutcome, EntryStatus, ImportSummary};

pub use hotkeys::{HotkeyOutcome, HotkeyRepository};

mod committer;
mod composite;
mod timed;

pub use committer::CommitObserver;
pub use composite::CompositeBackend;

pub use timed::{DEFAULT_ATTEMPTS, DEFAULT_TIMEOUT, TimedBackend};

/// The application: every use case, behind the `Api` traits. The code is
/// split by concern: `control` (reading and writing controls, through the
/// `WriteGate`), `presets`, `hotkeys`, `monitor_ids`, and settings here.
/// Presets and hotkeys change monitors only through the control use cases,
/// so every write follows the same safety rules (SPEC-WR, SPEC-IN).
pub struct MonitorService {
    backend: Arc<dyn MonitorBackend>,
    settings: Arc<dyn SettingsRepository>,
    presets: Arc<dyn PresetRepository>,
    hotkeys: Arc<dyn HotkeyRepository>,
    input_prompter: Arc<dyn InputChangePrompter>,
    clock: Arc<dyn Clock>,
    /// What every monitor write goes through: buffer, limits, known values.
    gate: control::WriteGate,
    last_applied_preset: Mutex<Option<String>>,
    /// Held across each load-change-save of presets, hotkeys or settings,
    /// so concurrent edits (Settings window, IPC clients) cannot overwrite
    /// each other.
    edits: Mutex<()>,
}

type WriteKey = (MonitorId, ControlKey);

fn locked<'a, T>(
    mutex: &'a Mutex<T>,
    what: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, UseCaseError> {
    mutex.lock().map_err(|_| UseCaseError::Internal(what))
}

impl MonitorService {
    pub fn new(
        backend: Arc<dyn MonitorBackend>,
        settings: Arc<dyn SettingsRepository>,
        presets: Arc<dyn PresetRepository>,
        hotkeys: Arc<dyn HotkeyRepository>,
        input_prompter: Arc<dyn InputChangePrompter>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            backend,
            settings,
            presets,
            hotkeys,
            input_prompter,
            clock,
            gate: control::WriteGate::default(),
            last_applied_preset: Mutex::new(None),
            edits: Mutex::new(()),
        }
    }

    /// Serializes read-modify-write use cases (see `edits`). Not re-entrant:
    /// helpers called while it is held must not take it again.
    pub(crate) fn editing(&self) -> std::sync::MutexGuard<'_, ()> {
        self.edits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SettingsApi for MonitorService {
    fn settings(&self) -> Result<AppSettings, UseCaseError> {
        Ok(self.settings.load()?)
    }

    fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError> {
        let _edit = self.editing();
        settings.validate().map_err(UseCaseError::SettingsInvalid)?;
        self.settings.save(&settings)?;
        Ok(())
    }
}
