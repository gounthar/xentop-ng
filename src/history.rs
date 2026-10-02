//! Fixed-size time series feeding the graphs.

use crate::model::Rates;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

pub const CAP: usize = 1024;
/// Points kept per SR and per disk: enough for the trend column.
const IO_CAP: usize = 120;
/// Time constant of the smoothed IOPS that rows are ranked by, seconds.
const RANK_TAU: f64 = 10.0;
/// A row only moves above the one before it when its smoothed IOPS is
/// this much higher (and by at least `RANK_MIN` IOPS), so rows with
/// similar loads don't trade places on noise.
const RANK_MARGIN: f64 = 1.2;
const RANK_MIN: f64 = 2.0;

#[derive(Default, Clone)]
pub struct Series(VecDeque<f64>);

impl Series {
    pub fn push(&mut self, v: f64) {
        if self.0.len() == CAP {
            self.0.pop_front();
        }
        self.0.push_back(if v.is_finite() { v } else { f64::NAN });
    }
    /// NaN is an internal graph gap, never a serialized metric.
    pub fn push_optional(&mut self, v: Option<f64>) {
        self.push(v.unwrap_or(f64::NAN));
    }
    /// The most recent `n` points, oldest first.
    pub fn tail(&self, n: usize) -> Vec<f64> {
        let skip = self.0.len().saturating_sub(n);
        self.0.iter().skip(skip).copied().collect()
    }
}

/// Recent IOPS and latency of one SR or disk, for the SR view.
#[derive(Default)]
pub struct IoHistory {
    pub iops: VecDeque<f64>,
    /// Worst of read/write latency (µs) at each point; None when idle or
    /// without latency counters.
    pub lat: VecDeque<Option<f64>>,
    /// IOPS smoothed over about `RANK_TAU` seconds.
    pub smooth: f64,
    last_seen: u64,
}

impl IoHistory {
    fn push(&mut self, iops: f64, lat: Option<f64>, dt: f64, tick: u64) {
        let iops = if iops.is_finite() { iops.max(0.0) } else { f64::NAN };
        if self.iops.len() == IO_CAP {
            self.iops.pop_front();
            self.lat.pop_front();
        }
        self.iops.push_back(iops);
        self.lat.push_back(lat.filter(|l| l.is_finite()));
        // First point: start at the value rather than ramping up from 0.
        let a = if self.last_seen == 0 {
            1.0
        } else {
            1.0 - (-dt.max(0.0) / RANK_TAU).exp()
        };
        if iops.is_finite() {
            self.smooth += a * (iops - self.smooth);
        }
        self.last_seen = tick;
    }
}

