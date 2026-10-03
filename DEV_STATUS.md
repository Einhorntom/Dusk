# dispcontrol: Development status

This document tracks implementation progress, validation, and planned development work. Product behavior belongs in [SPEC.md](./SPEC.md); architecture, design decisions, and engineering guidelines belong in [ARCH.md](./ARCH.md).

## Current implementation status

The repository contains a Windows-first Rust workspace following the crate boundaries described in ARCH.md. Implemented features include:

- Windows DDC/CI discovery and monitor-control reads/writes.
- CLI commands: `list`, `get`, `set`, `settings show`, plus `--json`.
- Daemon-only CLI operation over Windows named-pipe IPC.
- TOML settings persistence with Windows file replacement.
- Default grouped Win32 Settings window with monitor/control selection, slider, apply/refresh, quiet-period setting, input-change confirmation, and configurable input-revert timer. The previous compact layout remains available with `dispcontrold --native-ui`.
- Notification-area tray icon; minimizing or closing Settings hides the window, and the tray menu can reopen Settings or quit.
- Confirmation before changing the active monitor input, plus a timed keep/revert prompt.

Both Settings presentations use the same view model and application API. The current grouped default is a functional prototype, not yet the mock-matching UI required for v0. The mock is [mockups/settings.html](./mockups/settings.html); tray behavior and input switching/rollback also need the remaining manual acceptance checks below.

## Known gaps before v0 is complete

- Add interactive Windows acceptance checks for tray hide/restore/quit, Settings persistence, monitor reads, and input confirmation/revert. Do not automate potentially disruptive physical input changes in ordinary CI.
- Input and volume writes have not been verified on the reference monitor; Phase 0 evidence is recorded in [spikes/windows-ddc/results.md](./spikes/windows-ddc/results.md).
- The `--native-ui` argument selects the previous compact layout; invalid daemon arguments are rejected with a usage error.
- The default Settings UI still needs to be implemented to match the Windows 11 visual design and interaction structure of [mockups/settings.html](./mockups/settings.html). The current grouped Win32 window is not sufficient to close v0.

## Next implementation steps

1. Run the interactive Windows v0 acceptance flow on the reference setup, including the input revert path when safe to do so.
2. Implement the mock-matching Windows 11 Settings UI as the default, while showing only v0 features that are implemented.
3. Visually review the mock-matching UI and the compact `--native-ui` fallback at the reference Windows 11 display scale; correct clipping and accessibility issues.

## Release plan and version scope

This section is the roadmap for what each release contains. The implementation checklist and current verification state are tracked above; product behavior details remain in [SPEC.md](./SPEC.md), and the original product rationale is in [PRD.md](./PRD.md).

### Phase 0 — hardware feasibility (complete)

- Verified DDC/CI communication with the reference Lenovo L32p-30 over USB-C on Intel Iris Xe.
- Confirmed brightness read/write and restore. Input and volume behavior still need supervised verification on the physical monitor.
- Evidence and limitations: [spikes/windows-ddc/results.md](./spikes/windows-ddc/results.md).

### v0 — first Windows 11 release (in progress)

- Discover monitors and expose only controls each display reports as supported.
- Read and set monitor controls through the running daemon; provide a CLI with machine-readable output and meaningful exit codes.
- Provide the tray icon and a Settings window that follows the visual design and interaction structure of [mockups/settings.html](./mockups/settings.html), containing only implemented v0 features. Do not show non-functional Presets, Hotkeys, or Schedule pages before their planned releases.
- Include quiet-period/latest-value buffering and write-rate limits to reduce unnecessary monitor writes.
- Include input-change confirmation and a configurable timed keep/revert flow.
- Persist settings; make the mock-matching Settings presentation the default and retain the compact native layout as the opt-in `dispcontrold --native-ui` fallback.
- Release gate: implement and visually review the mock-matching UI, then complete supervised Windows acceptance for Settings persistence, tray behavior, and safe input confirmation/revert on the reference setup. Current test/build/runtime results and outstanding checks are in the sections above.

### v1 — presets, hotkeys, and integrations (planned)

- Save, capture, edit, rename, delete, reorder, and apply named monitor configurations.
- Add configurable global hotkeys for preset selection/cycling and supported control adjustments; report registration conflicts.
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

- Multiple-identical-monitor aliasing and synchronization.
- Broader graphics hardware and monitor-model validation, additional Linux desktops/distributions, macOS, ambient/context-based automation, and laptop internal-panel control.

## Validation record

Latest recorded verification for the Windows GNU target:

- `cargo fmt --all --check`
- `cargo test --workspace --target x86_64-pc-windows-gnu`
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
- `cargo build --workspace --bins --target x86_64-pc-windows-gnu`
- 28 workspace tests pass, including deterministic quiet-period/coalescing/rate-limit/input-revert tests, IPC dispatch success/error and settings persistence, CLI JSON usage cases, and settings-file replacement/backup behavior.
- Runtime smoke checks: CLI reports exit code 7 with a JSON error when the daemon is absent; with the daemon running, monitor listing, brightness read, settings JSON, 20 sequential requests, and 20 concurrent CLI/IPC requests succeeded. The default window opened at 880x720; `--native-ui` opened at 760x640; both served CLI requests. Closing the Settings window hid it without stopping the daemon, and the tray-icon left-click message restored it. An invalid daemon argument exits with code 1.

No physical monitor input switch was performed during these checks. Manual acceptance of tray-menu quit, edited-settings persistence through the window, physical input confirmation/revert, and visual UI/accessibility review remain outstanding; this verification does not certify v0 release readiness.
