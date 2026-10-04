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

/// Sends one encoded request to the daemon and returns the encoded response.
pub type Transport<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>, IpcError>;

pub fn run(args: &[String]) -> Result<String, CliError> {
    run_with(args, &transact)
}

/// Runs the CLI against `transport` instead of the daemon's named pipe.
pub fn run_with(args: &[String], transport: Transport<'_>) -> Result<String, CliError> {
    let json_output = args.iter().any(|argument| argument == "--json");
    let command_args = args
        .iter()
        .filter(|argument| argument.as_str() != "--json")
        .cloned()
        .collect::<Vec<_>>();
    run_command(&command_args, json_output, transport)
}

fn run_command(
    args: &[String],
    json_output: bool,
    transport: Transport<'_>,
) -> Result<String, CliError> {
    if let Some(output) = file_command(args, json_output, transport)? {
        return Ok(if json_output {
            json!({ "ok": true, "result": output, "error": null }).to_string()
        } else {
            output
        });
    }
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
        _ => preset_request(args, json_output)?,
    };

    let parsed = exchange(&request, json_output, transport)?;
    let partial = matches!(parsed["result"]["failed"].as_u64(), Some(failed) if failed > 0);
    if json_output {
        let output = json!({
            "ok": !partial,
            "result": parsed["result"],
            "error": if partial { json!("some preset entries failed") } else { Value::Null },
        });
        let text = serde_json::to_string(&output)
            .map_err(|error| cli_error(&error.to_string(), 1, true))?;
        return if partial {
            Err(CliError {
                message: text,
                exit_code: 5,
                json_output: true,
            })
        } else {
            Ok(text)
        };
    }
    let text = serde_json::to_string_pretty(&parsed["result"]).map_err(|error| CliError {
        message: error.to_string(),
        exit_code: 1,
        json_output: false,
    })?;
    if partial {
        return Err(CliError {
            message: format!("{text}\nsome preset entries failed"),
            exit_code: 5,
            json_output: false,
        });
    }
    Ok(text)
}

fn exchange(
    request: &Value,
    json_output: bool,
    transport: Transport<'_>,
) -> Result<Value, CliError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|error| cli_error(&error.to_string(), 1, json_output))?;
    let response = transport(&bytes).map_err(|error| {
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
    Ok(parsed)
}

/// `preset export [file]` and `preset import <file> [--replace]` touch the local file system.
fn file_command(
    args: &[String],
    json_output: bool,
    transport: Transport<'_>,
) -> Result<Option<String>, CliError> {
    let replace = args.iter().any(|argument| argument == "--replace");
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|argument| *argument != "--replace")
        .collect();
    match positional.as_slice() {
        ["preset", "export", rest @ ..] if rest.len() <= 1 => {
            let parsed = exchange(&json!({ "op": "preset_export" }), json_output, transport)?;
            let text = parsed["result"]["text"].as_str().unwrap_or_default();
            match rest.first() {
                None => Ok(Some(text.to_owned())),
                Some(path) => {
                    std::fs::write(path, text).map_err(|error| {
                        cli_error(&format!("cannot write {path}: {error}"), 1, json_output)
                    })?;
                    Ok(Some(format!("exported presets to {path}")))
                }
            }
        }
        ["preset", "import", path] => {
            let text = std::fs::read_to_string(path).map_err(|error| {
                cli_error(&format!("cannot read {path}: {error}"), 2, json_output)
            })?;
            let parsed = exchange(
                &json!({ "op": "preset_import", "text": text, "replace": replace }),
                json_output,
                transport,
            )?;
            let result = &parsed["result"];
            Ok(Some(format!(
                "imported presets: {} added, {} updated, {} discarded",
                result["added"], result["updated"], result["discarded"]
            )))
        }
        _ => Ok(None),
    }
}

