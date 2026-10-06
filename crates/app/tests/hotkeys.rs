//! SPEC-HK: stored bindings and what pressing a hotkey does.

use std::time::Duration;

use dusk_app::{BackendError, HotkeyOutcome, UseCaseError};
use dusk_app::{ControlApi, HotkeyApi, PresetApi, SettingsApi};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness, ScriptedPrompter};
use dusk_domain::{ControlKey, DomainError, HotkeyAction, HotkeyBinding, Key, KeyCombo, MonitorId};

fn keys(text: &str) -> KeyCombo {
    text.parse().unwrap()
}

fn binding(combo: &str, action: &str) -> HotkeyBinding {
    HotkeyBinding {
        keys: keys(combo),
        action: action.parse().unwrap(),
        monitor: None,
    }
}

fn id(text: &str) -> MonitorId {
    MonitorId::new(text).unwrap()
}

fn reference() -> Harness {
    Harness::new(FakeBackend::single(FakeMonitor::reference("m")))
}

fn bound(h: &Harness, combo: &str, action: &str) -> KeyCombo {
    h.service.save_hotkey(binding(combo, action)).unwrap();
    keys(combo)
}

// ------------------------------------------------------------------ storage

#[test]
fn saving_validates_and_replaces_bindings_with_the_same_keys() {
    let h = reference();
    let bare = HotkeyBinding {
        keys: KeyCombo {
            ctrl: false,
            alt: false,
            shift: true,
            win: false,
            key: Key::Up,
        },
        action: HotkeyAction::NextPreset,
        monitor: None,
    };
    assert!(matches!(
        h.service.save_hotkey(bare),
        Err(UseCaseError::Domain(DomainError::InvalidHotkey(_)))
    ));
    assert!(matches!(
        h.service.save_hotkey(binding("Ctrl+Alt+N", "preset:Night")),
        Err(UseCaseError::PresetNotFound(_))
    ));

    h.service
        .save_hotkey(binding("Ctrl+Alt+Up", "brightness+"))
        .unwrap();
    h.service
        .save_hotkey(binding("Ctrl+Alt+Up", "contrast+"))
        .unwrap();
    h.service
        .save_hotkey(binding("Ctrl+Alt+Down", "contrast-"))
        .unwrap();
    let stored = h.service.list_hotkeys().unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].action.to_string(), "contrast+");
}

#[test]
fn removing_an_unbound_combination_is_reported() {
    let h = reference();
    let combo = bound(&h, "Ctrl+Alt+Up", "brightness+");
    h.service.remove_hotkey(&combo).unwrap();
    assert!(h.hotkeys.stored().is_empty());
    assert!(matches!(
        h.service.remove_hotkey(&combo),
        Err(UseCaseError::HotkeyNotFound(_))
    ));
    assert!(matches!(
        h.service.run_hotkey(&combo),
        Err(UseCaseError::HotkeyNotFound(_))
    ));
}

#[test]
fn step_sizes_are_validated_with_the_other_settings() {
    let h = reference();
    let mut settings = h.service.settings().unwrap();
    settings.volume_step = 0;
    assert!(h.service.update_settings(settings.clone()).is_err());
    settings.volume_step = 26;
    assert!(h.service.update_settings(settings.clone()).is_err());
    settings.volume_step = 25;
    h.service.update_settings(settings).unwrap();
}

// -------------------------------------------------------------------- steps

#[test]
fn held_step_keys_accumulate_and_write_once_after_the_quiet_period() {
    let h = reference();
    let up = bound(&h, "Ctrl+Alt+Up", "brightness+");
    for expected in [55, 60, 65] {
        let outcome = h.service.run_hotkey(&up).unwrap();
        assert_eq!(
            outcome,
            HotkeyOutcome::Stepped {
                control: ControlKey::Brightness,
                values: vec![(id("m"), expected)],
            }
        );
    }
    assert!(h.backend.writes().is_empty());
    h.clock.advance(Duration::from_millis(400));
    assert_eq!(h.service.flush_pending_adjustments().unwrap(), (1, None));
    assert_eq!(h.backend.value(&id("m"), ControlKey::Brightness), Some(65));
}

