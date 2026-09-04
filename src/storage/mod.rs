//! Bounded storage: [`Engine`], [`Memory`], pools, and tables.
//!
//! Acquire / release / rotate is written once against [`Storage`]. Message
//! parse and encode live in [`crate::message`]. [`Engine`] methods
//! [`Engine::decode_rx`] / [`Engine::encode_tx`] (and the TX/RX mirrors)
//! glue those codecs to occupied datagram slots when the backend implements
//! [`DatagramSlots`]. [`Engine::write_rx`] associates an [`Endpoint`] sidecar
//! when RX bytes are written. Pending CON state is sidecar on TX slots
//! ([`PendingCon`]); empty ACK/RST matching is not the Dedup Table.
//! Token matching ([`ExchangeEntry`]) is a compact table keyed by Token and
//! remote [`Endpoint`], sized from the TX pool count, not a seventh area.
//! Observe interest rows ([`ObserveInterest`]) fill the existing
//! [`ObserveTable`] (Token + remote [`Endpoint`]; RFC 7641 observer-list
//! key). Classic Block1 / Block2 body assembly uses a [`BlockTransfer`]
//! sidecar on body-pool slots when block-wise is enabled. They do not
//! invent 4.02 / RST policy.
//!
//! See `design.md` and `knowledge/memory.md`.

mod block;
mod builder;
mod capacities;
mod endpoint;
mod engine;
mod exchange;
mod memory;
mod occupancy;
mod pending;
mod pool;
pub mod profiles;
mod slot;
mod table;

#[cfg(feature = "alloc")]
mod alloc_memory;

#[cfg(test)]
mod tests;

#[cfg(feature = "alloc")]
pub use alloc_memory::AllocMemory;
pub use block::{BlockKey, BlockProgress, BlockRole, BlockTransfer, OutgoingBlock};
pub use builder::{EngineBuilder, Missing, Present};
pub use capacities::Capacities;
pub use endpoint::Endpoint;
pub use engine::Engine;
pub use exchange::{ExchangeEntry, ExchangeKey, ExchangeTable, Exchanges};
pub use memory::{Memory, MemoryProfile, NoBodies, WithBodies};
pub use pending::{PendingCon, PendingCons};
pub use pool::{BodyPool, DatagramPool};
pub use slot::{SlotError, SlotId};
pub use table::{DedupEntry, DedupKey, DedupTable, ObserveInterest, ObserveKey, ObserveTable};

/// Acquire, release, and rotate occupancy for one pool or table.
///
/// Logic lives here so [`Engine`] stays generic over [`Storage`].
pub trait SlotPool {
    /// Occupy the next free slot starting at the rotating cursor.
    ///
    /// Returns `None` when every slot is occupied (saturation).
    fn acquire(&mut self) -> Option<SlotId>;

    /// Free `id` so a later acquire can reuse it. Does not move the cursor.
    fn release(&mut self, id: SlotId) -> Result<(), SlotError>;

    /// Advance the rotating cursor by one slot, wrapping at the end.
    fn rotate(&mut self);

    /// Configured slot or entry count.
    fn slot_count(&self) -> usize;

    /// How many slots are currently occupied.
    fn occupied_count(&self) -> usize;

    /// Whether `id` names an occupied slot in this pool.
    fn is_occupied(&self, id: SlotId) -> bool;

    /// Index where the next acquire search starts.
    fn cursor(&self) -> usize;
}

/// Backing store for the six core areas that are present.
///
/// RX and TX datagrams are two [`DatagramPool`] values. Body pools exist
/// only when block-wise is enabled; [`Self::rx_body`] / [`Self::tx_body`] then
/// return `None`.
///
/// See `design.md` and `knowledge/memory.md`.
pub trait Storage {
    /// Configured sizes, including absent body fields when pools are omitted.
    fn capacities(&self) -> Capacities;

    /// Incoming Datagram Pool.
    fn rx_datagram(&mut self) -> &mut dyn SlotPool;

    /// Outgoing Datagram Pool.
    fn tx_datagram(&mut self) -> &mut dyn SlotPool;

    /// Dedup table.
    fn dedup(&mut self) -> &mut dyn SlotPool;

    /// Observe interest table.
    fn observe(&mut self) -> &mut dyn SlotPool;

    /// Incoming Body Pool, if block-wise storage includes body areas.
    fn rx_body(&mut self) -> Option<&mut dyn SlotPool>;

    /// Outgoing Body Pool, if block-wise storage includes body areas.
    fn tx_body(&mut self) -> Option<&mut dyn SlotPool>;
}

/// Filled datagram bytes for occupied RX/TX slots.
///
/// [`Memory`] and [`AllocMemory`] implement this so [`Engine`] can decode
/// and encode without inventing protocol responses.
pub trait DatagramSlots {
    /// Filled RX datagram, if `id` is occupied.
    fn rx_payload(&self, id: SlotId) -> Option<&[u8]>;

    /// Filled TX datagram, if `id` is occupied.
    fn tx_payload(&self, id: SlotId) -> Option<&[u8]>;

