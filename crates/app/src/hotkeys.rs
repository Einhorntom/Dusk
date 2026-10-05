//! Hotkey use cases (SPEC-HK): stored bindings and what pressing one does.
//! Key registration itself belongs to the UI adapter; it calls `run_hotkey`
//! with the combination that was pressed.

use dispcontrol_domain::{
    ControlKey, ControlValue, DomainError, HotkeyAction, HotkeyBinding, KeyCombo, MonitorId,
    preset_names_equal,
};

use crate::{ApplyReport, BackendError, MonitorService, UseCaseError};

pub trait HotkeyRepository: Send + Sync {
    fn load_hotkeys(&self) -> Result<Vec<HotkeyBinding>, BackendError>;
    fn save_hotkeys(&self, hotkeys: &[HotkeyBinding]) -> Result<(), BackendError>;
}

/// What a hotkey press changed, for the on-screen indicator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotkeyOutcome {
    /// New targets per monitor; they are written after the quiet period.
    Stepped {
        control: ControlKey,
        values: Vec<(MonitorId, u32)>,
    },
    Preset(ApplyReport),
    /// Monitors switched to the input (0 when all were already on it).
    Input {
        value: u32,
        changed: usize,
    },
    Power {
        on: bool,
        changed: usize,
    },
}

const POWER_ON: u32 = 0x01;
/// Off modes in order of preference. Soft off (0x04) keeps DDC/CI alive on
/// most monitors, so the same hotkey can turn the display back on.
const POWER_OFF_MODES: [u32; 4] = [0x04, 0x02, 0x03, 0x05];

impl MonitorService {
    pub fn list_hotkeys(&self) -> Result<Vec<HotkeyBinding>, UseCaseError> {
        Ok(self.hotkeys.load_hotkeys()?)
    }

    /// Adds a binding, or replaces the one with the same key combination.
    pub fn save_hotkey(&self, binding: HotkeyBinding) -> Result<(), UseCaseError> {
        binding.keys.validate()?;
        if let HotkeyAction::ApplyPreset(name) = &binding.action
            && !self
                .presets
                .load_presets()?
                .iter()
                .any(|preset| preset_names_equal(&preset.name, name))
        {
            return Err(UseCaseError::PresetNotFound(name.clone()));
        }
        let mut hotkeys = self.hotkeys.load_hotkeys()?;
        match hotkeys.iter_mut().find(|item| item.keys == binding.keys) {
            Some(existing) => *existing = binding,
            None => hotkeys.push(binding),
        }
        self.hotkeys.save_hotkeys(&hotkeys)?;
        Ok(())
    }

    pub fn remove_hotkey(&self, keys: &KeyCombo) -> Result<(), UseCaseError> {
        let mut hotkeys = self.hotkeys.load_hotkeys()?;
        let before = hotkeys.len();
        hotkeys.retain(|item| item.keys != *keys);
        if hotkeys.len() == before {
            return Err(UseCaseError::HotkeyNotFound(keys.to_string()));
        }
        self.hotkeys.save_hotkeys(&hotkeys)?;
        Ok(())
    }

    /// Key combinations whose action applies the named preset (SPEC-PRE-5).
    pub fn hotkeys_using_preset(&self, name: &str) -> Result<Vec<KeyCombo>, UseCaseError> {
        Ok(self
            .hotkeys
            .load_hotkeys()?
            .into_iter()
            .filter(|item| {
                item.action
                    .preset_name()
                    .is_some_and(|preset| preset_names_equal(preset, name))
            })
            .map(|item| item.keys)
            .collect())
    }

    /// Points hotkeys for a renamed preset at its new name.
    pub(crate) fn rename_preset_in_hotkeys(
        &self,
        name: &str,
        new_name: &str,
    ) -> Result<(), UseCaseError> {
        let mut hotkeys = self.hotkeys.load_hotkeys()?;
        let mut changed = false;
        for item in &mut hotkeys {
            if let HotkeyAction::ApplyPreset(preset) = &mut item.action
                && preset_names_equal(preset, name)
            {
                *preset = new_name.to_owned();
                changed = true;
            }
        }
        if changed {
            self.hotkeys.save_hotkeys(&hotkeys)?;
        }
        Ok(())
    }

    /// Removes hotkeys that apply a deleted preset.
    pub(crate) fn remove_preset_from_hotkeys(&self, name: &str) -> Result<(), UseCaseError> {
        let mut hotkeys = self.hotkeys.load_hotkeys()?;
        let before = hotkeys.len();
        hotkeys.retain(|item| {
            !item
                .action
                .preset_name()
                .is_some_and(|preset| preset_names_equal(preset, name))
        });
        if hotkeys.len() != before {
            self.hotkeys.save_hotkeys(&hotkeys)?;
        }
        Ok(())
    }