#[test]
fn steps_use_the_configured_size_and_clamp_to_the_range() {
    let h = reference();
    let mut settings = h.service.settings().unwrap();
    settings.contrast_step = 20;
    h.service.update_settings(settings).unwrap();
    let up = bound(&h, "Ctrl+Alt+Up", "contrast+");
    let down = bound(&h, "Ctrl+Alt+Down", "contrast-");

    let target = |outcome| match outcome {
        HotkeyOutcome::Stepped { values, .. } => values[0].1,
        other => panic!("unexpected outcome {other:?}"),
    };
    assert_eq!(target(h.service.run_hotkey(&up).unwrap()), 70);
    assert_eq!(target(h.service.run_hotkey(&up).unwrap()), 90);
    assert_eq!(target(h.service.run_hotkey(&up).unwrap()), 100);
    assert_eq!(target(h.service.run_hotkey(&up).unwrap()), 100);
    for _ in 0..6 {
        h.service.run_hotkey(&down).unwrap();
    }
    assert_eq!(target(h.service.run_hotkey(&down).unwrap()), 0);
}

#[test]
fn unbound_steps_act_on_every_monitor_that_supports_the_control() {
    let left = FakeMonitor::reference("left");
    let right = FakeMonitor::new("right").numeric(ControlKey::Brightness, 20, 100);
    let h = Harness::new(FakeBackend::new([left, right]));
    let volume = bound(&h, "Ctrl+Alt+V", "volume+");
    let brightness = bound(&h, "Ctrl+Alt+B", "brightness-");

    let HotkeyOutcome::Stepped { values, .. } = h.service.run_hotkey(&brightness).unwrap() else {
        panic!("expected a step");
    };
    assert_eq!(values, vec![(id("left"), 45), (id("right"), 15)]);
    let HotkeyOutcome::Stepped { values, .. } = h.service.run_hotkey(&volume).unwrap() else {
        panic!("expected a step");
    };
    assert_eq!(values, vec![(id("left"), 25)]);
}

#[test]
fn a_step_no_monitor_supports_is_an_error() {
    let h = Harness::new(FakeBackend::single(FakeMonitor::new("m").numeric(
        ControlKey::Brightness,
        50,
        100,
    )));
    let volume = bound(&h, "Ctrl+Alt+V", "volume+");
    assert!(matches!(
        h.service.run_hotkey(&volume),
        Err(UseCaseError::UnsupportedControl(ControlKey::Volume))
    ));
}

#[test]
fn a_binding_for_a_disconnected_monitor_reports_it() {
    let left = FakeMonitor::reference("left");
    let right = FakeMonitor::reference("right");
    let h = Harness::new(FakeBackend::new([left, right]));
    h.service
        .save_hotkey(HotkeyBinding {
            monitor: Some(id("right")),
            ..binding("Ctrl+Alt+Up", "brightness+")
        })
        .unwrap();
    let combo = keys("Ctrl+Alt+Up");
    let HotkeyOutcome::Stepped { values, .. } = h.service.run_hotkey(&combo).unwrap() else {
        panic!("expected a step");
    };
    assert_eq!(values, vec![(id("right"), 55)]);

    h.backend.disconnect(&id("right"));
    assert!(matches!(
        h.service.run_hotkey(&combo),
        Err(UseCaseError::Backend(BackendError::NotFound(_)))
    ));
}

// ------------------------------------------------------------------ presets

