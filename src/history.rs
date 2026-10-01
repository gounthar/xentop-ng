//! Fixed-size time series feeding the graphs.

use crate::model::Rates;
use std::collections::{HashMap, VecDeque};

pub const CAP: usize = 1024;

#[derive(Default, Clone)]
pub struct Series(VecDeque<f64>);

impl Series {
    pub fn push(&mut self, v: f64) {
        if self.0.len() == CAP {
            self.0.pop_front();
        }
        self.0.push_back(if v.is_finite() { v } else { 0.0 });
    }
    /// The most recent `n` points, oldest first.
    pub fn tail(&self, n: usize) -> Vec<f64> {
        let skip = self.0.len().saturating_sub(n);
        self.0.iter().skip(skip).copied().collect()
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
        self.rd.push(h.disk_rd_bps);
        self.wr.push(h.disk_wr_bps);
        self.riops.push(h.disk_rd_iops);
        self.wiops.push(h.disk_wr_iops);
        self.rlat.push(h.disk_rd_lat_us.unwrap_or(0.0));
        self.wlat.push(h.disk_wr_lat_us.unwrap_or(0.0));

        for d in &r.domains {
            let e = self.doms.entry(d.id).or_default();
            if e.name != d.name {
                *e = DomHistory {
                    name: d.name.clone(),
                    ..Default::default()
                };
            }
            e.last_seen = self.tick;
            e.cpu.push(d.cpu_pct);
            e.rx.push(d.net_rx_bps);
            e.tx.push(d.net_tx_bps);
            e.rd.push(d.disk_rd_bps);
            e.wr.push(d.disk_wr_bps);
            e.lat.push(d.lat_us().unwrap_or(0.0));
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
    }
}
