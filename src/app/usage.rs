use crate::storage::{PoolUsage, StorageUsage};

/// Caller-owned snapshot of App admission storage.
///
/// Scans bounded tables per sample, without allocation or retained state.
/// Does not advance time, expire cooldowns, release slots or perform IO. Counts
/// describe the currently retained state, including entries awaiting a poll.
/// No peer identifiers, credentials or payloads are included.
///
/// These are separate capacity constraints, not additive RAM measurements:
/// one Call can occupy both client tables and packet/body storage. A free slot
/// does not guarantee admission; congestion, policy and other resources apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppUsage {
    /// Six core storage areas.
    pub storage: StorageUsage,
    /// Registered paths (multiple methods on one path occupy one slot).
    pub routes: PoolUsage,
    /// Retained client request identities, including cooldown-only rows.
    pub client_requests: PoolUsage,
    /// Subset of client request rows retained solely for overload cooldown.
    pub client_cooldowns: usize,
    /// Reserved client reply rows, including replies not yet complete.
    pub client_replies: PoolUsage,
    /// Subset of reply rows with a completion available for collection.
    pub completed_replies: usize,
    /// Deferred server work retained until completion, cancellation or expiry.
    pub deferred: PoolUsage,
    /// Installed security context slots, including the primary context and
    /// revoked associations awaiting pinned-storage cleanup. `None` when the
    /// OSCORE feature is absent; otherwise capacity may explicitly be zero.
    pub security: Option<PoolUsage>,
    /// Subset of installed contexts revoked but not yet reclaimed.
    pub revoked_associations: usize,
}
