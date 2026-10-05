//! End-to-end smoke test, the tip of the test pyramid: the real `duskd`
//! (simulated monitors, temporary config, private pipe, window hidden) driven
//! by the real CLI over the named pipe. It needs no monitor and never touches
//! the user's running daemon or settings.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use dusk_cli::{CliError, run};
use serde_json::Value;

/// The daemon process and its config folder, cleaned up even if an assertion fails.
struct Daemon {
    process: Child,
    folder: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn cli(args: &[&str]) -> Result<Value, CliError> {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let output = run(&args)?;
    Ok(serde_json::from_str(&output).expect("--json output is JSON"))
}

#[test]
fn the_daemon_serves_cli_requests_end_to_end() {
    let pipe = format!("dusk-smoke-{}", std::process::id());
    let folder = std::env::temp_dir().join(&pipe);
    std::fs::create_dir_all(&folder).unwrap();
    let config = folder.join("config.toml");
    let process = Command::new(env!("CARGO_BIN_EXE_duskd"))
        .args(["--demo", "--background", "--config"])
        .arg(&config)
        .env(dusk_ipc::PIPE_ENV, &pipe)
        .spawn()
        .expect("duskd starts");
    let _daemon = Daemon { process, folder };
    // SAFETY: this test binary runs no other test that reads the environment.
    unsafe { std::env::set_var(dusk_ipc::PIPE_ENV, &pipe) };

    let deadline = Instant::now() + Duration::from_secs(20);
    let listed = loop {
        match cli(&["--json", "list"]) {
            Ok(listed) => break listed,
            Err(error) if error.exit_code == 7 && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("list failed: {}", error.message),
        }
    };
    assert_eq!(listed["result"][0]["id"], "Demo-monitor");
    assert_eq!(listed["result"][1]["name"], "Built-in display");

    // The built-in display takes part like any monitor (SPEC-PNL-3).
    cli(&["--json", "set", "Built-in-display", "brightness", "35"]).unwrap();
    let panel = cli(&["--json", "get", "Built-in-display", "brightness"]).unwrap();
    assert_eq!(panel["result"]["value"], "35");
    let contrast = cli(&["--json", "get", "Built-in-display", "contrast"]).unwrap_err();
    assert_eq!(contrast.exit_code, 2);

    cli(&["--json", "set", "Demo-monitor", "brightness", "70"]).unwrap();
    let reading = cli(&["--json", "get", "Demo-monitor", "brightness"]).unwrap();
    assert_eq!(reading["result"]["value"], "70");

    cli(&["--json", "preset", "save", "Smoke"]).unwrap();
    let saved = std::fs::read_to_string(&config).expect("the config file was written");
    assert!(saved.contains("name = \"Smoke\""));

    let unknown = cli(&["--json", "get", "No-such-monitor", "brightness"]).unwrap_err();
    assert_eq!(unknown.exit_code, 3);
}
