#[cfg(windows)]
mod windows_pipe;

use std::fmt;
use std::str::FromStr;

use dispcontrol_app::{ApplyReport, BackendError, EntryStatus, MonitorService, UseCaseError};
use dispcontrol_domain::{AppSettings, ControlKey, ControlValue, MonitorId, Preset, PresetEntry};
use serde::{Deserialize, Serialize};

const MAX_MESSAGE_SIZE: usize = 1_048_576;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    List,
    Get {
        monitor: String,
        control: String,
    },
    Set {
        monitor: String,
        control: String,
        value: RequestValue,
    },
    SettingsGet,
    SettingsSet {
        settings: SettingsDto,
    },
    PresetList,
    PresetApply {
        name: String,
    },
    PresetSave {
        name: String,
        monitor: Option<String>,
        #[serde(default)]
        include_input: bool,
    },
    PresetDelete {
        name: String,
    },
    PresetRename {
        name: String,
        new_name: String,
    },
    PresetMove {
        name: String,
        offset: i32,
    },
    PresetCycle {
        #[serde(default = "default_forward")]
        forward: bool,
    },
    PresetSetEntry {
        name: String,
        monitor: String,
        control: String,
        value: RequestValue,
    },
    PresetRemoveEntry {
        name: String,
        monitor: String,
        control: String,
    },
    PresetExport,
    PresetImport {
        text: String,
        #[serde(default)]
        replace: bool,
    },
    PresetPath,
}

fn default_forward() -> bool {
    true
}

fn report_to_json(report: &ApplyReport) -> serde_json::Value {
    let entries: Vec<_> = report
        .outcomes
        .iter()
        .map(|outcome| {
            let (status, detail) = match &outcome.status {
                EntryStatus::Applied => ("applied", None),
                EntryStatus::Unchanged => ("unchanged", None),
                EntryStatus::Skipped(reason) => ("skipped", Some(reason.as_str())),
                EntryStatus::Failed(reason) => ("failed", Some(reason.as_str())),
            };
            serde_json::json!({
                "monitor": outcome.entry.monitor.as_str(),
                "control": outcome.entry.control.as_str(),
                "status": status,
                "detail": detail,
            })
        })
        .collect();
    serde_json::json!({
        "preset": report.preset,
        "applied": report.applied(),
        "unchanged": report.unchanged(),
        "skipped": report.skipped(),
        "failed": report.failed(),
        "entries": entries,
    })
}

fn preset_to_json(preset: &Preset) -> serde_json::Value {
    let entries: Vec<_> = preset
        .entries
        .iter()
        .map(|entry| {
            let (kind, value) = match entry.value {
                ControlValue::Normalized(value) => ("normalized", value),
                ControlValue::Enum(value) => ("enum", value),
            };
            serde_json::json!({
                "monitor": entry.monitor.as_str(),
                "control": entry.control.as_str(),
                "kind": kind,
                "value": value,
            })
        })
        .collect();
    serde_json::json!({ "name": preset.name, "entries": entries })
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum RequestValue {
    Normalized(u32),
    Enum(u32),
}

#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct SettingsDto {
    pub debounce_ms: u32,
    pub confirm_input_change: bool,
    pub input_revert_seconds: u32,
    pub live_preview: bool,
}

impl From<AppSettings> for SettingsDto {
    fn from(value: AppSettings) -> Self {
        Self {
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
        }
    }
}

impl From<SettingsDto> for AppSettings {
    fn from(value: SettingsDto) -> Self {
        Self {
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
        }
    }
}

#[derive(Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct MonitorDto {
    id: String,
    name: String,
    unstable_id: bool,
}

#[derive(Serialize)]
struct ReadingDto {
    control: String,
    value: String,
    native_min: u32,
    native_max: u32,
    enum_values: Vec<u32>,
}

#[derive(Serialize)]
struct SetDto {
    changed: bool,
}

