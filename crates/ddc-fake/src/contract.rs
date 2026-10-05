//! The `MonitorBackend` contract: behaviour every backend must share, so that
//! tests written against `FakeBackend` hold for the real adapters. It runs
//! against the fake in this crate's tests and, opt-in, against real monitors
//! in `ddc-windows`. It never changes a monitor: the only writes go to a
//! monitor that does not exist or to a control the monitor does not report.

use std::collections::HashSet;

use dusk_app::{BackendError, MonitorBackend};
use dusk_domain::{ControlKey, MonitorId};

/// Panics with a description of the first broken rule.
pub fn check_backend_contract(backend: &dyn MonitorBackend) {
    let monitors = backend.list_monitors().expect("listing monitors succeeds");
    let ids: HashSet<&MonitorId> = monitors.iter().map(|monitor| &monitor.id).collect();
    assert_eq!(ids.len(), monitors.len(), "monitor ids are unique");

    let unknown = MonitorId::new("dusk-contract-no-such-monitor").unwrap();
    assert!(
        matches!(
            backend.read_control(&unknown, ControlKey::Brightness),
            Err(BackendError::NotFound(_))
        ),
        "reading an unknown monitor is NotFound"
    );
    assert!(
        matches!(
            backend.write_control(&unknown, ControlKey::Brightness, 0),
            Err(BackendError::NotFound(_))
        ),
        "writing an unknown monitor is NotFound"
    );

    for monitor in &monitors {
        let mut unsupported = None;
        for control in ControlKey::ALL {
            match backend.read_control(&monitor.id, control) {
                Ok(None) => {
                    unsupported.get_or_insert(control);
                }
                Ok(Some((capability, value))) => {
                    assert_eq!(capability.key, control, "{}: capability key", monitor.id);
                    if control.is_numeric() {
                        assert!(
                            capability.native_max > 0 && value <= capability.native_max,
                            "{}: {control} reads {value} within 0..={}",
                            monitor.id,
                            capability.native_max
                        );
                    } else {
                        assert!(
                            !capability.enum_values.is_empty(),
                            "{}: {control} lists its values",
                            monitor.id
                        );
                    }
                }
                // Real displays may be slow or asleep; that is reported, not a crash.
                Err(BackendError::NotResponding(_)) => {}
                Err(error) => panic!("{}: reading {control} failed: {error}", monitor.id),
            }
        }
        if let Some(control) = unsupported {
            assert!(
                backend.write_control(&monitor.id, control, 0).is_err(),
                "{}: writing unsupported {control} fails",
                monitor.id
            );
        }
    }
}
