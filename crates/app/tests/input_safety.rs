//! SPEC-IN: input changes are confirmed and can be reverted.

use dusk_app::{EntryStatus, UseCaseError};
use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness, ScriptedPrompter};
use dusk_domain::{ControlKey, ControlValue, MonitorId, Preset, PresetEntry};

const HDMI: u32 = 0x11;
const USB_C: u32 = 0x31;

fn input_monitor(prompter: ScriptedPrompter) -> (Harness, MonitorId) {
    let monitor = FakeMonitor::new("test-monitor")
        .enumerated(ControlKey::Input, HDMI, &[HDMI, USB_C])
        .numeric(ControlKey::Brightness, 50, 100);
    let id = monitor.id().clone();
    (
        Harness::with_prompter(FakeBackend::single(monitor), prompter),
        id,
    )
}

fn switch_to_usb_c(h: &Harness, id: &MonitorId) -> Result<bool, UseCaseError> {
    h.service
        .set(id, ControlKey::Input, ControlValue::Enum(USB_C))
}

#[test]
fn input_change_requires_confirmation_before_writing() {
    let (h, id) = input_monitor(ScriptedPrompter::declining());
    assert!(matches!(
        switch_to_usb_c(&h, &id),
        Err(UseCaseError::InputChangeDeclined)
    ));
    assert_eq!(h.prompter.confirmations_asked(), 1);
    assert!(h.backend.writes().is_empty());
}

#[test]
fn accepted_input_change_is_written_once() {
    let (h, id) = input_monitor(ScriptedPrompter::accepting());
    assert!(switch_to_usb_c(&h, &id).unwrap());
    assert_eq!(h.backend.written_values(), vec![USB_C]);
}

#[test]
fn confirmation_can_be_turned_off_in_settings() {
    let (h, id) = input_monitor(ScriptedPrompter::declining());
    let mut settings = h.settings.stored();
    settings.confirm_input_change = false;
    settings.input_revert_seconds = 0;
    h.settings.replace(settings);
    assert!(switch_to_usb_c(&h, &id).unwrap());
    assert_eq!(h.prompter.confirmations_asked(), 0);
}

#[test]
fn rejected_keep_prompt_restores_the_previous_input() {
    let (h, id) = input_monitor(ScriptedPrompter::new(true, Ok(false)));
    assert!(matches!(
        switch_to_usb_c(&h, &id),
        Err(UseCaseError::InputChangeReverted)
    ));
    assert_eq!(h.backend.written_values(), vec![USB_C, HDMI]);
}

#[test]
fn keep_prompt_failure_attempts_to_restore_the_previous_input() {
    let (h, id) = input_monitor(ScriptedPrompter::new(
        true,
        Err("prompt unavailable".into()),
    ));
    assert!(matches!(
        switch_to_usb_c(&h, &id),
        Err(UseCaseError::InputKeepPromptFailed(_))
    ));
    assert_eq!(h.backend.written_values(), vec![USB_C, HDMI]);
}

#[test]
fn preset_input_change_is_confirmed_and_a_decline_does_not_stop_other_entries() {
    let (h, id) = input_monitor(ScriptedPrompter::declining());
    h.service
        .save_preset(Preset {
            name: "Laptop".into(),
            entries: vec![
                PresetEntry {
                    monitor: id.clone(),
                    control: ControlKey::Brightness,
                    value: ControlValue::Normalized(10),
                },
                PresetEntry {
                    monitor: id.clone(),
                    control: ControlKey::Input,
                    value: ControlValue::Enum(USB_C),
                },
            ],
        })
        .unwrap();

    let report = h.service.apply_preset("Laptop").unwrap();
    assert_eq!(h.prompter.confirmations_asked(), 1);
    assert_eq!(report.failed(), 1);
    assert_eq!(report.applied(), 1);
    let input = report
        .outcomes
        .iter()
        .find(|outcome| outcome.entry.control == ControlKey::Input)
        .unwrap();
    assert!(
        matches!(&input.status, EntryStatus::Failed(reason) if reason.contains("not confirmed"))
    );
    assert_eq!(h.backend.written_controls(), vec![ControlKey::Brightness]);
    assert_eq!(h.backend.value(&id, ControlKey::Input), Some(HDMI));
}