pub fn dispatch(service: &MonitorService, input: &[u8]) -> Vec<u8> {
    let result = dispatch_inner(service, input);
    let response = match result {
        Ok(value) => Response {
            ok: true,
            result: Some(value),
            code: None,
            error: None,
        },
        Err(error) => Response {
            ok: false,
            result: None,
            code: Some(error.code),
            error: Some(error.message),
        },
    };
    serde_json::to_vec(&response).unwrap_or_else(|error| {
        format!(
            "{{\"ok\":false,\"code\":1,\"error\":\"could not encode response: {}\"}}",
            error
        )
        .into_bytes()
    })
}

struct DispatchError {
    code: i32,
    message: String,
}

fn dispatch_inner(
    service: &MonitorService,
    input: &[u8],
) -> Result<serde_json::Value, DispatchError> {
    let request: Request =
        serde_json::from_slice(input).map_err(|error| DispatchError::new(2, error.to_string()))?;
    match request {
        Request::List => {
            let monitors = service
                .list_monitors()
                .map_err(DispatchError::from_use_case)?
                .into_iter()
                .map(|monitor| MonitorDto {
                    id: monitor.id.to_string(),
                    name: monitor.name,
                    unstable_id: monitor.unstable_id,
                })
                .collect::<Vec<_>>();
            serde_json::to_value(monitors).map_err(|error| DispatchError::new(1, error.to_string()))
        }
        Request::Get { monitor, control } => {
            let monitor = MonitorId::new(monitor)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let control = ControlKey::from_str(&control)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let reading = service
                .read(&monitor, control)
                .map_err(DispatchError::from_use_case)?;
            let value = match reading.value {
                ControlValue::Normalized(value) => value.to_string(),
                ControlValue::Enum(value) => value.to_string(),
            };
            serde_json::to_value(ReadingDto {
                control: control.to_string(),
                value,
                native_min: reading.capability.native_min,
                native_max: reading.capability.native_max,
                enum_values: reading.capability.enum_values,
            })
            .map_err(|error| DispatchError::new(1, error.to_string()))
        }
        Request::Set {
            monitor,
            control,
            value,
        } => {
            let monitor = MonitorId::new(monitor)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let control = ControlKey::from_str(&control)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let value = match value {
                RequestValue::Normalized(value) => ControlValue::Normalized(value),
                RequestValue::Enum(value) => ControlValue::Enum(value),
            };
            let changed = service
                .set(&monitor, control, value)
                .map_err(DispatchError::from_use_case)?;
            serde_json::to_value(SetDto { changed })
                .map_err(|error| DispatchError::new(1, error.to_string()))
        }
        Request::SettingsGet => {
            let settings = service.settings().map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({
                "debounce_ms": settings.debounce_ms,
                "confirm_input_change": settings.confirm_input_change,
                "input_revert_seconds": settings.input_revert_seconds,
            }))
        }
        Request::SettingsSet { settings } => {
            service
                .update_settings(settings.into())
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "saved": true }))
        }
        Request::PresetList => {
            let presets = service
                .list_presets()
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::Value::Array(
                presets.iter().map(preset_to_json).collect(),
            ))
        }
        Request::PresetApply { name } => {
            let report = service
                .apply_preset(&name)
                .map_err(DispatchError::from_use_case)?;
            Ok(report_to_json(&report))
        }
        Request::PresetSave {
            name,
            monitor,
            include_input,
        } => {
            let monitor = match monitor {
                Some(monitor) => MonitorId::new(monitor)
                    .map_err(|error| DispatchError::new(2, error.to_string()))?,
                None => service
                    .list_monitors()
                    .map_err(DispatchError::from_use_case)?
                    .into_iter()
                    .next()
                    .map(|monitor| monitor.id)
                    .ok_or_else(|| DispatchError::new(3, "no monitors found".into()))?,
            };
            let preset = service
                .capture_preset(&name, &monitor, include_input)
                .map_err(DispatchError::from_use_case)?;
            Ok(preset_to_json(&preset))
        }
        Request::PresetDelete { name } => {
            service
                .delete_preset(&name)
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "deleted": true }))
        }
        Request::PresetRename { name, new_name } => {
            service
                .rename_preset(&name, &new_name)
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "renamed": true }))
        }
        Request::PresetMove { name, offset } => {
            service
                .move_preset(&name, offset)
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "moved": true }))
        }
        Request::PresetSetEntry {
            name,
            monitor,
            control,
            value,
        } => {
            let monitor = MonitorId::new(monitor)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let control = ControlKey::from_str(&control)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let value = match value {
                RequestValue::Normalized(value) => ControlValue::Normalized(value),
                RequestValue::Enum(value) => ControlValue::Enum(value),
            };
            service
                .set_preset_entry(
                    &name,
                    PresetEntry {
                        monitor,
                        control,
                        value,
                    },
                )
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "saved": true }))
        }
        Request::PresetRemoveEntry {
            name,
            monitor,
            control,
        } => {
            let monitor = MonitorId::new(monitor)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            let control = ControlKey::from_str(&control)
                .map_err(|error| DispatchError::new(2, error.to_string()))?;
            service
                .remove_preset_entry(&name, &monitor, control)
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "removed": true }))
        }
        Request::PresetExport => {
            let text = service
                .export_presets()
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "text": text }))
        }
        Request::PresetImport { text, replace } => {
            let summary = service
                .import_presets(&text, replace)
                .map_err(|error| match error {
                    UseCaseError::Backend(BackendError::Failed(message)) => {
                        DispatchError::new(2, message)
                    }
                    other => DispatchError::from_use_case(other),
                })?;
            Ok(serde_json::json!({
                "added": summary.added,
                "updated": summary.updated,
                "discarded": summary.removed,
            }))
        }
        Request::PresetPath => Ok(serde_json::json!({ "path": service.presets_location() })),
        Request::PresetCycle { forward } => {
            let report = service
                .cycle_preset(forward)
                .map_err(DispatchError::from_use_case)?;
            Ok(report_to_json(&report))
        }
    }
}