fn preset_request(args: &[String], json_output: bool) -> Result<Value, CliError> {
    let mut include_input = false;
    let mut force = false;
    let positional: Vec<&String> = args
        .iter()
        .filter(|argument| match argument.as_str() {
            "--include-input" => {
                include_input = true;
                false
            }
            "--force" => {
                force = true;
                false
            }
            _ => true,
        })
        .collect();
    let request = match positional.as_slice() {
        [command, verb] if command.as_str() == "preset" && verb.as_str() == "list" => {
            json!({ "op": "preset_list" })
        }
        [command, verb, name] if command.as_str() == "preset" => match verb.as_str() {
            "apply" => json!({ "op": "preset_apply", "name": name }),
            "save" => json!({ "op": "preset_save", "name": name, "include_input": include_input }),
            "delete" => json!({ "op": "preset_delete", "name": name, "force": force }),
            _ => return Err(usage_error("invalid command or arguments", json_output)),
        },
        [command, verb, name, other] if command.as_str() == "preset" => match verb.as_str() {
            "save" => json!({
                "op": "preset_save",
                "name": name,
                "monitor": other,
                "include_input": include_input,
            }),
            "rename" => json!({ "op": "preset_rename", "name": name, "new_name": other }),
            "move" => {
                let offset: i32 = other
                    .parse()
                    .map_err(|_| usage_error("offset must be an integer", json_output))?;
                json!({ "op": "preset_move", "name": name, "offset": offset })
            }
            _ => return Err(usage_error("invalid command or arguments", json_output)),
        },
        [command, verb] if command.as_str() == "preset" && verb.as_str() == "next" => {
            json!({ "op": "preset_cycle", "forward": true })
        }
        [command, verb] if command.as_str() == "preset" && verb.as_str() == "prev" => {
            json!({ "op": "preset_cycle", "forward": false })
        }
        [command, verb] if command.as_str() == "preset" && verb.as_str() == "path" => {
            json!({ "op": "preset_path" })
        }
        [command, verb, name, monitor, control]
            if command.as_str() == "preset" && verb.as_str() == "unset" =>
        {
            parse_control(control, json_output)?;
            json!({
                "op": "preset_remove_entry",
                "name": name,
                "monitor": monitor,
                "control": control,
            })
        }
        [command, verb, name, monitor, control, value]
            if command.as_str() == "preset" && verb.as_str() == "set" =>
        {
            let key = parse_control(control, json_output)?;
            let number: u32 = value
                .parse()
                .map_err(|_| usage_error("value must be an unsigned integer", json_output))?;
            if key.is_numeric() && number > 100 {
                return Err(usage_error(
                    "normalized value must be between 0 and 100",
                    json_output,
                ));
            }
            let kind = if key.is_numeric() {
                "normalized"
            } else {
                "enum"
            };
            json!({
                "op": "preset_set_entry",
                "name": name,
                "monitor": monitor,
                "control": control,
                "value": { "kind": kind, "value": number },
            })
        }
        _ => return Err(usage_error("invalid command or arguments", json_output)),
    };
    Ok(request)
}

fn parse_control(value: &str, json_output: bool) -> Result<ControlKey, CliError> {
    ControlKey::from_str(value).map_err(|error| usage_error(&error.to_string(), json_output))
}

fn usage_error(message: &str, json_output: bool) -> CliError {
    cli_error(
        &format!(
            "{message}\nUsage: dispcontrol [--json] list | get <monitor-id> <control> | \
             set <monitor-id> <control> <value> | settings show | preset list | \
             preset apply <name> | preset next | preset prev | \
             preset save <name> [monitor-id] [--include-input] | preset delete <name> [--force] | \
             preset rename <name> <new-name> | preset move <name> <offset> | \
             preset set <name> <monitor-id> <control> <value> | \
             preset unset <name> <monitor-id> <control> | preset path | \
             preset export [file] | preset import <file> [--replace]"
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
    fn preset_commands_build_the_expected_requests() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            preset_request(&args(&["preset", "apply", "Night"]), false).unwrap(),
            json!({ "op": "preset_apply", "name": "Night" })
        );
        assert_eq!(
            preset_request(&args(&["preset", "save", "Day", "--include-input"]), false).unwrap(),
            json!({ "op": "preset_save", "name": "Day", "include_input": true })
        );
        assert_eq!(
            preset_request(&args(&["preset", "move", "Day", "-1"]), false).unwrap(),
            json!({ "op": "preset_move", "name": "Day", "offset": -1 })
        );
        assert_eq!(
            preset_request(
                &args(&["preset", "set", "Day", "m", "brightness", "40"]),
                false
            )
            .unwrap(),
            json!({
                "op": "preset_set_entry", "name": "Day", "monitor": "m",
                "control": "brightness", "value": { "kind": "normalized", "value": 40 }
            })
        );
        assert!(
            preset_request(
                &args(&["preset", "set", "Day", "m", "brightness", "140"]),
                false
            )
            .is_err()
        );
        assert_eq!(
            preset_request(&args(&["preset", "unset", "Day", "m", "gain-red"]), false).unwrap(),
            json!({
                "op": "preset_remove_entry", "name": "Day", "monitor": "m", "control": "gain-red"
            })
        );
        assert_eq!(
            preset_request(&args(&["preset", "prev"]), false).unwrap(),
            json!({ "op": "preset_cycle", "forward": false })
        );
        assert_eq!(
            preset_request(&args(&["preset", "bogus", "x"]), false)
                .unwrap_err()
                .exit_code,
            2
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
