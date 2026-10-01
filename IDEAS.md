# Ideas

A working list of ideas: some small, some research. Once one becomes real
work, move it into a GitHub issue.

## Dig deeper: instrument on demand

The main screen shows cheap, always-on counters. The next step is **drilling
down**: select a domain, disk or pCPU, press a key, and xentop-ng collects
something expensive for a few seconds and shows it in place.

### Hypercalls and VM exits with xentrace

- **What:** per-domain hypercall rates by type (PV), VM-exit reasons (HVM/PVH:
  EPT violations, I/O, MSR, HLT, interrupts), and event-channel and IRQ
  rates.
- **How:** Xen's trace buffers, the same data `xentrace` records and
  `xenalyze` summarises. Enable a narrow event mask (`TRC_PV_HYPERCALL`,
  `TRC_HVM_VMX_EXIT`/`TRC_HVM_SVM_EXIT`, `TRC_SCHED_*`) for a 2–5 s window.
  Filter to the selected domain and decode the binary records in Rust; the
  record format is documented in `xen/include/public/trace.h`. Then show a
  ranked table and a live sparkline per call or exit reason.
- **Watch out for:** needs root; the trace buffers are global (one user at a
  time, so check for a running `xentrace`); overhead grows with the event
  mask, so keep windows short and masks narrow. Worth measuring the cost.

### Scheduler latency and steal time

- Time runnable-but-not-running per vCPU is the virtualisation-specific
  number xentop never showed. It's the Xen equivalent of steal time.
- **Cheap version:** a new libxenstat/domctl path that exposes runstate
  times for other domains. Today only a domain's own runstate area has
  them.
- **Detailed version:** xentrace `TRC_SCHED_*` over a window, giving a
  latency histogram per vCPU, which pCPUs it ran on, and migrations.

### Latency flamegraphs

- **Disk:** for a selected VBD, sample the tapdisk3 process in dom0 (perf
  or eBPF uprobes on the request path in `td-req.c` and the driver
  callbacks). Render a flamegraph of where service time goes: ring handling,
  grant copy, VHD/qcow2 metadata, the backing file system. The same idea
  applies to blkback with kprobes.
- **Guest CPU:** sample guest RIPs from the hypervisor (Intel PT through
  Xen's `vmtrace`, or plain sampling of vCPU context via
  `xc_vcpu_getcontext`), then symbolise with the guest's kernel symbols if
  we have them.
- **Rendering:** fold stacks and draw an icicle/flame view in the terminal,
  with an option to export an SVG. The `inferno` crate does the folding and
  the SVG.

### Topology-aware views

- **Grouping:** `xc_topologyinfo` / `xc_numainfo` give socket, core, thread
  and NUMA node per pCPU. Group the heatmap by socket or node, and mark SMT
  siblings.
- **Placement:** show each domain's vCPU placement and memory per NUMA node,
  and flag domains whose memory and vCPUs sit on different nodes.

## Metrics we still lack

- **In-flight requests per VBD:** needs a tapdisk fix (count completed flushes,
  or a separate in-flight gauge). See
  [libxenstat/README.md](libxenstat/README.md#caveat-no-in-flight-count).
- **Latency distribution, not just averages:** tapdisk already keeps
  per-VBD max latency (`st_{rd,wr}_max_usecs` in its xenvbd stats), which is
  a cheap first step. After that, histograms.
- **blkback and qdisk latency:** nothing is exposed today.
- **Network drops and errors on Open vSwitch:** OVS keeps per-port counters
  that /proc/net/dev doesn't show.
- **Grant table and event channel usage per domain,** for spotting domains
  near their limits.

## XCP-ng integration

- ~~VM UUIDs from xenstore; map VBD → VDI → SR, so latency can be aggregated
  per storage repository.~~ Done: detail panel, `v` SR view, `--batch`.
  Still missing: SR/VDI **name-labels** (xapi only), and per-SR history
  graphs.
- **Pool view:** poll several hosts over SSH, or read xcp-rrdd / Xen Orchestra
  data, for a cluster-wide top.
- Optional read-only xapi mode for richer names: SR types, networks.

## Tooling and quality

- **Scenario files for the demo:** host shape, fleet and scripted events
  (a domain crashes, a tapdisk restarts and its counters reset, SMT is off
  and pCPU ids have gaps, a domid is reused).
- **Fake `libxenstat.so`** built from a scenario, to test the real FFI path
  including missing optional symbols.
- **`--record` / `--replay`** of raw snapshots, so traces from real hosts
  become test cases and bug reports.
- Prometheus/OpenMetrics output alongside `--batch`.

## Upstream

- Send `libxenstat/upstream/*` to xen-devel. 0002 is a plain bug fix and
  should be uncontroversial.
- Later: discuss whether xentop-ng belongs in the Xen tree. That would mean
  Rust in `tools/`, which is a bigger conversation. Until then, keep it an
  external project that uses libxenstat and upstream everything data-related.
