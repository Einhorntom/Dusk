#[cfg(windows)]
mod windows_pipe;

use std::fmt;
use std::str::FromStr;

use dusk_app::{Api, ApplyReport, BackendError, EntryStatus, UseCaseError};
use dusk_domain::{AppSettings, ControlKey, ControlValue, MonitorId, Preset, PresetEntry};
use serde::{Deserialize, Serialize};

const MAX_MESSAGE_SIZE: usize = 1_048_576;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    /// The protocol and daemon version.
    Version,
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
        /// Also delete the hotkeys that apply this preset (SPEC-PRE-5).
        #[serde(default)]
        force: bool,
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

/// Settings on the wire. Fields added after v0 are optional in requests so
/// that an older client's `settings_set` keeps their current values.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct SettingsDto {
    pub debounce_ms: u32,
    pub confirm_input_change: bool,
    pub input_revert_seconds: u32,
    pub live_preview: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_osd: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_log: Option<bool>,
}

impl SettingsDto {
    /// Applies these settings over `current`, keeping fields the client left out.
    pub fn apply_to(self, current: AppSettings) -> AppSettings {
        AppSettings {
            debounce_ms: self.debounce_ms,
            confirm_input_change: self.confirm_input_change,
            input_revert_seconds: self.input_revert_seconds,
            live_preview: self.live_preview,
            brightness_step: self.brightness_step.unwrap_or(current.brightness_step),
            contrast_step: self.contrast_step.unwrap_or(current.contrast_step),
            volume_step: self.volume_step.unwrap_or(current.volume_step),
            show_osd: self.show_osd.unwrap_or(current.show_osd),
            diagnostic_log: self.diagnostic_log.unwrap_or(current.diagnostic_log),
        }
    }
}

