//! Bounded storage: [`Engine`], [`Memory`], pools, and tables.
//!
//! Acquire / release / rotate is written once against [`Storage`]. Message
//! parse and encode live in [`crate::message`] and do not require an engine.
//!
//! See `design.md` and `knowledge/memory.md`.

mod builder;
mod capacities;
mod engine;
mod memory;
mod occupancy;
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
pub use builder::{EngineBuilder, Missing, Present};
pub use capacities::Capacities;
pub use engine::Engine;
pub use memory::{Memory, MemoryProfile, NoBodies, WithBodies};
pub use pool::{BodyPool, DatagramPool};
pub use slot::{Peer, SlotError, SlotId};
pub use table::{DedupTable, ObserveTable};

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
