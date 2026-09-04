//! Pending CON sidecar on outgoing datagram slots.
//!
//! Confirm matching is Message ID plus remote [`Endpoint`]. This is not the
//! Dedup Table. See `design.md` (Outgoing Datagram Pool) and
//! `knowledge/rfcs/rfc7252.txt` §4.4.

use super::endpoint::Endpoint;
use super::slot::SlotId;
use crate::message::MessageId;

/// Occupied TX slot waiting for an empty ACK or RST.
///
/// Stored as sidecar on the outgoing datagram slot. Capacity is the TX
/// Datagram Pool. This is not a seventh core memory area.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PendingCon {
    message_id: MessageId,
    endpoint: Endpoint,
    tx_slot: SlotId,
}

impl PendingCon {
    /// Pending CON on `tx_slot` for `message_id` sent to `endpoint`.
    #[must_use]
    pub const fn new(message_id: MessageId, endpoint: Endpoint, tx_slot: SlotId) -> Self {
        Self {
            message_id,
            endpoint,
            tx_slot,
        }
    }

    /// Message ID of the outgoing CON.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.message_id
    }

    /// Remote endpoint the CON was sent to.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.endpoint
    }

    /// TX datagram slot that holds the outgoing bytes.
    #[must_use]
    pub const fn tx_slot(self) -> SlotId {
        self.tx_slot
    }
}

/// Typed pending-CON access on the outgoing datagram pool.
///
/// Insert, lookup, and take scan the configured TX slot count (O(n) in
/// capacity). Capacity is the TX Datagram Pool: a slot must already be
/// occupied. [`super::Memory`] and [`super::AllocMemory`] implement this.
pub trait PendingCons {
    /// Mark occupied TX `id` as pending (sets sidecar [`Endpoint`] and MID).
    ///
    /// `None` when `id` is free or out of range (saturation of the TX pool,
    /// or a slot that was never acquired). Idempotent when the same MID and
    /// endpoint are already recorded.
    fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<SlotId>;

    /// Occupied TX slot pending for `message_id` and `endpoint`, if any.
    fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId>;

    /// Clear pending for the matching TX slot. The slot stays occupied.
    fn take_pending_con(&mut self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId>;

    /// Pending CON view at TX `id`, if that slot is marked pending.
    fn pending_con(&self, id: SlotId) -> Option<PendingCon>;
}
