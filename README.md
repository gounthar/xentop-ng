# xentop-ng

A modern, btop-inspired resource monitor for the Xen hypervisor.

![xentop-ng on a simulated 128-pCPU, 1 TiB host](docs/xentop-ng.png)

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
- **Domain list**: sortable and filterable, with CPU meter, CPU history,
  memory, network, disk throughput, IOPS and latency. Columns adapt to the
  terminal width.
- **Domain details** (`⏎`): per-vCPU load, per-disk IOPS/throughput/latency,
  per-vif traffic, packets and errors.
- **Themes**: `btop`, `xcp-ng`, `dracula`, `gruvbox`.
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

Any `--demo-*` option implies `--demo`.

- **VM counts** follow the pCPU count.
- **VM sizes** follow the RAM.
- **Load** is calibrated at startup so the host averages the requested
  figure.
- **Big hosts** get extra workloads: ML training VMs from 64 pCPUs, and a
  256 GiB database VM per TiB of RAM from 512 GiB up.

```sh
xentop-ng --demo-cpus 128 --demo-mem 1T                  # the screenshot above
xentop-ng --demo-cpus 256 --demo-mem 2T --demo-load 85   # a busy large host
xentop-ng --demo-cpus 4 --demo-mem 16G                   # a small lab box
xentop-ng --demo-cpus 1024 --demo-mem 8T                 # stress the heatmap
```

## Running on a Xen host

xentop-ng runs in dom0 as root, like `xentop`. It loads **libxenstat at
runtime** rather than linking it, so one binary works with any Xen release.

It prefers a library found through `LD_LIBRARY_PATH`, so a patched copy can
sit next to the system one without replacing it. Use `--lib PATH` to force a
specific library.

### What you get with stock vs. patched libxenstat

| | stock libxenstat | with [our patches](libxenstat/) |
|---|---|---|
| Domains, vCPUs, memory, disk and network throughput/IOPS | ✓ | ✓ |
| Per-pCPU load and heatmap | host load estimated from domain CPU time; "top domains" shown instead | ✓ measured from pCPU idle time |
| Disk latency (tapdisk3 VBDs) | – | ✓ |
| Disk I/O error count | error flag only | ✓ |
| Network stats on Open vSwitch hosts (XCP-ng default) | ✗ all VIFs missing ([bug](libxenstat/README.md#0002-vifs-missing-on-open-vswitch-hosts)) | ✓ |

Missing metrics show `-` rather than a made-up value. blkback and qdisk disks
have no latency counters, so they always show `-` for latency.

### XCP-ng 8.3

The scripts in [`build/`](build/) do everything inside the
`ghcr.io/xcp-ng/xcp-ng-build-env:8.3` container, so the output runs on the
host's glibc 2.17.

```sh
build/build-libxenstat.sh     # patched libxenstat.so.4.17 (XCP-ng 4.17.6 + our patches)
build/build-xentop-ng.sh      # xentop-ng binary
build/deploy.sh HOST          # installs to /opt/xentop-ng on HOST, nothing else touched
ssh -t root@HOST /opt/xentop-ng/bin/xtop
```

`xtop` is a small wrapper that puts `/opt/xentop-ng/lib` on
`LD_LIBRARY_PATH`. The system `xentop` and libxenstat are left untouched.
`deploy.sh` also installs `xenstat-ext-test`, which prints the raw extended
counters for checking.

## Keys

| Key | Action |
|---|---|
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `g` `G` | select domain |
| `⏎`, `space`, double-click | domain details |
| `s` `S` / `←` `→` | next / previous sort column |
| `c` `m` `n` `d` `l` | sort by cpu, memory, network, disk, latency |
| `r` | reverse sort |
| `0` | pin Domain-0 on top |
| `/` or `f` | filter by name or id |
| `1` `2` `3` `4` | toggle cpu / mem / net / disk boxes |
| `+` `-` | slower / faster refresh |
| `p` | pause |
| `t` `T` | cycle themes |
| `?` | help |
| `q` | quit |

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
  source/demo.rs      simulated host
  model.rs            raw counters → per-interval rates
  history.rs          ring buffers behind the graphs
  ui/                 layout, boxes, braille graphs, meters, heatmap
libxenstat/           libxenstat patches: XCP-ng 4.17 and upstream versions
build/                container builds for XCP-ng and deploy script
docs/                 screenshots and the tools that generate them
```

Screenshots are generated from the demo:
`docs/tools/screenshot.sh docs/xentop-ng.png 200x56 "" -- --demo-cpus 128 --demo-mem 1T`.

Ideas and possible next steps are in [IDEAS.md](IDEAS.md).

## License

GPL-2.0-only; see [LICENSE](LICENSE). The libxenstat patches follow the
license of the Xen files they modify.
