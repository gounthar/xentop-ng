# libxenstat patches

xentop-ng reads everything through libxenstat. These patches add the
metrics it needs, and fix one existing bug, so the extra data can go
upstream to Xen and help every libxenstat user, not only this tool.

| Directory | Base | Used for |
|---|---|---|
| [`xcp-ng-4.17/`](xcp-ng-4.17/) | Xen 4.17.6 + XCP-ng's patch queue (`xen-4.17.6-12.3.xcpng8.3`) | building the drop-in `libxenstat.so.4.17` for XCP-ng 8.3 (`build/build-libxenstat.sh`) |
| [`upstream/`](upstream/) | xen.git `master` | submission to xen-devel |

Both sets make the same changes. `git apply --check` passes on master for
`upstream/0001` + `0002`.

## Compatibility and fallback

The patches only **add** exported functions and append fields to private
structures. The soname and existing ABI are unchanged, so the patched library
drops in for the stock one: the system `xentop` keeps working against it.

xentop-ng looks up every new symbol at runtime and treats it as optional. With
a stock library it still runs, and the matching fields show `-`. See the
table in the [main README](../README.md#what-you-get-with-stock-vs-patched-libxenstat).

## 0001: extended VBD3 stats and per-pCPU idle time

New API:

```c
/* VBD3 (tapdisk3) only: 1 if the fields below are valid, else 0 */
unsigned int       xenstat_vbd_has_ext(xenstat_vbd *vbd);
unsigned long long xenstat_vbd_rd_reqs_done(xenstat_vbd *vbd);  /* completed reads */
unsigned long long xenstat_vbd_wr_reqs_done(xenstat_vbd *vbd);  /* completed writes */
unsigned long long xenstat_vbd_rd_usecs(xenstat_vbd *vbd);      /* cumulative service time, µs */
unsigned long long xenstat_vbd_wr_usecs(xenstat_vbd *vbd);
unsigned long long xenstat_vbd_io_errors(xenstat_vbd *vbd);

/* cumulative idle ns of physical CPU `cpu` (xc_getcpuinfo); 0 if offline/unknown */
unsigned long long xenstat_node_pcpu_idle_ns(xenstat_node *node, unsigned int cpu);
unsigned int       xenstat_node_num_pcpu_idle(xenstat_node *node);   /* max_cpu_id + 1 */
```

- **VBD fields**: tapdisk3 already writes these counters to its shared-memory
  stats file. libxenstat read the file but threw them away. Average latency
  over an interval is Δusecs / Δreqs_done.
- **pCPU idle time**: one extra `xc_getcpuinfo()` hypercall per
  `xenstat_get_node()` call.
- **Also fixed**: an on-stack `xenstat_vbd` used to be left partially
  uninitialised.

### Caveat: no in-flight count

`submitted − completed` looks like an in-flight count, but it isn't one.
tapdisk counts an empty flush (a `WRITE_BARRIER` with no segments) as a
submitted write and never as a completed one, so the difference grows
forever: we measured 3216 on an idle guest. xentop-ng doesn't display it. A
real in-flight gauge needs a small tapdisk change (see
[IDEAS.md](../IDEAS.md)).

## 0002: VIFs missing on Open vSwitch hosts

`xenstat_collect_networks()` looks for a Linux bridge so that, with bonding,
dom0 can report the bridge's counters. Open vSwitch hosts, which includes
every XCP-ng host by default, have no Linux bridge, so the bridge name stays
empty.

- `strstr(iface, "")` matches every interface, so each VIF takes the bridge
  branch and is never attached to its domain.
- Every domain ends up with zero networks, and stock `xentop` shows `NETS 0`.

The fix takes the bridge branch only when a bridge was actually found.

## Before submitting upstream

- Add your `Signed-off-by:` (DCO) to each patch.
- Harden the existing `read_attributes_vbd3()`. It is not ours, but sits in
  the same path: it opens `/dev/shm/td3-<pid>/vbd-*` with plain `fopen()`.
  Use `O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC`, then `fstat()` for a root-owned
  regular file of at least `sizeof(struct vbd3_stats)` bytes. That is what
  xentop-ng's own fallback does.
- Consider exporting a version marker (e.g. `xenstat_ext_version()`), so
  consumers can tell the semantics apart if the accessors change during
  review.

