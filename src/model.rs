//! Raw counter snapshots as delivered by a data source, and the per-interval
//! "rates" view derived from two consecutive snapshots.

use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

/// One sample of the whole host. All counters are cumulative.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub at: Instant,
    pub hostname: String,
    pub xen_version: String,
    pub num_cpus: u32,
    pub cpu_hz: u64,
    pub tot_mem: u64,
    pub free_mem: u64,
    /// Cumulative idle time per online physical CPU, as (cpu id, ns).
    /// `None` when the loaded libxenstat lacks `xenstat_node_pcpu_idle_ns`.
    pub pcpu_idle_ns: Option<Vec<(u32, u64)>>,
    pub domains: Vec<DomainRaw>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DomState {
    Running,
    Blocked,
    Paused,
    Shutdown,
    Crashed,
    Dying,
}

impl DomState {
    pub fn label(self) -> &'static str {
        match self {
            DomState::Running => "run",
            DomState::Blocked => "idle",
            DomState::Paused => "pause",
            DomState::Shutdown => "shut",
            DomState::Crashed => "CRASH",
            DomState::Dying => "dying",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DomainRaw {
    pub id: u32,
    pub name: String,
    pub state: DomState,
    pub cpu_ns: u64,
    pub vcpus: Vec<VcpuRaw>,
    pub cur_mem: u64,
    pub max_mem: u64,
    pub nets: Vec<NetRaw>,
    pub vbds: Vec<VbdRaw>,
}

#[derive(Clone, Copy, Debug)]
pub struct VcpuRaw {
    pub online: bool,
    pub ns: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetRaw {
    pub id: u32,
    pub rbytes: u64,
    pub rpackets: u64,
    pub rerrs: u64,
    pub rdrop: u64,
    pub tbytes: u64,
    pub tpackets: u64,
    pub terrs: u64,
    pub tdrop: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VbdKind {
    Blkback,
    Tap,
    Vbd3,
    Qdisk,
    Unknown,
}

impl VbdKind {
    pub fn from_xenstat(t: u32) -> Self {
        match t {
            1 => VbdKind::Blkback,
            2 => VbdKind::Tap,
            3 => VbdKind::Vbd3,
            4 => VbdKind::Qdisk,
            _ => VbdKind::Unknown,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            VbdKind::Blkback => "blkback",
            VbdKind::Tap => "tap",
            VbdKind::Vbd3 => "tapdisk3",
            VbdKind::Qdisk => "qdisk",
            VbdKind::Unknown => "?",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VbdRaw {
    pub dev: u32,
    pub kind: VbdKind,
    pub oo_reqs: u64,
    pub rd_reqs: u64,
    pub wr_reqs: u64,
    pub rd_sects: u64,
    pub wr_sects: u64,
    pub error: bool,
    pub ext: Option<VbdExt>,
}

/// Extended per-VBD counters (tapdisk3 only, patched libxenstat).
#[derive(Clone, Copy, Debug, Default)]
pub struct VbdExt {
    pub rd_done: u64,
    pub wr_done: u64,
    pub rd_usecs: u64,
    pub wr_usecs: u64,
    pub io_errors: u64,
}

// ---------------------------------------------------------------------------
// Rates

#[derive(Clone, Debug, Default, Serialize)]
pub struct HostRates {
    pub hostname: String,
    pub xen_version: String,
    pub num_cpus: u32,
    pub cpu_mhz: u64,
    /// Per-pCPU busy fraction (0..1); empty when unavailable.
    pub pcpu_busy: Vec<f64>,
    /// Hypervisor CPU id for each entry of `pcpu_busy`.
    pub pcpu_ids: Vec<u32>,
    /// Whole-host busy fraction (0..1).
    pub cpu_busy: f64,
    /// True if `cpu_busy` is estimated from domain CPU time rather than
    /// measured from pCPU idle counters.
    pub cpu_estimated: bool,
    pub mem_total: u64,
    pub mem_free: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub disk_rd_bps: f64,
    pub disk_wr_bps: f64,
    pub disk_rd_iops: f64,
    pub disk_wr_iops: f64,
    /// Request-weighted mean service latency across all extended VBDs (µs).
    pub disk_rd_lat_us: Option<f64>,
    pub disk_wr_lat_us: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DomRates {
    pub id: u32,
    pub name: String,
    pub state: Option<DomState>,
    /// 100.0 == one physical CPU fully used.
    pub cpu_pct: f64,
    pub vcpu_pct: Vec<f64>,
    pub vcpus_online: usize,
    pub mem: u64,
    pub max_mem: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub net_errs: u64,
    pub net_drops: u64,
    pub disk_rd_bps: f64,
    pub disk_wr_bps: f64,
    pub disk_rd_iops: f64,
    pub disk_wr_iops: f64,
    pub disk_rd_lat_us: Option<f64>,
    pub disk_wr_lat_us: Option<f64>,
    pub disk_oo_ps: f64,
    pub disk_errors: u64,
    pub vbds: Vec<VbdRates>,
    pub nets: Vec<NetRates>,
}

impl DomRates {
    pub fn disk_bps(&self) -> f64 {
        self.disk_rd_bps + self.disk_wr_bps
    }
    pub fn net_bps(&self) -> f64 {
        self.net_rx_bps + self.net_tx_bps
    }
    /// Worst of read/write latency, for sorting and colouring.
    pub fn lat_us(&self) -> Option<f64> {
        match (self.disk_rd_lat_us, self.disk_wr_lat_us) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct VbdRates {
    pub dev: u32,
    pub name: String,
    pub kind: Option<VbdKind>,
    pub rd_bps: f64,
    pub wr_bps: f64,
    pub rd_iops: f64,
    pub wr_iops: f64,
    pub rd_lat_us: Option<f64>,
    pub wr_lat_us: Option<f64>,
    pub oo_ps: f64,
    pub errors: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NetRates {
    pub id: u32,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_pps: f64,
    pub tx_pps: f64,
    pub errs: u64,
    pub drops: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Rates {
    pub interval_s: f64,
    pub host: HostRates,
    pub domains: Vec<DomRates>,
}

/// Counter delta that survives resets (domain reboot, device replug).
fn d(cur: u64, prev: u64) -> u64 {
    cur.saturating_sub(prev)
}

fn lat(usecs: u64, reqs: u64) -> Option<f64> {
    (reqs > 0).then(|| usecs as f64 / reqs as f64)
}

/// Linux-style disk name for a Xen virtual block device number.
pub fn vbd_name(dev: u32) -> String {
    let (major, minor) = (dev >> 8, dev & 0xff);
    let letters = |n: u32| -> String {
        let mut n = n;
        let mut s = Vec::new();
        loop {
            s.push(b'a' + (n % 26) as u8);
            if n < 26 {
                break;
            }
            n = n / 26 - 1;
        }
        s.reverse();
        String::from_utf8(s).unwrap_or_default()
    };
    if dev & (1 << 28) != 0 {
        // Extended scheme: 1 << 28 | disk << 8 | partition
        let disk = (dev >> 8) & 0xfffff;
        return format!("xvd{}", letters(disk));
    }
    match major {
        202 => format!("xvd{}", letters(minor >> 4)),
        3 => format!("hd{}", letters(minor >> 6)),
        22 => format!("hd{}", letters(2 + (minor >> 6))),
        8 => format!("sd{}", letters(minor >> 4)),
        _ => format!("{dev}"),
    }
}

pub fn compute(prev: &Snapshot, cur: &Snapshot) -> Rates {
    let dt = cur.at.duration_since(prev.at).as_secs_f64().max(1e-3);
    let dt_ns = dt * 1e9;

    let prev_doms: HashMap<u32, &DomainRaw> = prev
        .domains
        .iter()
        .map(|d| (d.id, d))
        .collect();

    let mut host = HostRates {
        hostname: cur.hostname.clone(),
        xen_version: cur.xen_version.clone(),
        num_cpus: cur.num_cpus,
        cpu_mhz: cur.cpu_hz / 1_000_000,
        mem_total: cur.tot_mem,
        mem_free: cur.free_mem,
        ..Default::default()
    };

    let (mut h_rd_us, mut h_wr_us, mut h_rd_done, mut h_wr_done) = (0u64, 0u64, 0u64, 0u64);
    let mut dom_cpu_total = 0f64;

    let mut domains = Vec::with_capacity(cur.domains.len());
    for dom in &cur.domains {
        let p = prev_doms.get(&dom.id).filter(|p| p.name == dom.name);
        let mut r = DomRates {
            id: dom.id,
            name: dom.name.clone(),
            state: Some(dom.state),
            mem: dom.cur_mem,
            max_mem: dom.max_mem,
            vcpus_online: dom.vcpus.iter().filter(|v| v.online).count(),
            ..Default::default()
        };

        if let Some(p) = p {
            r.cpu_pct = d(dom.cpu_ns, p.cpu_ns) as f64 / dt_ns * 100.0;
            r.vcpu_pct = dom
                .vcpus
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let pv = p.vcpus.get(i).map(|v| v.ns).unwrap_or(v.ns);
                    (d(v.ns, pv) as f64 / dt_ns * 100.0).min(100.0)
                })
                .collect();
        } else {
            r.vcpu_pct = vec![0.0; dom.vcpus.len()];
        }
        dom_cpu_total += r.cpu_pct / 100.0;

        for n in &dom.nets {
            let pn = p.and_then(|p| p.nets.iter().find(|x| x.id == n.id));
            let mut nr = NetRates {
                id: n.id,
                errs: n.rerrs + n.terrs,
                drops: n.rdrop + n.tdrop,
                ..Default::default()
            };
            if let Some(pn) = pn {
                // xenstat reports vif counters from dom0's point of view:
                // what the backend receives is what the guest transmits.
                nr.tx_bps = d(n.rbytes, pn.rbytes) as f64 / dt;
                nr.rx_bps = d(n.tbytes, pn.tbytes) as f64 / dt;
                nr.tx_pps = d(n.rpackets, pn.rpackets) as f64 / dt;
                nr.rx_pps = d(n.tpackets, pn.tpackets) as f64 / dt;
            }
            r.net_rx_bps += nr.rx_bps;
            r.net_tx_bps += nr.tx_bps;
            r.net_errs += nr.errs;
            r.net_drops += nr.drops;
            r.nets.push(nr);
        }

        let (mut rd_us, mut wr_us, mut rd_done, mut wr_done) = (0u64, 0u64, 0u64, 0u64);
        for v in &dom.vbds {
            let pv = p.and_then(|p| p.vbds.iter().find(|x| x.dev == v.dev && x.kind == v.kind));
            let mut vr = VbdRates {
                dev: v.dev,
                name: vbd_name(v.dev),
                kind: Some(v.kind),
                errors: v.ext.map(|e| e.io_errors).unwrap_or(0) + v.error as u64,
                ..Default::default()
            };
            // No in-flight estimate: tapdisk counts empty flushes as
            // submitted writes but never as completed ones, so
            // submitted - completed drifts upward forever.
            if let Some(pv) = pv {
                vr.rd_bps = d(v.rd_sects, pv.rd_sects) as f64 * 512.0 / dt;
                vr.wr_bps = d(v.wr_sects, pv.wr_sects) as f64 * 512.0 / dt;
                vr.rd_iops = d(v.rd_reqs, pv.rd_reqs) as f64 / dt;
                vr.wr_iops = d(v.wr_reqs, pv.wr_reqs) as f64 / dt;
                vr.oo_ps = d(v.oo_reqs, pv.oo_reqs) as f64 / dt;
                if let (Some(e), Some(pe)) = (v.ext, pv.ext) {
                    let (ru, wu) = (d(e.rd_usecs, pe.rd_usecs), d(e.wr_usecs, pe.wr_usecs));
                    let (rn, wn) = (d(e.rd_done, pe.rd_done), d(e.wr_done, pe.wr_done));
                    vr.rd_lat_us = lat(ru, rn);
                    vr.wr_lat_us = lat(wu, wn);
                    rd_us += ru;
                    wr_us += wu;
                    rd_done += rn;
                    wr_done += wn;
                }
            }
            r.disk_rd_bps += vr.rd_bps;
            r.disk_wr_bps += vr.wr_bps;
            r.disk_rd_iops += vr.rd_iops;
            r.disk_wr_iops += vr.wr_iops;
            r.disk_oo_ps += vr.oo_ps;
            r.disk_errors += vr.errors;
            r.vbds.push(vr);
        }
        r.disk_rd_lat_us = lat(rd_us, rd_done);
        r.disk_wr_lat_us = lat(wr_us, wr_done);

        host.net_rx_bps += r.net_rx_bps;
        host.net_tx_bps += r.net_tx_bps;
        host.disk_rd_bps += r.disk_rd_bps;
        host.disk_wr_bps += r.disk_wr_bps;
        host.disk_rd_iops += r.disk_rd_iops;
        host.disk_wr_iops += r.disk_wr_iops;
        h_rd_us += rd_us;
        h_wr_us += wr_us;
        h_rd_done += rd_done;
        h_wr_done += wr_done;
        domains.push(r);
    }
    host.disk_rd_lat_us = lat(h_rd_us, h_rd_done);
    host.disk_wr_lat_us = lat(h_wr_us, h_wr_done);

    match (&prev.pcpu_idle_ns, &cur.pcpu_idle_ns) {
        (Some(pi), Some(ci)) if !ci.is_empty() => {
            let prev_idle: HashMap<u32, u64> = pi.iter().copied().collect();
            for &(id, c) in ci {
                let p = prev_idle.get(&id).copied().unwrap_or(c);
                host.pcpu_ids.push(id);
                host.pcpu_busy.push((1.0 - d(c, p) as f64 / dt_ns).clamp(0.0, 1.0));
            }
            host.cpu_busy = host.pcpu_busy.iter().sum::<f64>() / host.pcpu_busy.len() as f64;
        }
        _ => {
            host.cpu_estimated = true;
            host.cpu_busy = (dom_cpu_total / cur.num_cpus.max(1) as f64).clamp(0.0, 1.0);
        }
    }

    Rates {
        interval_s: dt,
        host,
        domains,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vbd_names() {
        assert_eq!(vbd_name(51712), "xvda");
        assert_eq!(vbd_name(51728), "xvdb");
        assert_eq!(vbd_name(51808), "xvdg");
        assert_eq!(vbd_name(768), "hda");
        assert_eq!(vbd_name((1 << 28) | (27 << 8)), "xvdab");
    }
}
