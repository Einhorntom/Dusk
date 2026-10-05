//! The Settings view model over simulated monitors.

use std::time::Duration;

use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dusk_domain::{ControlKey, ControlValue, MonitorId};
use dusk_ui_model::MonitorSettingsModel;

fn model_with(monitors: Vec<FakeMonitor>) -> (MonitorSettingsModel, Harness) {
    let h = Harness::new(FakeBackend::new(monitors));
    let mut model = MonitorSettingsModel::new(h.service.clone());
    model.refresh().unwrap();
    (model, h)
}

fn shown_value(model: &MonitorSettingsModel, key: ControlKey) -> Option<ControlValue> {
    model
        .controls()
        .iter()
        .find(|reading| reading.capability.key == key)
        .map(|reading| reading.value)
}

#[test]
fn refresh_selects_the_first_monitor_and_lists_only_supported_controls() {
    let (model, _) = model_with(vec![
        FakeMonitor::new("a")
            .numeric(ControlKey::Contrast, 30, 100)
            .numeric(ControlKey::Brightness, 70, 100),
        FakeMonitor::reference("b"),
    ]);
    assert_eq!(model.selected_monitor().unwrap().as_str(), "a");
    let keys: Vec<_> = model
        .controls()
        .iter()
        .map(|reading| reading.capability.key)
        .collect();
    assert_eq!(keys, vec![ControlKey::Brightness, ControlKey::Contrast]);
    assert_eq!(model.selected_control(), Some(ControlKey::Brightness));
}

#[test]
fn switching_monitors_reloads_controls_and_rejects_unknown_ones() {
    let (mut model, _) = model_with(vec![
        FakeMonitor::new("a").numeric(ControlKey::Brightness, 70, 100),
        FakeMonitor::reference("b"),
    ]);
    model.select_monitor(&MonitorId::new("b").unwrap()).unwrap();
    assert_eq!(model.controls().len(), 9);
    assert!(
        model
            .select_monitor(&MonitorId::new("gone").unwrap())
            .is_err()
    );
    assert_eq!(model.selected_monitor().unwrap().as_str(), "b");
}

#[test]
fn a_written_value_is_shown_without_reading_it_back() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    let id = MonitorId::new("m").unwrap();
    model.select_control(ControlKey::ColorPreset);
    assert!(model.set_selected_value(ControlValue::Enum(0x0B)).unwrap());
    assert_eq!(h.backend.value(&id, ControlKey::ColorPreset), Some(0x0B));

    // A busy monitor may still report the old value; the model keeps the write.
    h.backend.set_value(&id, ControlKey::ColorPreset, 0x06);
    assert_eq!(
        shown_value(&model, ControlKey::ColorPreset),
        Some(ControlValue::Enum(0x0B))
    );
    model.refresh().unwrap();
    assert_eq!(
        shown_value(&model, ControlKey::ColorPreset),
        Some(ControlValue::Enum(0x06))
    );
}

#[test]
fn slider_moves_are_written_once_after_the_quiet_period() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    model.select_control(ControlKey::Brightness);
    for value in [55, 60, 65] {
        model
            .adjust_selected_value(ControlValue::Normalized(value))
            .unwrap();
    }
    assert_eq!(model.flush_pending_adjustments().unwrap().0, 0);
    h.clock.advance(Duration::from_millis(400));
    assert_eq!(model.flush_pending_adjustments().unwrap(), (1, None));
    assert_eq!(h.backend.written_values(), vec![65]);
    assert_eq!(
        shown_value(&model, ControlKey::Brightness),
        Some(ControlValue::Normalized(65))
    );
}

#[test]
fn edits_without_a_selection_are_rejected() {
    let (mut model, h) = model_with(vec![]);
    assert!(model.selected_monitor().is_none());
    assert!(
        model
            .set_selected_value(ControlValue::Normalized(1))
            .is_err()
    );
    assert!(
        model
            .adjust_selected_value(ControlValue::Normalized(1))
            .is_err()
    );
    assert!(model.save_current_as_preset("P", false).is_err());
    assert!(h.backend.writes().is_empty());
}

#[test]
fn presets_are_saved_applied_and_marked_as_matching() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    let id = MonitorId::new("m").unwrap();
    model.save_current_as_preset("Day", false).unwrap();
    assert_eq!(model.presets().len(), 1);
    assert_eq!(model.matching_preset(), Some("Day"));

    h.backend.set_value(&id, ControlKey::Brightness, 10);
    model.refresh_presets().unwrap();
    assert_eq!(model.matching_preset(), None);

    let report = model.apply_preset("Day").unwrap();
    assert_eq!(report.applied(), 1);
    assert_eq!(model.matching_preset(), Some("Day"));
    assert_eq!(
        shown_value(&model, ControlKey::Brightness),
        Some(ControlValue::Normalized(50))
    );
}

