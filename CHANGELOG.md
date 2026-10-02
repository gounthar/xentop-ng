# Changelog

Notable changes to xentop-ng. Versions follow [Semantic Versioning](https://semver.org/);
release binaries are on the [releases page](https://github.com/olivierlambert/xentop-ng/releases).

## [Unreleased]

### Security

- Harden both libxenstat tapdisk readers against symlinks, unsafe ownership
  and permissions, hard links, special files, invalid PIDs and malformed records.
  The bundled patches must be applied to protect libxenstat itself.

### Fixed

- Keep the terminal responsive while Xen collection is blocked. Preserve
  initialization errors, exit cleanly on failure, and mark stale samples.
- Establish initial rates with two quick samples; slow samples no longer
  skip the next collection deadline. Sleep longer when collection is idle.
- Reset VM and disk baselines on identity changes or counter resets, and
  discard disk intervals spanning failed reads. Preserve known VM UUIDs
  across transient xenstore read failures without requiring XAPI.
- Preserve partial disk sums in host, VM and SR graphs. New disks wait for
  a baseline without marking coverage degraded; missing measurements show
  gaps rather than zero. Highlight disk collection failures separately
  from disk-reported I/O errors.
- Keep physical CPU history attached to its CPU ID across hotplug; newly
  observed or reset counters remain unknown until a valid interval exists.

### Changed

- JSON consumers: `host.pcpu_busy` entries can now be `null`, and source
  availability has a new `partial` value. Coverage objects report `available`,
  `total` and `pending`; `disk_samples` and `pcpu_samples` describe valid
  intervals. Disk rates expose `stats_valid`, `collection_error` and
  `warming_up`; domain rates expose `baseline_reset`.

## [0.4.1] - 2026-10-02

### Fixed

- The SR view (`v`) lists the SRs plugged into the host even when no VM
  uses them, instead of "no storage mapping (xenstore)" on a host with no
  running VM ([#6](https://github.com/olivierlambert/xentop-ng/issues/6)).
  They come from xapi; DVD drives, removable media and ISO libraries are
  left out. Without xapi, an empty view now says no VM disk is active,
  and "no storage mapping" only shows when disks can't be mapped.

### Security

- `--lib PATH` as root loaded the path as given after checking the file it
  resolved to. If the path went through a symlink in a directory its owner
  controls, the symlink could be swapped in between and another library
  loaded as root (relevant when xentop-ng is granted through sudo). It now
  loads the file it checked.

### Internal

- Code reorganised with no change in behavior: the UI is split into one
  file per panel, the Xen libraries are loaded in one place, and reading
  libxenstat is separate from filling its gaps (now unit-tested).

## [0.4.0] - 2026-10-02

### Added

- Downloads for arm64 (`aarch64-linux-gnu`, glibc 2.17 or newer) and
  RISC-V (`riscv64-linux-gnu`, glibc 2.27 or newer) dom0s. They are
  cross-built, and each release runs the test suite and the demo under
  qemu-user; they haven't been tried on a real Arm or RISC-V Xen host yet:
  reports are welcome, see [Architectures](https://github.com/olivierlambert/xentop-ng/blob/v0.4.0/README.md#architectures).

## [0.3.2] - 2026-10-02

### Fixed

- Domain names are no longer cut while there is room to show them:
  - the mem box cut them at 12 characters; names now get as much room as
    the longest one needs, up to half the row, so a wide mem box (cpu box
    hidden with `1`) shows them in full;
  - the same in the cpu box's "top domains" list (stock libxenstat),
    which cut them at 9;
  - the domain table's NAME column grew by at most 12 columns; it now
    grows to the longest name, leaving the CPU history at least half of
    the spare width. Long names never push other columns out;
  - SR names in the SR view (`v`) use the width left over past 20
    columns;
  - these widths grow at once but shrink only after 30 s, so short-lived
    VMs (CI jobs, backups) don't shift the columns as they come and go.

## [0.3.1] - 2026-10-01

### Changed

- Screenshots of the 0.3.0 features in the README (SR view, domain
  details), and a refreshed tour GIF.
- Release notes on GitHub now include the release's CHANGELOG entry.

## [0.3.0] - 2026-10-01

Screenshots: [SR view](https://github.com/olivierlambert/xentop-ng/blob/v0.3.1/docs/sr-view.png) and
[domain details](https://github.com/olivierlambert/xentop-ng/blob/v0.3.1/docs/detail.png).

### Added

- **Names from xapi** on XCP-ng/XenServer hosts: SR and VDI name-labels, the
  exact SR type (`lvmoiscsi` rather than `lvm`) and the network behind each
  VIF, in the SR view, the domain details and `--batch` (`sr_name`,
  `vdi_name`, `network`; `sources.xapi`). xentop-ng uses xapi's local socket
  and makes only read-only calls, from a background thread, so xapi never
  holds up the display. On plain Xen, where there is no xapi, nothing
  changes. `--no-xapi` turns it off. The `i` panel shows the xapi state.
- **SR view trend column:** an IOPS sparkline per SR and per busy disk, each
  bar coloured by the latency at that moment.

### Changed

- **SR view rows stay put:** SRs and busiest disks are ranked by IOPS
  averaged over about 10 s and only swap places on a clear change (20%
  and at least 2 IOPS), instead of re-sorting on every sample.

## [0.2.2] - 2026-10-01

### Changed

- Disks are named as the guest sees them with PV drivers (`xvda`…), not by
  their emulated IDE number (`hda`…). `--batch` keeps the raw number in
  `dev`.

## [0.2.1] - 2026-10-01

### Fixed

- The SR view (`v`) is reachable on short terminals with domain details
  open, and listed in the footer hints.

## [0.2.0] - 2026-10-01

### Added

- Steal time: STEAL column, per-vCPU steal (with the hypervisor patch), and
  a per-domain fallback on XCP-ng/XenServer hypervisors.
- Storage awareness: VBD → SR/VDI (or backing path), VM UUIDs and balloon
  targets from xenstore, and the per-SR view (`v`).
- xentop drop-in mode (`--xentop`, or run as `xentop`), including
  `xentop -b` output.
- Preferences file, column chooser (`o`), sort by any column, memory history
  in the domain details.
- Accessibility: `NO_COLOR`, `--colors mono`, a colorblind theme.

## [0.1.2] - 2026-10-01

### Added

- Key `5` / `--domains-only` shows just the domain list.

### Fixed

- The latency graph fills the disk box's side panel.

## [0.1.1] - 2026-10-01

First public release: same as 0.1.0, with build provenance attestations.

## [0.1.0] - 2026-10-01

Initial release: a btop-style Xen monitor with fallbacks for what stock
libxenstat lacks, security hardening, CI and signed release builds.

[0.4.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/olivierlambert/xentop-ng/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/olivierlambert/xentop-ng/releases/tag/v0.1.0
