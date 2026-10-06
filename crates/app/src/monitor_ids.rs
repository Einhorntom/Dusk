//! Moving saved references when monitor IDs change (SPEC-MON-1).

use crate::*;
use std::collections::{HashMap, HashSet};

use dusk_domain::{ControlKey, MonitorId};

impl MonitorService {
    /// Moves preset entries and hotkeys from old monitor IDs to new ones,
    /// e.g. after IDs started to come from the EDID (SPEC-MON-1). A rename
    /// is skipped while its old ID still names a connected monitor, and an
    /// entry the preset already has under the new ID wins over a moved one.
    /// Returns how many references moved.
    pub fn rename_monitor_ids(
        &self,
        renames: &[(MonitorId, MonitorId)],
    ) -> Result<usize, UseCaseError> {
        let _edit = self.editing();
        let connected: HashSet<MonitorId> = self
            .backend
            .list_monitors()?
            .into_iter()
            .map(|monitor| monitor.id)
            .collect();
        let renames: HashMap<&MonitorId, &MonitorId> = renames
            .iter()
            .filter(|(old, new)| old != new && !connected.contains(old))
            .map(|(old, new)| (old, new))
            .collect();
        if renames.is_empty() {
            return Ok(0);
        }

        let mut moved = 0;
        let mut presets = self.presets.load_presets()?;
        let mut presets_changed = false;
        for preset in &mut presets {
            let kept: HashSet<(MonitorId, ControlKey)> = preset
                .entries
                .iter()
                .filter(|entry| !renames.contains_key(&entry.monitor))
                .map(|entry| (entry.monitor.clone(), entry.control))
                .collect();
            let entries = std::mem::take(&mut preset.entries);
            for mut entry in entries {
                if let Some(new) = renames.get(&entry.monitor) {
                    presets_changed = true;
                    if kept.contains(&((*new).clone(), entry.control)) {
                        continue;
                    }
                    entry.monitor = (*new).clone();
                    moved += 1;
                }
                preset.entries.push(entry);
            }
        }
        if presets_changed {
            self.presets.save_presets(&presets)?;
        }

        let mut hotkeys = self.hotkeys.load_hotkeys()?;
        let mut hotkeys_changed = false;
        for binding in &mut hotkeys {
            if let Some(new) = binding
                .monitor
                .as_ref()
                .and_then(|monitor| renames.get(monitor))
            {
                binding.monitor = Some((*new).clone());
                hotkeys_changed = true;
                moved += 1;
            }
        }
        if hotkeys_changed {
            self.hotkeys.save_hotkeys(&hotkeys)?;
        }
        Ok(moved)
    }
}
