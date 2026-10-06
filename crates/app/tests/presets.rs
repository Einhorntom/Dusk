//! SPEC-PRE: capture, apply, edit, import/export and matching of presets.

use dusk_app::PresetApi;
use dusk_app::{BackendError, COLOR_MODE_SETTLE, Clock, EntryStatus, UseCaseError};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, DomainError, MonitorId, Preset, PresetEntry};

const SIX_FIVE_K: u32 = 0x05;
const SEVEN_FIVE_K: u32 = 0x06;
const USER_1: u32 = 0x0B;

fn monitor(id: &str) -> FakeMonitor {
    FakeMonitor::new(id)
        .numeric(ControlKey::Brightness, 50, 100)
        .numeric(ControlKey::Contrast, 50, 100)
        .enumerated(
            ControlKey::ColorPreset,
            SIX_FIVE_K,
            &[SIX_FIVE_K, SEVEN_FIVE_K, USER_1],
        )
        .enumerated(ControlKey::Input, 0x31, &[0x11, 0x31])
}

fn harness_for(monitor: FakeMonitor) -> (Harness, MonitorId) {
    let id = monitor.id().clone();
    (Harness::new(FakeBackend::single(monitor)), id)
}

fn harness() -> (Harness, MonitorId) {
    harness_for(monitor("test-monitor"))
}

fn entry(monitor: &MonitorId, control: ControlKey, value: ControlValue) -> PresetEntry {
    PresetEntry {
        monitor: monitor.clone(),
        control,
        value,
    }
}

fn percent(monitor: &MonitorId, control: ControlKey, value: u32) -> PresetEntry {
    entry(monitor, control, ControlValue::Normalized(value))
}

fn colour(monitor: &MonitorId, value: u32) -> PresetEntry {
    entry(monitor, ControlKey::ColorPreset, ControlValue::Enum(value))
}

fn preset(name: &str, entries: Vec<PresetEntry>) -> Preset {
    Preset {
        name: name.into(),
        entries,
    }
}

fn set_values(h: &Harness, id: &MonitorId, brightness: u32, contrast: u32, colour: u32) {
    h.backend.set_value(id, ControlKey::Brightness, brightness);
    h.backend.set_value(id, ControlKey::Contrast, contrast);
    h.backend.set_value(id, ControlKey::ColorPreset, colour);
}

fn names(h: &Harness) -> Vec<String> {
    h.presets
        .stored()
        .into_iter()
        .map(|preset| preset.name)
        .collect()
}

// ------------------------------------------------------------------ capture

#[test]
fn capture_stores_supported_controls_and_skips_input_by_default() {
    let (h, id) = harness();
    let preset = h.service.capture_preset(" Day ", &id, false).unwrap();
    assert_eq!(preset.name, "Day");
    let controls: Vec<_> = preset.entries.iter().map(|entry| entry.control).collect();
    assert_eq!(
        controls,
        vec![
            ControlKey::Brightness,
            ControlKey::Contrast,
            ControlKey::ColorPreset
        ]
    );
    let with_input = h.service.capture_preset("Day", &id, true).unwrap();
    assert_eq!(with_input.entries.len(), 4);
    assert_eq!(names(&h), vec!["Day"]);
}

#[test]
fn capture_fails_for_a_monitor_without_capturable_controls() {
    let (h, id) = harness_for(FakeMonitor::new("bare"));
    assert!(matches!(
        h.service.capture_preset("Empty", &id, true),
        Err(UseCaseError::NothingToCapture(monitor)) if monitor == id
    ));
    assert!(h.presets.stored().is_empty());
}

// -------------------------------------------------------------------- apply

#[test]
fn apply_writes_in_spec_order_and_second_apply_is_a_no_op() {
    let (h, id) = harness();
    set_values(&h, &id, 20, 30, SEVEN_FIVE_K);
    h.service.capture_preset("Night", &id, false).unwrap();
    set_values(&h, &id, 90, 80, SIX_FIVE_K);

    let report = h.service.apply_preset("night").unwrap();
    assert_eq!(report.preset, "Night");
    assert_eq!(report.applied(), 3);
    assert_eq!(
        h.backend.written_controls(),
        vec![
            ControlKey::ColorPreset,
            ControlKey::Brightness,
            ControlKey::Contrast
        ]
    );
    let again = h.service.apply_preset("Night").unwrap();
    assert_eq!(again.applied(), 0);
    assert_eq!(again.unchanged(), 3);
    assert_eq!(h.backend.writes().len(), 3);
}

