//! In-memory implementations of the application's non-monitor ports.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use dusk_app::{
    BackendError, Clock, HotkeyRepository, InputChangePrompter, PresetRepository,
    SettingsRepository,
};
use dusk_domain::{AppSettings, HotkeyBinding, Monitor, Preset};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
pub struct MemorySettings(Mutex<AppSettings>);

impl MemorySettings {
    pub fn new(settings: AppSettings) -> Self {
        Self(Mutex::new(settings))
    }

    pub fn stored(&self) -> AppSettings {
        lock(&self.0).clone()
    }

    pub fn replace(&self, settings: AppSettings) {
        *lock(&self.0) = settings;
    }
}

impl SettingsRepository for MemorySettings {
    fn load(&self) -> Result<AppSettings, BackendError> {
        Ok(lock(&self.0).clone())
    }

    fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
        *lock(&self.0) = settings.clone();
        Ok(())
    }
}

/// Preset store whose export format is an opaque handle: `export_text`
/// remembers the presets and returns `fake-presets:<n>`, and `parse_text`
/// accepts only such handles. That keeps import/export round trips faithful
/// without depending on a real file format.
#[derive(Default)]
pub struct MemoryPresets {
    presets: Mutex<Vec<Preset>>,
    documents: Mutex<Vec<Vec<Preset>>>,
}

const DOCUMENT_PREFIX: &str = "fake-presets:";

impl MemoryPresets {
    pub fn stored(&self) -> Vec<Preset> {
        lock(&self.presets).clone()
    }

    pub fn replace(&self, presets: Vec<Preset>) {
        *lock(&self.presets) = presets;
    }

    /// Returns an importable document holding `presets`.
    pub fn document(&self, presets: Vec<Preset>) -> String {
        let mut documents = lock(&self.documents);
        documents.push(presets);
        format!("{DOCUMENT_PREFIX}{}", documents.len() - 1)
    }
}

impl PresetRepository for MemoryPresets {
    fn load_presets(&self) -> Result<Vec<Preset>, BackendError> {
        Ok(self.stored())
    }

    fn save_presets(&self, presets: &[Preset]) -> Result<(), BackendError> {
        self.replace(presets.to_vec());
        Ok(())
    }

    fn export_text(&self, presets: &[Preset]) -> Result<String, BackendError> {
        Ok(self.document(presets.to_vec()))
    }

    fn parse_text(&self, text: &str) -> Result<Vec<Preset>, BackendError> {
        text.trim()
            .strip_prefix(DOCUMENT_PREFIX)
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| lock(&self.documents).get(index).cloned())
            .ok_or_else(|| BackendError::Failed("cannot parse presets: unknown document".into()))
    }

    fn location(&self) -> String {
        "memory://presets".into()
    }
}

#[derive(Default)]
pub struct MemoryHotkeys(Mutex<Vec<HotkeyBinding>>);

impl MemoryHotkeys {
    pub fn stored(&self) -> Vec<HotkeyBinding> {
        lock(&self.0).clone()
    }

    pub fn replace(&self, hotkeys: Vec<HotkeyBinding>) {
        *lock(&self.0) = hotkeys;
    }
}

impl HotkeyRepository for MemoryHotkeys {
    fn load_hotkeys(&self) -> Result<Vec<HotkeyBinding>, BackendError> {
        Ok(self.stored())
    }

    fn save_hotkeys(&self, hotkeys: &[HotkeyBinding]) -> Result<(), BackendError> {
        self.replace(hotkeys.to_vec());
        Ok(())
    }
}

/// Answers input-change prompts with fixed responses and counts them.
pub struct ScriptedPrompter {
    accept_change: bool,
    keep: Result<bool, String>,
    asked: Mutex<usize>,
}

impl ScriptedPrompter {
    pub fn new(accept_change: bool, keep: Result<bool, String>) -> Self {
        Self {
            accept_change,
            keep,
            asked: Mutex::new(0),
        }
    }

    /// Confirms every change and keeps it.
    pub fn accepting() -> Self {
        Self::new(true, Ok(true))
    }

    /// Declines every change.
    pub fn declining() -> Self {
        Self::new(false, Ok(true))
    }

    /// How many times the user was asked to confirm an input change.
    pub fn confirmations_asked(&self) -> usize {
        *lock(&self.asked)
    }
}

impl InputChangePrompter for ScriptedPrompter {
    fn confirm_input_change(&self, _monitor: &Monitor, _from: u32, _to: u32) -> bool {
        *lock(&self.asked) += 1;
        self.accept_change
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

/// Time that only moves when a test advances it or the code under test sleeps.
pub struct ManualClock(Mutex<Instant>);

impl ManualClock {
    pub fn new() -> Self {
        Self(Mutex::new(Instant::now()))
    }

    pub fn advance(&self, duration: Duration) {
        *lock(&self.0) += duration;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *lock(&self.0)
    }

    fn sleep(&self, duration: Duration) {
        self.advance(duration);
    }
}
