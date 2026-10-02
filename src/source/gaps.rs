//! Patching over what the loaded libxenstat lacks, on a snapshot already
//! copied out of it: pCPU idle time, VIFs, steal time and tapdisk3
//! latency. Plain Rust over [`Snapshot`]; the collectors that read the
//! host (see fallback.rs) sit behind [`Collectors`], so tests can fake them.

use super::fallback::{self, Vbd3Index, XcCpuInfo, XcDomRunstate};
use super::Avail;
use crate::model::{DomainRaw, NetRaw, Snapshot, VbdExt, VbdKind};
use std::collections::HashMap;

/// Where each class of metrics came from, as [`fill`] found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gaps {
    pub pcpu: Avail,
    pub vifs: Avail,
    pub steal: Avail,
    pub vbd_latency: Avail,
}

/// What the fallbacks read from the host. Each is only called when the
/// snapshot needs it.
pub trait Collectors {
    /// (cpu id, cumulative idle ns) for every online pCPU.
    fn pcpu_idle(&mut self) -> Option<Vec<(u32, u64)>>;
    /// VIF counters by (domid, devid).
    fn vifs(&mut self) -> HashMap<(u32, u32), NetRaw>;
    /// A domain's runnable time summed over its vCPUs (ns).
    fn dom_runnable_ns(&mut self, domid: u32, nr_vcpus: usize) -> Option<u64>;
    /// tapdisk3 counters by (domid, dev).
    fn vbd3(&mut self) -> HashMap<(u32, u32), VbdExt>;
}

/// The real collectors: libxenctrl, /proc/net/dev and tapdisk3's stats.
pub struct HostCollectors {
    /// Opened only when libxenstat lacks pCPU idle times.
    xc: Option<XcCpuInfo>,
    /// Opened the first time it is needed (outer None: not tried yet).
    xc_runstate: Option<Option<XcDomRunstate>>,
}

impl HostCollectors {
    pub fn new(need_pcpu: bool) -> Self {
        HostCollectors {
            xc: if need_pcpu { XcCpuInfo::open() } else { None },
            xc_runstate: None,
        }
    }
}

impl Collectors for HostCollectors {
    fn pcpu_idle(&mut self) -> Option<Vec<(u32, u64)>> {
        self.xc.as_mut().and_then(|xc| xc.idle())
    }
    fn vifs(&mut self) -> HashMap<(u32, u32), NetRaw> {
        fallback::proc_net_vifs()
    }
    fn dom_runnable_ns(&mut self, domid: u32, nr_vcpus: usize) -> Option<u64> {
        let xc = self.xc_runstate.get_or_insert_with(XcDomRunstate::open);
        xc.as_mut().and_then(|xc| xc.runnable_ns(domid, nr_vcpus))
    }
    fn vbd3(&mut self) -> HashMap<(u32, u32), VbdExt> {
        Vbd3Index::scan().into_map()
    }
}

/// Fill what `snap` lacks from `c`. `lib_vbd_ext`: the library has the
/// tapdisk3 latency symbols (then a missing figure is not ours to fill).
pub fn fill(snap: &mut Snapshot, lib_vbd_ext: bool, c: &mut impl Collectors) -> Gaps {
    Gaps {
        pcpu: pcpu(snap, c),
        vifs: vifs(snap, c),
        steal: steal(snap, c),
        vbd_latency: vbd_latency(snap, lib_vbd_ext, c),
    }
}

fn pcpu(snap: &mut Snapshot, c: &mut impl Collectors) -> Avail {
    if snap.pcpu_idle_ns.is_some() {
        Avail::Lib
    } else if let Some(v) = c.pcpu_idle() {
        snap.pcpu_idle_ns = Some(v);
        Avail::Fallback
    } else {
        Avail::Missing
    }
}

/// Stock libxenstat drops every VIF on hosts without a Linux bridge.
/// Rebuild any guest's VIFs from /proc/net/dev.
fn vifs(snap: &mut Snapshot, c: &mut impl Collectors) -> Avail {
    let lacking = |d: &DomainRaw| d.id != 0 && d.nets.is_empty();
    if !snap.domains.iter().any(lacking) {
        return Avail::Lib;
    }
    let vifs = c.vifs();
    let mut status = Avail::Lib;
    for d in snap.domains.iter_mut().filter(|d| lacking(d)) {
        let mut nets: Vec<NetRaw> = vifs
            .iter()
            .filter(|((domid, _), _)| *domid == d.id)
            .map(|(_, n)| n.clone())
            .collect();
        if !nets.is_empty() {
            nets.sort_by_key(|n| n.id);
            d.nets = nets;
            status = Avail::Fallback;
        }
    }
    status
}

