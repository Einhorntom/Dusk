# Dusk: Development status

This document tracks implementation progress, validation, and planned development work. Product behavior belongs in [SPEC.md](./SPEC.md); architecture, design decisions, and engineering guidelines belong in [ARCH.md](./ARCH.md).

## Current implementation status

The repository contains a Windows-first Rust workspace following the crate boundaries described in ARCH.md. Implemented features include:

- Windows DDC/CI discovery and monitor-control reads/writes.
- CLI commands: `list`, `get`, `set`, `settings show`, plus `--json`.
- Daemon-only CLI operation over Windows named-pipe IPC.
- TOML settings persistence with Windows file replacement.
- Default mock-style Settings window (Windows 11 sidebar, cards, light/dark following the system theme) with Monitors, Safety & writes, and General pages: monitor selection, supported-control chips, sliders/combos, quiet-period write setting, input-change confirmation, and input-revert timer. The compact layout remains available with `duskd --native-ui`.
- Click a slider's percentage to type an exact value (Enter applies, Esc cancels); typed values use the same buffered write as dragging.
- Input source raw `0x31` is labelled "USB-C" on the reference monitor.
- After a successful enum write (for example, Color preset) the UI keeps the written value and does not read it back immediately, because the monitor is busy and the read fails (DDC/CI error) or returns the old value. "Re-read from monitor" shows the actual state.
- Notification-area tray icon; minimizing or closing Settings hides the window, and the tray menu can reopen Settings or quit.
- Confirmation before changing the active monitor input, plus a timed keep/revert prompt.

Both Settings presentations use the same view model and application API. The default UI follows [mockups/settings.html](./mockups/settings.html) for the implemented v0 features only (no theme selector; theme follows Windows; Presets/Hotkeys/Schedule pages omitted). A screenshot of the default window on the reference monitor was reviewed; tray behavior, settings persistence and input switching/rollback were manually accepted by the owner on the reference setup.

**Renamed to Dusk (2026-10-05):** executables `dusk` (CLI) and `duskd` (daemon), crates `dusk-*`, pipe `dusk-v0` (`DUSK_PIPE`), settings in `%APPDATA%\Dusk\config.toml`, PowerToys Run keyword `dusk`, C# projects `Dusk.*`, Command Palette package `Dusk.CommandPalette` (new COM class ID). On first start `duskd` copies `%APPDATA%\dispcontrol\config.toml` if the new file does not exist (the old file is kept); the install scripts remove the old PowerToys Run plugin folder and Command Palette package.

**Icons (2026-10-05):** "Screen at dusk" (a monitor whose screen shows the sun setting). `assets/icons/make_icons.py` (Python standard library only) generates the multi-size app icon embedded in `duskd.exe` and `dusk.exe`, pixel-snapped tray glyphs for 16-32 px (white for dark taskbars, black for light; the tray icon follows the taskbar theme and updates on `WM_SETTINGCHANGE`), and the PowerToys Run and Command Palette images. A test checks that `duskd.exe` carries the icon resources.

## Known gaps

- Hardware quirks (colour-mode settle time, power-off modes, input `0x31` = USB-C) are still constants in code; a quirk profile (SPEC-QRK) is planned for the second monitor model or the Ubuntu work.

- Input and volume behaviour was verified manually by the owner on the reference monitor (v0 acceptance); automated CI intentionally does not switch physical inputs.
- Light theme, other display scales and a formal keyboard/accessibility review were not exhaustively covered by the v0 acceptance.
- CI (GitHub Actions, MSVC toolchain) runs on every push to https://github.com/Einhorntom/Dusk; local builds use the GNU toolchain (see README). Window procedures and DDC/CI calls remain untested by design (humble objects); `ddc-fake` does not simulate latency or hangs yet.

## Architecture and code review TODO (2026-10-06)

Findings of a full design and code review, most important first. Tick an item when it is fixed and verified.

### P1: before the first release

- [x] **R1. Monitor IDs do not follow SPEC-MON-1.** IDs are `"{Windows description}#{position}"`, never EDID model + serial, and every monitor is flagged unstable. Impact: after a monitor is added, reordered, or re-enumerated on wake, presets and hotkeys can act on the wrong monitor (including switching its input); monitors that Windows calls "Generic PnP Monitor" cannot be told apart; the capability cache (keyed by ID, never cleared) can describe the wrong monitor after a hotplug. Fix: IDs from EDID (manufacturer, product code, serial), position only as a fallback; clear the cache when the display set changes.
- [x] **R2. `duskd` is a console program and can run twice.** No `windows_subsystem`, no single-instance check, and the pipe lacks `FILE_FLAG_FIRST_PIPE_INSTANCE` and `PIPE_REJECT_REMOTE_CLIENTS`. Impact: starting at sign-in opens a console window (closing it kills Dusk); a second copy adds a second tray icon, fails to register hotkeys, and serves the same pipe, so requests go to either copy and each has its own rate limiter, defeating the write protection; another program could create the pipe first. Blocks "Start with Windows".
- [x] **R3. No logging (SPEC-NFR-3).** Warnings use `eprintln!`, including in `app`; once `duskd` has no console they are lost. ARCH says a `log` facade is in place; it is not. Impact: failures on a user's machine cannot be diagnosed.

