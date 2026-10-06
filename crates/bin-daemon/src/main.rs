// No console window: Dusk lives in the tray. Fatal startup errors are
// shown in a message box; everything else goes to the log.
#![windows_subsystem = "windows"]

mod logging;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dusk_app::{
    BackendError, Clock, CompositeBackend, HotkeyRepository, MonitorBackend, MonitorService,
    PresetRepository, SettingsRepository, TimedBackend,
};
use dusk_ddc_fake::{FakeBackend, FakeMonitor};
use dusk_ddc_windows::WindowsDdcBackend;
use dusk_domain::AppSettings;
use dusk_ipc::dispatch;
use dusk_panel_windows::PanelBackend;
use dusk_store_file::FileSettingsRepository;

use logging::DaemonLogger;

struct DesktopClock;

impl Clock for DesktopClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// Turns the log file on or off as soon as the setting is saved.
struct LogSwitchingSettings {
    inner: Arc<FileSettingsRepository>,
    logger: &'static DaemonLogger,
}

impl SettingsRepository for LogSwitchingSettings {
    fn load(&self) -> Result<AppSettings, BackendError> {
        self.inner.load()
    }

    fn save(&self, settings: &AppSettings) -> Result<(), BackendError> {
        self.inner.save(settings)?;
        self.logger.set_file_enabled(settings.diagnostic_log);
        Ok(())
    }
}

fn main() {
    let logger = DaemonLogger::install(DaemonLogger::new(
        log_directory(),
        false,
        std::env::var("USERPROFILE").ok(),
    ));
    if let Err(error) = run(logger) {
        log::error!("duskd: {error}");
        dusk_ui_win32::show_startup_error(&error.to_string());
        std::process::exit(1);
    }
}

/// `%LOCALAPPDATA%\Dusk\logs` (machine-local, not roamed like the settings).
fn log_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Dusk")
        .join("logs")
}

const USAGE: &str = "usage: duskd [--native-ui] [--background] [--demo] [--log] [--config <path>]";

#[derive(Debug, Default, PartialEq)]
struct Options {
    native_ui: bool,
    /// Start in the tray without showing Settings.
    background: bool,
    /// Simulated monitors instead of real ones (demos and the smoke test).
    demo: bool,
    /// Write the diagnostic log file regardless of the setting.
    log: bool,
    config: Option<PathBuf>,
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--native-ui" => options.native_ui = true,
            "--background" => options.background = true,
            "--demo" => options.demo = true,
            "--log" => options.log = true,
            "--config" => {
                let path = arguments.next().ok_or(USAGE)?;
                options.config = Some(PathBuf::from(path));
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok(options)
}

fn run(logger: &'static DaemonLogger) -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = parse_options(&arguments)?;

    // One daemon per pipe: a second start shows the running one's Settings.
    let instance_key = dusk_ipc::pipe_name();
    let _instance = match dusk_ui_win32::claim_instance(&instance_key)? {
        dusk_ui_win32::Claim::First(guard) => guard,
        dusk_ui_win32::Claim::AlreadyRunning => {
            log::info!("Dusk is already running");
            if !options.background {
                dusk_ui_win32::request_show_settings(&instance_key);
            }
            return Ok(());
        }
    };
    // Claimed before the UI starts, so a program squatting on the pipe name
    // is reported instead of silently sharing it.
    let server = dusk_ipc::PipeServer::bind()?;

    let path = match options.config {
        Some(path) => path,
        None => {
            let path = FileSettingsRepository::default_path()?;
            // Settings, presets and hotkeys from before the rename to Dusk.
            let legacy = FileSettingsRepository::legacy_path()?;
            match FileSettingsRepository::migrate_legacy(&path, &legacy) {
                Ok(true) => log::info!(
                    "copied settings from {} to {}",
                    legacy.display(),
                    path.display()
                ),
                Ok(false) => {}
                Err(error) => log::warn!("could not copy old settings: {error}"),
            }
            path
        }
    };
    // External monitors over DDC/CI plus the built-in display (SPEC-PNL).
    // `renames` maps position-based monitor IDs to EDID-based ones (SPEC-MON-1).
    let mut renames = Vec::new();
    let monitors: Arc<dyn MonitorBackend> = if options.demo {
        Arc::new(FakeBackend::new([
            FakeMonitor::reference("Demo-monitor"),
            FakeMonitor::built_in("Built-in-display"),
        ]))
    } else {
        let ddc = Arc::new(WindowsDdcBackend::new());
        // Discovery opens handles and reads the registry only (no DDC/CI
        // traffic), so it cannot hang here.
        renames = ddc.legacy_id_renames().unwrap_or_else(|error| {
            log::warn!("cannot check for renamed monitors: {error}");
            Vec::new()
        });
        Arc::new(CompositeBackend::new(vec![
            ddc,
            Arc::new(PanelBackend::new()),
        ]))
    };
    // Every DDC/CI call runs on a per-monitor worker with a timeout (SPEC-MON-4).
    let backend = Arc::new(TimedBackend::with_defaults(monitors));
    let store = Arc::new(FileSettingsRepository::new(path));
    let log_setting = store.load().map(|settings| settings.diagnostic_log);
    logger.set_file_enabled(options.log || log_setting.as_ref().is_ok_and(|on| *on));
    log::info!("Dusk {} started", env!("CARGO_PKG_VERSION"));
    if let Err(error) = log_setting {
        log::warn!("cannot read the settings: {error}");
    }
    let settings: Arc<dyn SettingsRepository> = Arc::new(LogSwitchingSettings {
        inner: store.clone(),
        logger,
    });
    let presets: Arc<dyn PresetRepository> = store.clone();
    let hotkeys: Arc<dyn HotkeyRepository> = store;
    let prompter = Arc::new(dusk_ui_win32::DesktopInputPrompter);
    let service = Arc::new(MonitorService::new(
        backend,
        settings,
        presets,
        hotkeys,
        prompter,
        Arc::new(DesktopClock),
    ));
    match service.rename_monitor_ids(&renames) {
        Ok(0) => {}
        Ok(moved) => log::info!("moved {moved} preset and hotkey references to new monitor IDs"),
        Err(error) => log::warn!("cannot move presets and hotkeys to new monitor IDs: {error}"),
    }
    // Buffered slider and hotkey values are written by the application,
    // off the UI thread (SPEC-WR-2); the Settings window shows the results.
    let commits = Arc::new(dusk_ui_win32::CommitNotifier::default());
    service.start_committer(commits.clone())?;
    let request_service = service.clone();
    let _server = thread::spawn(move || {
        if let Err(error) =
            server.serve_forever(move |request| dispatch(&*request_service, request))
        {
            log::error!("IPC server stopped: {error}");
        }
    });
    dusk_ui_win32::run_with_options(
        service,
        options.native_ui,
        options.background,
        &instance_key,
        commits,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Options, String> {
        let arguments: Vec<String> = arguments.iter().map(|item| item.to_string()).collect();
        parse_options(&arguments)
    }

    #[test]
    fn options_parse_in_any_order_and_reject_unknown_ones() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        let options = parse(&["--background", "--config", "c.toml", "--demo", "--log"]).unwrap();
        assert!(options.background && options.demo && options.log && !options.native_ui);
        assert_eq!(options.config, Some(PathBuf::from("c.toml")));
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--verbose"]).is_err());
    }
}