    /// Writable RX slot buffer (full capacity).
    fn rx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]>;

    /// Writable TX slot buffer (full capacity).
    fn tx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]>;

    /// Record the filled RX datagram length.
    fn set_rx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError>;

    /// Record the filled TX datagram length.
    fn set_tx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError>;

    /// Sidecar [`Endpoint`] for an occupied RX slot.
    fn rx_endpoint(&self, id: SlotId) -> Option<Endpoint>;

    /// Sidecar [`Endpoint`] for an occupied TX slot.
    fn tx_endpoint(&self, id: SlotId) -> Option<Endpoint>;

    /// Set RX sidecar [`Endpoint`]. Not written into the byte buffer.
    fn set_rx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError>;

    /// Set TX sidecar [`Endpoint`]. Not written into the byte buffer.
    fn set_tx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError>;

    /// Pending CON Message ID on an occupied TX slot, if marked.
    fn tx_pending_mid(&self, id: SlotId) -> Option<crate::message::MessageId>;

    /// Mark occupied TX `id` as pending CON with `message_id`.
    fn set_tx_pending(
        &mut self,
        id: SlotId,
        message_id: crate::message::MessageId,
    ) -> Result<(), SlotError>;

    /// Clear the pending-CON mark on TX `id`. The slot stays occupied.
    fn clear_tx_pending(&mut self, id: SlotId) -> Result<(), SlotError>;

    /// Occupied TX slot pending for `message_id` and `endpoint`, if any.
    fn lookup_tx_pending(
        &self,
        message_id: crate::message::MessageId,
        endpoint: Endpoint,
    ) -> Option<SlotId>;
}

/// Typed Dedup Table access.
///
/// Insert, lookup, and remove scan the configured entry count (O(n) in
/// capacity). Capacity is fixed at construction. [`Memory`] and
/// [`AllocMemory`] implement this so [`Engine`] can store [`DedupEntry`]
/// values in the existing table slots.
pub trait DedupSlots {
    /// Insert `entry`, or return the existing slot if the key is present.
    ///
    /// `None` when the table is full and the key is not already stored.
    fn insert_dedup(&mut self, entry: DedupEntry) -> Option<SlotId>;

    /// Occupied slot matching `key`, if any.
    fn lookup_dedup(&self, key: DedupKey) -> Option<SlotId>;

    /// Release the slot matching `key`, if occupied.
    fn remove_dedup(&mut self, key: DedupKey) -> bool;

    /// Occupied payload at `id`.
    fn dedup_entry(&self, id: SlotId) -> Option<DedupEntry>;
}

/// Typed Observe Interest Table access.
///
/// Insert, lookup, remove, and take scan the configured entry count (O(n) in
/// capacity). Capacity is the profile observe-entry count. [`Memory`] and
/// [`AllocMemory`] implement this so [`Engine`] can store
/// [`ObserveInterest`] values in the existing table slots.
pub trait ObserveSlots {
    /// Insert `interest`, or return the existing slot if the key is present.
    ///
    /// `None` when the table is full and the key is not already stored.
    fn insert_observe(&mut self, interest: ObserveInterest) -> Option<SlotId>;

    /// Occupied slot matching `key`, if any.
    fn lookup_observe(&self, key: ObserveKey) -> Option<SlotId>;

    /// Release the slot matching `key`, if occupied.
    fn remove_observe(&mut self, key: ObserveKey) -> bool;

    /// Remove and return the occupied payload matching `key`, if any.
    fn take_observe(&mut self, key: ObserveKey) -> Option<ObserveInterest>;

    /// Occupied payload at `id`.
    fn observe_interest(&self, id: SlotId) -> Option<ObserveInterest>;
}

/// Classic Block1 / Block2 access to Incoming / Outgoing Body Pools.
///
/// Present only when block-wise is enabled. [`Memory`] without body pools and
/// `AllocMemory` built with `.block_wise(false)` return `None` /
/// [`crate::BlockTransferError::NoBodyPools`]. See `design.md`.
pub trait BodySlots {
    /// Filled incoming body, if `id` is occupied.
    fn rx_body_payload(&self, id: SlotId) -> Option<&[u8]>;

    /// Filled outgoing body, if `id` is occupied.
    fn tx_body_payload(&self, id: SlotId) -> Option<&[u8]>;

    /// Incoming Block1 sidecar, if `id` holds a transfer.
    fn rx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer>;

    /// Outgoing Block2 sidecar, if `id` holds a transfer.
    fn tx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer>;

    /// Incoming body slot matching `key`, if any. O(n) in RX body occupancy.
    fn lookup_rx_body(&self, key: BlockKey) -> Option<SlotId>;

    /// Outgoing body slot matching `key`, if any. O(n) in TX body occupancy.
    fn lookup_tx_body(&self, key: BlockKey) -> Option<SlotId>;

    /// Admit a new incoming Block1 body (first block, typically NUM 0).
    fn admit_block1(
        &mut self,
        key: BlockKey,
        block: crate::message::BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, crate::error::BlockTransferError>;

    /// Write the next in-order incoming Block1 range into `id`.
    fn write_block1(
        &mut self,
        id: SlotId,
        block: crate::message::BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, crate::error::BlockTransferError>;

    /// Admit or continue incoming Block1 for `key`.
    fn apply_block1(
        &mut self,
        key: BlockKey,
        block: crate::message::BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, crate::error::BlockTransferError>;

    /// Admit an outgoing Block2 body (complete body copied into a TX slot).
    fn start_block2(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, crate::error::BlockTransferError>;

    /// Issue the next in-order outgoing Block2 range from `id`.
    fn next_block2(
        &mut self,
        id: SlotId,
    ) -> Result<OutgoingBlock, crate::error::BlockTransferError>;
}
