use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dispcontrol_app::{
    Clock, CompositeBackend, HotkeyRepository, MonitorBackend, MonitorService, PresetRepository,
    SettingsRepository, TimedBackend,
};
use dispcontrol_ddc_fake::{FakeBackend, FakeMonitor};
use dispcontrol_ddc_windows::WindowsDdcBackend;
use dispcontrol_ipc::dispatch;
use dispcontrol_panel_windows::PanelBackend;
use dispcontrol_store_file::FileSettingsRepository;

struct DesktopClock;

impl Clock for DesktopClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("dispcontrold: {error}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: dispcontrold [--native-ui] [--background] [--demo] [--config <path>]";

#[derive(Debug, Default, PartialEq)]
struct Options {
    native_ui: bool,
    /// Start in the tray without showing Settings.
    background: bool,
    /// Simulated monitors instead of real ones (demos and the smoke test).
    demo: bool,
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
            "--config" => {
                let path = arguments.next().ok_or(USAGE)?;
                options.config = Some(PathBuf::from(path));
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok(options)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = parse_options(&arguments)?;

    let path = match options.config {
        Some(path) => path,
        None => FileSettingsRepository::default_path()?,
    };
    // External monitors over DDC/CI plus the built-in display (SPEC-PNL).
    let monitors: Arc<dyn MonitorBackend> = if options.demo {
        Arc::new(FakeBackend::new([
            FakeMonitor::reference("Demo-monitor"),
            FakeMonitor::built_in("Built-in-display"),
        ]))
    } else {
        Arc::new(CompositeBackend::new(vec![
            Arc::new(WindowsDdcBackend::new()),
            Arc::new(PanelBackend::new()),
        ]))
    };
    // Every DDC/CI call runs on a per-monitor worker with a timeout (SPEC-MON-4).
    let backend = Arc::new(TimedBackend::with_defaults(monitors));
    let store = Arc::new(FileSettingsRepository::new(path));
    let settings: Arc<dyn SettingsRepository> = store.clone();
    let presets: Arc<dyn PresetRepository> = store.clone();
    let hotkeys: Arc<dyn HotkeyRepository> = store;
    let prompter = Arc::new(dispcontrol_ui_win32::DesktopInputPrompter);
    let service = Arc::new(MonitorService::new(
        backend,
        settings,
        presets,
        hotkeys,
        prompter,
        Arc::new(DesktopClock),
    ));
    let request_service = service.clone();
    let _server = thread::spawn(move || {
        if let Err(error) =
            dispcontrol_ipc::serve_forever(move |request| dispatch(&*request_service, request))
        {
            eprintln!("dispcontrold IPC server stopped: {error}");
        }
    });
    dispcontrol_ui_win32::run_with_options(service, options.native_ui, options.background)?;
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
        let options = parse(&["--background", "--config", "c.toml", "--demo"]).unwrap();
        assert!(options.background && options.demo && !options.native_ui);
        assert_eq!(options.config, Some(PathBuf::from("c.toml")));
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--verbose"]).is_err());
    }
}