#[test]
fn preset_hotkeys_apply_and_cycle_presets() {
    let h = reference();
    h.service.capture_preset("Day", &id("m"), false).unwrap();
    h.backend.set_value(&id("m"), ControlKey::Brightness, 10);
    h.service.capture_preset("Night", &id("m"), false).unwrap();

    let day = bound(&h, "Ctrl+Alt+D", "preset:day");
    let next = bound(&h, "Ctrl+Alt+Right", "preset-next");
    let HotkeyOutcome::Preset(report) = h.service.run_hotkey(&day).unwrap() else {
        panic!("expected a preset");
    };
    assert_eq!(report.preset, "Day");
    assert_eq!(h.backend.value(&id("m"), ControlKey::Brightness), Some(50));
    let HotkeyOutcome::Preset(report) = h.service.run_hotkey(&next).unwrap() else {
        panic!("expected a preset");
    };
    assert_eq!(report.preset, "Night");
}

#[test]
fn renaming_a_preset_updates_its_hotkeys_and_deleting_removes_them() {
    let h = reference();
    h.service.capture_preset("Night", &id("m"), false).unwrap();
    let night = bound(&h, "Ctrl+Alt+N", "preset:Night");
    bound(&h, "Ctrl+Alt+Up", "brightness+");
    assert_eq!(
        h.service.hotkeys_using_preset("night").unwrap(),
        vec![night]
    );

    h.service.rename_preset("Night", "Evening").unwrap();
    assert_eq!(
        h.hotkeys.stored()[0].action,
        HotkeyAction::ApplyPreset("Evening".into())
    );
    assert!(h.service.run_hotkey(&night).is_ok());

    h.service.delete_preset("evening").unwrap();
    let remaining: Vec<_> = h
        .hotkeys
        .stored()
        .into_iter()
        .map(|item| item.keys)
        .collect();
    assert_eq!(remaining, vec![keys("Ctrl+Alt+Up")]);
}

// ------------------------------------------------------------ input, power

#[test]
fn input_hotkeys_ask_for_confirmation() {
    let h = Harness::with_prompter(
        FakeBackend::single(FakeMonitor::reference("m")),
        ScriptedPrompter::declining(),
    );
    let hdmi = bound(&h, "Ctrl+Alt+H", "input:0x11");
    assert!(matches!(
        h.service.run_hotkey(&hdmi),
        Err(UseCaseError::InputChangeDeclined)
    ));
    assert_eq!(h.prompter.confirmations_asked(), 1);
    assert!(h.backend.writes().is_empty());

    let accepting = reference();
    let hdmi = bound(&accepting, "Ctrl+Alt+H", "input:0x11");
    assert_eq!(
        accepting.service.run_hotkey(&hdmi).unwrap(),
        HotkeyOutcome::Input {
            value: 0x11,
            changed: 1
        }
    );
    assert_eq!(
        accepting.service.run_hotkey(&hdmi).unwrap(),
        HotkeyOutcome::Input {
            value: 0x11,
            changed: 0
        }
    );
}

#[test]
fn an_input_value_no_monitor_offers_is_rejected() {
    let h = reference();
    let dp2 = bound(&h, "Ctrl+Alt+P", "input:0x10");
    assert!(matches!(
        h.service.run_hotkey(&dp2),
        Err(UseCaseError::Domain(
            DomainError::EnumValueUnavailable { .. }
        ))
    ));
}

#[test]
fn power_toggles_between_on_and_soft_off() {
    let h = reference();
    let power = bound(&h, "Ctrl+Alt+F12", "power-toggle");
    assert_eq!(
        h.service.run_hotkey(&power).unwrap(),
        HotkeyOutcome::Power {
            on: false,
            changed: 1
        }
    );
    assert_eq!(h.backend.value(&id("m"), ControlKey::Power), Some(0x04));
    assert_eq!(
        h.service.run_hotkey(&power).unwrap(),
        HotkeyOutcome::Power {
            on: true,
            changed: 1
        }
    );
    assert_eq!(h.backend.value(&id("m"), ControlKey::Power), Some(0x01));
}
