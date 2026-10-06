//! SPEC-MON-1: when monitor IDs change (e.g. from position-based to EDID
//! serial-based), saved presets and hotkeys follow the monitor.

use dusk_app::PresetApi;
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, HotkeyBinding, MonitorId, Preset, PresetEntry};

fn id(text: &str) -> MonitorId {
    MonitorId::new(text).unwrap()
}

fn entry(monitor: &str, control: ControlKey, value: u32) -> PresetEntry {
    PresetEntry {
        monitor: id(monitor),
        control,
        value: ControlValue::Normalized(value),
    }
}

fn hotkey(keys: &str, monitor: Option<&str>) -> HotkeyBinding {
    HotkeyBinding {
        keys: keys.parse().unwrap(),
        action: "brightness+".parse().unwrap(),
        monitor: monitor.map(id),
    }
}

/// The monitor is now listed as `L32p-30#U512AY02`; it used to be `L32p-30#0`.
fn renamed() -> Harness {
    Harness::new(FakeBackend::new([
        FakeMonitor::reference("L32p-30#U512AY02"),
        FakeMonitor::built_in("Built-in-display"),
    ]))
}

#[test]
fn presets_and_hotkeys_follow_a_renamed_monitor() {
    let h = renamed();
    h.presets.replace(vec![Preset {
        name: "Night".into(),
        entries: vec![
            entry("L32p-30#0", ControlKey::Brightness, 20),
            entry("Built-in-display", ControlKey::Brightness, 30),
        ],
    }]);
    h.hotkeys.replace(vec![
        hotkey("Ctrl+Alt+Up", Some("L32p-30#0")),
        hotkey("Ctrl+Alt+Down", None),
    ]);

    let moved = h
        .service
        .rename_monitor_ids(&[(id("L32p-30#0"), id("L32p-30#U512AY02"))])
        .unwrap();

    assert_eq!(moved, 2);
    assert_eq!(
        h.presets.stored()[0].entries,
        [
            entry("L32p-30#U512AY02", ControlKey::Brightness, 20),
            entry("Built-in-display", ControlKey::Brightness, 30),
        ]
    );
    assert_eq!(
        h.hotkeys.stored(),
        [
            hotkey("Ctrl+Alt+Up", Some("L32p-30#U512AY02")),
            hotkey("Ctrl+Alt+Down", None)
        ]
    );
    // Applying the preset now reaches the monitor again.
    let report = h.service.apply_preset("Night").unwrap();
    assert_eq!(report.skipped(), 0, "{report:?}");
}

#[test]
fn an_entry_already_under_the_new_id_wins() {
    let h = renamed();
    h.presets.replace(vec![Preset {
        name: "Day".into(),
        entries: vec![
            entry("L32p-30#0", ControlKey::Brightness, 20),
            entry("L32p-30#U512AY02", ControlKey::Brightness, 80),
            entry("L32p-30#0", ControlKey::Contrast, 60),
        ],
    }]);

    let moved = h
        .service
        .rename_monitor_ids(&[(id("L32p-30#0"), id("L32p-30#U512AY02"))])
        .unwrap();

    assert_eq!(moved, 1);
    assert_eq!(
        h.presets.stored()[0].entries,
        [
            entry("L32p-30#U512AY02", ControlKey::Brightness, 80),
            entry("L32p-30#U512AY02", ControlKey::Contrast, 60),
        ]
    );
}

#[test]
fn an_old_id_that_still_names_a_connected_monitor_is_left_alone() {
    // Two unidentifiable monitors keep positional IDs; `Twin#1` is still a
    // monitor, so references to it must not move.
    let h = Harness::new(FakeBackend::new([
        FakeMonitor::reference("Twin#0"),
        FakeMonitor::reference("Twin#1"),
    ]));
    let presets = vec![Preset {
        name: "Both".into(),
        entries: vec![entry("Twin#1", ControlKey::Brightness, 40)],
    }];
    h.presets.replace(presets.clone());

    let moved = h
        .service
        .rename_monitor_ids(&[(id("Twin#1"), id("Twin#0")), (id("x"), id("x"))])
        .unwrap();

    assert_eq!(moved, 0);
    assert_eq!(h.presets.stored(), presets);
}