### P2: correctness and robustness

- [x] **R4. Monitor I/O on the UI thread.** Slider/hotkey commits (`flush_pending_adjustments`, driven by a Win32 timer), Apply, refresh and `matching_preset` (which reads every entry of every preset) run on the UI thread, each call bounded only by the 6 s timeout. Impact: long freezes with a hung monitor. Also, when buffered writes are committed is decided by the UI's timer instead of `app`.
- [x] **R5. Lost updates and fragile settings.** Load-modify-save of presets, hotkeys and settings happens in separate locked steps while the UI and IPC threads run concurrently; the settings file is re-read and parsed on every `set` and `adjust`. Impact: concurrent edits can overwrite each other; one typo in the hand-edited file stops every control. Fix: in-memory state, a transactional `update` port, reload on change keeping the last good version.
- [x] **R6. Undefined behavior in the window procedure.** Each message creates a new `&mut WindowContext`; modal calls (`MessageBoxW`, `TrackPopupMenu`, `SendMessageW`) re-enter, so two live `&mut` exist. About 200 `unsafe` sites with 2 `SAFETY` comments; `modern.rs` is 2,657 lines. Impact: latent crashes in optimized builds; hard to maintain. Fix: `RefCell`-guarded context or deferred work, documented safety, split `modern.rs` by page.
- [x] **R7. Strict, thinly tested capability parser.** An unbalanced capabilities string fails the parse and makes the monitor unusable; 3 tests, all from the L32p-30. Impact: many real monitors would not work. Fix: lenient parsing and a corpus of real capability strings.
- [x] **R8. Timeout retries overlap on one monitor.** After a timeout `TimedBackend` starts a new worker while the stuck call may still run, so two DDC/CI calls can hit the same monitor (ARCH section 7 says this is unsafe); writes are retried although the first may still complete; abandoned threads and the worker map are never cleaned up. Fix: keep the monitor busy until the stuck call returns; do not retry writes.

### P3: maintainability and polish

- [x] **R9. Error types.** Mostly `BackendError::Failed(String)`; "no presets are defined" and "monitor has no supported controls" use `SettingsInvalid` (CLI exit code 2, "invalid usage"); a missing preset entry is `PresetNotFound`.
- [x] **R10. `MonitorService` does too much.** Four APIs in about 900 lines plus about 130 lines of delegation; v2 schedules would grow it. Fix: control, preset and hotkey services sharing one write gate.
- [x] **R11. Redundant work per call.** Each read or write re-enumerates all monitors, opens and closes their handles and queries the display configuration; each slider commit reads before writing (double DDC/CI traffic). Fix: cache the monitor list, refresh it when displays change.
- [x] **R12. C# client has no read timeout.** PowerToys Run or Command Palette hangs while the daemon waits on an input confirmation or a monitor timeout.
- [x] **R13. Doc drift and release basics.** ARCH describes background capability prefetch that does not exist (EDID IDs and logging now match ARCH); `spikes/windows-ddc` is still a workspace member; no workspace lints or release profile; every crate is 0.1.0; the pipe protocol has no version check.
- [x] **R14. Rate-limit deferral message as info.** "monitor write rate limit deferred <control> for <monitor>" is logged as a warning; the owner wants it kept in the log as an info message (the deferral is expected behavior, not a problem).

## Next implementation steps

1. **v1 features are complete and accepted by the owner** (presets, hotkeys, built-in display, PowerToys Run and Command Palette). Built-in display acceptance (2026-10-06): shown as "Built-in display" with brightness only, captured and applied by presets, moved by "All monitors" brightness hotkeys, never deferred, removed cleanly when the lid closes; with only the panel connected `dusk list` shows only "Built-in display" and both displays after reconnecting the L32p-30.
2. **To release v1.0:**
   - Push the review commits and confirm CI passes on the MSVC toolchain (local builds use GNU).
   - ~~Start with Windows~~ done (SPEC-UI-12): Settings > General toggle for the per-user Run entry (`"<duskd.exe>" --background`); the entry is the setting. Owner check pending: turn it on, sign out and in, confirm Dusk starts in the tray without a window.
   - ~~Release workflow~~ done: `.github/workflows/release.yml` runs on a `v*.*.*` tag (checks the tag against the sources, tests, builds, packages with `scripts/package_release.ps1`, publishes the zip and its SHA-256); `scripts/set_version.py` sets the version in all four places. Version set to **1.0.0**. Packaging verified locally (8.9 MB zip; the packaged daemon reports 1.0.0). The Command Palette extension needs the .NET 9 Runtime (checked by its install script); the PowerToys Run plugin uses PowerToys' own.
   - Publish: follow [RELEASING.md](RELEASING.md) (push, wait for CI, then tag `v1.0.0` and push the tag).
   - Reinstall the PowerToys Run and Command Palette integrations on the reference PC to pick up the client timeout and protocol version (R12, R13).
