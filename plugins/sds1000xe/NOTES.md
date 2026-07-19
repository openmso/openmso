# SDS1104X-E behavioral notes (live characterization, 2026-07-19)

Unit: SDS1104X-E, serial SDSMMGKC6R0663, firmware 8.3.6.1.37R8.

## Transport ranking (measured)

| Transport | Verdict | Throughput (14 MB block) |
|---|---|---|
| **VXI-11** (portmap :111 → core :954) | **Preferred.** Works reliably after a clean boot; the path vendor software uses. | 6.46 MB/s |
| **Raw TCP :5025** | Works, same speed, but the daemon is fragile (see crash notes). | 6.66 MB/s |
| **USBTMC** (kernel driver) | **Unusable for waveforms** on this scope — see below. | n/a |

Waveform reads: a **full 14 Mpt (14 MB) `WF? DAT2` block works in one
transaction** on both network transports. `WFSU NP/FP` paging also works
(verified: 10 × 1.4 Mpt pages reassemble with zero discontinuity — 1 kHz cal
period measured 0.999997 ms ± 7.3 ns across all page boundaries) but costs
~35% throughput (4.3 vs 6.5 MB/s), so the plugin reads the whole depth in one
block and keeps paging as a fallback for deeper-memory models.

## The network daemon can crash — and hard-lock the scope

Observed twice, once escalating to a **full front-panel lock requiring
power-off**:

- Abandoning a TCP connection with unread response data can kill the
  instrument-network daemon: 5024/5025 refuse, portmapper may half-survive
  (GETPORT answers, core channel refuses). Web server stays up. Only a
  reboot recovers.
- Connecting during the boot window (before the scope UI is fully up),
  aggressive retry loops, and bare TCP connect/close port probes are all
  suspected contributors to the hard lock.

Rules encoded in the plugin / to keep:
- One long-lived session per capture run; always drain replies fully.
- Never bare connect/close "port probes"; a session is open → `*IDN?` →
  drain → clean close.
- After power-on, leave the scope alone until fully booted; space
  reconnection attempts ≥ 10 s.
- Post-crash (and post-recovery-reboot), **channel/probe settings can be
  mangled** (observed: all traces hidden, probe factors changed to 0.1x).
  Frontends should explicitly configure everything they rely on rather than
  trusting device state.
- VXI-11 "broken" symptoms (registered in portmap, connection refused) are
  crash fallout, not a firmware property — after a clean power cycle VXI-11
  works fine.

## USBTMC: full-speed USB + kernel-driver incompatibility

- Interface enumerates as **USB 1.1 full speed** (64-byte max packet), so
  even a working stack would manage ≲1 MB/s — strictly worse than Ethernet.
- Via the Linux kernel `usbtmc` driver, any reply longer than **52 bytes**
  (64 − 12-byte TMC header) truncates at the first bulk packet; the rest
  never arrives, and the undrained remainder **wedges the interface**:
  subsequent opens see 0-byte reads or ETIMEDOUT, and Device Clear ioctl
  returns EPIPE. Recovery requires a USB device reset (USBDEVFS_RESET) +
  reopen — the plugin's `UsbTmcScpi` does this automatically once per
  session. Short control queries (< 52 byte replies) work.
- libsigrok dodges this by speaking TMC framing itself over libusb
  (`src/scpi/scpi_usbtmc_libusb.c`); a future native plugin should do the
  same if USB support matters. For Python today: use the network.
- udev rule needed for access (`99-openmso-usbtmc.rules`); it applies on
  (re)plug or `udevadm trigger /dev/usbtmc0`, not retroactively.

## Waveform readout

- `C<n>:WF? DAT2` returns `C<n>:WF DAT2,#9<9-digit len><int8 codes>\n\n`
  (with `CHDR OFF` the prefix shrinks but a `DAT2,` fragment remains —
  always parse from the `#`).
- Volts = code × (vdiv/25) − offset; codes are int8.
- `D0:WF? DAT2` with no LA probe attached returns an empty block
  (`#9000000000`) rather than an error. Digital path implemented per the
  programming guide but never exercised on hardware (no SLA1016 available).

## Acquisition behavior

- `SAST?` values seen: `Trig'd`, `Stop`, `Ready`, `Auto`, `Armed`.
- `TRMD SINGLE` arms one acquisition; poll `SAST?` for `Stop`. (`INR?` bit0
  also signals completion but reading clears the register.)
- Sample 0 is at −(tdiv × 14/2) relative to the trigger point (14
  horizontal divisions); verified: trigger rising edge lands exactly at
  t = 0 in captures.
- **Interleaving** (verified live): C1+C2 or C3+C4 active together → each
  ADC pair shares: 500 MSa/s and half the memory per channel. C1+C3 (or any
  cross-pair combo) keeps 1 GSa/s / full depth per channel. All four: 500
  MSa/s, quarter depth (3.5 Mpt at MSIZ 14M).
- `SARA?`/`SANU?` report the values of the acquisition **in memory**; while
  stopped, config changes (e.g. `MSIZ`) don't reflect until the next real
  acquisition. Query after the capture completes, not after config.set (the
  plugin does this).
- Cal-signal validation: single-shot capture measured 1000.00 Hz, 50.0%
  duty, ~3.1 Vpp on C1 and C3 — matches the 1 kHz / 3 Vpp cal output.