    /// Runs the binding currently stored for `keys`. Looking it up at press
    /// time picks up preset renames made elsewhere (for example by the CLI).
    pub fn run_hotkey(&self, keys: &KeyCombo) -> Result<HotkeyOutcome, UseCaseError> {
        let binding = self
            .hotkeys
            .load_hotkeys()?
            .into_iter()
            .find(|item| item.keys == *keys)
            .ok_or_else(|| UseCaseError::HotkeyNotFound(keys.to_string()))?;
        match &binding.action {
            HotkeyAction::ApplyPreset(name) => Ok(HotkeyOutcome::Preset(self.apply_preset(name)?)),
            HotkeyAction::NextPreset => Ok(HotkeyOutcome::Preset(self.cycle_preset(true)?)),
            HotkeyAction::PreviousPreset => Ok(HotkeyOutcome::Preset(self.cycle_preset(false)?)),
            HotkeyAction::Step { control, up } => {
                let targets = self.hotkey_targets(&binding)?;
                let values = each_target(&targets, |monitor| {
                    self.step(monitor, *control, *up)
                        .map(|value| (monitor.clone(), value))
                })?;
                Ok(HotkeyOutcome::Stepped {
                    control: *control,
                    values,
                })
            }
            HotkeyAction::SetInput(value) => {
                let targets = self.hotkey_targets(&binding)?;
                let results = each_target(&targets, |monitor| {
                    self.set(monitor, ControlKey::Input, ControlValue::Enum(*value))
                })?;
                Ok(HotkeyOutcome::Input {
                    value: *value,
                    changed: results.into_iter().filter(|changed| *changed).count(),
                })
            }
            HotkeyAction::TogglePower => self.toggle_power(&binding),
        }
    }

    /// Moves a control's target by its step size, starting from a pending
    /// target if one exists so that a held key keeps stepping. The write is
    /// buffered like a slider move (SPEC-HK-4, SPEC-WR-2); steps clamp to
    /// 0-100 (SPEC-CTL-5). Returns the new target.
    pub fn step(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        up: bool,
    ) -> Result<u32, UseCaseError> {
        let step = self
            .settings
            .load()?
            .step_for(control)
            .ok_or(DomainError::InvalidNumericCapability(control))?;
        let pending = self.write_buffer()?.target(&(monitor.clone(), control));
        let current = match pending {
            Some(ControlValue::Normalized(value)) => value,
            _ => match self.read(monitor, control)?.value {
                ControlValue::Normalized(value) => value,
                ControlValue::Enum(_) => {
                    return Err(DomainError::InvalidNumericCapability(control).into());
                }
            },
        };
        let target = if up {
            current.saturating_add(step).min(100)
        } else {
            current.saturating_sub(step)
        };
        if pending.is_some() || target != current {
            self.adjust(monitor, control, ControlValue::Normalized(target))?;
        }
        Ok(target)
    }

    /// The binding's monitor, or every connected monitor.
    fn hotkey_targets(&self, binding: &HotkeyBinding) -> Result<Vec<MonitorId>, UseCaseError> {
        let connected: Vec<MonitorId> = self
            .backend
            .list_monitors()?
            .into_iter()
            .map(|monitor| monitor.id)
            .collect();
        match &binding.monitor {
            Some(monitor) if connected.contains(monitor) => Ok(vec![monitor.clone()]),
            Some(monitor) => {
                Err(BackendError::NotFound(format!("monitor not found: {monitor}")).into())
            }
            None if connected.is_empty() => {
                Err(BackendError::NotFound("no monitor is connected".into()).into())
            }
            None => Ok(connected),
        }
    }

    /// Turns every target off if any is on, otherwise turns them all on.
    fn toggle_power(&self, binding: &HotkeyBinding) -> Result<HotkeyOutcome, UseCaseError> {
        let targets = self.hotkey_targets(binding)?;
        let readings = each_target(&targets, |monitor| {
            let (capability, current) = self
                .backend
                .read_control(monitor, ControlKey::Power)?
                .ok_or(UseCaseError::UnsupportedControl(ControlKey::Power))?;
            Ok((monitor.clone(), capability, current))
        })?;
        let turn_on = !readings.iter().any(|(_, _, current)| *current == POWER_ON);
        let mut changed = 0;
        for (monitor, capability, _) in &readings {
            let value = if turn_on {
                POWER_ON
            } else {
                let Some(off) = POWER_OFF_MODES
                    .into_iter()
                    .find(|mode| capability.enum_values.contains(mode))
                else {
                    continue;
                };
                off
            };
            if self.set(monitor, ControlKey::Power, ControlValue::Enum(value))? {
                changed += 1;
            }
        }
        Ok(HotkeyOutcome::Power {
            on: turn_on,
            changed,
        })
    }
}

/// Runs `action` for each target. Monitors that do not support it are
/// skipped; the first other error stops the hotkey, and if no monitor
/// supports it, that is the error.
fn each_target<T>(
    targets: &[MonitorId],
    mut action: impl FnMut(&MonitorId) -> Result<T, UseCaseError>,
) -> Result<Vec<T>, UseCaseError> {
    let mut results = Vec::with_capacity(targets.len());
    let mut unsupported = None;
    for monitor in targets {
        match action(monitor) {
            Ok(result) => results.push(result),
            Err(
                error @ (UseCaseError::UnsupportedControl(_)
                | UseCaseError::Domain(DomainError::EnumValueUnavailable { .. })),
            ) => {
                unsupported.get_or_insert(error);
            }
            Err(error) => return Err(error),
        }
    }
    match unsupported {
        Some(error) if results.is_empty() => Err(error),
        _ => Ok(results),
    }
}
