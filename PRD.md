# PRD: dispcontrol

A lightweight, open-source tool for controlling external monitors and the laptop's built-in display from the desktop: quickly, with hotkeys, presets and schedules.
Status: Draft v0.5. Design and technology notes live in [ARCH.md](./ARCH.md).

## 1. Problem
Lenovo Display Control Center is unreliable, and ClickMonitorDDC is no longer maintained. Users need a fast, tiny, dependable way to change external-monitor settings and jump between saved setups.

## 2. Target user and hardware
- A single power user with an external monitor, working on Windows 11 (primary) and Ubuntu (secondary).
- Supported hardware (v0 and later until extended): **Lenovo L32p-30**, connected directly over **USB-C**, also used as the USB hub, on a PC with **Intel Iris Xe Graphics**.
- From v1: the **laptop's built-in display** on that PC (brightness, through Windows).
- Other monitors and graphics hardware may work but are not supported yet (see Future features).

## 3. Goals
1. Very light and snappy: minimal CPU and memory use, instant startup, no noticeable delay when acting.
2. Presets: save and recall named configurations in one action.
3. Hotkeys: change settings without opening the app.
4. Integrations: control from the command line, PowerToys Run and PowerToys Command Palette.
5. Windows 11 first; Ubuntu support as a secondary target.
6. Protect the monitor's memory from wear: apply changes only once the user has finished adjusting.
7. Automatic preset switching at fixed times of day.
8. Control the laptop's built-in display brightness together with the external monitors, in the same presets, hotkeys and commands.

## 4. Non-goals (for now)
- Software dimming overlays.
- Monitor firmware updates or vendor-specific features.
- Sunrise/sunset-based scheduling.
- Multi-user or enterprise management.
- Support for hardware other than that listed in section 2.

## 5. Features

### 5.1 Controls
Brightness, contrast, input source, volume, power mode (on/standby), color preset, and RGB gain. Only controls the monitor actually supports are shown.

The laptop's built-in display offers brightness (it has no other monitor controls). It appears as "Built-in display" next to the external monitors and works with presets, hotkeys, the command line and integrations like any other monitor. Windows keeps changing it too (brightness keys, adaptive brightness, battery saver); the app does not fight those changes.

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
- Lives in the system tray; there is no always-visible main window and no taskbar button.
- Clicking the tray icon opens a compact panel with per-monitor sliders, preset buttons and schedule status; it closes when it loses focus.
- Minimizing any app window (for example Settings) sends it to the notification area's hidden-icons overflow instead of the taskbar; the app keeps running and is reopened from its tray icon.
- Looks native to Windows 11 and follows the system light/dark theme.
- Settings: hotkeys, step sizes, schedule, write delay, live preview, start with Windows.

### 5.7 Protecting monitor memory
- While a slider is dragged or a hotkey is held, changes are only buffered.
- The monitor is updated once, after the user stops adjusting (default 400 ms, configurable).
- Unchanged values are never rewritten, and nothing is written at app startup or exit.
- Writes are rate-limited per setting. (The built-in display's brightness is not stored in monitor memory, so it is not rate-limited; it is still only updated once the user stops adjusting.)
- **Live preview** (monitor updating during the drag) is **off by default** and can be enabled in settings, with a one-time notice about the wear risk.
- An optional audit log shows how many writes were made per setting.

### 5.8 Input switching safety
- Changing the input can disconnect devices: a monitor that doubles as a USB hub (as in the reference hardware) drops the USB devices on it (e.g. keyboard and mouse) when the input changes, and an input with no signal can leave the user without a picture.
- **Any input change away from the currently active input requires confirmation by default**, with a clear warning. The confirmation can be turned off in settings.
- Input changes offer an automatic revert if the user does not confirm within a timeout (default 10 seconds, configurable).

### 5.9 Integrations
- **Command line** (works only while the app is running; otherwise it reports an error and changes nothing): list monitors, read/set a control, list/apply presets, pause/resume the schedule; machine-readable output and meaningful exit codes.
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
- All controls in the first release work on the L32p-30 over USB-C with Intel Iris Xe Graphics.
- A scheduled preset applies within 1 second of its time, including after waking from sleep.

## 8. Risks
- Monitor control over USB-C may be unreliable on some graphics drivers, docks or hubs.
- The monitor may respond slowly, incorrectly, or ignore commands.
- Switching input can leave the user without keyboard/mouse or without a picture (mitigated by 5.8).
- Ubuntu on Wayland limits global hotkeys and tray behavior.
- PowerToys extension interfaces may change between releases.

## 9. Release plan
- **Phase 0 (spike)**: Prove the L32p-30 can be read and controlled over USB-C on Intel Iris Xe (brightness, input, volume).
- **v0 (first release, Windows 11)**: Background app with tray icon; Settings window showing only what is implemented (monitor controls, safety and write settings, general settings); confirmation for input changes; memory-protecting writes; command line (works while the app runs).
- **v1**: Presets and hotkeys (tray panel, Presets and Hotkeys pages in Settings), then built-in display brightness, then PowerToys Run and Command Palette integrations.
- **v2**: Scheduling (Schedule page in Settings).
- **v3**: Ubuntu support using `ddcutil` (command line, presets, scheduling first); on-screen indicator, installer, package-manager release, documentation as they become ready.
- The Settings window only shows pages for features that exist in the installed release.
## 10. Future features
- Support for all graphics hardware (other Intel, AMD, NVIDIA) and drivers.
- Support for other monitors and brands; per-model profiles contributed by the community.
- Sunrise/sunset-based scheduling; scheduling by ambient light or app/context (e.g. game or video detected).
- Settings window and full tray UI on Ubuntu; other Linux desktops (KDE) and distributions; macOS.
- Per-application or per-window automatic presets.
- Software dimming fallback for monitors without DDC/CI support.
- Built-in display brightness on Ubuntu.
- Preset import/export and sharing.
- Syncing brightness across multiple monitors.
- Reliable handling of multiple identical monitors (same model, no serial): stable aliases and sync rules.
- Mobile/remote control and smart-home integration.

## 11. Decisions
1. Controls: brightness, contrast, input, volume, power, color preset, RGB gain.
2. License: MIT.
3. Connection: directly over USB-C; the monitor is also the USB hub.
4. Initial graphics hardware: Intel Iris Xe Graphics; all others are a future goal.
5. Linux target: Ubuntu.
6. Scheduling: fixed clock times only.
7. Live preview: off by default, opt-in.
8. Any input change away from the currently active input requires confirmation by default.
9. Minimizing an app window sends it to the hidden-icons tray overflow, not the taskbar.
10. Built-in display: brightness only, through Windows; a monitor like any other for presets, hotkeys and integrations; no write-rate limit; changes made by Windows are respected, not reverted. Delivered in v1, before the PowerToys integrations.

## 12. Open questions
- None currently.
8. Release order: v0 = control and Settings window; v1 = presets, hotkeys, integrations; v2 = scheduling; v3 = Ubuntu.
9. Ubuntu Settings window is a future feature.
