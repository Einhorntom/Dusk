# PRD: dispcontrol

A lightweight, open-source tool for controlling an external monitor from the desktop: quickly, with hotkeys, presets and schedules.
Status: Draft v0.4. Design and technology notes live in [ARCH.md](./ARCH.md).

## 1. Problem
Lenovo Display Control Center is unreliable, and ClickMonitorDDC is no longer maintained. Users need a fast, tiny, dependable way to change external-monitor settings and jump between saved setups.

## 2. Target user and hardware
- A single power user with an external monitor, working on Windows 11 (primary) and Ubuntu (secondary).
- v1 supported hardware: **Lenovo L32p-30**, connected directly over **USB-C**, also used as the USB hub, on a PC with **Intel Iris Xe Graphics**.
- Other monitors and graphics hardware may work but are not supported in v1 (see Future features).

## 3. Goals
1. Very light and snappy: minimal CPU and memory use, instant startup, no noticeable delay when acting.
2. Presets: save and recall named configurations in one action.
3. Hotkeys: change settings without opening the app.
4. Integrations: control from the command line, PowerToys Run and PowerToys Command Palette.
5. Windows 11 first; Ubuntu support as a secondary target.
6. Protect the monitor's memory from wear: apply changes only once the user has finished adjusting.
7. Automatic preset switching at fixed times of day.

## 4. Non-goals (v1)
- Laptop internal-panel brightness; software dimming overlays.
- Monitor firmware updates or vendor-specific features.
- Sunrise/sunset-based scheduling.
- Multi-user or enterprise management.
- Support for hardware other than that listed in section 2.

## 5. Features

### 5.1 Controls
Brightness, contrast, input source, volume, power mode (on/standby), color preset, and RGB gain. Only controls the monitor actually supports are shown.

### 5.2 Monitor handling
- Presets stay bound to the right monitor even if it is unplugged, re-plugged or reordered.
- The app never freezes if the monitor is slow or not responding; it shows a clear "not responding" state.
- The app notices when monitors are connected, disconnected or the PC wakes from sleep.

### 5.3 Presets
- Capture the monitor's current state as a preset in one click.
- Rename, edit, delete and reorder presets.
- A preset may include any subset of controls and may cover more than one monitor.
- Applying a preset changes only values that differ from the current ones.
- A hotkey can cycle through presets.

### 5.4 Scheduling
- Rules of the form "apply preset X at HH:MM on selected days".
- After sleep, unlock or reconnecting a monitor, the most recent missed rule is applied (can be turned off).
- A manual change is never overridden until the next scheduled time.
- The schedule can be paused and resumed.

### 5.5 Hotkeys
- User-configurable global hotkeys; conflicts and registration failures are reported to the user.
- Actions: apply preset N, cycle presets, increase/decrease brightness, contrast or volume, switch input, toggle standby, pause/resume schedule.
- Optional small on-screen indicator showing the new value (can be disabled).

### 5.6 User interface
- Lives in the system tray; no main window.
- Clicking the tray icon opens a compact panel with per-monitor sliders, preset buttons and schedule status; it closes when it loses focus.
- Looks native to Windows 11 and follows the system light/dark theme.
- Settings: hotkeys, step sizes, schedule, write delay, live preview, start with Windows.

### 5.7 Protecting monitor memory
- While a slider is dragged or a hotkey is held, changes are only buffered.
- The monitor is updated once, after the user stops adjusting (default 400 ms, configurable).
- Unchanged values are never rewritten, and nothing is written at app startup or exit.
- Writes are rate-limited per setting.
- **Live preview** (monitor updating during the drag) is **off by default** and can be enabled in settings, with a one-time notice about the wear risk.
- An optional audit log shows how many writes were made per setting.

### 5.8 Input switching safety
- The monitor doubles as the user's USB hub, so switching away from USB-C disconnects USB devices on it (e.g. keyboard and mouse).
- **Switching away from USB-C requires confirmation by default**, with a clear warning.
- Input changes offer an automatic revert if the user does not confirm within a timeout (default 10 seconds, configurable).

### 5.9 Integrations
- **Command line**: list monitors, read/set a control, list/apply presets, pause/resume the schedule; machine-readable output and meaningful exit codes.
- **PowerToys Run**: search presets and run commands such as setting brightness.
- **PowerToys Command Palette**: the same commands.
- Integrations respond immediately when the app is running.
- Optional link format to apply a preset (e.g. from shortcuts or scripts).

## 6. Quality requirements
- Idle CPU use effectively 0%; idle memory under 30 MB; startup under 300 ms.
- Small download (under 10 MB).
- No telemetry, no network access, no administrator rights; only one instance runs.
- Settings stored in a single human-editable file; a portable mode is available.
- Logging is off by default.
- Open source under the **MIT** license, hosted on GitHub, with documentation of tested hardware and troubleshooting.

## 7. Success metrics
- Hotkey press to visible monitor change under 200 ms (excluding the write delay and the monitor's own latency).
- No UI freeze when the monitor ignores commands.
- At most one write per setting per user adjustment with live preview off.
- All v1 controls work on the L32p-30 over USB-C with Intel Iris Xe Graphics.
- A scheduled preset applies within 1 second of its time, including after waking from sleep.

## 8. Risks
- Monitor control over USB-C may be unreliable on some graphics drivers, docks or hubs.
- The monitor may respond slowly, incorrectly, or ignore commands.
- Switching input can leave the user without keyboard/mouse or without a picture (mitigated by 5.8).
- Ubuntu on Wayland limits global hotkeys and tray behavior.
- PowerToys extension interfaces may change between releases.

## 9. Release plan
- **Phase 0**: Prove the L32p-30 can be read and controlled over USB-C on Intel Iris Xe (brightness, input, volume).
- **Phase 1**: Command-line control of all v1 settings with memory-protecting writes.
- **Phase 2**: Tray panel, presets, hotkeys, settings.
- **Phase 3**: Scheduling.
- **Phase 4**: PowerToys Run and Command Palette integrations.
- **Phase 5**: Ubuntu support (command line and scheduling first, tray later).
- **Phase 6**: On-screen indicator, installer, package-manager release, documentation.

## 10. Future features (post v1)
- Support for all graphics hardware (other Intel, AMD, NVIDIA) and drivers.
- Support for other monitors and brands; per-model profiles contributed by the community.
- Sunrise/sunset-based scheduling; scheduling by ambient light or app/context (e.g. game or video detected).
- Full Ubuntu tray UI; other Linux desktops (KDE) and distributions; macOS.
- Per-application or per-window automatic presets.
- Software dimming fallback for monitors without DDC/CI support.
- Laptop internal-panel brightness control.
- Preset import/export and sharing.
- Syncing brightness across multiple monitors.
- Mobile/remote control and smart-home integration.

## 11. Decisions
1. v1 controls: brightness, contrast, input, volume, power, color preset, RGB gain.
2. License: MIT.
3. Connection: directly over USB-C; the monitor is also the USB hub.
4. v1 graphics hardware: Intel Iris Xe Graphics; all others are a future goal.
5. Linux target: Ubuntu.
6. Scheduling: fixed clock times only.
7. Live preview: off by default, opt-in.
8. Switching away from USB-C requires confirmation by default.

## 12. Open questions
- None currently.
