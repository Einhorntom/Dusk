//! Several backends behind one port: the built-in display (WMI) next to
//! external monitors (DDC/CI), SPEC-PNL.

use std::sync::Arc;

use dispcontrol_app::{BackendError, CompositeBackend, MonitorBackend};
use dispcontrol_ddc_fake::contract::check_backend_contract;
use dispcontrol_ddc_fake::{FakeBackend, FakeMonitor};
use dispcontrol_domain::{ControlCapability, ControlKey, Monitor, MonitorId};

fn id(text: &str) -> MonitorId {
    MonitorId::new(text).unwrap()
}

/// A backend whose discovery fails, e.g. WMI unavailable.
struct Broken;

impl MonitorBackend for Broken {
    fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        Err(BackendError::Failed("WMI is unavailable".into()))
    }

    fn read_control(
        &self,
        monitor: &MonitorId,
        _control: ControlKey,
    ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
        Err(BackendError::NotFound(format!(
            "monitor not found: {monitor}"
        )))
    }

    fn write_control(
        &self,
        monitor: &MonitorId,
        _control: ControlKey,
        _native_value: u32,
    ) -> Result<(), BackendError> {
        Err(BackendError::NotFound(format!(
            "monitor not found: {monitor}"
        )))
    }
}

fn laptop() -> (Arc<FakeBackend>, Arc<FakeBackend>, CompositeBackend) {
    let external = Arc::new(FakeBackend::single(FakeMonitor::reference("L32p-30#0")));
    let panel = Arc::new(FakeBackend::single(FakeMonitor::built_in(
        "Built-in-display",
    )));
    let composite = CompositeBackend::new(vec![external.clone(), panel.clone()]);
    (external, panel, composite)
}

#[test]
fn monitors_of_all_backends_are_listed_and_calls_go_to_their_owner() {
    let (external, panel, composite) = laptop();
    let names: Vec<String> = composite
        .list_monitors()
        .unwrap()
        .into_iter()
        .map(|monitor| monitor.name)
        .collect();
    assert_eq!(names, vec!["Fake L32p-30#0", "Built-in display"]);

    composite
        .write_control(&id("Built-in-display"), ControlKey::Brightness, 25)
        .unwrap();
    composite
        .write_control(&id("L32p-30#0"), ControlKey::Brightness, 75)
        .unwrap();
    assert_eq!(panel.written_values(), vec![25]);
    assert_eq!(external.written_values(), vec![75]);
    let (capability, value) = composite
        .read_control(&id("Built-in-display"), ControlKey::Brightness)
        .unwrap()
        .unwrap();
    assert_eq!(value, 25);
    assert!(!capability.rate_limited);
}

#[test]
fn calls_before_any_listing_are_routed_and_unknown_monitors_are_not_found() {
    let (_, panel, composite) = laptop();
    composite
        .write_control(&id("Built-in-display"), ControlKey::Brightness, 5)
        .unwrap();
    assert_eq!(panel.written_values(), vec![5]);
    assert!(matches!(
        composite.read_control(&id("nowhere"), ControlKey::Brightness),
        Err(BackendError::NotFound(_))
    ));
}

#[test]
fn a_failing_backend_does_not_hide_the_others_unless_all_fail() {
    let panel = Arc::new(FakeBackend::single(FakeMonitor::built_in(
        "Built-in-display",
    )));
    let composite = CompositeBackend::new(vec![Arc::new(Broken), panel]);
    assert_eq!(composite.list_monitors().unwrap().len(), 1);

    let nothing = CompositeBackend::new(vec![Arc::new(Broken), Arc::new(Broken)]);
    assert!(nothing.list_monitors().is_err());
}

#[test]
fn the_first_backend_owns_a_duplicated_id() {
    let first = Arc::new(FakeBackend::single(FakeMonitor::reference("same")));
    let second = Arc::new(FakeBackend::single(FakeMonitor::built_in("same")));
    let composite = CompositeBackend::new(vec![first.clone(), second.clone()]);
    assert_eq!(composite.list_monitors().unwrap().len(), 1);
    composite
        .write_control(&id("same"), ControlKey::Brightness, 1)
        .unwrap();
    assert_eq!(first.writes().len(), 1);
    assert!(second.writes().is_empty());
}

#[test]
fn the_composite_meets_the_backend_contract() {
    let (external, panel, composite) = laptop();
    check_backend_contract(&composite);
    assert!(external.writes().is_empty() && panel.writes().is_empty());
}
