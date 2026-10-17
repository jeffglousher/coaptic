//! Caller-owned snapshots of the six core storage areas.

use super::SlotPool;

/// Current occupancy of one bounded area, including pinned slots.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PoolUsage {
    /// Slots currently reserved, including empty acquired slots.
    pub occupied: usize,
    /// Total configured slots.
    pub capacity: usize,
}

impl PoolUsage {
    pub(crate) fn read(pool: &dyn SlotPool) -> Self {
        Self {
            occupied: pool.occupied_count(),
            capacity: pool.slot_count(),
        }
    }
}

/// Instantaneous storage occupancy, with no retained counters or allocation.
///
/// Reads existing O(1) occupancy counters. Taking a snapshot does not acquire,
/// release, rotate or reset anything. Body areas are `None` when absent, distinct
/// from an enabled empty pool. Callers may retain maxima between samples; these
/// are sampled maxima, not proof of a lifetime high-water mark.
///
/// Counts describe slots, not bytes or total RAM. They exclude App routes,
/// Calls, security associations, sidecar protocol state and transport storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageUsage {
    /// Incoming datagrams.
    pub rx_datagram: PoolUsage,
    /// Outgoing datagrams.
    pub tx_datagram: PoolUsage,
    /// Duplicate suppression entries.
    pub dedup: PoolUsage,
    /// Observation interests.
    pub observe: PoolUsage,
    /// Incoming complete bodies, when enabled.
    pub rx_body: Option<PoolUsage>,
    /// Outgoing complete bodies, when enabled.
    pub tx_body: Option<PoolUsage>,
}