3. **Decision pending (owner):** live preview (SPEC-WR-9) is stored as a setting but has no effect; implement it or remove the setting.

## Release plan and version scope

This section is the roadmap for what each release contains. The implementation checklist and current verification state are tracked above; product behavior details remain in [SPEC.md](./SPEC.md), and the original product rationale is in [PRD.md](./PRD.md).

### Phase 0 — hardware feasibility (complete)

- Verified DDC/CI communication with the reference Lenovo L32p-30 over USB-C on Intel Iris Xe.
- Confirmed brightness read/write and restore. Input and volume behavior still need supervised verification on the physical monitor.
- Evidence and limitations: [spikes/windows-ddc/results.md](./spikes/windows-ddc/results.md).

### v0 — first Windows 11 release (complete)

- Discover monitors and expose only controls each display reports as supported.
- Read and set monitor controls through the running daemon; provide a CLI with machine-readable output and meaningful exit codes.
- Provide the tray icon and a Settings window that follows the visual design and interaction structure of [mockups/settings.html](./mockups/settings.html), containing only implemented v0 features. Do not show non-functional Presets, Hotkeys, or Schedule pages before their planned releases.
- Include quiet-period/latest-value buffering and write-rate limits to reduce unnecessary monitor writes.
- Include input-change confirmation and a configurable timed keep/revert flow.
- Persist settings; make the mock-matching Settings presentation the default and retain the compact native layout as the opt-in `duskd --native-ui` fallback.
- Release gate: implement and visually review the mock-matching UI, then complete supervised Windows acceptance for Settings persistence, tray behavior, and safe input confirmation/revert on the reference setup. Current test/build/runtime results and outstanding checks are in the sections above.

### v1 — presets, hotkeys, and integrations (features complete and accepted; release pending)

**Done:** presets (domain, app, TOML store, IPC `preset_*` ops, CLI `preset list|apply|next|prev|save|delete|rename|move` with exit 5 on partial failure, Presets page in the default UI), per-preset value editor (UI "Edit" page, CLI `preset set|unset`), TOML export/import (CLI `preset export|import|path`, UI buttons using `dusk-presets.toml` next to `config.toml`), plan-then-write apply (one read pass, then back-to-back writes; values after a colour-mode switch are rewritten after one 800 ms settle), and the colour-preset fix (gains ignored under a fixed colour preset, SPEC-PRE-4a). Fmt, clippy and the full test suite pass; the CLI save/apply/delete round trip was smoke-tested on the reference monitor (apply was a no-op, values already matched). Presets were accepted by the owner on the reference setup.

**Done (hotkeys, accepted by the owner on 2026-10-05):** domain key combinations/actions/bindings with text forms, `HotkeyRepository` + `[[hotkeys]]` in `config.toml`, use cases (save/remove/run, buffered steps from the pending target, per-monitor or all-monitor targets, power toggle, preset rename/delete keep hotkeys in sync), step sizes and `show_osd` settings (IPC `settings_set` keeps them when an older client omits them), `preset delete --force` when hotkeys use the preset, `RegisterHotKey` registration with per-binding conflict reporting, `WM_HOTKEY` handling, on-screen indicator, and the Hotkeys Settings page. Not implemented: `schedule-toggle` (v2), hotkey editing from the CLI. **Done (built-in display, accepted by the owner on 2026-10-06):** `ControlCapability::rate_limited` (SPEC-PNL-4), `app::CompositeBackend` (one port over several backends; a failing backend does not hide the others), `panel-windows` (WMI `WmiMonitorBrightness` / `WmiSetBrightness`, one cached connection per worker thread, no monitors and no error on PCs without a panel), daemon wiring `TimedBackend(CompositeBackend[ddc-windows, panel-windows])`, and a simulated built-in display in `--demo`. **Done (integrations):** `integrations/` C# solution: shared `Dusk.Client` (pipe protocol, `dc` parsing, results), PowerToys Run plugin (keyword `dusk`, accepted by the owner), Command Palette extension (search page plus one top-level command per preset; registered from the build folder with Developer Mode; accepted by the owner), install scripts, 19 C# tests including a contract test against the real daemon, and a CI step that builds and tests them. SPEC-INT-4 (`dusk://` links) is not implemented (optional).

