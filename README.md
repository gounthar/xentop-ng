# xentop-ng

[![CI](https://github.com/olivierlambert/xentop-ng/actions/workflows/ci.yml/badge.svg)](https://github.com/olivierlambert/xentop-ng/actions/workflows/ci.yml)

A modern, btop-inspired resource monitor for the Xen hypervisor.

![xentop-ng tour: live graphs, domain details, themes, domains-only view](docs/xentop-ng.gif)

<sub>Simulated 128-pCPU, 1 TiB host (`--demo-cpus 128 --demo-mem 1T`).
[Full-size screenshot](docs/xentop-ng.png).</sub>

`xentop` shows cumulative counters in a fixed ncurses table. xentop-ng shows
what is happening **now**: rates per interval, history graphs, per-pCPU load,
and disk latency. Point it at a domain and it breaks usage down per vCPU,
per disk and per network interface.

> **Status: demonstrator.** It runs on real XCP-ng 8.3 hosts. The interface
> and the libxenstat additions may still change before anything goes
> upstream.

## Features

- **CPU**: host load history. Per-pCPU mini graphs, switching to a heatmap
  on large hosts (128, 256, 1024+ pCPUs) with the hottest cores called out.
- **Memory**: host usage, plus the biggest domains.
- **Network and disk**: throughput graphs for traffic to/from VMs and for
  reads/writes, with IOPS and peaks.
- **Disk latency**: read/write service time from tapdisk3, with history.
- **Domain list**: filterable, with CPU meter, CPU history, memory,
  network, disk throughput, IOPS and latency. Sort by any column (click its
  title, or `s`). Pick and reorder columns with `o`; when the terminal is
  too narrow, the least important ones step aside and the title says how
  many.
- **Domain details** (`⏎`): per-vCPU load, memory history, per-disk
  IOPS/throughput/latency, per-vif traffic, packets and errors.
- **Themes**: `btop`, `xcp-ng`, `dracula`, `gruvbox`, and `colorblind`.
  `NO_COLOR` is honoured (see [Accessibility](#accessibility)).
- **Remembers your setup**: theme, boxes, sort, columns and refresh rate
  are kept in a [config file](#configuration).
- **Domains only**: `5` (or `--domains-only`) hides every other box; `5`
  again brings them back.
- **Mouse**: click to select, double-click for details, wheel to scroll.
- **Batch mode**: one JSON object per interval, for scripts and benchmarks.
- **Demo mode**: a simulated host of any size, no Xen needed.

![Domain details, xcp-ng theme](docs/detail.png)

## Try it: demo mode

No hypervisor needed:

```sh
cargo run --release -- --demo
```

The simulated host has a realistic fleet: web frontends, Postgres
primaries/replicas, Kubernetes workers, a domain controller, a backup proxy.
CI jobs come and go. Graphs start with ten minutes of history already filled
in. The fleet scales with the host you ask for:

| Option | Meaning | Default |
|---|---|---|
| `--demo-cpus N` | physical CPUs | 16 |
| `--demo-mem SIZE` | RAM (`512G`, `1T`, `1.5T`…) | 128G |
| `--demo-load PCT` | average host CPU load to aim for | 55 |
| `--demo-mem-use PCT` | share of RAM assigned to VMs | 80 |
| `--demo-stock` | behave like a stock libxenstat with no fallbacks | off |

Any `--demo-*` option implies `--demo`.

- **VM counts** follow the pCPU count.
- **VM sizes** follow the RAM.
- **Load** is calibrated at startup so the host averages the requested
  figure.
- **Big hosts** get extra workloads: ML training VMs from 64 pCPUs, and a
  256 GiB database VM per TiB of RAM from 512 GiB up.

```sh
xentop-ng --demo-cpus 128 --demo-mem 1T                  # the tour above
xentop-ng --demo-cpus 256 --demo-mem 2T --demo-load 85   # a busy large host
xentop-ng --demo-cpus 4 --demo-mem 16G                   # a small lab box
xentop-ng --demo-cpus 1024 --demo-mem 8T                 # stress the heatmap
```

## Install

Download from [Releases](https://github.com/olivierlambert/xentop-ng/releases):

- **XCP-ng 8.3:** `xentop-ng-<version>-xcp-ng-8.3.tar.gz`
  ```sh
  grep xcp-ng-8.3 SHA256SUMS | sha256sum -c -   # optional
  tar xzf xentop-ng-*-xcp-ng-8.3.tar.gz && cd xentop-ng-*-xcp-ng-8.3
  ./install.sh                                  # as root; installs to /opt/xentop-ng only
  /opt/xentop-ng/bin/xtop
  ```
  The bundle includes our patched libxenstat. Only the `xtop` launcher uses
  it; the system `xentop` and libxenstat stay untouched.
- **Any other x86_64 Linux dom0** (glibc 2.17 or newer):
  `xentop-ng-<version>-x86_64-linux-gnu.tar.gz` contains the binary. It uses
  the system libxenstat, with built-in fallbacks for the gaps (below).

Release archives come with `SHA256SUMS` and GitHub build provenance
attestations (`gh attestation verify <file> --repo olivierlambert/xentop-ng`).

## Running on a Xen host

xentop-ng runs in dom0 as root, like `xentop`. It loads **libxenstat at
runtime** rather than linking it, so one binary works with any Xen release.

It prefers a library found through `LD_LIBRARY_PATH`, so a patched copy can
sit next to the system one without replacing it. `--lib PATH` forces a
specific library. When running as root, that file and every directory above
it must be root-owned and not group/world-writable.

### Stock libxenstat, fallbacks, and our patches

Until [our libxenstat patches](libxenstat/) are upstream, xentop-ng collects
whatever the loaded libxenstat lacks by itself:

| Data | stock libxenstat | xentop-ng fallback | with our patches |
|---|---|---|---|
| Domains, vCPUs, memory, disk and network throughput/IOPS | ✓ | | ✓ |
| Per-pCPU load and heatmap | – | ✓ via libxenctrl `xc_getcpuinfo()` | ✓ |
| Disk latency (tapdisk3 VBDs) | – | ✓ reads tapdisk3's stats in `/dev/shm` | ✓ |
| Network on Open vSwitch hosts (XCP-ng default) | ✗ every VIF lost ([bug](libxenstat/README.md#0002-vifs-missing-on-open-vswitch-hosts)) | ✓ from `/proc/net/dev` | ✓ |

When something is filled in by a fallback or missing altogether, the header
shows a discreet **◐** marker. Press **`i`** for the data sources panel,
which says where each metric comes from. `--batch` output includes the same
information under `"sources"`.

Missing values show as `-`, never a made-up number. blkback and qdisk disks
have no latency counters at all. Without per-pCPU data, host CPU is
estimated from domain CPU time and labelled `est.`.

### Building for XCP-ng 8.3 yourself

The scripts in [`build/`](build/) do everything inside the
`ghcr.io/xcp-ng/xcp-ng-build-env:8.3` container, pinned by digest, so the
output runs on the host's glibc 2.17. Every input is pinned: the Xen tag
(checked against its commit), the XCP-ng patch queue commit, the Rust
toolchain (`rust-toolchain.toml`) and rustup-init (by checksum).

```sh
build/build-libxenstat.sh     # patched libxenstat.so.4.17 (XCP-ng 4.17.6 + our patches)
build/build-xentop-ng.sh      # xentop-ng binary
build/deploy.sh HOST          # installs to /opt/xentop-ng on HOST over ssh, nothing else touched
dist/package.sh v0.1.0        # or: release archives in build/out/release/
```

### Security notes

- xentop-ng only **reads** statistics. It doesn't start, stop or change
  domains.
- **VM names** can be set by toolstack users who are less privileged than
  dom0 root. They are sanitised: control characters, bidi overrides and
  invisible characters are replaced. Wide characters are laid out by display
  width, and the side lists show domain IDs, so a VM named "Domain-0" can't
  pass for the real one.
- **Fallback files:** stats files in world-writable `/dev/shm` are only
  trusted if they and their directory are root-owned, opened without
  following symlinks, and belong to a live tapdisk.
- **Config file as root:** only read or written if root owns it (and its
  directory) and nobody else can write to it; symlinks aren't followed.
  This matters if `sudo` keeps another user's `$HOME`.
- **sudo:** don't grant xentop-ng to other users through `sudo`. If you do
  anyway, `--lib` only accepts root-owned files in root-owned directories.

Please report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## Keys

`?` shows all of them, grouped like this.

| Key | Action |
|---|---|
| **Navigate** | |
| `↑` `↓` / `j` `k`, wheel | select domain |
| `PgUp` `PgDn`, `g` `G` | page / first / last |
| `⏎`, `space`, double-click | domain details |
| `esc` | close details / clear filter |
| **Sort and filter** | |
| `s` `S` / `←` `→` | next / previous sort column (among those on screen) |
| click a column title | sort by it; click again to reverse |
| `c` `m` `n` `d` `l` | sort by cpu, memory, network, disk, latency |
| `r` | reverse sort |
| `0` | pin Domain-0 on top |
| `/` or `f` | filter by name or id |
| **View** | |
| `1` `2` `3` `4` | toggle cpu / mem / net / disk boxes |
| `5` | domains only; press again to bring the boxes back (`--domains-only` starts that way) |
| `o` | column chooser: `space` show/hide, `J` `K` (or `⇧↑` `⇧↓`) move, `d` defaults; mouse works too |
| `t` `T` | next / previous theme |
| `i` | data sources: what libxenstat provides, what comes from fallbacks |
| **Sampling and settings** | |
| `+` `-` | slower / faster refresh |
| `p` | pause |
| `W` | save settings now (they are also saved on quit, see below) |
| `?`, `h`, `F1` | help (`↑` `↓` scroll it on small terminals) |
| `q`, `ctrl-c` | quit |

The bottom line of the domain list shows the most useful of these, as many
as fit, and always `? help`. Short messages (theme changed, settings saved,
a problem with the config file) appear for a few seconds in the top right.

## Configuration

Preferences live in `$XDG_CONFIG_HOME/xentop-ng/config.toml`, which is
usually `~/.config/xentop-ng/config.toml` (`/root/.config/...` in dom0).

- **Saved on quit**, but only what you changed in the UI. Command-line
  flags such as `--theme` apply to that run only. `W` saves everything as
  it is right now, flags included.
- **Loaded at start**, then command-line flags override it.
- **Forgiving**: a missing file means defaults; a bad value falls back to
  its default with a warning in the header; unknown keys are ignored; a
  malformed file never stops xentop-ng from starting. Delete the file to
  reset everything.
- Written atomically (temporary file, then rename), mode `0600`.
- `--config PATH` uses another file; `--no-config` neither reads nor
  writes one. Batch mode ignores the file.

```toml
theme = "colorblind"         # btop, xcp-ng, dracula, gruvbox, colorblind
colors = "auto"              # auto, truecolor, 256, mono
interval = 2.0               # seconds
boxes = ["cpu", "disk"]      # cpu, mem, net, disk; [] = domains only
sort = "iops"                # a column id, or net / disk (totals)
reverse = false
dom0_first = true
# Display order; columns that aren't listed keep their default place.
columns = ["id", "name", "state", "cpu", "cpu_hist", "mem", "iops", "lat",
           "disk_rd", "disk_wr", "net_rx", "net_tx", "vcpu"]
hidden_columns = ["vcpu"]
```

Column ids: `id`, `name`, `state`, `vcpu`, `cpu`, `cpu_hist`, `mem`,
`net_rx`, `net_tx`, `disk_rd`, `disk_wr`, `iops`, `lat`. `id` and `name`
are always shown.

## Accessibility

- **`NO_COLOR`** (any non-empty value, see [no-color.org](https://no-color.org))
  or `--colors mono`: no colours at all. Meters keep their shape (`■■■···`),
  the pCPU heatmap uses shades (`·░▒▓█`), the selected row and badges are
  in reverse video, secondary text is dim. An explicit `--colors` or a
  `colors` setting in the config file wins over `NO_COLOR`.
- **`colorblind` theme**: blue → yellow → orange ramps (Okabe-Ito colours)
  instead of green → red.
- **Not colour alone**: domain state has a symbol (`●` running, `○` idle,
  `‖` paused, `✖` crashed), and latency of 5 ms or more is flagged with `!`
  in the list and the domain details.

## Other options

- `--colors 256`: for terminals without 24-bit colour. This is the default
  on the Linux console. `--colors mono`: no colour (as with `NO_COLOR`).
- `--theme NAME`: start with a given theme.
- `-d SECS`: refresh interval.
- `--config PATH`, `--no-config`: see [Configuration](#configuration).

## Batch mode

```sh
xentop-ng --batch -d 1 -n 60 > run.jsonl
```

Each line is a JSON object with host and per-domain rates: CPU %, per-vCPU %,
network B/s and pps, disk B/s, IOPS and latency, per VBD and per VIF. This is
handy next to a benchmark run.

## Layout

```
src/
  source/xenstat.rs   libxenstat binding (dlopen, optional extended symbols)
  source/fallback.rs  collectors for what the loaded libxenstat lacks
  source/demo.rs      simulated host
  model.rs            raw counters → per-interval rates
  history.rs          ring buffers behind the graphs
  config.rs           preferences file
  ui/                 layout, boxes, braille graphs, meters, heatmap
  ui/columns.rs       domain table columns: one entry per column
libxenstat/           libxenstat patches: XCP-ng 4.17 and upstream versions
build/                container builds for XCP-ng and deploy script
dist/                 release packaging and the XCP-ng installer
.github/workflows/    CI (fmt, clippy, tests, MSRV, cargo-deny, shellcheck) and releases
docs/                 screenshots and the tools that generate them
```

Screenshots and the animated tour are generated from the demo, so they can
be refreshed after any UI change:

```sh
docs/tools/screenshot.sh docs/xentop-ng.png 200x56 "" -- --demo-cpus 128 --demo-mem 1T
docs/tools/screenshot.sh docs/detail.png 160x46 "j j j Enter" -- --demo-cpus 32 --demo-mem 256G --theme xcp-ng
docs/tools/record.sh docs/xentop-ng.gif 160x45 docs/tools/tour.steps -- --demo-cpus 128 --demo-mem 1T
```

Ideas and possible next steps are in [IDEAS.md](IDEAS.md).

## License

GPL-2.0-only; see [LICENSE](LICENSE). The libxenstat patches follow the
license of the Xen files they modify.
