#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dispcontrol_domain::{
    AppSettings, ControlCapability, ControlKey, ControlReading, ControlValue, DomainError,
    HotkeyBinding, KeyCombo, Monitor, MonitorId, Preset, PresetEntry, RateLimiter, WriteBuffer,
    preset_names_equal, validate_preset_name,
};

mod hotkeys;

pub use hotkeys::{HotkeyOutcome, HotkeyRepository};

mod timed;

pub use timed::{DEFAULT_ATTEMPTS, DEFAULT_TIMEOUT, TimedBackend};

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

/// Pause after a colour-preset write before the next write to that monitor.
pub const COLOR_MODE_SETTLE: Duration = Duration::from_millis(800);

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
    HotkeyNotFound(String),
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
            Self::HotkeyNotFound(keys) => write!(f, "no hotkey is bound to {keys}"),
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
    hotkeys: Arc<dyn HotkeyRepository>,
    input_prompter: Arc<dyn InputChangePrompter>,
    clock: Arc<dyn Clock>,
    write_buffer: Mutex<WriteBuffer<WriteKey>>,
    rate_limiter: Mutex<RateLimiter<WriteKey>>,
    last_applied_preset: Mutex<Option<String>>,
}

type WriteKey = (MonitorId, ControlKey);

fn locked<'a, T>(
    mutex: &'a Mutex<T>,
    what: &str,
) -> Result<std::sync::MutexGuard<'a, T>, UseCaseError> {
    mutex
        .lock()
        .map_err(|_| BackendError::Failed(format!("{what} is unavailable")).into())
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
            write_buffer: Mutex::new(WriteBuffer::default()),
            rate_limiter: Mutex::new(RateLimiter::default()),
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
        self.write_buffer()?.set_target(
            (monitor.clone(), control),
            value,
            self.clock.now(),
            Duration::from_millis(settings.debounce_ms.into()),
        );
        Ok(())
    }

    pub fn flush_pending_adjustments(&self) -> Result<(usize, Option<Duration>), UseCaseError> {
        let mut committed = 0;
        loop {
            let now = self.clock.now();
            let Some(target) = self.write_buffer()?.take_due(now) else {
                return Ok((committed, self.write_buffer()?.next_due(now)));
            };
            let (monitor, control) = &target.key;
            let (capability, previous_native) = self
                .backend
                .read_control(monitor, *control)?
                .ok_or(UseCaseError::UnsupportedControl(*control))?;
            let native = target_native(&capability, target.value)?;
            if native == previous_native {
                continue;
            }
            if let Err(next_slot) = self.rate_limiter()?.try_reserve(&target.key, now) {
                if !target.deferred {
                    eprintln!("warning: monitor write rate limit deferred {control} for {monitor}");
                }
                self.write_buffer()?.defer(target, next_slot);
                continue;
            }
            self.backend.write_control(monitor, *control, native)?;
            committed += 1;
        }
    }

    fn write_buffer(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, WriteBuffer<WriteKey>>, UseCaseError> {
        locked(&self.write_buffer, "pending monitor adjustments")
    }

    fn rate_limiter(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RateLimiter<WriteKey>>, UseCaseError> {
        locked(&self.rate_limiter, "monitor write limiter state")
    }

    fn discard_pending_adjustment(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        self.write_buffer()?.discard(&(monitor.clone(), control));
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

    /// Blocks until the rate limit allows a write (explicit writes, SPEC-WR-7).
    fn wait_for_write_slot(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        let key = (monitor.clone(), control);
        loop {
            let now = self.clock.now();
            match self.rate_limiter()?.try_reserve(&key, now) {
                Ok(()) => return Ok(()),
                Err(next_slot) => self.clock.sleep(next_slot.saturating_duration_since(now)),
            }
        }
    }

    pub fn settings(&self) -> Result<AppSettings, UseCaseError> {
        Ok(self.settings.load()?)
    }

    pub fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError> {
        settings.validate().map_err(UseCaseError::SettingsInvalid)?;
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
        self.remove_preset_from_hotkeys(name)
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
        let old_name = std::mem::replace(&mut presets[index].name, new_name.clone());
        self.presets.save_presets(&presets)?;
        self.rename_preset_in_hotkeys(&old_name, &new_name)
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

impl ControlApi for MonitorService {
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
}

impl SettingsApi for MonitorService {
    fn settings(&self) -> Result<AppSettings, UseCaseError> {
        MonitorService::settings(self)
    }

    fn update_settings(&self, settings: AppSettings) -> Result<(), UseCaseError> {
        MonitorService::update_settings(self, settings)
    }
}

impl PresetApi for MonitorService {
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

impl HotkeyApi for MonitorService {
    fn list_hotkeys(&self) -> Result<Vec<HotkeyBinding>, UseCaseError> {
        MonitorService::list_hotkeys(self)
    }

    fn save_hotkey(&self, binding: HotkeyBinding) -> Result<(), UseCaseError> {
        MonitorService::save_hotkey(self, binding)
    }

    fn remove_hotkey(&self, keys: &KeyCombo) -> Result<(), UseCaseError> {
        MonitorService::remove_hotkey(self, keys)
    }

    fn hotkeys_using_preset(&self, name: &str) -> Result<Vec<KeyCombo>, UseCaseError> {
        MonitorService::hotkeys_using_preset(self, name)
    }

    fn run_hotkey(&self, keys: &KeyCombo) -> Result<HotkeyOutcome, UseCaseError> {
        MonitorService::run_hotkey(self, keys)
    }
}