impl DispatchError {
    fn new(code: i32, message: String) -> Self {
        Self { code, message }
    }

    fn from_use_case(error: UseCaseError) -> Self {
        let code = match &error {
            UseCaseError::Backend(BackendError::NotFound(_)) | UseCaseError::PresetNotFound(_) => 3,
            UseCaseError::Backend(BackendError::NotResponding(_)) => 4,
            UseCaseError::InputChangeDeclined | UseCaseError::InputChangeReverted => 6,
            UseCaseError::UnsupportedControl(_)
            | UseCaseError::PresetExists(_)
            | UseCaseError::Domain(
                dispcontrol_domain::DomainError::NormalizedValueOutOfRange(_)
                | dispcontrol_domain::DomainError::EnumValueUnavailable { .. }
                | dispcontrol_domain::DomainError::UnknownControl(_)
                | dispcontrol_domain::DomainError::InvalidNumericCapability(_)
                | dispcontrol_domain::DomainError::InvalidPresetName,
            )
            | UseCaseError::SettingsInvalid(_) => 2,
            _ => 1,
        };
        Self::new(code, error.to_string())
    }
}

#[derive(Debug)]
pub enum IpcError {
    DaemonUnavailable,
    Io(std::io::Error),
    Protocol(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonUnavailable => f.write_str("dispcontrold is not running"),
            Self::Io(error) => write!(f, "IPC I/O failed: {error}"),
            Self::Protocol(error) => write!(f, "IPC protocol error: {error}"),
        }
    }
}

impl std::error::Error for IpcError {}

#[cfg(windows)]
pub use windows_pipe::{serve_forever, transact};

#[cfg(test)]
mod tests {
    use super::*;
    use dispcontrol_app::{
        BackendError, Clock, InputChangePrompter, MonitorBackend, PresetRepository,
        SettingsRepository,
    };
    use dispcontrol_domain::{ControlCapability, Monitor, MonitorId};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct FakeBackend {
        monitor: Monitor,
    }