/// Per vCPU from libxenstat (needs the hypervisor patch too), else per
/// domain from XCP-ng's own domctl.
fn steal(snap: &mut Snapshot, c: &mut impl Collectors) -> Avail {
    let per_vcpu = |d: &DomainRaw| !d.vcpus.is_empty() && d.vcpus.iter().all(|v| v.runnable_ns.is_some());
    if snap.domains.iter().any(per_vcpu) {
        return Avail::Lib;
    }
    let mut found = false;
    for d in &mut snap.domains {
        d.runnable_ns = c.dom_runnable_ns(d.id, d.vcpus.len());
        found |= d.runnable_ns.is_some();
    }
    if found {
        Avail::Fallback
    } else {
        Avail::Missing
    }
}

fn vbd_latency(snap: &mut Snapshot, lib_vbd_ext: bool, c: &mut impl Collectors) -> Avail {
    let vbds = || snap.domains.iter().flat_map(|d| d.vbds.iter());
    if !vbds().any(|v| v.kind == VbdKind::Vbd3) {
        return Avail::NotApplicable;
    }
    if lib_vbd_ext {
        return if vbds().any(|v| v.ext.is_some()) {
            Avail::Lib
        } else {
            Avail::Missing
        };
    }
    let idx = c.vbd3();
    let mut found = false;
    if !idx.is_empty() {
        for d in &mut snap.domains {
            for v in d.vbds.iter_mut().filter(|v| v.kind == VbdKind::Vbd3) {
                v.ext = idx.get(&(d.id, v.dev)).copied();
                found |= v.ext.is_some();
            }
        }
    }
    if found {
        Avail::Fallback
    } else {
        Avail::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{flag, DomState, VbdRaw, VcpuRaw};
    use std::time::Instant;

    /// Canned answers, and which collectors were asked.
    #[derive(Default)]
    struct Fake {
        pcpu: Option<Vec<(u32, u64)>>,
        vifs: HashMap<(u32, u32), NetRaw>,
        runnable: HashMap<u32, u64>,
        vbd3: HashMap<(u32, u32), VbdExt>,
        asked: Vec<&'static str>,
    }

    impl Collectors for Fake {
        fn pcpu_idle(&mut self) -> Option<Vec<(u32, u64)>> {
            self.asked.push("pcpu");
            self.pcpu.clone()
        }
        fn vifs(&mut self) -> HashMap<(u32, u32), NetRaw> {
            self.asked.push("vifs");
            self.vifs.clone()
        }
        fn dom_runnable_ns(&mut self, domid: u32, _: usize) -> Option<u64> {
            self.asked.push("runnable");
            self.runnable.get(&domid).copied()
        }
        fn vbd3(&mut self) -> HashMap<(u32, u32), VbdExt> {
            self.asked.push("vbd3");
            self.vbd3.clone()
        }
    }

    fn dom(id: u32) -> DomainRaw {
        DomainRaw {
            id,
            name: format!("d{id}"),
            state: DomState::Running,
            flags: flag::RUNNING,
            ssid: 0,
            cpu_ns: 0,
            vcpus: vec![VcpuRaw {
                online: true,
                ns: 0,
                runnable_ns: None,
            }],
            cur_mem: 0,
            max_mem: 0,
            nets: vec![],
            vbds: vec![],
            vm_uuid: None,
            mem_target: None,
            runnable_ns: None,
        }
    }

    fn vbd(kind: VbdKind, ext: Option<VbdExt>) -> VbdRaw {
        VbdRaw {
            dev: 51712,
            kind,
            oo_reqs: 0,
            rd_reqs: 0,
            wr_reqs: 0,
            rd_sects: 0,
            wr_sects: 0,
            error: false,
            ext,
            backing: None,
        }
    }

    fn net(id: u32) -> NetRaw {
        NetRaw {
            id,
            ..Default::default()
        }
    }

    fn snap(domains: Vec<DomainRaw>) -> Snapshot {
        Snapshot {
            at: Instant::now(),
            hostname: "h".into(),
            xen_version: "x".into(),
            num_cpus: 2,
            cpu_hz: 0,
            tot_mem: 0,
            free_mem: 0,
            pcpu_idle_ns: None,
            domains,
        }
    }

    #[test]
    fn complete_library_needs_no_collector() {
        let mut d = dom(1);
        d.nets = vec![net(0)];
        d.vcpus[0].runnable_ns = Some(5);
        d.vbds = vec![vbd(VbdKind::Vbd3, Some(VbdExt::default()))];
        let mut s = snap(vec![dom(0), d]);
        s.pcpu_idle_ns = Some(vec![(0, 1)]);
        let mut f = Fake::default();
        let g = fill(&mut s, true, &mut f);
        assert_eq!(
            g,
            Gaps {
                pcpu: Avail::Lib,
                vifs: Avail::Lib,
                steal: Avail::Lib,
                vbd_latency: Avail::Lib,
            }
        );
        assert!(f.asked.is_empty(), "{:?}", f.asked);
    }

    #[test]
    fn pcpu_from_the_fallback_or_missing() {
        let mut s = snap(vec![dom(0)]);
        let mut f = Fake {
            pcpu: Some(vec![(0, 10), (2, 30)]),
            ..Default::default()
        };
        assert_eq!(fill(&mut s, false, &mut f).pcpu, Avail::Fallback);
        assert_eq!(s.pcpu_idle_ns, Some(vec![(0, 10), (2, 30)]));
        let mut s = snap(vec![dom(0)]);
        assert_eq!(fill(&mut s, false, &mut Fake::default()).pcpu, Avail::Missing);
    }

    #[test]
    fn missing_vifs_rebuilt_per_guest_in_id_order() {
        let mut keep = dom(2);
        keep.nets = vec![net(7)];
        let mut s = snap(vec![dom(0), dom(1), keep, dom(3)]);
        let mut f = Fake::default();
        // Out of order, a dom0 entry, one for a guest that has its own.
        for k in [(1, 1), (1, 0), (0, 0), (2, 0)] {
            f.vifs.insert(k, net(k.1));
        }
        assert_eq!(fill(&mut s, false, &mut f).vifs, Avail::Fallback);
        let ids = |d: &DomainRaw| d.nets.iter().map(|n| n.id).collect::<Vec<_>>();
        assert_eq!(ids(&s.domains[0]), Vec::<u32>::new(), "dom0 left alone");
        assert_eq!(ids(&s.domains[1]), [0, 1]);
        assert_eq!(ids(&s.domains[2]), [7], "guests with VIFs untouched");
        assert_eq!(ids(&s.domains[3]), Vec::<u32>::new(), "nothing to add");

        // Every guest has its VIFs: /proc/net/dev isn't read.
        let mut s = snap(vec![dom(0)]);
        let mut f = Fake::default();
        assert_eq!(fill(&mut s, false, &mut f).vifs, Avail::Lib);
        assert!(!f.asked.contains(&"vifs"));
    }

    #[test]
    fn steal_per_domain_when_no_vcpu_has_it() {
        let mut s = snap(vec![dom(0), dom(1)]);
        let mut f = Fake::default();
        f.runnable.insert(1, 42);
        assert_eq!(fill(&mut s, false, &mut f).steal, Avail::Fallback);
        assert_eq!(s.domains[0].runnable_ns, None);
        assert_eq!(s.domains[1].runnable_ns, Some(42));

        let mut s = snap(vec![dom(0)]);
        assert_eq!(fill(&mut s, false, &mut Fake::default()).steal, Avail::Missing);
    }

    #[test]
    fn tapdisk3_latency_from_its_stats_files() {
        let ext = VbdExt {
            rd_done: 3,
            ..Default::default()
        };
        let with = |kind| {
            let mut d = dom(1);
            d.vbds = vec![vbd(kind, None)];
            snap(vec![d])
        };

        // No tapdisk3 disk: nothing to fill, nothing scanned.
        let mut s = with(VbdKind::Blkback);
        let mut f = Fake::default();
        assert_eq!(fill(&mut s, false, &mut f).vbd_latency, Avail::NotApplicable);
        assert!(!f.asked.contains(&"vbd3"));

        // The library has the symbols but no figures: missing, not scanned.
        let mut s = with(VbdKind::Vbd3);
        let mut f = Fake::default();
        assert_eq!(fill(&mut s, true, &mut f).vbd_latency, Avail::Missing);
        assert!(!f.asked.contains(&"vbd3"));

        // Stock library: read from the stats files.
        let mut s = with(VbdKind::Vbd3);
        let mut f = Fake::default();
        f.vbd3.insert((1, 51712), ext);
        assert_eq!(fill(&mut s, false, &mut f).vbd_latency, Avail::Fallback);
        assert_eq!(s.domains[0].vbds[0].ext.map(|e| e.rd_done), Some(3));

        let mut s = with(VbdKind::Vbd3);
        assert_eq!(
            fill(&mut s, false, &mut Fake::default()).vbd_latency,
            Avail::Missing
        );
    }
}
