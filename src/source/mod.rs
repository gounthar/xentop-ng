pub mod demo;
mod dl;
pub mod fallback;
mod gaps;
pub mod xapi;
pub mod xenstat;
pub mod xenstore;

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

/// Whether names come from xapi (XCP-ng/XenServer toolstack).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum XapiState {
    /// No xapi on this host (plain Xen): UUIDs only, as usual.
    #[default]
    Absent,
    /// Turned off with --no-xapi.
    Disabled,
    Connecting,
    Connected,
    /// xapi is there but unusable, and why.
    Failed(String),
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DataStatus {
    pub pcpu: Avail,
    pub vbd_latency: Avail,
    pub vifs: Avail,
    /// VBD -> SR/VDI mapping, VM UUIDs and balloon targets. libxenstat
    /// never has these: `Fallback` means read from xenstore.
    pub storage: Avail,
    /// Steal time (vCPUs runnable but not running). Needs a hypervisor
    /// patch, not just libxenstat, so it is left out of the header's
    /// partial/fallback marker; the `i` popup still shows it.
    pub steal: Avail,
    /// SR/VDI/network names. Extra, never a gap: not part of `degraded`.
    pub xapi: XapiState,
}

impl DataStatus {
    pub fn degraded(&self) -> bool {
        [self.pcpu, self.vbd_latency, self.vifs, self.storage].contains(&Avail::Missing)
    }
    /// Storage mapping always comes from xenstore, so it doesn't count:
    /// this flags gaps in libxenstat only.
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
