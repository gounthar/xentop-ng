# Changelog

Notable changes to xentop-ng. Versions follow [Semantic Versioning](https://semver.org/);
release binaries are on the [releases page](https://github.com/olivierlambert/xentop-ng/releases).

## [Unreleased]

### Added

- Downloads for arm64 (`aarch64-linux-gnu`, glibc 2.17 or newer) and
  RISC-V (`riscv64-linux-gnu`, glibc 2.27 or newer) dom0s. They are
  cross-built, and each release runs the test suite and the demo under
  qemu-user; they haven't been tried on a real Arm or RISC-V Xen host yet.

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

[0.3.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/olivierlambert/xentop-ng/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/olivierlambert/xentop-ng/releases/tag/v0.1.0
