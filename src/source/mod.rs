pub mod demo;
pub mod xenstat;

use crate::model::Snapshot;

/// Where snapshots come from: the real hypervisor or a simulation.
pub trait Source {
    fn sample(&mut self) -> anyhow::Result<Snapshot>;
    /// Short description shown in the header (e.g. "libxenstat 4.17 +ext").
    fn describe(&self) -> String;
    /// Snapshots covering the `secs` seconds before now, so graphs start
    /// full. Only a simulation can produce these.
    fn warmup(&mut self, _secs: u32) -> Vec<Snapshot> {
        Vec::new()
    }
}
