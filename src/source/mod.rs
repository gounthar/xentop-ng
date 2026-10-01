pub mod demo;
pub mod fallback;
pub mod xenstat;

use crate::model::Snapshot;

/// Where a class of metrics currently comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Avail {
    /// From libxenstat itself.
    #[default]
    Lib,
    /// Collected by xentop-ng because libxenstat lacks it.
    Fallback,
    /// Not available at all.
    Missing,
    /// Nothing to report on this host (e.g. no tapdisk3 disks).
    NotApplicable,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DataStatus {
    pub pcpu: Avail,
    pub vbd_latency: Avail,
    pub vifs: Avail,
    /// Steal time (vCPUs runnable but not running). Needs a hypervisor
    /// patch, not just libxenstat, so it is left out of the header's
    /// partial/fallback marker; the `i` popup still shows it.
    pub steal: Avail,
}

impl DataStatus {
    pub fn degraded(&self) -> bool {
        [self.pcpu, self.vbd_latency, self.vifs].contains(&Avail::Missing)
    }
    pub fn uses_fallback(&self) -> bool {
        [self.pcpu, self.vbd_latency, self.vifs].contains(&Avail::Fallback)
    }
}

/// Where snapshots come from: the real hypervisor or a simulation.
pub trait Source {
    fn sample(&mut self) -> anyhow::Result<Snapshot>;
    /// Short description shown in the header (e.g. "libxenstat 4.17 +ext").
    fn describe(&self) -> String;
    /// Which metrics are complete, rebuilt by fallbacks, or missing.
    fn status(&self) -> DataStatus {
        DataStatus::default()
    }
    /// Snapshots covering the `secs` seconds before now, so graphs start
    /// full. Only a simulation can produce these.
    fn warmup(&mut self, _secs: u32) -> Vec<Snapshot> {
        Vec::new()
    }
}
