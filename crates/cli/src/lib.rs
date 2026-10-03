#![forbid(unsafe_code)]

use std::str::FromStr;

use dispcontrol_domain::ControlKey;
use dispcontrol_ipc::{IpcError, transact};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub exit_code: i32,
    pub json_output: bool,
}

pub fn run(args: &[String]) -> Result<String, CliError> {
    let json_output = args.iter().any(|argument| argument == "--json");
    let command_args = args
        .iter()
        .filter(|argument| argument.as_str() != "--json")
        .cloned()
        .collect::<Vec<_>>();
    run_command(&command_args, json_output)
}

fn run_command(args: &[String], json_output: bool) -> Result<String, CliError> {
    let request = match args {
        [command] if command == "list" => json!({ "op": "list" }),
        [command, monitor, control] if command == "get" => {
            parse_control(control, json_output)?;
            json!({ "op": "get", "monitor": monitor, "control": control })
        }
        [command, monitor, control, value] if command == "set" => {
            let key = parse_control(control, json_output)?;
            let number: u32 = value
                .parse()
                .map_err(|_| usage_error("value must be an unsigned integer", json_output))?;
            let value = if key.is_numeric() {
                if number > 100 {
                    return Err(usage_error(
                        "normalized value must be between 0 and 100",
                        json_output,
                    ));
                }
                json!({ "kind": "normalized", "value": number })
            } else {
                json!({ "kind": "enum", "value": number })
            };
            json!({
                "op": "set",
                "monitor": monitor,
                "control": control,
                "value": value,
            })
        }
        [command, verb] if command == "settings" && verb == "show" => {
            json!({ "op": "settings_get" })
        }
        _ => return Err(usage_error("invalid command or arguments", json_output)),
    };

    let bytes = serde_json::to_vec(&request)
        .map_err(|error| cli_error(&error.to_string(), 1, json_output))?;
    let response = transact(&bytes).map_err(|error| {
        let exit_code = match error {
            IpcError::DaemonUnavailable => 7,
            _ => 1,
        };
        cli_error(&error.to_string(), exit_code, json_output)
    })?;
    let parsed: Value = serde_json::from_slice(&response).map_err(|error| CliError {
        message: format!("invalid daemon response: {error}"),
        exit_code: 1,
        json_output,
    })?;
    if parsed["ok"] != true {
        return Err(cli_error(
            parsed["error"]
                .as_str()
                .unwrap_or("daemon returned an unspecified error"),
            parsed["code"].as_i64().unwrap_or(1) as i32,
            json_output,
        ));
    }
    if json_output {
        let output = json!({
            "ok": true,
            "result": parsed["result"],
            "error": null,
        });
        return serde_json::to_string(&output)
            .map_err(|error| cli_error(&error.to_string(), 1, true));
    }
    serde_json::to_string_pretty(&parsed["result"]).map_err(|error| CliError {
        message: error.to_string(),
        exit_code: 1,
        json_output: false,
    })
}

fn parse_control(value: &str, json_output: bool) -> Result<ControlKey, CliError> {
    ControlKey::from_str(value).map_err(|error| usage_error(&error.to_string(), json_output))
}

fn usage_error(message: &str, json_output: bool) -> CliError {
    cli_error(
        &format!(
            "{message}\nUsage: dispcontrol [--json] list | get <monitor-id> <control> | \
             set <monitor-id> <control> <value> | settings show"
        ),
        2,
        json_output,
    )
}

fn cli_error(message: &str, exit_code: i32, json_output: bool) -> CliError {
    let message = if json_output {
        serde_json::to_string(&json!({
            "ok": false,
            "result": null,
            "error": message,
        }))
        .unwrap_or_else(|_| r#"{"ok":false,"result":null,"error":"could not encode error"}"#.into())
    } else {
        message.to_owned()
    };
    CliError {
        message,
        exit_code,
        json_output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_command_with_json_returns_a_json_usage_error_without_ipc() {
        let args = vec!["--json".into(), "unknown".into()];
        let error = run(&args).unwrap_err();
        let response: Value = serde_json::from_str(&error.message).unwrap();
        assert_eq!(error.exit_code, 2);
        assert!(error.json_output);
        assert_eq!(response["ok"], false);
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("invalid command")
        );
    }

    #[test]
    fn invalid_command_without_json_is_a_plain_usage_error() {
        let args = vec!["unknown".into()];
        let error = run(&args).unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(!error.json_output);
        assert!(error.message.contains("Usage: dispcontrol"));
    }
}
