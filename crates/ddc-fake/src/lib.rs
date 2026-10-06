#![forbid(unsafe_code)]
//! Simulated monitors and in-memory port implementations for tests and demos.
//!
//! `FakeBackend` behaves like the Windows DDC/CI backend at the port level:
//! unknown monitors are `NotFound`, unsupported controls read as `None`, and
//! every accepted write is recorded. The `ports` module holds in-memory
//! settings/preset/hotkey stores, a scripted input prompter and a manual clock, and
//! `Harness` wires them into a `MonitorService`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use dusk_app::{BackendError, MonitorBackend, MonitorService};
use dusk_domain::{ControlCapability, ControlKey, Monitor, MonitorId};

pub mod contract;
pub mod ports;

pub use ports::{ManualClock, MemoryHotkeys, MemoryPresets, MemorySettings, ScriptedPrompter};

/// A simulated display: its identity plus the controls it reports.
#[derive(Clone, Debug)]
pub struct FakeMonitor {
    info: Monitor,
    controls: BTreeMap<ControlKey, (ControlCapability, u32)>,
}

impl FakeMonitor {
    pub fn new(id: &str) -> Self {
        Self {
            info: Monitor {
                id: MonitorId::new(id).expect("fake monitor ids are not empty"),
                name: format!("Fake {id}"),
                unstable_id: false,
            },
            controls: BTreeMap::new(),
        }
    }

    /// A monitor shaped like the reference Lenovo L32p-30.
    pub fn reference(id: &str) -> Self {
        Self::new(id)
            .numeric(ControlKey::Brightness, 50, 100)
            .numeric(ControlKey::Contrast, 50, 100)
            .numeric(ControlKey::Volume, 20, 100)
            .numeric(ControlKey::GainRed, 100, 100)
            .numeric(ControlKey::GainGreen, 100, 100)
            .numeric(ControlKey::GainBlue, 100, 100)
            .enumerated(
                ControlKey::ColorPreset,
                0x06,
                &[0x01, 0x02, 0x05, 0x06, 0x08, 0x0B],
            )
            .enumerated(ControlKey::Input, 0x31, &[0x0F, 0x11, 0x31])
            .enumerated(ControlKey::Power, 0x01, &[0x01, 0x04, 0x05])
    }

    /// A laptop's built-in display: brightness only, not rate-limited
    /// (SPEC-PNL-2, SPEC-PNL-4).
    pub fn built_in(id: &str) -> Self {
        let mut monitor = Self::new(id).numeric(ControlKey::Brightness, 40, 100);
        monitor.info.name = "Built-in display".into();
        for (capability, _) in monitor.controls.values_mut() {
            capability.rate_limited = false;
        }
        monitor
    }

    pub fn id(&self) -> &MonitorId {
        &self.info.id
    }

    /// Adds a continuous control with native range `0..=max`.
    pub fn numeric(mut self, control: ControlKey, value: u32, max: u32) -> Self {
        assert!(control.is_numeric(), "{control} is not a numeric control");
        let capability = ControlCapability {
            key: control,
            native_min: 0,
            native_max: max,
            enum_values: Vec::new(),
            rate_limited: true,
        };
        self.controls.insert(control, (capability, value));
        self
    }

    /// Adds an enumerated control that accepts only `values`.
    pub fn enumerated(mut self, control: ControlKey, value: u32, values: &[u32]) -> Self {
        assert!(!control.is_numeric(), "{control} is a numeric control");
        let capability = ControlCapability {
            key: control,
            native_min: 0,
            native_max: 0,
            enum_values: values.to_vec(),
            rate_limited: true,
        };
        self.controls.insert(control, (capability, value));
        self
    }
}

/// One write the backend accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Write {
    pub monitor: MonitorId,
    pub control: ControlKey,
    pub value: u32,
}

#[derive(Default)]
struct State {
    monitors: Vec<FakeMonitor>,
    disconnected: HashSet<MonitorId>,
    writes: Vec<Write>,
    failing_reads: HashMap<(MonitorId, ControlKey), BackendError>,
    failing_writes: HashMap<(MonitorId, ControlKey), BackendError>,
    hung: HashSet<MonitorId>,
    reads: usize,
}

/// `MonitorBackend` over simulated monitors.
#[derive(Default)]
pub struct FakeBackend {
    state: Mutex<State>,
}

impl FakeBackend {
    pub fn new(monitors: impl IntoIterator<Item = FakeMonitor>) -> Self {
        Self {
            state: Mutex::new(State {
                monitors: monitors.into_iter().collect(),
                ..State::default()
            }),
        }
    }