impl From<AppSettings> for SettingsDto {
    fn from(value: AppSettings) -> Self {
        Self {
            debounce_ms: value.debounce_ms,
            confirm_input_change: value.confirm_input_change,
            input_revert_seconds: value.input_revert_seconds,
            live_preview: value.live_preview,
            brightness_step: Some(value.brightness_step),
            contrast_step: Some(value.contrast_step),
            volume_step: Some(value.volume_step),
            show_osd: Some(value.show_osd),
            diagnostic_log: Some(value.diagnostic_log),
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
    /// Canonical name of an enum value (SPEC-CTL-4), if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    value_name: Option<&'static str>,
    native_min: u32,
    native_max: u32,
    enum_values: Vec<u32>,
    /// Canonical names of `enum_values`, `null` where a value has none.
    enum_names: Vec<Option<&'static str>>,
}

#[derive(Serialize)]
struct SetDto {
    changed: bool,
}

/// The request/response format clients and the daemon agree on. Clients
/// send it as `"protocol"` in every request; a request without it is from
/// an older client and is accepted. Bump it for incompatible changes.
pub const PROTOCOL_VERSION: u64 = 1;

pub fn dispatch(service: &dyn Api, input: &[u8]) -> Vec<u8> {
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

fn dispatch_inner(service: &dyn Api, input: &[u8]) -> Result<serde_json::Value, DispatchError> {
    let request: serde_json::Value =
        serde_json::from_slice(input).map_err(|error| DispatchError::new(2, error.to_string()))?;
    if let Some(protocol) = request.get("protocol").and_then(serde_json::Value::as_u64)
        && protocol != PROTOCOL_VERSION
    {
        return Err(DispatchError::new(
            2,
            format!(
                "this Dusk speaks protocol {PROTOCOL_VERSION} but the client speaks protocol \
                 {protocol}; use a dusk, duskd and PowerToys plugin from the same release"
            ),
        ));
    }
    let request: Request = serde_json::from_value(request)
        .map_err(|error| DispatchError::new(2, error.to_string()))?;
    match request {
        Request::Version => Ok(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "daemon": env!("CARGO_PKG_VERSION"),
        })),
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
            let (value, value_name) = match reading.value {
                ControlValue::Normalized(value) => (value.to_string(), None),
                ControlValue::Enum(value) => (value.to_string(), control.value_name(value)),
            };
            let enum_names = reading
                .capability
                .enum_values
                .iter()
                .map(|value| control.value_name(*value))
                .collect();
            serde_json::to_value(ReadingDto {
                control: control.to_string(),
                value,
                value_name,
                native_min: reading.capability.native_min,
                native_max: reading.capability.native_max,
                enum_values: reading.capability.enum_values,
                enum_names,
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
            serde_json::to_value(SettingsDto::from(settings))
                .map_err(|error| DispatchError::new(1, error.to_string()))
        }
        Request::SettingsSet { settings } => {
            let current = service.settings().map_err(DispatchError::from_use_case)?;
            service
                .update_settings(settings.apply_to(current))
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
        Request::PresetDelete { name, force } => {
            let hotkeys = service
                .hotkeys_using_preset(&name)
                .map_err(DispatchError::from_use_case)?;
            if !hotkeys.is_empty() && !force {
                let keys: Vec<String> = hotkeys.iter().map(ToString::to_string).collect();
                return Err(DispatchError::new(
                    2,
                    format!(
                        "preset {name} is used by hotkeys {}; deleting it also deletes them (CLI: --force)",
                        keys.join(", ")
                    ),
                ));
            }
            service
                .delete_preset(&name)
                .map_err(DispatchError::from_use_case)?;
            Ok(serde_json::json!({ "deleted": true, "hotkeys_removed": hotkeys.len() }))
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
            UseCaseError::Backend(BackendError::NotFound(_))
            | UseCaseError::PresetNotFound(_)
            | UseCaseError::NoPresets
            | UseCaseError::PresetEntryNotFound { .. }
            | UseCaseError::HotkeyNotFound(_) => 3,
            UseCaseError::Backend(BackendError::NotResponding(_)) => 4,
            UseCaseError::InputChangeDeclined | UseCaseError::InputChangeReverted => 6,
            UseCaseError::UnsupportedControl(_)
            | UseCaseError::NothingToCapture(_)
            | UseCaseError::NoSelection(_)
            | UseCaseError::PresetExists(_)
            | UseCaseError::Domain(
                dusk_domain::DomainError::NormalizedValueOutOfRange(_)
                | dusk_domain::DomainError::EnumValueUnavailable { .. }
                | dusk_domain::DomainError::UnknownControl(_)
                | dusk_domain::DomainError::InvalidNumericCapability(_)
                | dusk_domain::DomainError::InvalidPresetName
                | dusk_domain::DomainError::InvalidHotkey(_),
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
    /// Another process already serves the daemon's pipe.
    PipeInUse,
    Io(std::io::Error),
    Protocol(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonUnavailable => f.write_str("duskd is not running"),
            Self::PipeInUse => f.write_str("another program is already serving the Dusk pipe"),
            Self::Io(error) => write!(f, "IPC I/O failed: {error}"),
            Self::Protocol(error) => write!(f, "IPC protocol error: {error}"),
        }
    }
}

impl std::error::Error for IpcError {}

#[cfg(windows)]
pub use windows_pipe::{PIPE_ENV, PipeServer, pipe_name, transact};

#[cfg(test)]
mod tests {
    use super::*;
    use dusk_app::HotkeyApi;
    use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
    use std::sync::Arc;

    fn harness() -> Harness {
        Harness::new(FakeBackend::single(FakeMonitor::new("fake-0").numeric(
            ControlKey::Brightness,
            50,
            100,
        )))
    }

    fn service() -> Arc<dusk_app::MonitorService> {
        harness().service
    }

    #[test]
    fn settings_dto_round_trips_values() {
        let value = SettingsDto::from(AppSettings::default());
        assert_eq!(
            value.clone().apply_to(AppSettings::default()),
            AppSettings::default()
        );
        assert_eq!(value.debounce_ms, 400);
        assert!(value.confirm_input_change);
    }

    #[test]
    fn invalid_request_is_reported_as_error_response() {
        let response: serde_json::Value =
            serde_json::from_slice(&dispatch(&*service(), b"{")).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["code"], 2);
        assert!(response["error"].as_str().unwrap().contains("EOF"));
    }

    #[test]
    fn dispatch_lists_monitors_in_the_wire_response() {
        let response: serde_json::Value =
            serde_json::from_slice(&dispatch(&*service(), br#"{"op":"list"}"#)).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"][0]["id"], "fake-0");
        assert!(response.get("code").is_none());
    }

    #[test]
    fn dispatch_reports_unsupported_controls_as_wire_errors() {
        let response: serde_json::Value = serde_json::from_slice(&dispatch(
            &*service(),
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
            &*service(),
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
            &*service,
            br#"{"op":"settings_set","settings":{"debounce_ms":800,"confirm_input_change":false,"input_revert_seconds":0,"live_preview":false}}"#,
        ))
        .unwrap();
        assert_eq!(saved["ok"], true);
        let read_back: serde_json::Value =
            serde_json::from_slice(&dispatch(&*service, br#"{"op":"settings_get"}"#)).unwrap();
        assert_eq!(read_back["result"]["debounce_ms"], 800);
        assert_eq!(read_back["result"]["confirm_input_change"], false);
        assert_eq!(read_back["result"]["input_revert_seconds"], 0);
    }

    fn call(service: &dusk_app::MonitorService, request: &str) -> serde_json::Value {
        serde_json::from_slice(&dispatch(service, request.as_bytes())).unwrap()
    }

    #[test]
    fn requests_from_another_protocol_are_refused_with_a_clear_message() {
        let h = harness();
        let current = call(&h.service, r#"{"op":"list","protocol":1}"#);
        assert_eq!(current["ok"], true, "{current}");
        let older_client = call(&h.service, r#"{"op":"list"}"#);
        assert_eq!(older_client["ok"], true, "{older_client}");

        let newer = call(&h.service, r#"{"op":"list","protocol":2}"#);
        assert_eq!(newer["code"], 2);
        assert!(
            newer["error"]
                .as_str()
                .unwrap()
                .contains("speaks protocol 1"),
            "{newer}"
        );

        let version = call(&h.service, r#"{"op":"version","protocol":1}"#);
        assert_eq!(version["result"]["protocol"], PROTOCOL_VERSION);
        assert_eq!(version["result"]["daemon"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn preset_errors_have_meaningful_exit_codes() {
        // No presets to cycle through: "not found" (3), not "invalid usage" (2).
        let empty = harness();
        let cycled = call(&empty.service, r#"{"op":"preset_cycle"}"#);
        assert_eq!(cycled["code"], 3, "{cycled}");
        assert_eq!(cycled["error"], "no presets are defined");

        // A monitor with nothing a preset can hold: like an unsupported control (2).
        let bare = Harness::new(FakeBackend::single(FakeMonitor::new("bare")));
        let saved = call(
            &bare.service,
            r#"{"op":"preset_save","name":"P","monitor":"bare"}"#,
        );
        assert_eq!(saved["code"], 2, "{saved}");
        assert_eq!(
            saved["error"],
            "monitor bare has no controls that a preset can save"
        );
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
    #[test]
    fn an_older_settings_request_keeps_the_hotkey_settings() {
        let h = harness();
        let mut settings = h.settings.stored();
        settings.brightness_step = 10;
        settings.show_osd = false;
        settings.diagnostic_log = true;
        h.settings.replace(settings);
        let saved = call(
            &h.service,
            r#"{"op":"settings_set","settings":{"debounce_ms":800,"confirm_input_change":true,"input_revert_seconds":10,"live_preview":false}}"#,
        );
        assert_eq!(saved["ok"], true);
        let stored = h.settings.stored();
        assert_eq!(stored.debounce_ms, 800);
        assert_eq!(stored.brightness_step, 10);
        assert!(!stored.show_osd);
        assert!(stored.diagnostic_log);
        let read = call(&h.service, r#"{"op":"settings_get"}"#);
        assert_eq!(read["result"]["brightness_step"], 10);
    }

    #[test]
    fn deleting_a_preset_used_by_a_hotkey_needs_force() {
        let h = harness();
        call(&h.service, r#"{"op":"preset_save","name":"Night"}"#);
        h.service
            .save_hotkey(dusk_domain::HotkeyBinding {
                keys: "Ctrl+Alt+N".parse().unwrap(),
                action: "preset:Night".parse().unwrap(),
                monitor: None,
            })
            .unwrap();
        let refused = call(&h.service, r#"{"op":"preset_delete","name":"Night"}"#);
        assert_eq!(refused["code"], 2);
        assert!(refused["error"].as_str().unwrap().contains("Ctrl+Alt+N"));
        assert_eq!(h.presets.stored().len(), 1);

        let deleted = call(
            &h.service,
            r#"{"op":"preset_delete","name":"Night","force":true}"#,
        );
        assert_eq!(deleted["result"]["hotkeys_removed"], 1);
        assert!(h.presets.stored().is_empty());
        assert!(h.hotkeys.stored().is_empty());
    }

    #[test]
    fn readings_carry_canonical_value_names() {
        let h = Harness::new(FakeBackend::single(FakeMonitor::reference("m")));
        let input = call(
            &h.service,
            r#"{"op":"get","monitor":"m","control":"input"}"#,
        );
        assert_eq!(input["result"]["value"], "49");
        assert_eq!(input["result"]["value_name"], "USB-C");
        assert_eq!(input["result"]["enum_values"][1], 0x11);
        assert_eq!(input["result"]["enum_names"][1], "HDMI 1");
        let brightness = call(
            &h.service,
            r#"{"op":"get","monitor":"m","control":"brightness"}"#,
        );
        assert!(brightness["result"].get("value_name").is_none());
    }

    #[test]
    fn dispatch_exports_and_imports_preset_documents() {
        let h = harness();
        call(&h.service, r#"{"op":"preset_save","name":"A"}"#);
        let exported = call(&h.service, r#"{"op":"preset_export"}"#);
        let text = exported["result"]["text"].as_str().unwrap().to_owned();
        call(&h.service, r#"{"op":"preset_save","name":"B"}"#);

        let request = serde_json::json!({ "op": "preset_import", "text": text, "replace": true });
        let imported = call(&h.service, &request.to_string());
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["result"]["added"], 1);
        assert_eq!(imported["result"]["discarded"], 2);
        assert_eq!(h.presets.stored().len(), 1);

        let invalid = call(
            &h.service,
            r#"{"op":"preset_import","text":"not a document"}"#,
        );
        assert_eq!(invalid["code"], 2);
        assert_eq!(h.presets.stored().len(), 1);
        let path = call(&h.service, r#"{"op":"preset_path"}"#);
        assert_eq!(path["result"]["path"], "memory://presets");
    }

    #[test]
    fn dispatch_reports_failed_preset_entries_in_the_result() {
        let h = harness();
        call(&h.service, r#"{"op":"preset_save","name":"A"}"#);
        call(
            &h.service,
            r#"{"op":"preset_set_entry","name":"A","monitor":"fake-0","control":"brightness","value":{"kind":"normalized","value":80}}"#,
        );
        let id = MonitorId::new("fake-0").unwrap();
        h.backend.fail_writes(
            &id,
            ControlKey::Brightness,
            BackendError::Failed("DDC/CI error".into()),
        );
        let applied = call(&h.service, r#"{"op":"preset_apply","name":"A"}"#);
        assert_eq!(applied["ok"], true);
        assert_eq!(applied["result"]["failed"], 1);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    //! Real named-pipe round trips on pipes private to each test; no monitor
    //! is touched and the daemon's own pipe is never used.

    use super::windows_pipe::{PipeServer, transact_on};
    use super::{IpcError, MAX_MESSAGE_SIZE, dispatch};
    use dusk_ddc_fake::{FakeBackend, FakeMonitor, Harness};
    use dusk_domain::ControlKey;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    fn unique_pipe_name() -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!(
            r"\\.\pipe\dusk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// Serves `handler` on a fresh pipe and waits until it accepts clients.
    fn start_server(handler: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static) -> String {
        let name = unique_pipe_name();
        let server = PipeServer::bind_on(&name).expect("a private pipe binds");
        thread::spawn(move || {
            let _ = server.serve_forever(handler);
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while transact_on(&name, b"").is_err() {
            assert!(Instant::now() < deadline, "test pipe server did not start");
            thread::sleep(Duration::from_millis(10));
        }
        name
    }

    #[test]
    fn a_pipe_name_that_is_already_served_cannot_be_bound_again() {
        let name = start_server(|request| request.to_vec());
        assert!(matches!(
            PipeServer::bind_on(&name),
            Err(IpcError::PipeInUse)
        ));
        // The first server keeps answering.
        assert_eq!(transact_on(&name, b"still here").unwrap(), b"still here");
    }

    #[test]
    fn protocol_limit_is_one_megabyte() {
        assert_eq!(MAX_MESSAGE_SIZE, 1_048_576);
    }

    #[test]
    fn a_request_round_trips_through_the_pipe() {
        let name = start_server(|request| request.iter().rev().copied().collect());
        assert_eq!(transact_on(&name, b"abc").unwrap(), b"cba");
    }

    #[test]
    fn a_maximum_size_message_round_trips() {
        let name = start_server(|request| request.to_vec());
        let payload = vec![7u8; MAX_MESSAGE_SIZE];
        assert_eq!(transact_on(&name, &payload).unwrap(), payload);
    }

    #[test]
    fn an_oversized_request_is_rejected_before_connecting() {
        let payload = vec![0u8; MAX_MESSAGE_SIZE + 1];
        assert!(matches!(
            transact_on(&unique_pipe_name(), &payload),
            Err(IpcError::Protocol(_))
        ));
    }

    #[test]
    fn a_missing_server_is_reported_as_daemon_unavailable() {
        assert!(matches!(
            transact_on(&unique_pipe_name(), b"{}"),
            Err(IpcError::DaemonUnavailable)
        ));
    }

    #[test]
    fn concurrent_clients_each_get_their_own_response() {
        let name = start_server(|request| request.to_vec());
        let clients: Vec<_> = (0..8)
            .map(|client| {
                let name = name.clone();
                thread::spawn(move || {
                    for request in 0..5 {
                        let payload = format!("client {client} request {request}");
                        let response = transact_on(&name, payload.as_bytes()).unwrap();
                        assert_eq!(response, payload.as_bytes());
                    }
                })
            })
            .collect();
        for client in clients {
            client.join().expect("client thread panicked");
        }
    }

    #[test]
    fn the_daemon_dispatcher_answers_over_the_pipe() {
        let service = Harness::new(FakeBackend::single(FakeMonitor::new("fake-0").numeric(
            ControlKey::Brightness,
            50,
            100,
        )))
        .service;
        let name = start_server(move |request| dispatch(&*service, request));
        let response: serde_json::Value =
            serde_json::from_slice(&transact_on(&name, br#"{"op":"list"}"#).unwrap()).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"][0]["id"], "fake-0");
    }
}
