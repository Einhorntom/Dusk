#[cfg(windows)]
mod windows_pipe;

use std::fmt;
use std::str::FromStr;

use dispcontrol_app::{MonitorService, UseCaseError};
use dispcontrol_domain::{AppSettings, ControlKey, ControlValue, MonitorId};
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
            error: None,
        },
        Err(message) => Response {
            ok: false,
            result: None,
            error: Some(message),
        },
    };
    serde_json::to_vec(&response).unwrap_or_else(|error| {
        format!(
            "{{\"ok\":false,\"error\":\"could not encode response: {}\"}}",
            error
        )
        .into_bytes()
    })
}

fn dispatch_inner(service: &MonitorService, input: &[u8]) -> Result<serde_json::Value, String> {
    let request: Request = serde_json::from_slice(input).map_err(|error| error.to_string())?;
    match request {
        Request::List => {
            let monitors = service
                .list_monitors()
                .map_err(format_use_case_error)?
                .into_iter()
                .map(|monitor| MonitorDto {
                    id: monitor.id.to_string(),
                    name: monitor.name,
                    unstable_id: monitor.unstable_id,
                })
                .collect::<Vec<_>>();
            serde_json::to_value(monitors).map_err(|error| error.to_string())
        }
        Request::Get { monitor, control } => {
            let monitor = MonitorId::new(monitor).map_err(|error| error.to_string())?;
            let control = ControlKey::from_str(&control).map_err(|error| error.to_string())?;
            let reading = service
                .read(&monitor, control)
                .map_err(format_use_case_error)?;
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
            .map_err(|error| error.to_string())
        }
        Request::Set {
            monitor,
            control,
            value,
        } => {
            let monitor = MonitorId::new(monitor).map_err(|error| error.to_string())?;
            let control = ControlKey::from_str(&control).map_err(|error| error.to_string())?;
            let value = match value {
                RequestValue::Normalized(value) => ControlValue::Normalized(value),
                RequestValue::Enum(value) => ControlValue::Enum(value),
            };
            let changed = service
                .set(&monitor, control, value)
                .map_err(format_use_case_error)?;
            serde_json::to_value(SetDto { changed }).map_err(|error| error.to_string())
        }
        Request::SettingsGet => {
            let settings = service.settings().map_err(format_use_case_error)?;
            Ok(serde_json::json!({
                "debounce_ms": settings.debounce_ms,
                "confirm_input_change": settings.confirm_input_change,
                "input_revert_seconds": settings.input_revert_seconds,
            }))
        }
        Request::SettingsSet { settings } => {
            service
                .update_settings(settings.into())
                .map_err(format_use_case_error)?;
            Ok(serde_json::json!({ "saved": true }))
        }
    }
}

fn format_use_case_error(error: UseCaseError) -> String {
    error.to_string()
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
            serde_json::from_slice(&dispatch_unreachable_for_this_test()).unwrap();
        assert_eq!(response["ok"], false);
    }

    fn dispatch_unreachable_for_this_test() -> Vec<u8> {
        serde_json::to_vec(&Response {
            ok: false,
            result: None,
            error: Some("invalid request".into()),
        })
        .unwrap()
    }
}

#[cfg(windows)]
mod windows_tests {
    #[test]
    fn protocol_limit_is_one_megabyte() {
        assert_eq!(super::MAX_MESSAGE_SIZE, 1_048_576);
    }
}
