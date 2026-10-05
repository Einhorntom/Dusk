# Windows DDC/CI Phase 0 spike

This prototype checks whether Windows' monitor-configuration APIs can discover
and control a monitor over DDC/CI on the reference PC.

## Run

From the repository root, using the GNU toolchain installed for this probe:

```powershell
cargo +stable-x86_64-pc-windows-gnu run -p dusk-windows-ddc-spike --target x86_64-pc-windows-gnu -- list
cargo +stable-x86_64-pc-windows-gnu run -p dusk-windows-ddc-spike --target x86_64-pc-windows-gnu -- read 0
cargo +stable-x86_64-pc-windows-gnu run -p dusk-windows-ddc-spike --target x86_64-pc-windows-gnu -- test-write 0 brightness 99
```

This setup also needs MinGW on `PATH`. With the MSVC linker installed, use
`cargo run -p dusk-windows-ddc-spike -- ...` instead.

`list` enumerates physical displays and prints each raw MCCS capabilities
string. `read` reads brightness (VCP `0x10`), input (VCP `0x60`) and volume
(VCP `0x62`). A capability-string failure or an unsupported VCP control is
reported per monitor/control without aborting the remaining reads.

`test-write` accepts a VCP numeric value for `brightness`, `input`, or
`volume`. It displays the current value, requires typing `APPLY`, writes the
requested value, waits five seconds, and then restores the original value.
Input changes can disconnect USB devices attached to the monitor or leave the
display without a signal. Keep a keyboard available independently of the
monitor hub. Do not terminate the process during the test; restoration is
performed by this process and is not crash-safe.

Monitor indexes are temporary enumeration order, not stable identities.
Connect the Lenovo L32p-30 directly over USB-C, enable DDC/CI in its on-screen
display if necessary, and record the output and observed latency. This spike
does not implement production retry, buffering, or revert guarantees.