#[test]
fn preset_edits_and_imports_refresh_the_list() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    model.save_current_as_preset("Day", false).unwrap();
    model.rename_preset("Day", "Sunny").unwrap();
    assert_eq!(model.presets()[0].name, "Sunny");

    let entry = model.presets()[0].entries[0].clone();
    model
        .remove_preset_entry("Sunny", &entry.monitor, entry.control)
        .unwrap();
    // Brightness, contrast, colour preset and volume were captured.
    assert_eq!(model.presets()[0].entries.len(), 3);

    let document = h.presets.document(vec![]);
    let summary = model.import_presets(&document, true).unwrap();
    assert_eq!(summary.removed, 1);
    assert!(model.presets().is_empty());
    assert_eq!(model.presets_location(), "memory://presets");
}

#[test]
fn settings_updates_are_stored_and_kept_in_the_model() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    let mut settings = model.settings().clone();
    settings.input_revert_seconds = 15;
    model.update_settings(settings.clone()).unwrap();
    assert_eq!(model.settings(), &settings);
    assert_eq!(h.settings.stored(), settings);

    settings.debounce_ms = 1;
    assert!(model.update_settings(settings).is_err());
    assert_eq!(model.settings().debounce_ms, 400);
}

fn binding(keys: &str, action: &str) -> dusk_domain::HotkeyBinding {
    dusk_domain::HotkeyBinding {
        keys: keys.parse().unwrap(),
        action: action.parse().unwrap(),
        monitor: None,
    }
}

#[test]
fn a_step_hotkey_shows_the_new_target_before_it_is_written() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    model
        .save_hotkey(binding("Ctrl+Alt+Up", "brightness+"))
        .unwrap();
    assert_eq!(model.hotkeys().len(), 1);
    let keys = model.hotkeys()[0].keys;
    model.run_hotkey(&keys).unwrap();
    model.run_hotkey(&keys).unwrap();
    assert_eq!(
        shown_value(&model, ControlKey::Brightness),
        Some(ControlValue::Normalized(60))
    );
    assert!(h.backend.writes().is_empty());
}

#[test]
fn preset_hotkeys_refresh_values_and_follow_renames_and_deletes() {
    let (mut model, h) = model_with(vec![FakeMonitor::reference("m")]);
    let id = MonitorId::new("m").unwrap();
    model.save_current_as_preset("Day", false).unwrap();
    model
        .save_hotkey(binding("Ctrl+Alt+D", "preset:Day"))
        .unwrap();
    h.backend.set_value(&id, ControlKey::Contrast, 5);

    model.run_hotkey(&model.hotkeys()[0].keys.clone()).unwrap();
    assert_eq!(
        shown_value(&model, ControlKey::Contrast),
        Some(ControlValue::Normalized(50))
    );

    model.rename_preset("Day", "Sunny").unwrap();
    assert_eq!(model.hotkeys()[0].action.to_string(), "preset:Sunny");
    assert_eq!(model.hotkeys_using_preset("sunny").unwrap().len(), 1);
    model.delete_preset("Sunny").unwrap();
    assert!(model.hotkeys().is_empty());

    model.save_hotkey(binding("Ctrl+Alt+V", "volume+")).unwrap();
    let keys = model.hotkeys()[0].keys;
    model.remove_hotkey(&keys).unwrap();
    assert!(model.hotkeys().is_empty());
}

#[test]
fn the_add_hotkey_form_offers_the_selected_monitors_inputs_and_builds_a_binding() {
    let (mut model, _) = model_with(vec![
        FakeMonitor::reference("left"),
        FakeMonitor::reference("right"),
    ]);
    model.save_current_as_preset("Night", false).unwrap();
    let choices: Vec<String> = model
        .hotkey_choices()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(choices.contains(&"preset:Night".to_owned()));
    assert!(choices.contains(&"input:0x11".to_owned()));
    let index = |text: &str| choices.iter().position(|choice| choice == text);

    let targeted = model
        .hotkey_from_form("Ctrl+Alt+Up", index("brightness+"), Some(2))
        .unwrap();
    assert_eq!(targeted.monitor, Some(MonitorId::new("right").unwrap()));
    let everywhere = model
        .hotkey_from_form("Ctrl+Alt+Up", index("brightness+"), Some(0))
        .unwrap();
    assert_eq!(everywhere.monitor, None);
    // Preset actions use the preset's own monitors.
    let preset = model
        .hotkey_from_form("Ctrl+Alt+N", index("preset:Night"), Some(1))
        .unwrap();
    assert_eq!(preset.monitor, None);
}

#[test]
fn an_incomplete_add_hotkey_form_explains_what_is_missing() {
    let (model, _) = model_with(vec![FakeMonitor::reference("m")]);
    let empty = model.hotkey_from_form("  ", Some(0), None).unwrap_err();
    assert!(empty.contains("press the combination"));
    let bare = model
        .hotkey_from_form("Shift+A", Some(0), None)
        .unwrap_err();
    assert!(bare.contains("Ctrl, Alt or Win"));
    let no_action = model.hotkey_from_form("F9", None, None).unwrap_err();
    assert!(no_action.contains("Choose what"));
    assert!(model.hotkey_from_form("F9", Some(999), None).is_err());
}