#[test]
fn apply_skips_absent_monitors_and_unsupported_controls() {
    let monitor = FakeMonitor::new("test-monitor").numeric(ControlKey::Brightness, 50, 100);
    let (h, id) = harness_for(monitor);
    let other = MonitorId::new("other-monitor").unwrap();
    h.service
        .save_preset(preset(
            "Mixed",
            vec![
                percent(&other, ControlKey::Brightness, 10),
                percent(&id, ControlKey::Volume, 10),
                percent(&id, ControlKey::Brightness, 10),
            ],
        ))
        .unwrap();
    let report = h.service.apply_preset("Mixed").unwrap();
    assert_eq!(report.skipped(), 2);
    assert_eq!(report.applied(), 1);
    assert_eq!(report.failed(), 0);
}

#[test]
fn a_disconnected_monitor_is_skipped() {
    let (h, id) = harness();
    h.service
        .save_preset(preset("P", vec![percent(&id, ControlKey::Brightness, 10)]))
        .unwrap();
    h.backend.disconnect(&id);
    let report = h.service.apply_preset("P").unwrap();
    assert_eq!(report.skipped(), 1);
    assert!(h.backend.writes().is_empty());
}

#[test]
fn an_invalid_entry_does_not_stop_the_others() {
    let (h, id) = harness();
    h.service
        .save_preset(preset(
            "Bad",
            vec![colour(&id, 0x99), percent(&id, ControlKey::Brightness, 10)],
        ))
        .unwrap();
    let report = h.service.apply_preset("Bad").unwrap();
    assert_eq!(report.failed(), 1);
    assert_eq!(report.applied(), 1);
}

#[test]
fn a_failing_write_does_not_stop_later_writes() {
    let (h, id) = harness();
    h.backend.fail_writes(
        &id,
        ControlKey::Brightness,
        BackendError::Failed("DDC/CI error".into()),
    );
    h.service
        .save_preset(preset(
            "P",
            vec![
                percent(&id, ControlKey::Brightness, 10),
                percent(&id, ControlKey::Contrast, 10),
            ],
        ))
        .unwrap();
    let report = h.service.apply_preset("P").unwrap();
    assert_eq!(report.failed(), 1);
    assert_eq!(report.applied(), 1);
    assert!(matches!(
        &report.outcomes[0].status,
        EntryStatus::Failed(reason) if reason.contains("DDC/CI error")
    ));
    assert_eq!(h.backend.written_controls(), vec![ControlKey::Contrast]);
}

#[test]
fn applying_an_unknown_preset_is_reported() {
    let (h, _) = harness();
    assert!(matches!(
        h.service.apply_preset("Nope"),
        Err(UseCaseError::PresetNotFound(_))
    ));
}

// ------------------------------------------------- colour preset and gains

fn gain_monitor(id: &str, colour: u32) -> FakeMonitor {
    monitor(id)
        .numeric(ControlKey::GainRed, 100, 100)
        .enumerated(
            ControlKey::ColorPreset,
            colour,
            &[SIX_FIVE_K, SEVEN_FIVE_K, USER_1],
        )
}

#[test]
fn fixed_colour_preset_ignores_gains_on_capture_and_apply() {
    let (h, id) = harness_for(gain_monitor("test-monitor", SEVEN_FIVE_K));
    let captured = h.service.capture_preset("Day", &id, false).unwrap();
    assert!(
        captured
            .entries
            .iter()
            .all(|entry| entry.control != ControlKey::GainRed)
    );

    // A preset saved before this rule still carries the gain entry.
    h.service
        .save_preset(preset(
            "Day",
            vec![
                colour(&id, SEVEN_FIVE_K),
                percent(&id, ControlKey::GainRed, 100),
            ],
        ))
        .unwrap();
    h.backend.set_value(&id, ControlKey::ColorPreset, USER_1);
    h.backend.set_value(&id, ControlKey::GainRed, 40);
    let report = h.service.apply_preset("Day").unwrap();
    assert_eq!(report.applied(), 1);
    assert_eq!(report.skipped(), 1);
    assert_eq!(h.backend.written_controls(), vec![ControlKey::ColorPreset]);
    assert_eq!(h.service.matching_preset().unwrap().as_deref(), Some("Day"));
}

#[test]
fn colour_mode_switch_writes_later_values_back_to_back_after_one_settle() {
    let (h, id) = harness_for(gain_monitor("test-monitor", SEVEN_FIVE_K));
    h.backend.set_value(&id, ControlKey::Brightness, 65);
    h.service
        .save_preset(preset(
            "Night",
            vec![
                percent(&id, ControlKey::Brightness, 65),
                percent(&id, ControlKey::GainRed, 100),
                colour(&id, USER_1),
            ],
        ))
        .unwrap();
    let started = h.clock.now();
    let report = h.service.apply_preset("Night").unwrap();
    // The monitor reloads per-mode values, so matching ones are rewritten.
    assert_eq!(report.applied(), 3);
    assert_eq!(
        h.backend.written_controls(),
        vec![
            ControlKey::ColorPreset,
            ControlKey::GainRed,
            ControlKey::Brightness
        ]
    );
    assert_eq!(h.clock.now() - started, COLOR_MODE_SETTLE);
}

