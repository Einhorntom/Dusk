//! End-to-end: a second `duskd` on the same pipe exits and leaves the first
//! one serving (one tray icon, one set of hotkeys, one write rate limiter).
//! Uses simulated monitors, temporary configs and a private pipe.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use dusk_cli::run;

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

fn start(pipe: &str, folder: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_duskd"))
        .args(["--demo", "--background", "--config"])
        .arg(folder.join("config.toml"))
        .env(dusk_ipc::PIPE_ENV, pipe)
        .spawn()
        .expect("duskd starts")
}

fn list() -> Result<String, i32> {
    run(&["--json".to_owned(), "list".to_owned()]).map_err(|error| error.exit_code)
}

#[test]
fn a_second_daemon_on_the_same_pipe_hands_over_and_exits() {
    let pipe = format!("dusk-single-{}", std::process::id());
    let folder = std::env::temp_dir().join(&pipe);
    std::fs::create_dir_all(&folder).unwrap();
    let first = Daemon {
        process: start(&pipe, &folder),
        folder: folder.clone(),
    };
    // SAFETY: this test binary has a single test, so nothing else reads the environment.
    unsafe { std::env::set_var(dusk_ipc::PIPE_ENV, &pipe) };
    let deadline = Instant::now() + Duration::from_secs(20);
    while let Err(code) = list() {
        assert!(
            code == 7 && Instant::now() < deadline,
            "first daemon did not start ({code})"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let second_folder = folder.join("second");
    std::fs::create_dir_all(&second_folder).unwrap();
    let mut second = start(&pipe, &second_folder);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = second.kill();
            panic!("the second daemon kept running");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "second daemon exit status: {status}");
    assert!(
        !second_folder.join("config.toml").exists(),
        "the second daemon must not touch its settings"
    );
    assert!(list().is_ok(), "the first daemon keeps serving");
    drop(first);
}
