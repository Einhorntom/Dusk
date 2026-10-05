//! Several monitor backends behind one `MonitorBackend`, e.g. DDC/CI for
//! external monitors plus WMI for the built-in display (SPEC-PNL). Each read
//! or write goes to the backend that listed the monitor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};

use crate::{BackendError, MonitorBackend};

pub struct CompositeBackend {
    backends: Vec<Arc<dyn MonitorBackend>>,
    /// Which backend listed each monitor, refreshed by every listing.
    owners: Mutex<HashMap<MonitorId, usize>>,
}

impl CompositeBackend {
    pub fn new(backends: Vec<Arc<dyn MonitorBackend>>) -> Self {
        Self {
            backends,
            owners: Mutex::new(HashMap::new()),
        }
    }

    fn owners(&self) -> std::sync::MutexGuard<'_, HashMap<MonitorId, usize>> {
        self.owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The backend for `monitor`, listing again if it is not known yet.
    fn owner(&self, monitor: &MonitorId) -> Result<&dyn MonitorBackend, BackendError> {
        let known = self.owners().get(monitor).copied();
        let index = match known {
            Some(index) => index,
            None => {
                self.list_monitors()?;
                self.owners().get(monitor).copied().ok_or_else(|| {
                    BackendError::NotFound(format!("monitor not found: {monitor}"))
                })?
            }
        };
        Ok(self.backends[index].as_ref())
    }
}

impl MonitorBackend for CompositeBackend {
    /// Monitors of every backend, in backend order. A backend that fails to
    /// list does not hide the others; the listing fails only if all do. If
    /// two backends report the same ID, the first one owns it.
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        let mut monitors: Vec<Monitor> = Vec::new();
        let mut owners = HashMap::new();
        let mut first_error = None;
        let mut any_listed = false;
        for (index, backend) in self.backends.iter().enumerate() {
            match backend.list_monitors() {
                Ok(listed) => {
                    any_listed = true;
                    for monitor in listed {
                        if owners.contains_key(&monitor.id) {
                            continue;
                        }
                        owners.insert(monitor.id.clone(), index);
                        monitors.push(monitor);
                    }
                }
                Err(error) => {
                    eprintln!("warning: a monitor backend could not list monitors: {error}");
                    first_error.get_or_insert(error);
                }
            }
        }
        if !any_listed && let Some(error) = first_error {
            return Err(error);
        }
        *self.owners() = owners;
        Ok(monitors)
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        self.owner(monitor)?.read_control(monitor, control)
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        self.owner(monitor)?
            .write_control(monitor, control, native_value)
    }
}