#[test]
fn a_colour_switch_on_one_monitor_does_not_force_writes_on_another() {
    let left = monitor("left");
    let right = monitor("right");
    let (left_id, right_id) = (left.id().clone(), right.id().clone());
    let h = Harness::new(FakeBackend::new([left, right]));
    h.service
        .save_preset(preset(
            "Both",
            vec![
                colour(&left_id, SEVEN_FIVE_K),
                percent(&left_id, ControlKey::Brightness, 50),
                percent(&right_id, ControlKey::Brightness, 50),
                percent(&right_id, ControlKey::Contrast, 20),
            ],
        ))
        .unwrap();
    let report = h.service.apply_preset("Both").unwrap();
    assert_eq!(report.applied(), 3);
    assert_eq!(report.unchanged(), 1);
    let right_writes: Vec<_> = h
        .backend
        .writes()
        .into_iter()
        .filter(|write| write.monitor == right_id)
        .map(|write| write.control)
        .collect();
    assert_eq!(right_writes, vec![ControlKey::Contrast]);
}

// ---------------------------------------------------- naming and ordering

#[test]
fn preset_names_are_unique_case_insensitively_and_can_be_renamed_moved_and_deleted() {
    let (h, id) = harness();
    for name in ["A", "B", "C"] {
        h.service.capture_preset(name, &id, false).unwrap();
    }
    h.service.capture_preset("b", &id, false).unwrap();
    assert_eq!(h.presets.stored().len(), 3);

    assert!(matches!(
        h.service.rename_preset("A", "C"),
        Err(UseCaseError::PresetExists(_))
    ));
    h.service.rename_preset("A", "Alpha").unwrap();
    h.service.move_preset("C", -2).unwrap();
    assert_eq!(names(&h), vec!["C", "Alpha", "b"]);

    h.service.delete_preset("alpha").unwrap();
    assert!(matches!(
        h.service.delete_preset("Alpha"),
        Err(UseCaseError::PresetNotFound(_))
    ));
    assert!(matches!(
        h.service.capture_preset("", &id, false),
        Err(UseCaseError::Domain(DomainError::InvalidPresetName))
    ));
}

#[test]
fn cycling_wraps_around_starting_from_the_last_applied_preset() {
    let (h, id) = harness();
    assert!(matches!(
        h.service.cycle_preset(true),
        Err(UseCaseError::NoPresets)
    ));
    for name in ["A", "B"] {
        h.service.capture_preset(name, &id, false).unwrap();
    }
    assert_eq!(h.service.cycle_preset(true).unwrap().preset, "A");
    assert_eq!(h.service.cycle_preset(true).unwrap().preset, "B");
    assert_eq!(h.service.cycle_preset(true).unwrap().preset, "A");
    assert_eq!(h.service.cycle_preset(false).unwrap().preset, "B");
}

// ----------------------------------------------------------------- matching

#[test]
fn matching_preset_reports_the_preset_equal_to_current_state() {
    let (h, id) = harness();
    set_values(&h, &id, 20, 30, SEVEN_FIVE_K);
    h.service.capture_preset("Night", &id, false).unwrap();
    assert_eq!(
        h.service.matching_preset().unwrap().as_deref(),
        Some("Night")
    );
    h.backend.set_value(&id, ControlKey::Brightness, 21);
    assert_eq!(h.service.matching_preset().unwrap(), None);
}

#[test]
fn matching_ignores_absent_monitors_but_needs_one_compared_entry() {
    let (h, id) = harness();
    let other = MonitorId::new("other").unwrap();
    h.service
        .save_preset(preset(
            "Elsewhere",
            vec![percent(&other, ControlKey::Brightness, 10)],
        ))
        .unwrap();
    h.service
        .save_preset(preset(
            "Here",
            vec![
                percent(&other, ControlKey::Brightness, 10),
                percent(&id, ControlKey::Brightness, 50),
            ],
        ))
        .unwrap();
    assert_eq!(
        h.service.matching_preset().unwrap().as_deref(),
        Some("Here")
    );
}

// ------------------------------------------------------------------ editing

