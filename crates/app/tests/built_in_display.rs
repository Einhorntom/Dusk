//! SPEC-PNL: the built-in display is a monitor like any other, except that
//! its writes are not rate-limited (SPEC-PNL-4).

use std::time::Duration;

use dusk_app::{Clock, UseCaseError};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, MonitorId, Preset, PresetEntry};

fn id(text: &str) -> MonitorId {
    MonitorId::new(text).unwrap()
}

fn laptop() -> Harness {
    Harness::new(FakeBackend::new([
        FakeMonitor::built_in("Built-in-display"),
        FakeMonitor::reference("L32p-30#0"),
    ]))
}

fn percent(value: u32) -> ControlValue {
    ControlValue::Normalized(value)
}

#[test]
fn explicit_writes_to_the_built_in_display_do_not_wait_for_a_rate_limit() {
    let h = laptop();
    let panel = id("Built-in-display");
    let start = h.clock.now();
    for value in [10, 20, 30] {
        h.service
            .set(&panel, ControlKey::Brightness, percent(value))
            .unwrap();
    }
    assert_eq!(h.backend.written_values(), vec![10, 20, 30]);
    assert_eq!(h.clock.now(), start);
}

#[test]
fn buffered_changes_keep_the_quiet_period_but_are_never_deferred() {
    let h = laptop();
    let panel = id("Built-in-display");
    let external = id("L32p-30#0");
    for (monitor, value) in [(&panel, 60), (&external, 60)] {
        h.service
            .adjust(monitor, ControlKey::Brightness, percent(value))
            .unwrap();
    }
    h.clock.advance(Duration::from_millis(399));
    assert_eq!(h.service.flush_pending_adjustments().unwrap().0, 0);
    h.clock.advance(Duration::from_millis(1));
    assert_eq!(h.service.flush_pending_adjustments().unwrap(), (2, None));

    // A second change 400 ms later: the panel commits, the external monitor
    // waits for its one-write-per-second slot (SPEC-WR-6).
    for (monitor, value) in [(&panel, 70), (&external, 70)] {
        h.service
            .adjust(monitor, ControlKey::Brightness, percent(value))
            .unwrap();
    }
    h.clock.advance(Duration::from_millis(400));
    let (committed, next) = h.service.flush_pending_adjustments().unwrap();
    assert_eq!(committed, 1);
    assert_eq!(next, Some(Duration::from_millis(600)));
    assert_eq!(h.backend.value(&panel, ControlKey::Brightness), Some(70));
    assert_eq!(h.backend.value(&external, ControlKey::Brightness), Some(60));
}

#[test]
fn the_built_in_display_offers_brightness_only_and_takes_part_in_presets() {
    let h = laptop();
    let panel = id("Built-in-display");
    let captured = h.service.capture_preset("Desk", &panel, true).unwrap();
    let controls: Vec<_> = captured.entries.iter().map(|entry| entry.control).collect();
    assert_eq!(controls, vec![ControlKey::Brightness]);
    assert!(matches!(
        h.service.set(&panel, ControlKey::Contrast, percent(10)),
        Err(UseCaseError::UnsupportedControl(ControlKey::Contrast))
    ));

    h.service
        .save_preset(Preset {
            name: "Night".into(),
            entries: vec![
                PresetEntry {
                    monitor: panel.clone(),
                    control: ControlKey::Brightness,
                    value: percent(15),
                },
                PresetEntry {
                    monitor: id("L32p-30#0"),
                    control: ControlKey::Brightness,
                    value: percent(20),
                },
            ],
        })
        .unwrap();
    let report = h.service.apply_preset("Night").unwrap();
    assert_eq!(report.applied(), 2);
    assert_eq!(h.backend.value(&panel, ControlKey::Brightness), Some(15));
}

#[test]
fn a_built_in_display_that_is_off_is_skipped_like_any_absent_monitor() {
    let h = laptop();
    let panel = id("Built-in-display");
    h.service.capture_preset("Desk", &panel, false).unwrap();
    h.backend.disconnect(&panel);
    let report = h.service.apply_preset("Desk").unwrap();
    assert_eq!(report.skipped(), 1);
    assert_eq!(report.failed(), 0);
}
