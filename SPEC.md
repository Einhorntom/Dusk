# dispcontrol: Specification

Status: Draft v0.1. Derived from [PRD.md](./PRD.md) v0.4. Describes *what* the product does, precisely and testably. How it is built lives in [ARCH.md](./ARCH.md).
The spec is monitor-model independent: nothing here hard-codes ranges, input names or capabilities of a specific monitor. Everything model-specific is discovered from the monitor at runtime (see section 2) or comes from optional quirk profiles (section 2.1). The Lenovo L32p-30 is the reference test hardware, not a spec dependency.

Requirement keywords: **MUST**, **SHOULD**, **MAY**. Each requirement has an ID for test traceability.

## 1. Terminology
- **Monitor**: a connected display that responds to control commands, including the laptop's built-in display (section 3.1).
- **Control**: an adjustable monitor setting (see 2).
- **Preset**: a named set of control values for one or more monitors.
- **Rule**: a schedule entry that applies a preset at a time of day.
- **Commit**: the moment a value is actually sent to the monitor.
- **Quiet period**: time without further changes after which a buffered value is committed.

## 2. Controls
| Control | Key | Type | Allowed values |
|---|---|---|---|
| Brightness | `brightness` | integer | 0-100 (normalized) |
| Contrast | `contrast` | integer | 0-100 (normalized) |
| Volume | `volume` | integer | 0-100 (normalized) |
| Input source | `input` | enum | the inputs the monitor reports as available |
| Power mode | `power` | enum | `on`, `standby` (and `off` if the monitor reports it) |
| Color preset | `color-preset` | enum | the presets the monitor reports |
| RGB gain | `gain-red`, `gain-green`, `gain-blue` | integer | 0-100 (normalized) |

- SPEC-CTL-1: The app MUST expose only controls the monitor reports as supported.
- SPEC-CTL-2: Capability discovery: for each monitor, the app determines supported controls, native numeric ranges and the allowed values of enum controls from the monitor itself (its self-reported capabilities, falling back to probing each control with a read). Nothing is assumed from the model name.
- SPEC-CTL-3: Numeric controls are exposed to users as 0-100 and mapped to and from the monitor's native range (min..max as discovered). Mapping is deterministic and round-trips: setting N and reading back gives N, or the nearest value the native range can represent. Native values are available in `get`/`set` with `--raw`.
- SPEC-CTL-4: Enum values have stable canonical keys where the standard defines them (e.g. `hdmi1`, `hdmi2`, `dp1`, `dp2`, `usbc`, `vga`) and fall back to `raw-<code>` for values the app does not recognize. Users can give any value a friendly alias in settings.
- SPEC-CTL-5: Setting an unsupported control, an unavailable enum value or an out-of-range value MUST fail with a clear error and change nothing. Hotkey steps clamp to the range limits.
- SPEC-CTL-6: `power=standby` turns the display off. Waking is attempted through the monitor's control interface; if the monitor does not support it, the UI and CLI say so rather than failing silently.
- SPEC-CTL-7: A preset stores normalized values and native enum values (as stored in the config file), so it stays valid if the monitor's native ranges differ; entries that the target monitor doesn't support are skipped and reported (SPEC-PRE-3).