    pub fn single(monitor: FakeMonitor) -> Self {
        Self::new([monitor])
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The monitor's current native value, as if read from its on-screen menu.
    pub fn value(&self, monitor: &MonitorId, control: ControlKey) -> Option<u32> {
        let state = self.state();
        let found = state.monitors.iter().find(|item| item.id() == monitor)?;
        found.controls.get(&control).map(|(_, value)| *value)
    }

    /// Changes a value without recording a write, like a user pressing the
    /// monitor's own buttons. Panics if the control is not simulated.
    pub fn set_value(&self, monitor: &MonitorId, control: ControlKey, value: u32) {
        let mut state = self.state();
        let found = state
            .monitors
            .iter_mut()
            .find(|item| item.id() == monitor)
            .expect("fake monitor exists");
        found
            .controls
            .get_mut(&control)
            .unwrap_or_else(|| panic!("fake monitor does not simulate {control}"))
            .1 = value;
    }

    pub fn writes(&self) -> Vec<Write> {
        self.state().writes.clone()
    }

    /// Controls written, in order, on any monitor.
    pub fn written_controls(&self) -> Vec<ControlKey> {
        self.state()
            .writes
            .iter()
            .map(|write| write.control)
            .collect()
    }

    /// Native values written, in order, on any monitor.
    pub fn written_values(&self) -> Vec<u32> {
        self.state()
            .writes
            .iter()
            .map(|write| write.value)
            .collect()
    }

    pub fn clear_writes(&self) {
        self.state().writes.clear();
    }

    /// How many control reads reached the simulated monitors.
    pub fn read_count(&self) -> usize {
        self.state().reads
    }

    pub fn fail_reads(&self, monitor: &MonitorId, control: ControlKey, error: BackendError) {
        self.state()
            .failing_reads
            .insert((monitor.clone(), control), error);
    }

    pub fn fail_writes(&self, monitor: &MonitorId, control: ControlKey, error: BackendError) {
        self.state()
            .failing_writes
            .insert((monitor.clone(), control), error);
    }

    /// Makes every read and write on the monitor block until `release`, like
    /// a display that stops answering DDC/CI (SPEC-MON-4).
    pub fn hang(&self, monitor: &MonitorId) {
        self.state().hung.insert(monitor.clone());
    }

    pub fn release(&self, monitor: &MonitorId) {
        self.state().hung.remove(monitor);
    }

    /// Blocks while the monitor is hung, without holding the state lock.
    fn wait_while_hung(&self, monitor: &MonitorId) {
        while self.state().hung.contains(monitor) {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Hides the monitor from discovery, as if its cable were unplugged.
    pub fn disconnect(&self, monitor: &MonitorId) {
        self.state().disconnected.insert(monitor.clone());
    }

    pub fn reconnect(&self, monitor: &MonitorId) {
        self.state().disconnected.remove(monitor);
    }
}

fn not_found(monitor: &MonitorId) -> BackendError {
    BackendError::NotFound(format!("monitor not found: {monitor}"))
}

impl MonitorBackend for FakeBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        let state = self.state();
        Ok(state
            .monitors
            .iter()
            .filter(|item| !state.disconnected.contains(item.id()))
            .map(|item| item.info.clone())
            .collect())
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        self.wait_while_hung(monitor);
        let mut state = self.state();
        state.reads += 1;
        if state.disconnected.contains(monitor) {
            return Err(not_found(monitor));
        }
        let found = state
            .monitors
            .iter()
            .find(|item| item.id() == monitor)
            .ok_or_else(|| not_found(monitor))?;
        if let Some(error) = state.failing_reads.get(&(monitor.clone(), control)) {
            return Err(error.clone());
        }
        Ok(found.controls.get(&control).cloned())
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        control: ControlKey,
        native_value: u32,
    ) -> Result<(), BackendError> {
        self.wait_while_hung(monitor);
        let mut state = self.state();
        if state.disconnected.contains(monitor) {
            return Err(not_found(monitor));
        }
        if let Some(error) = state.failing_writes.get(&(monitor.clone(), control)) {
            return Err(error.clone());
        }
        let found = state
            .monitors
            .iter_mut()
            .find(|item| item.id() == monitor)
            .ok_or_else(|| not_found(monitor))?;
        let Some(slot) = found.controls.get_mut(&control) else {
            return Err(BackendError::Failed(format!(
                "monitor does not report support for {control}"
            )));
        };
        slot.1 = native_value;
        state.writes.push(Write {
            monitor: monitor.clone(),
            control,
            value: native_value,
        });
        Ok(())
    }
}

/// A `MonitorService` over fakes, with handles to every fake for assertions.
pub struct Harness {
    pub service: Arc<MonitorService>,
    pub backend: Arc<FakeBackend>,
    pub clock: Arc<ManualClock>,
    pub settings: Arc<MemorySettings>,
    pub presets: Arc<MemoryPresets>,
    pub hotkeys: Arc<MemoryHotkeys>,
    pub prompter: Arc<ScriptedPrompter>,
}

impl Harness {
    /// Default settings, empty presets, and a prompter that accepts and keeps
    /// every input change.
    pub fn new(backend: FakeBackend) -> Self {
        Self::with_prompter(backend, ScriptedPrompter::accepting())
    }