    impl MonitorBackend for FakeBackend {
        fn list_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
            Ok(vec![self.monitor.clone()])
        }

        fn read_control(
            &self,
            monitor: &MonitorId,
            control: ControlKey,
        ) -> Result<Option<(ControlCapability, u32)>, BackendError> {
            if monitor != &self.monitor.id {
                return Err(BackendError::NotFound(format!(
                    "monitor not found: {monitor}"
                )));
            }
            if control != ControlKey::Brightness {
                return Ok(None);
            }
            Ok(Some((
                ControlCapability {
                    key: control,
                    native_min: 0,
                    native_max: 100,
                    enum_values: Vec::new(),
                },
                50,
            )))
        }

        fn write_control(
            &self,
            _monitor: &MonitorId,
            _control: ControlKey,
            _native_value: u32,
        ) -> Result<(), BackendError> {
            Ok(())
        }
    }

    struct MemorySettings(Mutex<AppSettings>);

    impl SettingsRepository for MemorySettings {
        fn load(&self) -> Result<AppSettings, BackendError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
            *self.0.lock().unwrap() = settings.clone();
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryPresets(Mutex<Vec<Preset>>);

    impl PresetRepository for MemoryPresets {
        fn load_presets(&self) -> Result<Vec<Preset>, BackendError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save_presets(&self, presets: &[Preset]) -> Result<(), BackendError> {
            *self.0.lock().unwrap() = presets.to_vec();
            Ok(())
        }
    }

    struct NoPrompts;

    impl InputChangePrompter for NoPrompts {
        fn confirm_input_change(&self, _monitor: &Monitor, _from: u32, _to: u32) -> bool {
            false
        }

        fn confirm_keep_input(
            &self,
            _monitor: &Monitor,
            _previous_input: u32,
            _new_input: u32,
            _timeout_seconds: u32,
        ) -> Result<bool, String> {
            Ok(false)
        }
    }

    struct TestClock;

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            Instant::now()
        }

        fn sleep(&self, _duration: Duration) {}
    }

    fn service() -> MonitorService {
        let backend = Arc::new(FakeBackend {
            monitor: Monitor {
                id: MonitorId::new("fake-0").unwrap(),
                name: "Fake display".into(),
                unstable_id: false,
            },
        });
        MonitorService::new(
            backend,
            Arc::new(MemorySettings(Mutex::new(AppSettings::default()))),
            Arc::new(MemoryPresets::default()),
            Arc::new(NoPrompts),
            Arc::new(TestClock),
        )
    }

    #[test]
    fn settings_dto_round_trips_values() {
        let value = SettingsDto::from(AppSettings::default());
        assert_eq!(AppSettings::from(value.clone()), AppSettings::default());
        assert_eq!(value.debounce_ms, 400);
        assert!(value.confirm_input_change);
    }

    #[test]
    fn invalid_request_is_reported_as_error_response() {
        let response: serde_json::Value =
            serde_json::from_slice(&dispatch(&service(), b"{")).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["code"], 2);
        assert!(response["error"].as_str().unwrap().contains("EOF"));
    }

    #[test]
    fn dispatch_lists_monitors_in_the_wire_response() {
        let response: serde_json::Value =
            serde_json::from_slice(&dispatch(&service(), br#"{"op":"list"}"#)).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"][0]["id"], "fake-0");
        assert!(response.get("code").is_none());
    }

    #[test]
    fn dispatch_reports_unsupported_controls_as_wire_errors() {
        let response: serde_json::Value = serde_json::from_slice(&dispatch(
            &service(),
            br#"{"op":"get","monitor":"fake-0","control":"volume"}"#,
        ))
        .unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["code"], 2);
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("does not support volume")
        );
    }

    #[test]
    fn dispatch_maps_missing_monitor_to_documented_exit_code() {
        let response: serde_json::Value = serde_json::from_slice(&dispatch(
            &service(),
            br#"{"op":"get","monitor":"missing","control":"brightness"}"#,
        ))
        .unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["code"], 3);
    }

    #[test]
    fn dispatch_saves_and_reads_back_settings() {
        let service = service();
        let saved: serde_json::Value = serde_json::from_slice(&dispatch(
            &service,
            br#"{"op":"settings_set","settings":{"debounce_ms":800,"confirm_input_change":false,"input_revert_seconds":0,"live_preview":false}}"#,
        ))
        .unwrap();
        assert_eq!(saved["ok"], true);
        let read_back: serde_json::Value =
            serde_json::from_slice(&dispatch(&service, br#"{"op":"settings_get"}"#)).unwrap();
        assert_eq!(read_back["result"]["debounce_ms"], 800);
        assert_eq!(read_back["result"]["confirm_input_change"], false);
        assert_eq!(read_back["result"]["input_revert_seconds"], 0);
    }

    fn call(service: &MonitorService, request: &str) -> serde_json::Value {
        serde_json::from_slice(&dispatch(service, request.as_bytes())).unwrap()
    }

    #[test]
    fn dispatch_saves_lists_applies_and_deletes_presets() {
        let service = service();
        let saved = call(&service, r#"{"op":"preset_save","name":"Day"}"#);
        assert_eq!(saved["ok"], true);
        assert_eq!(saved["result"]["entries"][0]["control"], "brightness");

        let listed = call(&service, r#"{"op":"preset_list"}"#);
        assert_eq!(listed["result"][0]["name"], "Day");

        let applied = call(&service, r#"{"op":"preset_apply","name":"day"}"#);
        assert_eq!(applied["ok"], true);
        assert_eq!(applied["result"]["unchanged"], 1);
        assert_eq!(applied["result"]["failed"], 0);

        assert_eq!(
            call(&service, r#"{"op":"preset_delete","name":"Day"}"#)["ok"],
            true
        );
        let missing = call(&service, r#"{"op":"preset_apply","name":"Day"}"#);
        assert_eq!(missing["ok"], false);
        assert_eq!(missing["code"], 3);
    }

    #[test]
    fn dispatch_rejects_invalid_and_duplicate_preset_names() {
        let service = service();
        assert_eq!(
            call(&service, r#"{"op":"preset_save","name":"  "}"#)["code"],
            2
        );
        call(&service, r#"{"op":"preset_save","name":"A"}"#);
        call(&service, r#"{"op":"preset_save","name":"B"}"#);
        let duplicate = call(
            &service,
            r#"{"op":"preset_rename","name":"A","new_name":"b"}"#,
        );
        assert_eq!(duplicate["code"], 2);
    }

    #[test]
    fn dispatch_edits_and_removes_preset_entries() {
        let service = service();
        call(&service, r#"{"op":"preset_save","name":"A"}"#);
        let set = call(
            &service,
            r#"{"op":"preset_set_entry","name":"a","monitor":"m","control":"brightness","value":{"kind":"normalized","value":42}}"#,
        );
        assert_eq!(set["ok"], true);
        let listed = call(&service, r#"{"op":"preset_list"}"#);
        let entries = listed["result"][0]["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e["monitor"] == "m" && e["control"] == "brightness" && e["value"] == 42)
        );
        let invalid = call(
            &service,
            r#"{"op":"preset_set_entry","name":"A","monitor":"m","control":"brightness","value":{"kind":"normalized","value":101}}"#,
        );
        assert_eq!(invalid["code"], 2);
        let removed = call(
            &service,
            r#"{"op":"preset_remove_entry","name":"A","monitor":"m","control":"brightness"}"#,
        );
        assert_eq!(removed["ok"], true);
        let again = call(
            &service,
            r#"{"op":"preset_remove_entry","name":"A","monitor":"m","control":"brightness"}"#,
        );
        assert_eq!(again["code"], 3);
    }
}

#[cfg(windows)]
mod windows_tests {
    #[test]
    fn protocol_limit_is_one_megabyte() {
        assert_eq!(super::MAX_MESSAGE_SIZE, 1_048_576);
    }
}
