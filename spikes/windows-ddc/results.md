# Phase 0 Windows DDC/CI read probe

## Observation

On 2026-10-03, the Windows monitor-configuration API discovered the reference
Lenovo L32p-30 as physical monitor `0`. Capability discovery and read-only
access succeeded over the current USB-C connection on Intel Iris Xe.

One sample run reported:

| Operation | Result | Elapsed |
|---|---:|---:|
| MCCS capabilities string | MCCS 2.2 | 2811 ms |
| Brightness (VCP `0x10`) | `100 / 100` | 73 ms |
| Input (VCP `0x60`) | `49 / 49` (`0x31`, USB-C input) | 75 ms |
| Volume (VCP `0x62`) | `100 / 100` | 59 ms |

These are single-run observations, not performance guarantees. The slow
capability query supports caching it outside the UI thread.

## Capability string

```text
(prot(monitor)type(LCD)model(Lenovo L32p-30)cmds(01 02 03 07 0C E3 F3)vcp(02 04 05 08 10 12 14(01 05 06 08 0B 0E 0F) 16 18 1A 52 60(11 0F 31) 86(02 05) AC AE B2 B6 C6(02 00 EF 20 A0 A6 00 68 00 C8 00 00 48 00 C6 00 00) C8 C9(4C) CA CC(02 03 04 05 06 09 0A 0D) D6(01 04 05) DC(00 01 03 05 06) DF E0(00 03 04 05 06) EA(00 01) EB(00 01) EC(00 01 FF 00 02 03) EF(00 06 07 08 09) F6(07) F7(01 02 04 0C 0D) F8(01 02 04 0C 0D) F9(00 01 02 03 04 05 06) FA(10 12 14 86 DC E0 EA EC(00 02) EF) FD)mswhql(1)asset_eep(40)mccs_ver(2.2))
```

## Reversible write test

A confirmed brightness test changed `100` to `99`. `SetVCPFeature` returned
success in 65 ms; after five seconds the spike restored `100` in 65 ms. A
follow-up read reported brightness `100`, confirming restoration. This
validates the write/restore path for brightness only; writes to input and
volume have not been tested. Input tests can temporarily disconnect the
monitor USB hub or lose picture; keep independent input devices connected.
