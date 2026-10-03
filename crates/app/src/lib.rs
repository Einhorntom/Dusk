#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dispcontrol_domain::{
    AppSettings, ControlCapability, ControlKey, ControlReading, ControlValue, DomainError, Monitor,
    MonitorId,
};

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
    input_prompter: Arc<dyn InputChangePrompter>,
    clock: Arc<dyn Clock>,
    write_history: Mutex<HashMap<(MonitorId, ControlKey), VecDeque<Instant>>>,
    pending_adjustments: Mutex<HashMap<(MonitorId, ControlKey), PendingAdjustment>>,
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
        input_prompter: Arc<dyn InputChangePrompter>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            backend,
            settings,
            input_prompter,
            clock,
            write_history: Mutex::new(HashMap::new()),
            pending_adjustments: Mutex::new(HashMap::new()),
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

        let native = match value {
            ControlValue::Normalized(normalized) => capability.normalized_to_native(normalized)?,
            ControlValue::Enum(native) => {
                capability.validate_enum(native)?;
                native
            }
        };
        self.pending_adjustments
            .lock()
            .map_err(|_| {
                BackendError::Failed("pending monitor adjustments are unavailable".into())
            })?
            .remove(&(monitor.clone(), control));
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
                    vec![0x11, 0x31]
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
}
