#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dispcontrol_domain::{
    AppSettings, ControlCapability, ControlKey, ControlReading, ControlValue, DomainError, Monitor,
    MonitorId, Preset, PresetEntry, preset_names_equal, validate_preset_name,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryStatus {
    Applied,
    Unchanged,
    Skipped(String),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryOutcome {
    pub entry: PresetEntry,
    pub status: EntryStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyReport {
    pub preset: String,
    pub outcomes: Vec<EntryOutcome>,
}

impl ApplyReport {
    fn count(&self, predicate: impl Fn(&EntryStatus) -> bool) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| predicate(&outcome.status))
            .count()
    }

    pub fn applied(&self) -> usize {
        self.count(|status| matches!(status, EntryStatus::Applied))
    }

    pub fn unchanged(&self) -> usize {
        self.count(|status| matches!(status, EntryStatus::Unchanged))
    }

    pub fn skipped(&self) -> usize {
        self.count(|status| matches!(status, EntryStatus::Skipped(_)))
    }

    pub fn failed(&self) -> usize {
        self.count(|status| matches!(status, EntryStatus::Failed(_)))
    }
}

const COLOR_MODE_SETTLE: Duration = Duration::from_millis(800);

const CAPTURE_CONTROLS: [ControlKey; 8] = [
    ControlKey::Brightness,
    ControlKey::Contrast,
    ControlKey::ColorPreset,
    ControlKey::GainRed,
    ControlKey::GainGreen,
    ControlKey::GainBlue,
    ControlKey::Volume,
    ControlKey::Input,
];

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

pub trait Api: Send + Sync {
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
    fn settings(&self) -> Result<AppSettings, UseCaseError>;
    fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError>;
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
    SettingsInvalid(&'static str),
    PresetNotFound(String),
    PresetExists(String),
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
            Self::SettingsInvalid(message) => f.write_str(message),
            Self::PresetNotFound(name) => write!(f, "preset not found: {name}"),
            Self::PresetExists(name) => write!(f, "a preset named {name} already exists"),
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

pub struct MonitorService {
    backend: Arc<dyn MonitorBackend>,
    settings: Arc<dyn SettingsRepository>,
    presets: Arc<dyn PresetRepository>,
    input_prompter: Arc<dyn InputChangePrompter>,
    clock: Arc<dyn Clock>,
    write_history: Mutex<HashMap<(MonitorId, ControlKey), VecDeque<Instant>>>,
    pending_adjustments: Mutex<HashMap<(MonitorId, ControlKey), PendingAdjustment>>,
    last_applied_preset: Mutex<Option<String>>,
}

#[derive(Clone, Copy)]
struct PendingAdjustment {
    value: ControlValue,
    due: Instant,
    rate_limit_reported: bool,
}

impl MonitorService {
    pub fn new(
        backend: Arc<dyn MonitorBackend>,
        settings: Arc<dyn SettingsRepository>,
        presets: Arc<dyn PresetRepository>,
        input_prompter: Arc<dyn InputChangePrompter>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            backend,
            settings,
            presets,
            input_prompter,
            clock,
            write_history: Mutex::new(HashMap::new()),
            pending_adjustments: Mutex::new(HashMap::new()),
            last_applied_preset: Mutex::new(None),
        }
    }

    pub fn list_monitors(&self) -> Result<Vec<Monitor>, UseCaseError> {
        Ok(self.backend.list_monitors()?)
    }

    pub fn read(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<ControlReading, UseCaseError> {
        let (capability, native) = self
            .backend
            .read_control(monitor, control)?
            .ok_or(UseCaseError::UnsupportedControl(control))?;
        let value = if control.is_numeric() {
            ControlValue::Normalized(capability.native_to_normalized(native)?)
        } else {
            capability.validate_enum(native)?;
            ControlValue::Enum(native)
        };
        Ok(ControlReading { capability, value })
    }

    pub fn set(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<bool, UseCaseError> {
        let settings = self.settings.load()?;
        let (capability, previous_native) = self
            .backend
            .read_control(monitor, control)?
            .ok_or(UseCaseError::UnsupportedControl(control))?;

        let native = target_native(&capability, value)?;
        self.discard_pending_adjustment(monitor, control)?;
        if native == previous_native {
            return Ok(false);
        }

        let monitor_info = if control == ControlKey::Input
            && (settings.confirm_input_change || settings.input_revert_seconds > 0)
        {
            Some(
                self.backend
                    .list_monitors()?
                    .into_iter()
                    .find(|item| item.id == *monitor)
                    .ok_or_else(|| BackendError::NotFound(monitor.to_string()))?,
            )
        } else {
            None
        };

        if settings.confirm_input_change
            && control == ControlKey::Input
            && !self.input_prompter.confirm_input_change(
                monitor_info
                    .as_ref()
                    .ok_or_else(|| BackendError::NotFound(monitor.to_string()))?,
                previous_native,
                native,
            )
        {
            return Err(UseCaseError::InputChangeDeclined);
        }

        self.wait_for_write_slot(monitor, control)?;
        self.backend.write_control(monitor, control, native)?;
        if control == ControlKey::Input && settings.input_revert_seconds > 0 {
            let monitor_info = monitor_info
                .as_ref()
                .ok_or_else(|| BackendError::NotFound(monitor.to_string()))?;
            match self.input_prompter.confirm_keep_input(
                monitor_info,
                previous_native,
                native,
                settings.input_revert_seconds,
            ) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) =
                        self.backend
                            .write_control(monitor, control, previous_native)
                    {
                        eprintln!(
                            "error: could not restore previous input {previous_native} for {monitor}: {error}"
                        );
                        return Err(error.into());
                    }
                    return Err(UseCaseError::InputChangeReverted);
                }
                Err(prompt_error) => {
                    if let Err(revert_error) =
                        self.backend
                            .write_control(monitor, control, previous_native)
                    {
                        eprintln!(
                            "error: input keep prompt failed ({prompt_error}); automatic revert for {monitor} failed ({revert_error})"
                        );
                        return Err(BackendError::Failed(format!(
                            "input keep prompt failed ({prompt_error}); automatic revert failed ({revert_error})"
                        ))
                        .into());
                    }
                    return Err(UseCaseError::InputKeepPromptFailed(prompt_error));
                }
            }
        }
        Ok(true)
    }

