//! Reading and writing controls (SPEC-CTL, SPEC-WR): input safety, the
//! quiet-period buffer, rate limits and recently known values.

use crate::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dusk_domain::{
    ControlCapability, ControlKey, ControlReading, ControlValue, DomainError, Monitor, MonitorId,
    PresetEntry, RateLimiter, WriteBuffer,
};

/// How long a value read from or written to a monitor is trusted, so that
/// committing buffered values does not read each one back first (R11). A
/// change made with the monitor's own buttons is noticed after this time.
pub const KNOWN_VALUE_TTL: Duration = Duration::from_secs(5);

/// A control's capability and native value as last read or written.
/// The write-safety state shared by every use case that writes to a
/// monitor: the quiet-period buffer (SPEC-WR-2/3), the rate limiter
/// (SPEC-WR-6), recently known values (SPEC-WR-4) and the committer's
/// wake-up.
#[derive(Default)]
pub(crate) struct WriteGate {
    write_buffer: Mutex<WriteBuffer<WriteKey>>,
    rate_limiter: Mutex<RateLimiter<WriteKey>>,
    known: Mutex<HashMap<WriteKey, Known>>,
    pub(crate) wake: Arc<crate::committer::Wake>,
}

#[derive(Clone)]
struct Known {
    capability: ControlCapability,
    native: u32,
    at: Instant,
}

/// A preset entry that must be written.
pub(crate) struct PlannedWrite {
    pub(crate) native: u32,
    pub(crate) rate_limited: bool,
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

impl MonitorService {
    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<WriteKey, Known>> {
        self.gate
            .known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remember(&self, key: &WriteKey, capability: &ControlCapability, native: u32) {
        self.known().insert(
            key.clone(),
            Known {
                capability: capability.clone(),
                native,
                at: self.clock.now(),
            },
        );
    }

    /// The control's capability and value: from `known` if recent enough,
    /// otherwise read from the monitor (and remembered).
    fn current(&self, key: &WriteKey) -> Result<(ControlCapability, u32), UseCaseError> {
        let now = self.clock.now();
        if let Some(known) = self.known().get(key)
            && now.saturating_duration_since(known.at) < KNOWN_VALUE_TTL
        {
            return Ok((known.capability.clone(), known.native));
        }
        let (monitor, control) = key;
        let (capability, native) = self
            .backend
            .read_control(monitor, *control)?
            .ok_or(UseCaseError::UnsupportedControl(*control))?;
        self.remember(key, &capability, native);
        Ok((capability, native))
    }

    pub(crate) fn write_buffer(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, WriteBuffer<WriteKey>>, UseCaseError> {
        locked(&self.gate.write_buffer, "pending monitor adjustments")
    }

    fn rate_limiter(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RateLimiter<WriteKey>>, UseCaseError> {
        locked(&self.gate.rate_limiter, "monitor write limiter state")
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
    pub(crate) fn plan_preset_write(
        &self,
        entry: &PresetEntry,
        force: bool,
    ) -> Result<Option<PlannedWrite>, UseCaseError> {
        let (capability, current) = self
            .backend
            .read_control(&entry.monitor, entry.control)?
            .ok_or(UseCaseError::UnsupportedControl(entry.control))?;
        let native = target_native(&capability, entry.value)?;
        self.discard_pending_adjustment(&entry.monitor, entry.control)?;
        Ok((force || native != current).then_some(PlannedWrite {
            native,
            rate_limited: capability.rate_limited,
        }))
    }

    /// Blocks until the rate limit allows a write (explicit writes, SPEC-WR-7).
    pub(crate) fn wait_for_write_slot(
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
}

impl ControlApi for MonitorService {
    fn list_monitors(&self) -> Result<Vec<Monitor>, UseCaseError> {
        Ok(self.backend.list_monitors()?)
    }

    fn read(
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

    fn set(
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

        if capability.rate_limited {
            self.wait_for_write_slot(monitor, control)?;
        }
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
                        log::error!(
                            "could not restore previous input {previous_native} for {monitor}: {error}"
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
                        log::error!(
                            "input keep prompt failed ({prompt_error}); automatic revert for {monitor} failed ({revert_error})"
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

    fn adjust(
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
        self.gate.wake.notify();
        Ok(())
    }

    fn flush_pending_adjustments(&self) -> Result<(usize, Option<Duration>), UseCaseError> {
        let mut committed = 0;
        loop {
            let now = self.clock.now();
            let Some(target) = self.write_buffer()?.take_due(now) else {
                return Ok((committed, self.write_buffer()?.next_due(now)));
            };
            let (monitor, control) = &target.key;
            let (capability, previous_native) = self.current(&target.key)?;
            let native = target_native(&capability, target.value)?;
            if native == previous_native {
                continue;
            }
            if capability.rate_limited
                && let Err(next_slot) = self.rate_limiter()?.try_reserve(&target.key, now)
            {
                if !target.deferred {
                    // Expected behaviour, kept in the log for diagnosis (R14).
                    log::info!("monitor write rate limit deferred {control} for {monitor}");
                }
                self.write_buffer()?.defer(target, next_slot);
                continue;
            }
            if let Err(error) = self.backend.write_control(monitor, *control, native) {
                self.known().remove(&target.key);
                return Err(error.into());
            }
            self.remember(&target.key, &capability, native);
            committed += 1;
        }
    }
}
