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
                    self.backend
                        .write_control(monitor, control, previous_native)?;
                    return Err(UseCaseError::InputChangeReverted);
                }
                Err(prompt_error) => {
                    if let Err(revert_error) =
                        self.backend
                            .write_control(monitor, control, previous_native)
                    {
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

    fn wait_for_write_slot(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        let key = (monitor.clone(), control);
        loop {
            let now = self.clock.now();
            let wait = {
                let mut history = self.write_history.lock().map_err(|_| {
                    BackendError::Failed("monitor write limiter state is unavailable".into())
                })?;
                let writes = history.entry(key.clone()).or_default();
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

                if next_slot <= now {
                    writes.push_back(now);
                    None
                } else {
                    Some(next_slot.duration_since(now))
                }
            };
            if let Some(wait) = wait {
                self.clock.sleep(wait);
            } else {
                return Ok(());
            }
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

    struct Confirm(bool);

    impl InputChangePrompter for Confirm {
        fn confirm_input_change(&self, _monitor: &Monitor, _from: u32, _to: u32) -> bool {
            self.0
        }

        fn confirm_keep_input(
            &self,
            _monitor: &Monitor,
            _previous_input: u32,
            _new_input: u32,
            _timeout_seconds: u32,
        ) -> Result<bool, String> {
            Ok(true)
        }
    }

    struct SystemClock;

    impl Clock for SystemClock {
        fn now(&self) -> Instant {
            Instant::now()
        }

        fn sleep(&self, duration: Duration) {
            std::thread::sleep(duration);
        }
    }

    fn service(
        control: ControlKey,
        current: u32,
        accepted: bool,
    ) -> (MonitorService, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::new(control, current));
        let service = MonitorService::new(
            backend.clone(),
            Arc::new(MemorySettings(Mutex::new(AppSettings::default()))),
            Arc::new(Confirm(accepted)),
            Arc::new(SystemClock),
        );
        (service, backend)
    }

    #[test]
    fn unchanged_target_does_not_write_to_monitor() {
        let (service, backend) = service(ControlKey::Brightness, 50, true);
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
        let (service, backend) = service(ControlKey::Input, 0x11, false);
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
        let (service, backend) = service(ControlKey::Input, 0x11, true);
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
}
