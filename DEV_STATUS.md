# dispcontrol: Development status

This document tracks implementation progress, validation, and planned development work. Product behavior belongs in [SPEC.md](./SPEC.md); architecture, design decisions, and engineering guidelines belong in [ARCH.md](./ARCH.md).

## Current implementation status

The repository contains a Windows-first Rust workspace following the crate boundaries described in ARCH.md. Implemented features include:

- Windows DDC/CI discovery and monitor-control reads/writes.
- CLI commands: `list`, `get`, `set`, `settings show`, plus `--json`.
- Daemon-only CLI operation over Windows named-pipe IPC.
- TOML settings persistence with Windows file replacement.
- Native Win32 Settings window with monitor/control selection, slider, apply/refresh, quiet-period setting, input-change confirmation, and configurable input-revert timer.
- Notification-area tray icon; minimizing or closing Settings hides the window, and the tray menu can reopen Settings or quit.
- Confirmation before changing the active monitor input, plus a timed keep/revert prompt.

The current Settings UI is functional but is not yet the planned polished Windows 11 presentation. Implemented behavior and UI have not completed manual acceptance checks for tray behavior or input switching and rollback.

## Known gaps before v0 is complete

- Slider quiet-period buffering currently lives in the UI; it is not a shared application-level write queue.
- Rate-limited explicit writes wait synchronously. They do not yet use a shared latest-wins queue, so callers may block under sustained rate limits.
- Add deterministic application tests for write buffering/coalescing, rate limits, and input keep/revert; add IPC tests that exercise actual dispatch paths and error responses.
- Complete Windows acceptance checks for tray hide/restore/quit, settings persistence, monitor reads, input confirmation/revert, and CLI behavior. Keep risky physical monitor writes out of ordinary CI.
- Input and volume writes have not been verified on the reference monitor; Phase 0 evidence is recorded in [spikes/windows-ddc/results.md](./spikes/windows-ddc/results.md).

## Next implementation steps

1. Move debounce/buffering and rate-limit scheduling behind the application boundary so Settings and CLI share the same latest-wins policy.
2. Use the injected `Clock` port to test quiet-period commits, coalescing, per-control rate limits, and unchanged-value skips deterministically.
3. Add focused application and IPC contract coverage for timed input keep/revert and daemon errors.
4. Run the Windows v0 acceptance flow on the reference setup.
5. Polish the Windows UI while preserving the existing native Win32 window as a fallback. When the polished UI is introduced, it should be the default; `dispcontrold --native-ui` should opt into the native UI. Both presentations must use the same application API.

## Release sequence

- **v0 (first release):** Windows monitor controls, safety options, Settings window, tray, and CLI.
- **v1:** Presets, hotkeys, PowerToys Run and Command Palette integrations.
- **v2:** Fixed-time schedules.
- **Later:** Ubuntu support through `ddcutil`, including Linux-specific UI and packaging work.

Ubuntu Settings UI is deferred. Multiple-identical-monitor aliasing and synchronization are future features.

## Validation record

Latest recorded verification for the Windows GNU target:

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace --bins`
- Runtime smoke checks: CLI reports exit code 7 when the daemon is absent; with the daemon running, monitor listing, brightness read, settings JSON, and 20 sequential IPC reconnects succeeded.

No physical monitor input switch was performed during these checks.