#[test]
fn entries_can_be_added_changed_and_removed() {
    let (h, id) = harness();
    h.service
        .save_preset(preset("P", vec![percent(&id, ControlKey::Brightness, 10)]))
        .unwrap();
    h.service
        .set_preset_entry("p", percent(&id, ControlKey::Brightness, 40))
        .unwrap();
    h.service
        .set_preset_entry("P", colour(&id, USER_1))
        .unwrap();
    assert_eq!(
        h.presets.stored()[0].entries,
        vec![
            percent(&id, ControlKey::Brightness, 40),
            colour(&id, USER_1)
        ]
    );

    h.service
        .remove_preset_entry("P", &id, ControlKey::Brightness)
        .unwrap();
    assert_eq!(h.presets.stored()[0].entries, vec![colour(&id, USER_1)]);
    assert!(matches!(
        h.service
            .remove_preset_entry("P", &id, ControlKey::Brightness),
        Err(UseCaseError::PresetEntryNotFound {
            control: ControlKey::Brightness,
            ..
        })
    ));
}

#[test]
fn invalid_entry_edits_change_nothing() {
    let (h, id) = harness();
    let original = preset("P", vec![percent(&id, ControlKey::Brightness, 10)]);
    h.service.save_preset(original.clone()).unwrap();
    assert!(
        h.service
            .set_preset_entry("P", percent(&id, ControlKey::Brightness, 101))
            .is_err()
    );
    assert!(
        h.service
            .set_preset_entry(
                "P",
                entry(&id, ControlKey::Brightness, ControlValue::Enum(1))
            )
            .is_err()
    );
    assert!(
        h.service
            .set_preset_entry("P", percent(&id, ControlKey::Input, 1))
            .is_err()
    );
    assert!(matches!(
        h.service
            .set_preset_entry("Missing", percent(&id, ControlKey::Brightness, 1)),
        Err(UseCaseError::PresetNotFound(_))
    ));
    assert_eq!(h.presets.stored(), vec![original]);
}

// ------------------------------------------------------------ import/export

#[test]
fn import_merges_by_name_and_export_round_trips() {
    let (h, id) = harness();
    let a = preset("A", vec![percent(&id, ControlKey::Brightness, 10)]);
    let b = preset("B", vec![percent(&id, ControlKey::Brightness, 20)]);
    h.presets.replace(vec![a.clone(), b]);

    let new_b = preset("b", vec![percent(&id, ControlKey::Brightness, 25)]);
    let c = preset("C", vec![percent(&id, ControlKey::Brightness, 30)]);
    let document = h.presets.document(vec![new_b.clone(), c.clone()]);
    let summary = h.service.import_presets(&document, false).unwrap();
    assert_eq!((summary.added, summary.updated, summary.removed), (1, 1, 0));
    assert_eq!(h.presets.stored(), vec![a, new_b, c]);

    let exported = h.service.export_presets().unwrap();
    let before = h.presets.stored();
    h.service.import_presets(&exported, true).unwrap();
    assert_eq!(h.presets.stored(), before);
}

#[test]
fn import_with_replace_discards_presets_missing_from_the_document() {
    let (h, id) = harness();
    h.presets.replace(vec![
        preset("A", vec![percent(&id, ControlKey::Brightness, 10)]),
        preset("B", vec![percent(&id, ControlKey::Brightness, 20)]),
    ]);
    let c = preset("C", vec![percent(&id, ControlKey::Brightness, 30)]);
    let document = h.presets.document(vec![c.clone()]);
    let summary = h.service.import_presets(&document, true).unwrap();
    assert_eq!((summary.added, summary.updated, summary.removed), (1, 0, 2));
    assert_eq!(h.presets.stored(), vec![c]);
}

#[test]
fn an_unreadable_import_changes_nothing() {
    let (h, id) = harness();
    let original = vec![preset("A", vec![percent(&id, ControlKey::Brightness, 10)])];
    h.presets.replace(original.clone());
    assert!(h.service.import_presets("garbage", true).is_err());
    assert_eq!(h.presets.stored(), original);
    assert_eq!(h.service.presets_location(), "memory://presets");
}

#[test]
fn concurrent_edits_never_lose_each_other() {
    // Regression: each edit loaded the presets, changed them and saved them
    // without a lock, so two clients saving at once could drop one preset.
    let h = Harness::new(FakeBackend::single(FakeMonitor::reference("m")));
    let service = h.service.clone();
    let writers: Vec<_> = (0..8)
        .map(|writer| {
            let service = service.clone();
            std::thread::spawn(move || {
                for index in 0..10 {
                    service
                        .save_preset(Preset {
                            name: format!("P{writer}-{index}"),
                            entries: Vec::new(),
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(h.presets.stored().len(), 80);
}