    pub fn with_prompter(backend: FakeBackend, prompter: ScriptedPrompter) -> Self {
        let backend = Arc::new(backend);
        let clock = Arc::new(ManualClock::new());
        let settings = Arc::new(MemorySettings::default());
        let presets = Arc::new(MemoryPresets::default());
        let hotkeys = Arc::new(MemoryHotkeys::default());
        let prompter = Arc::new(prompter);
        let service = Arc::new(MonitorService::new(
            backend.clone(),
            settings.clone(),
            presets.clone(),
            hotkeys.clone(),
            prompter.clone(),
            clock.clone(),
        ));
        Self {
            service,
            backend,
            clock,
            settings,
            presets,
            hotkeys,
            prompter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_meets_the_backend_contract_directly_and_with_timeouts() {
        let backend = Arc::new(FakeBackend::new([
            FakeMonitor::reference("full"),
            FakeMonitor::new("bare").numeric(ControlKey::Brightness, 30, 100),
        ]));
        contract::check_backend_contract(backend.as_ref());
        let timed =
            dusk_app::TimedBackend::new(backend.clone(), std::time::Duration::from_secs(1), 1);
        contract::check_backend_contract(&timed);
        assert!(backend.writes().is_empty());
    }

    #[test]
    fn backend_reports_like_the_real_adapter() {
        let monitor = FakeMonitor::new("m").numeric(ControlKey::Brightness, 40, 100);
        let id = monitor.id().clone();
        let backend = FakeBackend::single(monitor);
        assert_eq!(backend.list_monitors().unwrap().len(), 1);
        let (_, value) = backend
            .read_control(&id, ControlKey::Brightness)
            .unwrap()
            .unwrap();
        assert_eq!(value, 40);
        assert!(
            backend
                .read_control(&id, ControlKey::Volume)
                .unwrap()
                .is_none()
        );
        assert!(backend.write_control(&id, ControlKey::Volume, 1).is_err());

        backend
            .write_control(&id, ControlKey::Brightness, 70)
            .unwrap();
        assert_eq!(backend.value(&id, ControlKey::Brightness), Some(70));
        assert_eq!(backend.written_values(), vec![70]);

        let missing = MonitorId::new("other").unwrap();
        assert!(matches!(
            backend.read_control(&missing, ControlKey::Brightness),
            Err(BackendError::NotFound(_))
        ));
        backend.disconnect(&id);
        assert!(backend.list_monitors().unwrap().is_empty());
        assert!(
            backend
                .write_control(&id, ControlKey::Brightness, 1)
                .is_err()
        );
        backend.reconnect(&id);
        assert_eq!(backend.list_monitors().unwrap().len(), 1);
    }

    #[test]
    fn scripted_failures_and_menu_changes_are_not_recorded_as_writes() {
        let monitor = FakeMonitor::reference("m");
        let id = monitor.id().clone();
        let backend = FakeBackend::single(monitor);
        backend.set_value(&id, ControlKey::Contrast, 10);
        backend.fail_writes(
            &id,
            ControlKey::Contrast,
            BackendError::Failed("busy".into()),
        );
        backend.fail_reads(
            &id,
            ControlKey::Volume,
            BackendError::NotResponding("silent".into()),
        );
        assert!(
            backend
                .write_control(&id, ControlKey::Contrast, 20)
                .is_err()
        );
        assert!(backend.read_control(&id, ControlKey::Volume).is_err());
        assert_eq!(backend.value(&id, ControlKey::Contrast), Some(10));
        assert!(backend.writes().is_empty());
    }
}