### 2.1 Quirk profiles (optional)
- SPEC-QRK-1: Some monitors report wrong or incomplete capabilities. The app MAY load an optional per-model quirk profile (matched by manufacturer and model from the monitor's identification data) that overrides or supplements discovered capabilities, timing, or retry behavior.
- SPEC-QRK-2: With no profile, the app MUST still work using discovery alone. Profiles are plain data files that users can add or edit without rebuilding the app.
- SPEC-QRK-3: `list --verbose` shows what was discovered and whether a profile was applied, so users can report and fix problems for their own monitor.

## 3. Monitors
- SPEC-MON-1: Each monitor has a stable ID derived from model and serial number. If unavailable, fall back to model + connection position and flag as "unstable ID".
- SPEC-MON-2: Monitors are addressable by ID, by a user-assigned alias, or by index in the CLI.
- SPEC-MON-3: The app MUST detect monitor connect/disconnect and wake from sleep, and refresh its monitor list within 3 seconds.
- SPEC-MON-4: A command to a monitor that does not answer MUST time out (default 2 s, with up to 3 attempts) and report "not responding". It MUST NOT block other monitors, the UI, or hotkeys. A display that Windows cannot open for DDC/CI (such as a laptop panel) is left out of the DDC/CI monitor list instead of making the listing fail; the built-in display is listed through section 3.1 instead.
- SPEC-MON-5: Any monitor that supports the standard monitor-control interface is supported through discovery (section 2). Monitors that were not verified by the project are labeled "unverified" in the UI and `list` output; a project-maintained list records verified models (initially the L32p-30).

### 3.1 Built-in display
- SPEC-PNL-1: On Windows, the laptop's built-in display is listed as a monitor named "Built-in display" (ID `Built-in-display`; a second one, if any, gets a `#n` suffix) whenever Windows reports an active, adjustable brightness for it.
- SPEC-PNL-2: It exposes `brightness` only, 0-100, mapped to the brightness levels Windows reports. No other control is offered (SPEC-CTL-1).
- SPEC-PNL-3: It takes part in everything other monitors do: the Settings window, presets (capture and apply), hotkeys including "all monitors", the CLI and integrations.
- SPEC-PNL-4: Its brightness is not stored in monitor memory, so the write-rate limits (SPEC-WR-6) do not apply. The quiet period (SPEC-WR-2, SPEC-WR-3) and skipping unchanged values (SPEC-WR-4) still do.
- SPEC-PNL-5: Windows and the user may also change the brightness (brightness keys, adaptive brightness, battery saver). The app reads it on demand (SPEC-WR-8) and never re-applies a value on its own; applying a preset sets it once.
- SPEC-PNL-6: While the built-in display is off (for example, lid closed), it is not listed and that is not an error; preset entries for it are skipped like those for any absent monitor (SPEC-PRE-3).

## 4. Writing and monitor-memory protection
- SPEC-WR-1: User adjustments (slider drag, repeated hotkey, CLI `set`) update an in-memory target value immediately; the UI shows it instantly.
- SPEC-WR-2: A commit happens only after the quiet period (default 400 ms; allowed 150-2000 ms) with no newer change to that control.
- SPEC-WR-3: Only the latest target is committed; intermediate values are discarded.
- SPEC-WR-4: A commit MUST be skipped if the target equals the last known monitor value.
- SPEC-WR-5: The app MUST NOT write to a monitor at startup, at exit, on monitor refresh, or while only reading.
- SPEC-WR-6: Per monitor and control, commits are limited to 1 per second and 30 per minute (not for the built-in display, SPEC-PNL-4). Excess changes are held (latest wins) and applied when the limit allows; a warning is logged.
- SPEC-WR-7: CLI `set` and preset apply are explicit user actions: they commit without the quiet period, but still follow SPEC-WR-4 and SPEC-WR-6.
- SPEC-WR-8: Reads happen only on demand (opening the panel, `get`, `list`, after wake); no periodic polling.
- SPEC-WR-9 (live preview): when enabled in settings (default **off**), during a drag the app MAY commit intermediate values at most 4 times per second per control. The final value is still committed per SPEC-WR-2. Enabling shows a one-time notice about wear risk and requires acknowledgment.
- SPEC-WR-10 (audit): when enabled (default off), the app counts commits per monitor and control and exposes the totals in the settings view and via `dispcontrol audit`.

## 5. Input switching safety
- SPEC-IN-1: Changing `input` away from the currently active input MUST show a confirmation dialog by default. This covers the USB-C case: monitors that act as a USB hub disconnect their USB devices (e.g. keyboard, mouse) when the input changes, and the dialog says so. It applies to the panel, hotkeys, presets, schedule rules and CLI/integrations.
- SPEC-IN-2: The confirmation requirement is a setting (default **on**). When off, no dialog is shown.
- SPEC-IN-3: After any input change away from the current input, the app starts a revert timer (default 10 s, configurable 5-60 s, 0 = disabled). Unless the user confirms ("Keep") within the timer, the previous input is restored.
- SPEC-IN-4: For non-interactive callers (CLI, schedule), confirmation is requested via a dialog on the user's desktop; with `--yes` the dialog is skipped but the revert timer still applies unless `--no-revert` is also given.
- SPEC-IN-5: If the previous input cannot be restored (monitor unreachable), the failure is reported and logged. Whether a monitor answers control commands while showing another input varies by model; the app MUST NOT assume it does, and the dialog warns that reverting may not be possible from this PC.
- SPEC-IN-6: Presets containing an `input` change follow SPEC-IN-1 for the whole preset (one dialog, not one per control).

## 6. Presets
- SPEC-PRE-1: A preset has a unique name (case-insensitive, 1-40 characters), an ordering position, and a list of entries: `{monitor, control, value}`.
- SPEC-PRE-2: "Capture" reads current values of all supported controls on the selected monitors and stores them; the user may deselect controls before saving.
- SPEC-PRE-3: Apply commits each entry whose target differs from the monitor's current value (SPEC-WR-4, SPEC-WR-7). Entries for absent monitors are skipped; the result reports which were applied, skipped or failed.
- SPEC-PRE-4: Apply order is fixed: `power`, `input`, `color-preset`, gains, `brightness`, `contrast`, `volume`. A failure in one entry does not stop the others. DDC/CI has no multi-value write, so each entry is a separate command; to minimise the visible transition, all entries are read first and the differing values are then written back to back without reads in between. After a colour-preset change the monitor loads that mode's stored values, so the remaining entries for that monitor are written even if they matched before the switch, after a short settle delay.
- SPEC-PRE-4a: RGB gains belong to the monitor's user colour profiles (MCCS colour preset `0x0B`-`0x0D`), and writing a gain switches the monitor to one. When a preset selects any other colour preset for a monitor, its gain entries for that monitor are not captured, applied or compared (SPEC-PRE-7), and the editor marks them as not applied.
- SPEC-PRE-5: Presets can be renamed, edited, deleted and reordered. Renaming a preset updates the hotkeys that apply it. Deleting a preset used by a hotkey asks for confirmation (Settings) or requires `--force` (CLI) and deletes those hotkeys too; deleting one used by a schedule rule follows the same rule in v2.
- SPEC-PRE-5a: The stored values of a preset can be viewed and individually changed or removed (Settings "Edit" page; CLI `preset set|unset`).
- SPEC-PRE-5b: Presets are stored in the app's config file (`%APPDATA%\dispcontrol\config.toml`; CLI `preset path`). They can be exported to and imported from a commented, human-editable TOML document (CLI `preset export [file]`, `preset import <file> [--replace]`; Settings exports/imports `dispcontrol-presets.toml` next to the config file). Import merges by name unless `--replace` is given, and rejects an invalid document without changing the stored presets.
- SPEC-PRE-6: "Cycle presets" applies the next preset in order, wrapping around; the starting point is the last applied preset or the first if none.
- SPEC-PRE-7: The panel marks the preset whose values match the current monitor state, if any.

## 7. Scheduling
- SPEC-SCH-1: A rule has: preset, time `HH:MM` (24 h, local time), days of week (default all), enabled flag.
- SPEC-SCH-2: A rule fires within 1 s of its time while the app is running and the schedule is not paused.
- SPEC-SCH-3: On startup, wake from sleep, session unlock or monitor reconnect, the app finds, per monitor, the most recent rule whose time already passed today (considering enabled days) and applies it, unless "apply missed rules" is off (default on). Only the single most recent rule is applied.
- SPEC-SCH-4: A manual change (UI, hotkey, CLI) is never reverted by the schedule until the next rule fires.
- SPEC-SCH-5: The schedule can be paused/resumed from the tray menu, a hotkey and the CLI. Pause state persists across restarts and never auto-resumes; only an explicit resume ends it. The tray icon and panel show a clear paused indicator.
- SPEC-SCH-6: Two rules at the same time and day: the one listed later wins; the settings UI warns about the conflict.
- SPEC-SCH-7: Clock changes (time zone, daylight saving, manual) cause rules to be re-evaluated; a rule is never fired twice for the same occurrence.
- SPEC-SCH-8: Rule-triggered applies that include an input change follow SPEC-IN-1 (confirmation dialog).

## 8. Hotkeys
- SPEC-HK-1: Actions: `preset:<name>`, `preset-next`, `preset-prev`, `brightness+/-`, `contrast+/-`, `volume+/-`, `input:<value>`, `power-toggle`, `schedule-toggle`. `schedule-toggle` becomes available with schedules (v2). `power-toggle` turns every target off (soft off, `0xD6`=4, preferred because most monitors keep answering DDC/CI) if any is on, otherwise turns them on; a monitor that stops answering in standby must be woken with its own button.
- SPEC-HK-2: Each action is bound to a user-defined key combination (must include at least one modifier, except function keys). Hotkeys apply to all monitors unless a target monitor is chosen. A combination must include Ctrl, Alt or Win; Shift alone is not enough, because it would take over normal typing. Each combination can be bound once.
- SPEC-HK-3: Step size for `+/-` actions is configurable per control (1-25 %, default 5). Steps start from the pending target, so a held key keeps stepping, and clamp to 0-100 % (SPEC-CTL-5).
- SPEC-HK-4: Holding a hotkey repeats the action; commits follow SPEC-WR-2.
- SPEC-HK-5: If a combination cannot be registered (taken by another app), the user sees which combination and action failed, and the app keeps running.
- SPEC-HK-6: An optional on-screen indicator shows the control name and new value for about 1.5 s (setting, default on). It is not shown for confirmation dialogs. Errors from a hotkey are shown in the indicator as "failed" and in the Settings status line.

## 9. User interface
- SPEC-UI-1: The app runs as a tray icon with no taskbar button and no always-visible main window. Left-click toggles the control panel; right-click opens a menu: Presets, Pause/Resume schedule, Settings, Quit.
- SPEC-UI-8 (minimize to tray): Any app window other than the transient panel (Settings, confirmation dialogs excluded) MUST, when minimized, be hidden entirely: no taskbar button and no minimized stub. The app stays running, and the window is restored by left-clicking the tray icon or choosing Settings from the tray menu, restoring its previous size and position. Closing the Settings window also hides it rather than quitting; only Quit from the tray menu exits the app.
- SPEC-UI-9 (tray icon placement): The tray icon is created in the notification area and, on a fresh install, appears in the hidden-icons overflow (Windows' default for new icons). Which icons show in the main tray or the overflow is controlled by the user in Windows settings, and the app MUST NOT try to override that choice. The app MUST stay fully usable from the overflow, and the paused-schedule state (SPEC-SCH-5) is shown in the icon's tooltip and appearance.
- SPEC-UI-2: The panel shows, per monitor: name/alias, a slider and numeric value for each supported control, the input selector, and power. Below: preset buttons and schedule status (next rule, paused state).
- SPEC-UI-3: The panel closes on loss of focus and with `Esc`. It appears near the tray icon within 150 ms of the click.
- SPEC-UI-4: A "not responding" monitor shows a disabled state with a retry action.
- SPEC-UI-5: The panel follows the system light/dark theme and has a native Windows 11 appearance.
- SPEC-UI-6: Settings include: hotkeys, step sizes, schedule rules, write delay, live preview, input confirmation and revert timer, on-screen indicator, start with Windows, audit log, monitor aliases.
- SPEC-UI-7: The panel is fully keyboard-operable and exposes names/values to screen readers.
- SPEC-UI-10 (v0 Settings): The default Settings window follows the visual design and interaction structure of `mockups/settings.html`, including a Windows 11-style navigation/sidebar, grouped content cards, monitor selection and controls, and safety/write settings. It shows only implemented features: discovered monitor selection and controls, quiet-period write setting, input-change confirmation, and input-revert timer. A Presets page (list with apply/rename/reorder/delete, save current with optional input source, current-match marker) is included from v1; a Hotkeys page (list with registration status and remove, add with a key-recording field, action and target monitor, per-control step sizes, on-screen indicator toggle) is included from v1; the Schedule page is omitted until implemented; unsupported controls are not displayed as available.
- SPEC-UI-11 (Windows presentation fallback): `dispcontrold --native-ui` opens the original compact Win32 layout instead of the default grouped Settings layout. Both presentations use the same application API and behavior.

## 10. Command line
Executable: `dispcontrol`. Global flags: `--json`, `--monitor <id|alias|index>`, `--quiet`.

| Command | Behavior |
|---|---|
| `list` | List monitors with ID, alias, model, supported controls, support level. |
| `get <control>` | Print the current value (reads the monitor). |
| `set <control> <value>` | Commit value (SPEC-WR-7). Accepts `+N`/`-N` for relative numeric changes and `N` absolute. |
| `preset list` | List presets. |
| `preset apply <name>` | Apply preset, print per-entry result. Exit 5 if some entries failed. |
| `preset next` / `preset prev` | Cycle presets. |
| `preset save <name> [monitor] [--include-input]` | Capture current values (upsert). Power is never captured. |
| `preset delete <name>` / `rename <name> <new>` / `move <name> <offset>` | Manage presets. |
| `preset save <name>` | Capture current state as preset. |
| `schedule pause` / `resume` / `status` | Control the schedule. |
| `audit` | Print commit counts (SPEC-WR-10). |

Exit codes: `0` success; `1` general error; `2` invalid usage or value; `3` monitor not found; `4` monitor not responding; `5` partially applied (some preset entries failed); `6` user declined or revert triggered; `7` app not running.
- SPEC-CLI-1: With `--json`, stdout contains a single JSON object (`ok`, `result`, `error`); no other text on stdout.
- SPEC-CLI-2: The CLI is a client of the running app and always delegates to it, so buffering, rate limits and input confirmation are shared (SPEC-WR-6). If the app is not running, the CLI MUST NOT touch any monitor or start the app; it prints an error saying the app is not running and exits with code `7`. With `--json` the error is reported in the JSON object (SPEC-CLI-1).
- SPEC-CLI-3: Typical CLI latency to a monitor change is under 200 ms with the app running, excluding monitor latency.

## 11. Integrations
- SPEC-INT-1 (PowerToys Run): keyword `dc` lists presets (select to apply) and parses `brightness 40`, `contrast 60`, `volume 20`, `input <name>`; results show the current value.
- SPEC-INT-2 (Command Palette): offers the same commands and presets; listing completes in under 300 ms with the app running.
- SPEC-INT-3: Integrations never bypass SPEC-IN-1 or SPEC-WR rules.
- SPEC-INT-4 (optional): link `dispcontrol://preset/<name>` applies a preset.

## 12. Settings and data
- SPEC-DAT-1: All settings, presets, rules and hotkeys live in one human-editable file in the user's profile; a file beside the executable enables portable mode. Hotkeys are stored as `[[hotkeys]]` entries with `keys` (e.g. `"Ctrl+Alt+Up"`), `action` (e.g. `"brightness+"`, `"preset:Night"`, `"input:0x11"`) and an optional `monitor`; step sizes and the indicator setting are top-level keys (`brightness_step`, `contrast_step`, `volume_step`, `show_osd`).
- SPEC-DAT-2: Invalid or unknown fields in the file are reported with line information and never cause data loss; the app starts with defaults for the broken section and keeps a `.bak` copy.
- SPEC-DAT-3: The file is written only when settings change, and atomically.
- SPEC-DAT-4: The file carries a version number; future versions migrate older files.

## 13. Non-functional requirements
- SPEC-NFR-1: Idle CPU 0% (no activity between user/event triggers); idle working set under 30 MB; download under 10 MB; startup to tray under 300 ms.
- SPEC-NFR-2: No network access, telemetry or administrator rights. Single instance; launching a second one forwards the request and exits.
- SPEC-NFR-3: Logging is off by default; when on, logs rotate (max 5 files of 1 MB) and contain no personal data beyond monitor model and serial.
- SPEC-NFR-4: The app MUST recover from monitor, driver and sleep/wake errors without restart.

## 14. Platform scope
- Windows 11 (x64): everything above, delivered in releases: v0 = controls, safety, Settings window (implemented pages only, SPEC-UI), tray, CLI; v1 = presets, hotkeys, built-in display brightness (section 3.1), integrations; v2 = scheduling. Reference test hardware: Lenovo L32p-30 over USB-C on Intel Iris Xe Graphics, and that laptop's built-in display; other monitors work through discovery, other graphics hardware is a future goal.
- The Settings window lists only pages for features present in the installed release; Presets and Hotkeys appear in v1, Schedule in v2. Ubuntu Settings window: future.
- Ubuntu (v3): monitor control uses `ddcutil`. CLI, scheduling and presets (sections 2-7 and 10, 12); tray, hotkeys and on-screen indicator are best effort. Built-in display brightness on Ubuntu is a future feature. Global hotkeys where the desktop allows, otherwise through desktop shortcuts calling the CLI.

## 15. Acceptance criteria (summary)
1. On the reference hardware (L32p-30, USB-C, Intel Iris Xe), every control the monitor reports can be read and set, and numeric values round-trip per SPEC-CTL-3. On a monitor that reports fewer controls, only those are offered and nothing fails.
2. Dragging a slider for 5 s produces exactly 1 commit with live preview off (verified by audit); with it on, at most 4 per second plus the final value.
3. Applying a preset twice produces zero commits the second time.
4. Switching to another input shows the confirmation; declining leaves the input unchanged; accepting then not confirming restores the previous input after the timer.
5. A scheduled rule fires within 1 s; after sleep/wake the missed rule is applied once.
6. Unplugging the monitor does not freeze the app; replugging restores control and presets without reconfiguration.
7. Hotkeys work with the panel closed; registration conflicts are reported.
8. Idle CPU 0% and memory under 30 MB measured after 10 minutes idle.

## 16. Open items
- Handling of multiple identical monitors with no serial number (aliasing and syncing) is out of scope for now; see PRD future features. Until then, such monitors are told apart by connection position, flagged as an unstable ID (SPEC-MON-1).
- Hardware-validation spike results (e.g. latency numbers) will be recorded as verified-monitor notes, not as spec values.
