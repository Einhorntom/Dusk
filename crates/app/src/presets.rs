//! Preset use cases (SPEC-PRE): capture, edit, import/export, apply,
//! cycle and matching.

use crate::*;
use std::time::Duration;

use dusk_domain::{
    ControlKey, MonitorId, Preset, PresetEntry, preset_names_equal, validate_preset_name,
};

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

fn entry_error_status(error: UseCaseError) -> EntryStatus {
    match error {
        UseCaseError::UnsupportedControl(_) => {
            EntryStatus::Skipped("control is not supported by the monitor".into())
        }
        error => EntryStatus::Failed(error.to_string()),
    }
}

impl MonitorService {
    pub fn save_preset(&self, preset: Preset) -> Result<(), UseCaseError> {
        let _edit = self.editing();
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
}

impl PresetApi for MonitorService {
    fn list_presets(&self) -> Result<Vec<Preset>, UseCaseError> {
        Ok(self.presets.load_presets()?)
    }

    fn capture_preset(
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
            return Err(UseCaseError::NothingToCapture(monitor.clone()));
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

    fn delete_preset(&self, name: &str) -> Result<(), UseCaseError> {
        let _edit = self.editing();
        let mut presets = self.presets.load_presets()?;
        let before = presets.len();
        presets.retain(|preset| !preset_names_equal(&preset.name, name));
        if presets.len() == before {
            return Err(UseCaseError::PresetNotFound(name.to_owned()));
        }
        self.presets.save_presets(&presets)?;
        self.remove_preset_from_hotkeys(name)
    }

    fn rename_preset(&self, name: &str, new_name: &str) -> Result<(), UseCaseError> {
        let _edit = self.editing();
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

    fn move_preset(&self, name: &str, offset: i32) -> Result<(), UseCaseError> {
        let _edit = self.editing();
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

    fn set_preset_entry(&self, name: &str, entry: PresetEntry) -> Result<(), UseCaseError> {
        let _edit = self.editing();
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

    fn remove_preset_entry(
        &self,
        name: &str,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<(), UseCaseError> {
        let _edit = self.editing();
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
            return Err(UseCaseError::PresetEntryNotFound {
                preset: name.to_owned(),
                monitor: monitor.clone(),
                control,
            });
        }
        self.presets.save_presets(&presets)?;
        Ok(())
    }

    fn export_presets(&self) -> Result<String, UseCaseError> {
        let presets = self.presets.load_presets()?;
        Ok(self.presets.export_text(&presets)?)
    }

    /// Merges (or, with `replace`, replaces) presets from an exported document.
    fn import_presets(&self, text: &str, replace: bool) -> Result<ImportSummary, UseCaseError> {
        let _edit = self.editing();
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

    fn presets_location(&self) -> String {
        self.presets.location()
    }

    fn apply_preset(&self, name: &str) -> Result<ApplyReport, UseCaseError> {
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
                    Ok(Some(planned)) => {
                        if entry.control == ControlKey::ColorPreset {
                            mode_switch.push(entry.monitor.clone());
                        }
                        writes.push((index, planned));
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
        for (position, (index, planned)) in writes.into_iter().enumerate() {
            let entry = &entries[index];
            let slot = if planned.rate_limited {
                self.wait_for_write_slot(&entry.monitor, entry.control)
            } else {
                Ok(())
            };
            let result = slot.and_then(|()| {
                Ok(self
                    .backend
                    .write_control(&entry.monitor, entry.control, planned.native)?)
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
            .map_err(|_| UseCaseError::Internal("the last applied preset is unavailable"))? =
            Some(preset.name.clone());
        Ok(ApplyReport {
            preset: preset.name,
            outcomes,
        })
    }

    fn cycle_preset(&self, forward: bool) -> Result<ApplyReport, UseCaseError> {
        let presets = self.presets.load_presets()?;
        if presets.is_empty() {
            return Err(UseCaseError::NoPresets);
        }
        let last = self
            .last_applied_preset
            .lock()
            .map_err(|_| UseCaseError::Internal("the last applied preset is unavailable"))?
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
    fn matching_preset(&self) -> Result<Option<String>, UseCaseError> {
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