    pub fn adjust(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<(), UseCaseError> {
        if !control.is_numeric() {
            return Err(DomainError::InvalidNumericCapability(control).into());
        }
        let settings = self.settings.load()?;
        let ControlValue::Normalized(normalized) = value else {
            return Err(DomainError::InvalidNumericCapability(control).into());
        };
        if normalized > 100 {
            return Err(DomainError::NormalizedValueOutOfRange(normalized).into());
        }
        let due = self.clock.now() + Duration::from_millis(settings.debounce_ms.into());
        let mut pending = self.pending_adjustments.lock().map_err(|_| {
            BackendError::Failed("pending monitor adjustments are unavailable".into())
        })?;
        pending.insert(
            (monitor.clone(), control),
            PendingAdjustment {
                value,
                due,
                rate_limit_reported: false,
            },
        );
        Ok(())
    }

    pub fn flush_pending_adjustments(&self) -> Result<(usize, Option<Duration>), UseCaseError> {
        let mut committed = 0;
        loop {
            let due_adjustment = {
                let now = self.clock.now();
                let mut pending = self.pending_adjustments.lock().map_err(|_| {
                    BackendError::Failed("pending monitor adjustments are unavailable".into())
                })?;
                let Some((key, adjustment)) = pending
                    .iter()
                    .filter(|(_, adjustment)| adjustment.due <= now)
                    .min_by_key(|(_, adjustment)| adjustment.due)
                    .map(|(key, adjustment)| (key.clone(), *adjustment))
                else {
                    let next_wake = pending
                        .values()
                        .map(|adjustment| adjustment.due.saturating_duration_since(now))
                        .min();
                    return Ok((committed, next_wake));
                };
                pending.remove(&key);
                (key, adjustment)
            };

            let ((monitor, control), adjustment) = due_adjustment;
            let (capability, previous_native) = self
                .backend
                .read_control(&monitor, control)?
                .ok_or(UseCaseError::UnsupportedControl(control))?;
            let ControlValue::Normalized(normalized) = adjustment.value else {
                return Err(DomainError::InvalidNumericCapability(control).into());
            };
            let native = capability.normalized_to_native(normalized)?;
            if native == previous_native {
                continue;
            }

            if let Some(next_slot) = self.reserve_write_slot(&monitor, control)? {
                if !adjustment.rate_limit_reported {
                    eprintln!("warning: monitor write rate limit deferred {control} for {monitor}");
                }
                let mut pending = self.pending_adjustments.lock().map_err(|_| {
                    BackendError::Failed("pending monitor adjustments are unavailable".into())
                })?;
                pending.insert(
                    (monitor, control),
                    PendingAdjustment {
                        value: adjustment.value,
                        due: next_slot,
                        rate_limit_reported: true,
                    },
                );
                continue;
            }

            self.backend.write_control(&monitor, control, native)?;
            committed += 1;
        }
    }

    fn discard_pending_adjustment(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        self.pending_adjustments
            .lock()
            .map_err(|_| {
                BackendError::Failed("pending monitor adjustments are unavailable".into())
            })?
            .remove(&(monitor.clone(), control));
        Ok(())
    }

    /// Reads the entry's control and returns the native value to write, or
    /// `None` when the monitor already holds it and `force` is false.
    fn plan_preset_write(
        &self,
        entry: &PresetEntry,
        force: bool,
    ) -> Result<Option<u32>, UseCaseError> {
        let (capability, current) = self
            .backend
            .read_control(&entry.monitor, entry.control)?
            .ok_or(UseCaseError::UnsupportedControl(entry.control))?;
        let native = target_native(&capability, entry.value)?;
        self.discard_pending_adjustment(&entry.monitor, entry.control)?;
        Ok((force || native != current).then_some(native))
    }

    fn wait_for_write_slot(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        loop {
            if let Some(next_slot) = self.reserve_write_slot(monitor, control)? {
                self.clock
                    .sleep(next_slot.saturating_duration_since(self.clock.now()));
            } else {
                return Ok(());
            }
        }
    }

    fn reserve_write_slot(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<Instant>, UseCaseError> {
        let key = (monitor.clone(), control);
        let now = self.clock.now();
        let mut history = self.write_history.lock().map_err(|_| {
            BackendError::Failed("monitor write limiter state is unavailable".into())
        })?;
        let writes = history.entry(key).or_default();
        while writes
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            writes.pop_front();
        }
        let one_second_limit = writes.back().map(|time| *time + Duration::from_secs(1));
        let one_minute_limit = (writes.len() >= 30)
            .then(|| writes.front().copied())
            .flatten()
            .map(|time| time + Duration::from_secs(60));
        let next_slot = match (one_second_limit, one_minute_limit) {
            (Some(one_second), Some(one_minute)) => one_second.max(one_minute),
            (Some(one_second), None) => one_second,
            (None, Some(one_minute)) => one_minute,
            (None, None) => now,
        };
        if next_slot > now {
            Ok(Some(next_slot))
        } else {
            writes.push_back(now);
            Ok(None)
        }
    }

    pub fn settings(&self) -> Result<AppSettings, UseCaseError> {
        Ok(self.settings.load()?)
    }

    pub fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError> {
        if !(150..=2000).contains(&settings.debounce_ms) {
            return Err(UseCaseError::SettingsInvalid(
                "debounce must be between 150 and 2000 milliseconds",
            ));
        }
        if settings.input_revert_seconds != 0 && !(5..=60).contains(&settings.input_revert_seconds)
        {
            return Err(UseCaseError::SettingsInvalid(
                "input revert must be 0 or between 5 and 60 seconds",
            ));
        }
        self.settings.save(&settings)?;
        Ok(())
    }

    pub fn list_presets(&self) -> Result<Vec<Preset>, UseCaseError> {
        Ok(self.presets.load_presets()?)
    }

    pub fn save_preset(&self, preset: Preset) -> Result<(), UseCaseError> {
        let name = validate_preset_name(&preset.name)?;
        let preset = Preset { name, ..preset };
        let mut presets = self.presets.load_presets()?;
        match presets
            .iter_mut()
            .find(|existing| preset_names_equal(&existing.name, &preset.name))
        {
            Some(existing) => *existing = preset,
            None => presets.push(preset),
        }
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    pub fn capture_preset(
        &self,
        name: &str,
        monitor: &MonitorId,
        include_input: bool,
    ) -> Result<Preset, UseCaseError> {
        let name = validate_preset_name(name)?;
        let mut entries = Vec::new();
        for control in CAPTURE_CONTROLS {
            if control == ControlKey::Input && !include_input {
                continue;
            }
            match self.read(monitor, control) {
                Ok(reading) => entries.push(PresetEntry {
                    monitor: monitor.clone(),
                    control,
                    value: reading.value,
                }),
                Err(UseCaseError::UnsupportedControl(_)) => {}
                Err(error) => return Err(error),
            }
        }
        if entries.is_empty() {
            return Err(UseCaseError::SettingsInvalid(
                "monitor has no supported controls to capture",
            ));
        }
        let mut preset = Preset { name, entries };
        let inert: Vec<bool> = preset
            .entries
            .iter()
            .map(|entry| preset.is_entry_inert(entry))
            .collect();
        let mut inert = inert.into_iter();
        preset.entries.retain(|_| !inert.next().unwrap_or(false));
        self.save_preset(preset.clone())?;
        Ok(preset)
    }

    pub fn delete_preset(&self, name: &str) -> Result<(), UseCaseError> {
        let mut presets = self.presets.load_presets()?;
        let before = presets.len();
        presets.retain(|preset| !preset_names_equal(&preset.name, name));
        if presets.len() == before {
            return Err(UseCaseError::PresetNotFound(name.to_owned()));
        }
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    pub fn rename_preset(&self, name: &str, new_name: &str) -> Result<(), UseCaseError> {
        let new_name = validate_preset_name(new_name)?;
        let mut presets = self.presets.load_presets()?;
        let index = presets
            .iter()
            .position(|preset| preset_names_equal(&preset.name, name))
            .ok_or_else(|| UseCaseError::PresetNotFound(name.to_owned()))?;
        if presets
            .iter()
            .enumerate()
            .any(|(other, preset)| other != index && preset_names_equal(&preset.name, &new_name))
        {
            return Err(UseCaseError::PresetExists(new_name));
        }
        presets[index].name = new_name;
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    pub fn move_preset(&self, name: &str, offset: i32) -> Result<(), UseCaseError> {
        let mut presets = self.presets.load_presets()?;
        let index = presets
            .iter()
            .position(|preset| preset_names_equal(&preset.name, name))
            .ok_or_else(|| UseCaseError::PresetNotFound(name.to_owned()))?;
        let target = (index as i64 + i64::from(offset)).clamp(0, presets.len() as i64 - 1) as usize;
        if target != index {
            let preset = presets.remove(index);
            presets.insert(target, preset);
            self.presets.save_presets(&presets)?;
        }
        Ok(())
    }

    pub fn set_preset_entry(&self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError> {
        entry.validate()?;
        let mut presets = self.presets.load_presets()?;
        let preset = presets
            .iter_mut()
            .find(|preset| preset_names_equal(&preset.name, name))
            .ok_or_else(|| UseCaseError::PresetNotFound(name.to_owned()))?;
        match preset
            .entries
            .iter_mut()
            .find(|item| item.monitor == entry.monitor && item.control == entry.control)
        {
            Some(existing) => existing.value = entry.value,
            None => preset.entries.push(entry),
        }
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    pub fn remove_preset_entry(
        &self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        let mut presets = self.presets.load_presets()?;
        let preset = presets
            .iter_mut()
            .find(|preset| preset_names_equal(&preset.name, name))
            .ok_or_else(|| UseCaseError::PresetNotFound(name.to_owned()))?;
        let before = preset.entries.len();
        preset
            .entries
            .retain(|item| !(item.monitor == *monitor && item.control == control));
        if preset.entries.len() == before {
            return Err(UseCaseError::PresetNotFound(format!(
                "{name}: no {control} entry for {monitor}"
            )));
        }
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    pub fn export_presets(&self) -> Result<String, UseCaseError> {
        let presets = self.presets.load_presets()?;
        Ok(self.presets.export_text(&presets)?)
    }

    /// Merges (or, with `replace`, replaces) presets from an exported document.
    pub fn import_presets(&self, text: &str, replace: bool) -> Result<ImportSummary, UseCaseError> {
        let imported = self.presets.parse_text(text)?;
        let mut presets = if replace {
            Vec::new()
        } else {
            self.presets.load_presets()?
        };
        let removed = if replace {
            self.presets.load_presets()?.len()
        } else {
            0
        };
        let (mut added, mut updated) = (0, 0);
        for preset in imported {
            match presets
                .iter_mut()
                .find(|existing| preset_names_equal(&existing.name, &preset.name))
            {
                Some(existing) => {
                    *existing = preset;
                    updated += 1;
                }
                None => {
                    presets.push(preset);
                    added += 1;
                }
            }
        }
        self.presets.save_presets(&presets)?;
        Ok(ImportSummary {
            added,
            updated,
            removed,
        })
    }

    pub fn presets_location(&self) -> String {
        self.presets.location()
    }

    pub fn apply_preset(&self, name: &str) -> Result<ApplyReport, UseCaseError> {
        let preset = self
            .presets
            .load_presets()?
            .into_iter()
            .find(|preset| preset_names_equal(&preset.name, name))
            .ok_or_else(|| UseCaseError::PresetNotFound(name.to_owned()))?;
        let present = self.backend.list_monitors()?;
        let mut entries = preset.entries.clone();
        entries.sort_by_key(|entry| entry.control.preset_apply_rank());

        // DDC/CI has no multi-value write, so each control is its own command.
        // To keep the change as close to one step as possible, every value is
        // read first and the differing ones are then written back to back.
        // Power and input go through `set` for its confirmation/revert policy.
        let mut statuses: Vec<Option<EntryStatus>> = Vec::with_capacity(entries.len());
        let mut writes = Vec::new();
        let mut mode_switch: Vec<MonitorId> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let status = if !present.iter().any(|monitor| monitor.id == entry.monitor) {
                EntryStatus::Skipped("monitor is not connected".into())
            } else if preset.is_entry_inert(entry) {
                EntryStatus::Skipped("the preset's colour preset uses its own gains".into())
            } else if matches!(entry.control, ControlKey::Power | ControlKey::Input) {
                match self.set(&entry.monitor, entry.control, entry.value) {
                    Ok(true) => EntryStatus::Applied,
                    Ok(false) => EntryStatus::Unchanged,
                    Err(error) => entry_error_status(error),
                }
            } else {
                // Changing the colour mode makes the monitor load that mode's
                // stored values, so later entries for the same monitor are
                // written even if they matched before the switch.
                let force = mode_switch.contains(&entry.monitor);
                match self.plan_preset_write(entry, force) {
                    Ok(Some(native)) => {
                        if entry.control == ControlKey::ColorPreset {
                            mode_switch.push(entry.monitor.clone());
                        }
                        writes.push((index, native));
                        statuses.push(None);
                        continue;
                    }
                    Ok(None) => EntryStatus::Unchanged,
                    Err(error) => entry_error_status(error),
                }
            };
            statuses.push(Some(status));
        }

        let last_write = writes.len();
        for (position, (index, native)) in writes.into_iter().enumerate() {
            let entry = &entries[index];
            let result = self
                .wait_for_write_slot(&entry.monitor, entry.control)
                .and_then(|()| {
                    Ok(self
                        .backend
                        .write_control(&entry.monitor, entry.control, native)?)
                });
            statuses[index] = Some(match result {
                Ok(()) => EntryStatus::Applied,
                Err(error) => EntryStatus::Failed(error.to_string()),
            });
            // The monitor ignores or garbles commands while it switches mode.
            if entry.control == ControlKey::ColorPreset && position + 1 < last_write {
                self.clock.sleep(COLOR_MODE_SETTLE);
            }
        }
        let outcomes = entries
            .into_iter()
            .zip(statuses)
            .map(|(entry, status)| EntryOutcome {
                entry,
                status: status.unwrap_or(EntryStatus::Unchanged),
            })
            .collect();
        *self
            .last_applied_preset
            .lock()
            .map_err(|_| BackendError::Failed("preset state is unavailable".into()))? =
            Some(preset.name.clone());
        Ok(ApplyReport {
            preset: preset.name,
            outcomes,
        })
    }

    pub fn cycle_preset(&self, forward: bool) -> Result<ApplyReport, UseCaseError> {
        let presets = self.presets.load_presets()?;
        if presets.is_empty() {
            return Err(UseCaseError::SettingsInvalid("no presets are defined"));
        }
        let last = self
            .last_applied_preset
            .lock()
            .map_err(|_| BackendError::Failed("preset state is unavailable".into()))?
            .clone();
        let current = last.and_then(|last| {
            presets
                .iter()
                .position(|preset| preset_names_equal(&preset.name, &last))
        });
        let count = presets.len();
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        self.apply_preset(&presets[next].name)
    }

    /// Returns the first preset whose entries all equal the monitors' current state.
    pub fn matching_preset(&self) -> Result<Option<String>, UseCaseError> {
        let presets = self.presets.load_presets()?;
        if presets.is_empty() {
            return Ok(None);
        }
        let present = self.backend.list_monitors()?;
        'presets: for preset in presets {
            let mut compared = 0;
            for entry in &preset.entries {
                if !present.iter().any(|monitor| monitor.id == entry.monitor)
                    || preset.is_entry_inert(entry)
                {
                    continue;
                }
                match self.read(&entry.monitor, entry.control) {
                    Ok(reading) if reading.value == entry.value => compared += 1,
                    _ => continue 'presets,
                }
            }
            if compared > 0 {
                return Ok(Some(preset.name));
            }
        }
        Ok(None)
    }
}

fn entry_error_status(error: UseCaseError) -> EntryStatus {
    match error {
        UseCaseError::UnsupportedControl(_) => {
            EntryStatus::Skipped("control is not supported by the monitor".into())
        }
        error => EntryStatus::Failed(error.to_string()),
    }
}

fn target_native(capability: &ControlCapability, value: ControlValue) -> Result<u32, UseCaseError> {
    Ok(match value {
        ControlValue::Normalized(normalized) => capability.normalized_to_native(normalized)?,
        ControlValue::Enum(native) => {
            capability.validate_enum(native)?;
            native
        }
    })
}

impl Api for MonitorService {
    fn list_monitors(&self) -> Result<Vec<Monitor>, UseCaseError> {
        MonitorService::list_monitors(self)
    }

    fn read(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<ControlReading, UseCaseError> {
        MonitorService::read(self, monitor, control)
    }

    fn set(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<bool, UseCaseError> {
        MonitorService::set(self, monitor, control, value)
    }

    fn adjust(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        value: ControlValue,
    ) -> Result<(), UseCaseError> {
        MonitorService::adjust(self, monitor, control, value)
    }

    fn flush_pending_adjustments(&self) -> Result<(usize, Option<Duration>), UseCaseError> {
        MonitorService::flush_pending_adjustments(self)
    }

    fn settings(&self) -> Result<AppSettings, UseCaseError> {
        MonitorService::settings(self)
    }

    fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError> {
        MonitorService::update_settings(self, settings)
    }

    fn list_presets(&self) -> Result<Vec<Preset>, UseCaseError> {
        MonitorService::list_presets(self)
    }

    fn capture_preset(
        &self,
        name: &str,
        monitor: &MonitorId,
        include_input: bool,
    ) -> Result<Preset, UseCaseError> {
        MonitorService::capture_preset(self, name, monitor, include_input)
    }

    fn delete_preset(&self, name: &str) -> Result<(), UseCaseError> {
        MonitorService::delete_preset(self, name)
    }

    fn rename_preset(&self, name: &str, new_name: &str) -> Result<(), UseCaseError> {
        MonitorService::rename_preset(self, name, new_name)
    }

    fn move_preset(&self, name: &str, offset: i32) -> Result<(), UseCaseError> {
        MonitorService::move_preset(self, name, offset)
    }

    fn apply_preset(&self, name: &str) -> Result<ApplyReport, UseCaseError> {
        MonitorService::apply_preset(self, name)
    }

    fn cycle_preset(&self, forward: bool) -> Result<ApplyReport, UseCaseError> {
        MonitorService::cycle_preset(self, forward)
    }

    fn matching_preset(&self) -> Result<Option<String>, UseCaseError> {
        MonitorService::matching_preset(self)
    }

    fn set_preset_entry(&self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError> {
        MonitorService::set_preset_entry(self, name, entry)
    }

    fn remove_preset_entry(
        &self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        MonitorService::remove_preset_entry(self, name, monitor, control)
    }

    fn export_presets(&self) -> Result<String, UseCaseError> {
        MonitorService::export_presets(self)
    }

    fn import_presets(&self, text: &str, replace: bool) -> Result<ImportSummary, UseCaseError> {
        MonitorService::import_presets(self, text, replace)
    }

    fn presets_location(&self) -> String {
        MonitorService::presets_location(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispcontrol_domain::{ControlValue, MonitorId};
    use std::sync::Mutex;

    struct FakeBackend {
        monitor: Monitor,
        native_value: Mutex<u32>,
        writes: Mutex<Vec<u32>>,
    }

    impl FakeBackend {
        fn new(_control: ControlKey, current: u32) -> Self {
            Self {
                monitor: Monitor {
                    id: MonitorId::new("test-monitor").unwrap(),
                    name: "Fake monitor".into(),
                    unstable_id: false,
                },
                native_value: Mutex::new(current),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn capability(control: ControlKey) -> ControlCapability {
            ControlCapability {
                key: control,
                native_min: 0,
                native_max: if control.is_numeric() { 100 } else { 0 },
                enum_values: if control.is_numeric() {
                    vec![]
                } else {
                    vec![0x06, 0x0B, 0x11, 0x31]
                },
            }
        }
    }

    impl MonitorBackend for FakeBackend {
        fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
            Ok(vec![self.monitor.clone()])
        }

        fn read_control(
            &self,
            monitor: &MonitorId,
            control: ControlKey,
        ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
            if monitor != &self.monitor.id {
                return Ok(None);
            }
            Ok(Some((
                Self::capability(control),
                *self.native_value.lock().unwrap(),
            )))
        }

        fn write_control(
            &self,
            _monitor: &MonitorId,
            _control: ControlKey,
            native_value: u32,
        ) -> Result<(), BackendError> {
            self.writes.lock().unwrap().push(native_value);
            *self.native_value.lock().unwrap() = native_value;
            Ok(())
        }
    }

    struct MemorySettings(Mutex<AppSettings>);

    impl SettingsRepository for MemorySettings {
        fn load(&self) -> Result<AppSettings, BackendError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
            *self.0.lock().unwrap() = settings.clone();
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryPresets(Mutex<Vec<Preset>>);

    impl PresetRepository for MemoryPresets {
        fn load_presets(&self) -> Result<Vec<Preset>, BackendError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save_presets(&self, presets: &[Preset]) -> Result<(), BackendError> {
            *self.0.lock().unwrap() = presets.to_vec();
            Ok(())
        }
    }

    struct Confirm {
        accepted: bool,
        keep: Result<bool, String>,
    }

    impl InputChangePrompter for Confirm {
        fn confirm_input_change(&self, _monitor: &Monitor, _from: u32, _to: u32) -> bool {
            self.accepted
        }

        fn confirm_keep_input(
            &self,
            _monitor: &Monitor,
            _previous_input: u32,
            _new_input: u32,
            _timeout_seconds: u32,
        ) -> Result<bool, String> {
            self.keep.clone()
        }
    }

    struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().unwrap();
            *now += duration;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }

        fn sleep(&self, duration: Duration) {
            self.advance(duration);
        }
    }

    fn service(
        control: ControlKey,
        current: u32,
        accepted: bool,
    ) -> (MonitorService, Arc<FakeBackend>, Arc<ManualClock>) {
        service_with_prompts(control, current, accepted, Ok(true))
    }

    fn service_with_prompts(
        control: ControlKey,
        current: u32,
        accepted: bool,
        keep: Result<bool, String>,
    ) -> (MonitorService, Arc<FakeBackend>, Arc<ManualClock>) {
        let backend = Arc::new(FakeBackend::new(control, current));
        let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
        let service = MonitorService::new(
            backend.clone(),
            Arc::new(MemorySettings(Mutex::new(AppSettings::default()))),
            Arc::new(MemoryPresets::default()),
            Arc::new(Confirm { accepted, keep }),
            clock.clone(),
        );
        (service, backend, clock)
    }

    #[test]
    fn unchanged_target_does_not_write_to_monitor() {
        let (service, backend, _) = service(ControlKey::Brightness, 50, true);
        assert!(
            !service
                .set(
                    &backend.monitor.id,
                    ControlKey::Brightness,
                    ControlValue::Normalized(50),
                )
                .unwrap()
        );
        assert!(backend.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn input_change_requires_confirmation_before_writing() {
        let (service, backend, _) = service(ControlKey::Input, 0x11, false);
        let result = service.set(
            &backend.monitor.id,
            ControlKey::Input,
            ControlValue::Enum(0x31),
        );
        assert!(matches!(result, Err(UseCaseError::InputChangeDeclined)));
        assert!(backend.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn accepted_input_change_is_written_once() {
        let (service, backend, _) = service(ControlKey::Input, 0x11, true);
        assert!(
            service
                .set(
                    &backend.monitor.id,
                    ControlKey::Input,
                    ControlValue::Enum(0x31),
                )
                .unwrap()
        );
        assert_eq!(*backend.writes.lock().unwrap(), vec![0x31]);
    }

    #[test]
    fn slider_adjustments_coalesce_until_quiet_period_then_commit_latest_value() {
        let (service, backend, clock) = service(ControlKey::Brightness, 50, true);
        let monitor = &backend.monitor.id;
        service
            .adjust(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(60),
            )
            .unwrap();
        service
            .adjust(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(70),
            )
            .unwrap();

        assert_eq!(
            service.flush_pending_adjustments().unwrap(),
            (0, Some(Duration::from_millis(400)))
        );
        clock.advance(Duration::from_millis(399));
        assert_eq!(service.flush_pending_adjustments().unwrap().0, 0);
        clock.advance(Duration::from_millis(1));
        assert_eq!(service.flush_pending_adjustments().unwrap(), (1, None));
        assert_eq!(*backend.writes.lock().unwrap(), vec![70]);
    }

    #[test]
    fn rate_limited_adjustment_is_held_and_committed_when_slot_opens() {
        let (service, backend, clock) = service(ControlKey::Brightness, 50, true);
        let monitor = &backend.monitor.id;
        service
            .set(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(60),
            )
            .unwrap();
        service
            .adjust(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(70),
            )
            .unwrap();
        clock.advance(Duration::from_millis(400));

        assert_eq!(
            service.flush_pending_adjustments().unwrap(),
            (0, Some(Duration::from_millis(600)))
        );
        assert_eq!(*backend.writes.lock().unwrap(), vec![60]);
        clock.advance(Duration::from_millis(600));
        assert_eq!(service.flush_pending_adjustments().unwrap(), (1, None));
        assert_eq!(*backend.writes.lock().unwrap(), vec![60, 70]);
    }

    #[test]
    fn explicit_set_supersedes_a_pending_slider_target_for_the_same_control() {
        let (service, backend, clock) = service(ControlKey::Brightness, 50, true);
        let monitor = &backend.monitor.id;
        service
            .adjust(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(60),
            )
            .unwrap();
        service
            .set(
                monitor,
                ControlKey::Brightness,
                ControlValue::Normalized(70),
            )
            .unwrap();
        clock.advance(Duration::from_secs(2));
        assert_eq!(service.flush_pending_adjustments().unwrap(), (0, None));
        assert_eq!(*backend.writes.lock().unwrap(), vec![70]);
    }

    #[test]
    fn explicit_writes_obey_one_second_and_thirty_per_minute_limits() {
        let (service, backend, clock) = service(ControlKey::Brightness, 50, true);
        let monitor = &backend.monitor.id;
        let start = clock.now();
        for index in 0..30 {
            let value = if index % 2 == 0 { 0 } else { 100 };
            service
                .set(
                    monitor,
                    ControlKey::Brightness,
                    ControlValue::Normalized(value),
                )
                .unwrap();
        }
        assert_eq!(backend.writes.lock().unwrap().len(), 30);

        service
            .set(monitor, ControlKey::Brightness, ControlValue::Normalized(0))
            .unwrap();
        assert_eq!(backend.writes.lock().unwrap().len(), 31);
        assert!(clock.now().duration_since(start) >= Duration::from_secs(60));
    }

    #[test]
    fn rejected_keep_prompt_restores_the_previous_input() {
        let (service, backend, _) = service_with_prompts(ControlKey::Input, 0x11, true, Ok(false));
        let result = service.set(
            &backend.monitor.id,
            ControlKey::Input,
            ControlValue::Enum(0x31),
        );
        assert!(matches!(result, Err(UseCaseError::InputChangeReverted)));
        assert_eq!(*backend.writes.lock().unwrap(), vec![0x31, 0x11]);
    }

    #[test]
    fn keep_prompt_failure_attempts_to_restore_the_previous_input() {
        let (service, backend, _) = service_with_prompts(
            ControlKey::Input,
            0x11,
            true,
            Err("prompt unavailable".into()),
        );
        let result = service.set(
            &backend.monitor.id,
            ControlKey::Input,
            ControlValue::Enum(0x31),
        );
        assert!(matches!(
            result,
            Err(UseCaseError::InputKeepPromptFailed(_))
        ));
        assert_eq!(*backend.writes.lock().unwrap(), vec![0x31, 0x11]);
    }

    struct MultiBackend {
        monitor: Monitor,
        values: Mutex<HashMap<ControlKey, u32>>,
        writes: Mutex<Vec<ControlKey>>,
    }

    impl MonitorBackend for MultiBackend {
        fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
            Ok(vec![self.monitor.clone()])
        }

        fn read_control(
            &self,
            _monitor: &MonitorId,
            control: ControlKey,
        ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(&control)
                .map(|value| (FakeBackend::capability(control), *value)))
        }

        fn write_control(
            &self,
            _monitor: &MonitorId,
            control: ControlKey,
            native_value: u32,
        ) -> Result<(), BackendError> {
            self.writes.lock().unwrap().push(control);
            self.values.lock().unwrap().insert(control, native_value);
            Ok(())
        }
    }

    fn preset_service() -> (MonitorService, Arc<MultiBackend>, Arc<ManualClock>) {
        let backend = Arc::new(MultiBackend {
            monitor: Monitor {
                id: MonitorId::new("test-monitor").unwrap(),
                name: "Fake monitor".into(),
                unstable_id: false,
            },
            values: Mutex::new(HashMap::from([
                (ControlKey::Brightness, 50),
                (ControlKey::Contrast, 50),
                (ControlKey::ColorPreset, 0x11),
                (ControlKey::Input, 0x31),
            ])),
            writes: Mutex::new(Vec::new()),
        });
        let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
        let service = MonitorService::new(
            backend.clone(),
            Arc::new(MemorySettings(Mutex::new(AppSettings::default()))),
            Arc::new(MemoryPresets::default()),
            Arc::new(Confirm {
                accepted: true,
                keep: Ok(true),
            }),
            clock.clone(),
        );
        (service, backend, clock)
    }

    fn set_values(backend: &MultiBackend, brightness: u32, contrast: u32, color: u32) {
        let mut values = backend.values.lock().unwrap();
        values.insert(ControlKey::Brightness, brightness);
        values.insert(ControlKey::Contrast, contrast);
        values.insert(ControlKey::ColorPreset, color);
    }

    #[test]
    fn capture_stores_supported_controls_and_skips_input_by_default() {
        let (service, backend, _) = preset_service();
        let preset = service
            .capture_preset(" Day ", &backend.monitor.id, false)
            .unwrap();
        assert_eq!(preset.name, "Day");
        let controls: Vec<_> = preset.entries.iter().map(|entry| entry.control).collect();
        assert_eq!(
            controls,
            vec![
                ControlKey::Brightness,
                ControlKey::Contrast,
                ControlKey::ColorPreset
            ]
        );
        let with_input = service
            .capture_preset("Day", &backend.monitor.id, true)
            .unwrap();
        assert_eq!(with_input.entries.len(), 4);
        assert_eq!(service.list_presets().unwrap().len(), 1);
    }

    #[test]
    fn apply_writes_in_spec_order_and_second_apply_is_a_no_op() {
        let (service, backend, _) = preset_service();
        set_values(&backend, 20, 30, 0x31);
        service
            .capture_preset("Night", &backend.monitor.id, false)
            .unwrap();
        set_values(&backend, 90, 80, 0x11);

        let report = service.apply_preset("night").unwrap();
        assert_eq!(report.applied(), 3);
        assert_eq!(
            *backend.writes.lock().unwrap(),
            vec![
                ControlKey::ColorPreset,
                ControlKey::Brightness,
                ControlKey::Contrast
            ]
        );
        let again = service.apply_preset("Night").unwrap();
        assert_eq!(again.applied(), 0);
        assert_eq!(again.unchanged(), 3);
        assert_eq!(backend.writes.lock().unwrap().len(), 3);
    }

    fn entry(backend: &MultiBackend, control: ControlKey, value: ControlValue) -> PresetEntry {
        PresetEntry {
            monitor: backend.monitor.id.clone(),
            control,
            value,
        }
    }

    #[test]
    fn fixed_colour_preset_ignores_gains_on_capture_and_apply() {
        let (service, backend, _) = preset_service();
        backend
            .values
            .lock()
            .unwrap()
            .insert(ControlKey::GainRed, 100);
        set_values(&backend, 50, 50, 0x06);
        let captured = service
            .capture_preset("Day", &backend.monitor.id, false)
            .unwrap();
        assert!(
            captured
                .entries
                .iter()
                .all(|entry| entry.control != ControlKey::GainRed)
        );

        // A preset saved before this rule still carries the gain entry.
        service
            .save_preset(Preset {
                name: "Day".into(),
                entries: vec![
                    entry(&backend, ControlKey::ColorPreset, ControlValue::Enum(0x06)),
                    entry(&backend, ControlKey::GainRed, ControlValue::Normalized(100)),
                ],
            })
            .unwrap();
        set_values(&backend, 50, 50, 0x0B);
        backend
            .values
            .lock()
            .unwrap()
            .insert(ControlKey::GainRed, 40);
        let report = service.apply_preset("Day").unwrap();
        assert_eq!(report.applied(), 1);
        assert_eq!(report.skipped(), 1);
        assert_eq!(
            *backend.writes.lock().unwrap(),
            vec![ControlKey::ColorPreset]
        );
        assert_eq!(service.matching_preset().unwrap().as_deref(), Some("Day"));
    }

    #[test]
    fn colour_mode_switch_writes_later_values_back_to_back_after_one_settle() {
        let (service, backend, clock) = preset_service();
        backend
            .values
            .lock()
            .unwrap()
            .insert(ControlKey::GainRed, 100);
        set_values(&backend, 65, 50, 0x06);
        service
            .save_preset(Preset {
                name: "Night".into(),
                entries: vec![
                    entry(
                        &backend,
                        ControlKey::Brightness,
                        ControlValue::Normalized(65),
                    ),
                    entry(&backend, ControlKey::GainRed, ControlValue::Normalized(100)),
                    entry(&backend, ControlKey::ColorPreset, ControlValue::Enum(0x0B)),
                ],
            })
            .unwrap();
        let started = clock.now();
        let report = service.apply_preset("Night").unwrap();
        // The monitor reloads per-mode values, so matching ones are rewritten.
        assert_eq!(report.applied(), 3);
        assert_eq!(
            *backend.writes.lock().unwrap(),
            vec![
                ControlKey::ColorPreset,
                ControlKey::GainRed,
                ControlKey::Brightness
            ]
        );
        assert_eq!(clock.now() - started, COLOR_MODE_SETTLE);
    }

    #[test]
    fn apply_skips_absent_monitors_and_unsupported_controls() {
        let (service, backend, _) = preset_service();
        service
            .save_preset(Preset {
                name: "Mixed".into(),
                entries: vec![
                    PresetEntry {
                        monitor: MonitorId::new("other-monitor").unwrap(),
                        control: ControlKey::Brightness,
                        value: ControlValue::Normalized(10),
                    },
                    PresetEntry {
                        monitor: backend.monitor.id.clone(),
                        control: ControlKey::Volume,
                        value: ControlValue::Normalized(10),
                    },
                    PresetEntry {
                        monitor: backend.monitor.id.clone(),
                        control: ControlKey::Brightness,
                        value: ControlValue::Normalized(10),
                    },
                ],
            })
            .unwrap();
        let report = service.apply_preset("Mixed").unwrap();
        assert_eq!(report.skipped(), 2);
        assert_eq!(report.applied(), 1);
        assert_eq!(report.failed(), 0);
    }

    #[test]
    fn a_failing_entry_does_not_stop_the_others() {
        let (service, backend, _) = preset_service();
        service
            .save_preset(Preset {
                name: "Bad".into(),
                entries: vec![
                    PresetEntry {
                        monitor: backend.monitor.id.clone(),
                        control: ControlKey::ColorPreset,
                        value: ControlValue::Enum(0x99),
                    },
                    PresetEntry {
                        monitor: backend.monitor.id.clone(),
                        control: ControlKey::Brightness,
                        value: ControlValue::Normalized(10),
                    },
                ],
            })
            .unwrap();
        let report = service.apply_preset("Bad").unwrap();
        assert_eq!(report.failed(), 1);
        assert_eq!(report.applied(), 1);
    }

    #[test]
    fn preset_names_are_unique_case_insensitively_and_can_be_renamed_moved_and_deleted() {
        let (service, backend, _) = preset_service();
        for name in ["A", "B", "C"] {
            service
                .capture_preset(name, &backend.monitor.id, false)
                .unwrap();
        }
        service
            .capture_preset("b", &backend.monitor.id, false)
            .unwrap();
        assert_eq!(service.list_presets().unwrap().len(), 3);

        assert!(matches!(
            service.rename_preset("A", "C"),
            Err(UseCaseError::PresetExists(_))
        ));
        service.rename_preset("A", "Alpha").unwrap();
        service.move_preset("C", -2).unwrap();
        let names: Vec<_> = service
            .list_presets()
            .unwrap()
            .into_iter()
            .map(|preset| preset.name)
            .collect();
        assert_eq!(names, vec!["C", "Alpha", "b"]);

        service.delete_preset("alpha").unwrap();
        assert!(matches!(
            service.delete_preset("Alpha"),
            Err(UseCaseError::PresetNotFound(_))
        ));
        assert!(matches!(
            service.capture_preset("", &backend.monitor.id, false),
            Err(UseCaseError::Domain(DomainError::InvalidPresetName))
        ));
    }

    #[test]
    fn cycling_wraps_around_starting_from_the_last_applied_preset() {
        let (service, backend, _) = preset_service();
        for name in ["A", "B"] {
            service
                .capture_preset(name, &backend.monitor.id, false)
                .unwrap();
        }
        assert_eq!(service.cycle_preset(true).unwrap().preset, "A");
        assert_eq!(service.cycle_preset(true).unwrap().preset, "B");
        assert_eq!(service.cycle_preset(true).unwrap().preset, "A");
        assert_eq!(service.cycle_preset(false).unwrap().preset, "B");
    }

    #[test]
    fn matching_preset_reports_the_preset_equal_to_current_state() {
        let (service, backend, _) = preset_service();
        set_values(&backend, 20, 30, 0x31);
        service
            .capture_preset("Night", &backend.monitor.id, false)
            .unwrap();
        assert_eq!(service.matching_preset().unwrap().as_deref(), Some("Night"));
        set_values(&backend, 21, 30, 0x31);
        assert_eq!(service.matching_preset().unwrap(), None);
    }
}
