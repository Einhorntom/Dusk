//! SPEC-WR: unchanged values, quiet-period buffering and write-rate limits.

use std::time::Duration;

use dusk_app::{Clock, UseCaseError};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, MIN_WRITE_INTERVAL, MonitorId};

fn brightness_monitor() -> (Harness, MonitorId) {
    let monitor = FakeMonitor::new("test-monitor").numeric(ControlKey::Brightness, 50, 100);
    let id = monitor.id().clone();
    (Harness::new(FakeBackend::single(monitor)), id)
}

fn percent(value: u32) -> ControlValue {
    ControlValue::Normalized(value)
}

#[test]
fn unchanged_target_does_not_write_to_monitor() {
    let (h, id) = brightness_monitor();
    assert!(
        !h.service
            .set(&id, ControlKey::Brightness, percent(50))
            .unwrap()
    );
    assert!(h.backend.writes().is_empty());
}

#[test]
fn normalized_values_are_scaled_to_the_native_range() {
    let monitor = FakeMonitor::new("m").numeric(ControlKey::Brightness, 0, 200);
    let id = monitor.id().clone();
    let h = Harness::new(FakeBackend::single(monitor));
    h.service
        .set(&id, ControlKey::Brightness, percent(50))
        .unwrap();
    assert_eq!(h.backend.written_values(), vec![100]);
    assert_eq!(
        h.service.read(&id, ControlKey::Brightness).unwrap().value,
        percent(50)
    );
}

#[test]
fn invalid_targets_fail_without_writing() {
    let (h, id) = brightness_monitor();
    assert!(
        h.service
            .set(&id, ControlKey::Brightness, percent(101))
            .is_err()
    );
    assert!(matches!(
        h.service.set(&id, ControlKey::Volume, percent(10)),
        Err(UseCaseError::UnsupportedControl(ControlKey::Volume))
    ));
    assert!(
        h.service
            .adjust(&id, ControlKey::Input, ControlValue::Enum(1))
            .is_err()
    );
    assert!(h.backend.writes().is_empty());
}

#[test]
fn slider_adjustments_coalesce_until_quiet_period_then_commit_latest_value() {
    let (h, id) = brightness_monitor();
    h.service
        .adjust(&id, ControlKey::Brightness, percent(60))
        .unwrap();
    h.service
        .adjust(&id, ControlKey::Brightness, percent(70))
        .unwrap();

    assert_eq!(
        h.service.flush_pending_adjustments().unwrap(),
        (0, Some(Duration::from_millis(400)))
    );
    h.clock.advance(Duration::from_millis(399));
    assert_eq!(h.service.flush_pending_adjustments().unwrap().0, 0);
    h.clock.advance(Duration::from_millis(1));
    assert_eq!(h.service.flush_pending_adjustments().unwrap(), (1, None));
    assert_eq!(h.backend.written_values(), vec![70]);
}

#[test]
fn rate_limited_adjustment_is_held_and_committed_when_slot_opens() {
    let (h, id) = brightness_monitor();
    h.service
        .set(&id, ControlKey::Brightness, percent(60))
        .unwrap();
    h.service
        .adjust(&id, ControlKey::Brightness, percent(70))
        .unwrap();
    h.clock.advance(Duration::from_millis(400));

    assert_eq!(
        h.service.flush_pending_adjustments().unwrap(),
        (0, Some(Duration::from_millis(600)))
    );
    assert_eq!(h.backend.written_values(), vec![60]);
    h.clock.advance(Duration::from_millis(600));
    assert_eq!(h.service.flush_pending_adjustments().unwrap(), (1, None));
    assert_eq!(h.backend.written_values(), vec![60, 70]);
}

#[test]
fn explicit_set_supersedes_a_pending_slider_target_for_the_same_control() {
    let (h, id) = brightness_monitor();
    h.service
        .adjust(&id, ControlKey::Brightness, percent(60))
        .unwrap();
    h.service
        .set(&id, ControlKey::Brightness, percent(70))
        .unwrap();
    h.clock.advance(Duration::from_secs(2));
    assert_eq!(h.service.flush_pending_adjustments().unwrap(), (0, None));
    assert_eq!(h.backend.written_values(), vec![70]);
}

#[test]
fn explicit_writes_wait_for_the_rate_limiter() {
    // The limits themselves are unit-tested in domain::write_policy.
    let (h, id) = brightness_monitor();
    let start = h.clock.now();
    h.service
        .set(&id, ControlKey::Brightness, percent(10))
        .unwrap();
    h.service
        .set(&id, ControlKey::Brightness, percent(20))
        .unwrap();
    assert_eq!(h.backend.written_values(), vec![10, 20]);
    assert_eq!(h.clock.now() - start, MIN_WRITE_INTERVAL);
}

#[test]
fn settings_are_validated_before_saving() {
    let (h, _) = brightness_monitor();
    let mut settings = h.service.settings().unwrap();
    settings.debounce_ms = 100;
    assert!(matches!(
        h.service.update_settings(settings.clone()),
        Err(UseCaseError::SettingsInvalid(_))
    ));
    settings.debounce_ms = 800;
    h.service.update_settings(settings.clone()).unwrap();
    assert_eq!(h.settings.stored(), settings);
}
