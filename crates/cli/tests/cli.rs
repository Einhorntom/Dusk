//! The CLI end to end through the daemon's dispatcher, without the named pipe:
//! argument parsing, JSON/text output, exit codes and preset files.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use dispcontrol_app::BackendError;
use dispcontrol_cli::{CliError, run_with};
use dispcontrol_ddc_fake::{FakeBackend, FakeMonitor, Harness};
use dispcontrol_domain::{ControlKey, MonitorId};
use dispcontrol_ipc::{IpcError, dispatch};
use serde_json::Value;

fn harness() -> Harness {
    Harness::new(FakeBackend::single(FakeMonitor::reference("mon")))
}

fn id() -> MonitorId {
    MonitorId::new("mon").unwrap()
}

fn cli(h: &Harness, args: &[&str]) -> Result<String, CliError> {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let service = h.service.clone();
    run_with(&args, &move |request| Ok(dispatch(&*service, request)))
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// A temp file path that is removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "dispcontrol-cli-test-{}-{}.toml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )))
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn get_and_set_report_json_results() {
    let h = harness();
    let got = json(&cli(&h, &["--json", "get", "mon", "brightness"]).unwrap());
    assert_eq!(got["ok"], true);
    assert_eq!(got["error"], Value::Null);

    cli(&h, &["set", "mon", "brightness", "80", "--json"]).unwrap();
    assert_eq!(h.backend.value(&id(), ControlKey::Brightness), Some(80));
}

#[test]
fn errors_map_to_documented_exit_codes() {
    let h = harness();
    let usage = cli(&h, &["set", "mon", "brightness", "101"]).unwrap_err();
    assert_eq!(usage.exit_code, 2);
    let unknown = cli(&h, &["get", "mon", "sharpness"]).unwrap_err();
    assert_eq!(unknown.exit_code, 2);
    let missing = cli(&h, &["--json", "get", "nope", "brightness"]).unwrap_err();
    assert_eq!(missing.exit_code, 3);
    assert_eq!(json(&missing.message)["ok"], false);
    assert!(h.backend.writes().is_empty());
}

#[test]
fn a_stopped_daemon_is_exit_code_seven() {
    let args = vec!["--json".to_string(), "list".to_string()];
    let error = run_with(&args, &|_| Err(IpcError::DaemonUnavailable)).unwrap_err();
    assert_eq!(error.exit_code, 7);
    assert_eq!(json(&error.message)["ok"], false);
}

#[test]
fn a_partially_failed_preset_is_exit_code_five() {
    let h = harness();
    cli(&h, &["preset", "save", "Dim"]).unwrap();
    cli(&h, &["preset", "set", "Dim", "mon", "brightness", "10"]).unwrap();
    cli(&h, &["preset", "set", "Dim", "mon", "contrast", "10"]).unwrap();
    h.backend.fail_writes(
        &id(),
        ControlKey::Brightness,
        BackendError::Failed("DDC/CI error".into()),
    );

    let text = cli(&h, &["preset", "apply", "Dim"]).unwrap_err();
    assert_eq!(text.exit_code, 5);
    assert!(text.message.contains("some preset entries failed"));

    h.backend.set_value(&id(), ControlKey::Contrast, 50);
    let error = cli(&h, &["--json", "preset", "apply", "Dim"]).unwrap_err();
    assert_eq!(error.exit_code, 5);
    let output = json(&error.message);
    assert_eq!(output["ok"], false);
    assert_eq!(output["result"]["failed"], 1);
    assert_eq!(output["result"]["applied"], 1);
}

#[test]
fn presets_export_to_a_file_and_import_back() {
    let h = harness();
    cli(&h, &["preset", "save", "Day"]).unwrap();
    let file = TempFile::new();
    let message = cli(&h, &["preset", "export", file.path()]).unwrap();
    assert!(message.contains(file.path()));
    let stdout = cli(&h, &["preset", "export"]).unwrap();
    assert!(stdout.starts_with("fake-presets:"));

    cli(&h, &["preset", "save", "Night"]).unwrap();
    let merged = cli(&h, &["preset", "import", file.path()]).unwrap();
    assert!(merged.contains("0 added, 1 updated, 0 discarded"));
    assert_eq!(h.presets.stored().len(), 2);

    let replaced = cli(&h, &["preset", "import", "--replace", file.path()]).unwrap();
    assert!(replaced.contains("1 added, 0 updated, 2 discarded"));
    assert_eq!(h.presets.stored().len(), 1);
}

#[test]
fn bad_import_files_are_usage_errors_and_change_nothing() {
    let h = harness();
    cli(&h, &["preset", "save", "Day"]).unwrap();
    let file = TempFile::new();
    let missing = cli(&h, &["preset", "import", file.path()]).unwrap_err();
    assert_eq!(missing.exit_code, 2);

    std::fs::write(&file.0, "not a preset document").unwrap();
    let invalid = cli(&h, &["--json", "preset", "import", file.path()]).unwrap_err();
    assert_eq!(invalid.exit_code, 2);
    assert_eq!(json(&invalid.message)["ok"], false);
    assert_eq!(h.presets.stored().len(), 1);
}

#[test]
fn preset_path_reports_the_store_location() {
    let h = harness();
    let output = json(&cli(&h, &["--json", "preset", "path"]).unwrap());
    assert_eq!(output["result"]["path"], "memory://presets");
}