- Save, capture, edit, rename, delete, reorder, and apply named monitor configurations.
- Add configurable global hotkeys for preset selection/cycling and supported control adjustments; report registration conflicts.
- Control the laptop's built-in display brightness (WMI) as a monitor in presets, hotkeys, CLI and Settings; no write-rate limit for it.
- Add PowerToys Run and PowerToys Command Palette integrations using the daemon/IPC API.
- Add the corresponding Presets and Hotkeys Settings pages only when those features are implemented.
- Preserve the shared write policy and input-change safety behavior for UI, hotkey, CLI, and integration requests.

### v2 — fixed-time schedules (planned)

- Add schedules that apply presets at configured local times and days of the week.
- Support pause/resume, missed-rule handling after sleep/unlock/reconnect, and visible schedule status.
- Add the Schedule Settings page only when scheduling is implemented.
- Route schedule-triggered changes through the same write limits and input-change confirmation/revert policy.

### v3 — Ubuntu support (planned; Wayland target)

- Add monitor control through `ddcutil`, initially prioritizing the CLI, presets, and scheduling.
- Reuse the domain, application policy, and settings concepts where practical; account for Wayland's limits on global hotkeys and tray behavior.
- Defer the Ubuntu Settings window and full Linux tray UI. Packaging, installer, and distribution documentation are included as the platform is brought to release quality.

### Later / out of current version scope

- Specified but not yet scheduled for a release (SPEC section 14): the tray quick panel (SPEC-UI-1 to UI-5; the tray icon opens Settings instead), live preview (SPEC-WR-9), the write audit and `dusk audit` (SPEC-WR-10), monitor aliases and index addressing with `--monitor` and `--quiet` (SPEC-MON-2, CLI), and `dusk://preset/<name>` links (SPEC-INT-4).
- Multiple-identical-monitor aliasing and synchronization.
- Broader graphics hardware and monitor-model validation, additional Linux desktops/distributions, macOS, ambient/context-based automation, and built-in display brightness on Ubuntu.

## Validation record

Latest recorded verification for the Windows GNU target:

- `cargo fmt --all --check`
- `cargo test --workspace --target x86_64-pc-windows-gnu`
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
- `cargo build --workspace --bins --target x86_64-pc-windows-gnu`
- `python scripts/check_layers.py` (dependency rule, ARCH.md 5.3)
- Opt-in: `cargo test -p dusk-ddc-windows -- --ignored` (backend contract against the real monitor, read-only; passes on the reference L32p-30) and `cargo test -p dusk-ui-win32 -- --ignored` (real `RegisterHotKey` conflict check; passes on the reference desktop).
- 154 workspace tests pass (plus the opt-in tests above and `cargo test -p dusk-panel-windows -- --ignored`: built-in display contract, read-only, and a brightness round trip that changes the panel by 1 % for 1.5 s and restores it; both pass on the reference laptop).
- Earlier: 145 workspace tests pass (plus the 2 opt-in tests above). The suite was reviewed for Clean Architecture and the test pyramid on 2026-10-05: write policy moved into `domain` as pure state machines with unit tests, `TimedBackend` added for SPEC-MON-4, the `Api` split into `ControlApi`/`SettingsApi`/`PresetApi`/`HotkeyApi`, settings validation unified in `AppSettings::validate`, the input prompter moved into `ui-win32`, hotkey form logic moved into `ui-model`, a duplicated cross-layer assertion removed, a shared backend contract and a daemon smoke test added. Earlier: 131 (including hotkey domain parsing, use cases over `ddc-fake`, TOML round trips, view-model and indicator text, virtual-key mapping, and a real `RegisterHotKey` conflict check on message-only windows). Use-case tests (`crates/app/tests/`: write policy, input safety, presets incl. import/export, multi-monitor and failure cases) run on the shared `ddc-fake` crate; `ui-model` covers labels, value snapping, messages and view-model behaviour; the CLI is tested end to end against the daemon dispatcher (exit codes 2/3/5/7, export/import files); `ipc` includes real named-pipe round trips (max-size message, 8 concurrent clients, missing server); `ui-win32` checks that every control ID decodes back to its own control with no collisions.
- Runtime smoke checks: CLI reports exit code 7 with a JSON error when the daemon is absent; with the daemon running, monitor listing, brightness read, settings JSON, 20 sequential requests, and 20 concurrent CLI/IPC requests succeeded. The default window opened at 880x720; `--native-ui` opened at 760x640; both served CLI requests. Closing the Settings window hid it without stopping the daemon, and the tray-icon left-click message restored it. An invalid daemon argument exits with code 1.

No physical monitor input switch was performed during these checks. Manual acceptance of tray behavior (including Quit), settings persistence, input confirmation/revert and the colour-preset fix was completed by the owner on the reference setup; v0 is accepted.