/// Keys in display order, re-ranked by smoothed IOPS with hysteresis.
fn rerank<K: Clone + Eq + Hash>(order: &mut Vec<K>, io: &HashMap<K, IoHistory>) {
    order.retain(|k| io.contains_key(k));
    let smooth = |k: &K| io.get(k).map_or(0.0, |h| h.smooth);
    // Newcomers go in at their place, as if they had always been there.
    let have: HashSet<&K> = order.iter().collect();
    let mut new: Vec<&K> = io.keys().filter(|k| !have.contains(k)).collect();
    new.sort_by(|a, b| smooth(b).total_cmp(&smooth(a)));
    for k in new {
        let at = order
            .iter()
            .position(|o| smooth(o) < smooth(k))
            .unwrap_or(order.len());
        order.insert(at, k.clone());
    }
    // Then each row climbs while it clearly beats the one above it.
    for i in 1..order.len() {
        let mut j = i;
        while j > 0 {
            let (up, me) = (smooth(&order[j - 1]), smooth(&order[j]));
            if me > up * RANK_MARGIN && me - up >= RANK_MIN {
                order.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
}

#[derive(Default)]
pub struct DomHistory {
    pub name: String,
    pub cpu: Series,
    pub rx: Series,
    pub tx: Series,
    pub rd: Series,
    pub wr: Series,
    pub lat: Series,
    /// Current memory, bytes.
    pub mem: Series,
    pub vcpus: Vec<Series>,
    last_seen: u64,
}

#[derive(Default)]
pub struct History {
    pub cpu: Series,
    pub pcpu: Vec<Series>,
    pub mem: Series,
    pub rx: Series,
    pub tx: Series,
    pub rd: Series,
    pub wr: Series,
    pub riops: Series,
    pub wiops: Series,
    pub rlat: Series,
    pub wlat: Series,
    pub doms: HashMap<u32, DomHistory>,
    /// Per storage key (SR UUID or backing directory), as in `Rates::srs`.
    pub srs: HashMap<String, IoHistory>,
    /// Per disk: (domid, dev).
    pub vbds: HashMap<(u32, u32), IoHistory>,
    /// SR view rows, in a stable order (busiest first, give or take).
    pub sr_order: Vec<String>,
    pub vbd_order: Vec<(u32, u32)>,
    tick: u64,
}

impl History {
    pub fn record(&mut self, r: &Rates) {
        self.tick += 1;
        let h = &r.host;
        self.cpu.push(h.cpu_busy * 100.0);
        if self.pcpu.len() != h.pcpu_busy.len() {
            self.pcpu = vec![Series::default(); h.pcpu_busy.len()];
        }
        for (s, v) in self.pcpu.iter_mut().zip(&h.pcpu_busy) {
            s.push(v * 100.0);
        }
        self.mem
            .push(h.mem_total.saturating_sub(h.mem_free) as f64 / h.mem_total.max(1) as f64 * 100.0);
        self.rx.push(h.net_rx_bps);
        self.tx.push(h.net_tx_bps);
        self.rd
            .push_optional(h.disk_samples.complete().then_some(h.disk_rd_bps));
        self.wr
            .push_optional(h.disk_samples.complete().then_some(h.disk_wr_bps));
        self.riops
            .push_optional(h.disk_samples.complete().then_some(h.disk_rd_iops));
        self.wiops
            .push_optional(h.disk_samples.complete().then_some(h.disk_wr_iops));
        self.rlat.push_optional(h.disk_rd_lat_us);
        self.wlat.push_optional(h.disk_wr_lat_us);

        for d in &r.domains {
            let e = self.doms.entry(d.id).or_default();
            if d.baseline_reset || e.last_seen + 1 != self.tick {
                *e = DomHistory {
                    name: d.name.clone(),
                    ..Default::default()
                };
                // A new domain under a reused id: its disks start over too.
                self.vbds.retain(|(id, _), _| *id != d.id);
            }
            e.name = d.name.clone();
            e.last_seen = self.tick;
            e.cpu.push(d.cpu_pct);
            e.rx.push(d.net_rx_bps);
            e.tx.push(d.net_tx_bps);
            e.rd.push_optional(d.disk_samples.complete().then_some(d.disk_rd_bps));
            e.wr.push_optional(d.disk_samples.complete().then_some(d.disk_wr_bps));
            e.lat.push_optional(d.lat_us());
            e.mem.push(d.mem as f64);
            if e.vcpus.len() != d.vcpu_pct.len() {
                e.vcpus = vec![Series::default(); d.vcpu_pct.len()];
            }
            for (s, v) in e.vcpus.iter_mut().zip(&d.vcpu_pct) {
                s.push(*v);
            }
        }
        let tick = self.tick;
        self.doms.retain(|_, h| tick - h.last_seen < 5);

        let dt = r.interval_s;
        let worst = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        for s in &r.srs {
            let e = self.srs.entry(s.sr.clone()).or_default();
            e.push(
                if s.disk_samples.complete() {
                    s.iops()
                } else {
                    f64::NAN
                },
                worst(s.rd_lat_us, s.wr_lat_us),
                dt,
                tick,
            );
        }
        for d in &r.domains {
            for v in d.vbds.iter().filter(|v| v.backing.group().is_some()) {
                if !v.stats_valid {
                    self.vbds.remove(&(d.id, v.dev));
                    continue;
                }
                let e = self.vbds.entry((d.id, v.dev)).or_default();
                e.push(v.rd_iops + v.wr_iops, worst(v.rd_lat_us, v.wr_lat_us), dt, tick);
            }
        }
        // Gone for good, not just missing from one sample.
        self.srs.retain(|_, h| tick - h.last_seen < 5);
        self.vbds.retain(|_, h| tick - h.last_seen < 5);
        rerank(&mut self.sr_order, &self.srs);
        rerank(&mut self.vbd_order, &self.vbds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(v: &[(&'static str, f64)]) -> HashMap<&'static str, IoHistory> {
        v.iter()
            .map(|&(k, s)| {
                (
                    k,
                    IoHistory {
                        smooth: s,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn missing_history_is_a_gap_not_zero() {
        let mut s = Series::default();
        s.push_optional(Some(0.0));
        s.push_optional(None);
        assert_eq!(s.tail(2)[0], 0.0);
        assert!(s.tail(2)[1].is_nan());
    }

    #[test]
    fn rows_hold_their_place_on_noise() {
        let mut order = vec!["a", "b", "c"];
        // b is a little busier than a: not enough to move.
        rerank(&mut order, &io(&[("a", 100.0), ("b", 115.0), ("c", 50.0)]));
        assert_eq!(order, ["a", "b", "c"]);
        // Near idle, a few IOPS of difference is still noise.
        let mut order = vec!["a", "b"];
        rerank(&mut order, &io(&[("a", 0.5), ("b", 2.0)]));
        assert_eq!(order, ["a", "b"]);
    }

    #[test]
    fn clear_changes_reorder() {
        let mut order = vec!["a", "b", "c"];
        // c clearly beats both: it climbs all the way.
        rerank(&mut order, &io(&[("a", 100.0), ("b", 90.0), ("c", 500.0)]));
        assert_eq!(order, ["c", "a", "b"]);
        // a gone, d new: d goes in at its place.
        rerank(&mut order, &io(&[("b", 90.0), ("c", 500.0), ("d", 200.0)]));
        assert_eq!(order, ["c", "d", "b"]);
        // From nothing: plain busiest-first.
        let mut order = Vec::new();
        rerank(&mut order, &io(&[("x", 1.0), ("y", 30.0), ("z", 7.0)]));
        assert_eq!(order, ["y", "z", "x"]);
    }

    #[test]
    fn smoothing() {
        let mut h = IoHistory::default();
        h.push(100.0, None, 1.0, 1);
        // The first point is taken as is.
        assert_eq!(h.smooth, 100.0);
        // A one-second spike moves the average by about a tenth of it.
        h.push(1100.0, Some(500.0), 1.0, 2);
        assert!((h.smooth - 195.2).abs() < 1.0, "{}", h.smooth);
        let before_gap = h.smooth;
        for t in 3..200 {
            h.push(f64::NAN, None, 1.0, t);
        }
        assert_eq!(h.smooth, before_gap, "missing is not idle");
        assert!(h.iops.iter().all(|v| v.is_nan()));
        h.push(0.0, None, 200.0, 200);
        assert!(h.smooth < 1e-3, "real idle measurements decay the average");
        assert_eq!(h.iops.len(), IO_CAP);
        assert_eq!(h.lat.len(), IO_CAP);
    }
}
