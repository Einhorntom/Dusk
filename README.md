# Dusk

Control your monitors from the Windows tray. Switch between day and night setups in one keystroke, without reaching for the monitor's buttons.

Dusk talks to external monitors over **DDC/CI** (the standard monitor-control channel in HDMI, DisplayPort and USB-C) and to a laptop's **built-in display** through Windows. It is a small Rust tray app with presets, global hotkeys, a command line, and PowerToys Run / Command Palette integration.

> **Status:** v1 features are complete on Windows 11. There are no prebuilt releases yet; build from source (below). Scheduled presets (v2) and Ubuntu support (v3) are planned.

## Features

- **Monitor controls:** brightness, contrast, volume, input source, color preset, RGB gains and power, limited to what each monitor reports it supports. The laptop's built-in display offers brightness.
- **Presets:** save the current setup in one click, apply it with one click, a hotkey or a command. Presets can cover several monitors, can be edited value by value, and can be exported and imported as a commented TOML file.
- **Global hotkeys:** step brightness, contrast or volume, apply or cycle presets, switch input, turn the display off and on. An optional on-screen indicator shows the new value.
- **Command line:** `dusk` reads and sets controls and manages presets, with `--json` output and meaningful exit codes for scripts.
- **PowerToys:** type `dusk brightness 40` or a preset name in PowerToys Run, or use the Dusk commands in Command Palette.
- **Safe by design:**
  - Changes are written only after you stop adjusting, unchanged values are never rewritten, and writes are rate-limited, to spare the monitor's memory.
  - Switching input asks first and reverts automatically unless you keep it, because a monitor acting as a USB hub disconnects your keyboard and mouse.
  - A monitor that stops answering times out instead of freezing the app.
- **Light:** a single tray process, no network access, no telemetry, no administrator rights. Settings live in one human-editable file.

## Requirements

- Windows 11 (x64).
- A monitor with DDC/CI enabled (usually an on-screen menu option). Tested on a **Lenovo L32p-30** over USB-C with Intel Iris Xe graphics, plus that laptop's built-in display. Other monitors work through capability discovery but are not yet verified.
- To build: Rust 1.88 or newer (edition 2024, let chains). For the PowerToys integrations: the .NET 9 SDK and PowerToys 0.96.

## Build and run

```powershell
git clone https://github.com/<you>/dusk.git
cd dusk
cargo build --release --bins
.\target\release\duskd.exe
```

`duskd` is the tray app: it owns the monitors, the Settings window, hotkeys, and the pipe that the CLI and integrations talk to. Closing or minimizing Settings hides it; use **Quit** in the tray menu to exit. Useful options:

| Option | Effect |
|---|---|
| `--background` | Start in the tray without showing Settings (e.g. at sign-in). |
| `--demo` | Use simulated monitors instead of real ones. |
| `--config <path>` | Use another settings file. |
| `--native-ui` | Use the compact fallback Settings window. |

Settings, presets and hotkeys are stored in `%APPDATA%\Dusk\config.toml`.

## Command line

The CLI works only while `duskd` is running, so every change goes through the same buffering, rate limits and input confirmation.

```powershell
dusk list                                   # monitors and their IDs
dusk get L32p-30#0 brightness
dusk set L32p-30#0 brightness 40            # 0-100
dusk set Built-in-display brightness 30
dusk preset apply "Night mode"
dusk preset next                            # cycle through presets
dusk preset save "Day" --include-input      # capture the current setup
dusk preset export presets.toml             # edit in any text editor...
dusk preset import presets.toml             # ...and bring it back
dusk --json preset list
```

Run `dusk` without arguments for the full command list. Exit codes: `0` success, `1` error, `2` invalid usage or value, `3` monitor or preset not found, `4` monitor not responding, `5` preset partly applied, `6` input change declined or reverted, `7` Dusk is not running.

## PowerToys integration

Both integrations are thin clients of the running `duskd`.

**PowerToys Run** (Alt+Space): type `dusk` and then a preset name, `brightness 40`, `contrast 60`, `volume 20` or `input hdmi`. Control words can be shortened (`dusk bri 30`), and extra words pick a monitor (`dusk bri 30 built`).

```powershell
# Quit PowerToys first, then:
powershell -ExecutionPolicy Bypass -File .\integrations\install-powertoys-run.ps1
```

**Command Palette**: open **Dusk** and type the same commands, or type a preset name directly ("Apply monitor preset: Night mode"). Installing it requires Windows Developer Mode (Settings > System > Advanced > For developers):

```powershell
powershell -ExecutionPolicy Bypass -File .\integrations\install-command-palette.ps1
```

## How it is built

Dusk follows Clean Architecture: pure rules in `domain`, use cases in `app`, and adapters for everything external (DDC/CI, WMI, the settings file, the named pipe, the Win32 UI). The layering is checked in CI by `scripts/check_layers.py`.

| Crate | Role |
|---|---|
| `domain` | Controls, presets, hotkeys, write policy (pure, unit-tested) |
| `app` | Use cases (`MonitorService`), ports, per-monitor timeouts, backend composition |
| `ddc-windows`, `panel-windows`, `mccs` | DDC/CI and built-in display backends, capability parsing |
| `store-file`, `ipc`, `cli` | TOML settings, named-pipe protocol, command line |
| `ui-model`, `ui-win32` | View models and the Windows UI (tray, Settings, hotkeys, indicator) |
| `ddc-fake` | Simulated monitors and in-memory ports for tests and `--demo` |
| `bin-cli`, `bin-daemon` | The `dusk` and `duskd` executables |
| `integrations/` | C# client library, PowerToys Run plugin, Command Palette extension |

More detail:

- [PRD.md](PRD.md): product goals and scope.
- [SPEC.md](SPEC.md): precise, testable behavior.
- [ARCH.md](ARCH.md): architecture and design decisions.
- [DEV_STATUS.md](DEV_STATUS.md): progress, validation and roadmap.

## Development

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python scripts/check_layers.py

dotnet test integrations/Dusk.Integrations.sln -c Release -p:Platform=x64
```

- **No hardware needed:** the regular tests use simulated monitors, and the end-to-end smoke test starts `duskd --demo` on a private pipe.
- **Hardware checks are opt-in:** `cargo test -p dusk-ddc-windows -- --ignored` and `cargo test -p dusk-panel-windows -- --ignored`. The first only reads. The panel test changes the built-in display's brightness by 1 % for a moment and restores it.
- **C# contract test:** set `DUSKD_EXE` to a built `duskd.exe` to run the C# client against the real daemon.

## Roadmap

- **v2:** schedules that apply presets at set times, with pause/resume and missed-rule handling after sleep.
- **v3:** Ubuntu support through `ddcutil` (CLI, presets and schedules first).
- **Later:**
  - more monitor and graphics hardware;
  - per-model quirk profiles;
  - GPU-level contrast for built-in displays.

## License

MIT. See [LICENSE](LICENSE).
