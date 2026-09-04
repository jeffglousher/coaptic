//! [`Engine`]: acquire / release / rotate against [`Storage`].

use crate::capacities::Capacities;
use crate::slot::{SlotError, SlotId};
use crate::storage::Storage;

/// Protocol engine, generic over [`Storage`].
///
/// This crate currently implements the storage layer only: occupancy,
/// acquire/release, and rotating cursors. Protocol state machines are not
/// implemented.
///
/// See `design.md` and `knowledge/memory.md`.
#[derive(Debug)]
pub struct Engine<S: Storage> {
    storage: S,
}

impl<S: Storage> Engine<S> {
    pub(crate) fn from_storage(storage: S) -> Self {
        Self { storage }
    }

    /// Borrow the backing store.
    #[must_use]
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Borrow the backing store mutably.
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    /// Configured sizes of the backing store.
    #[must_use]
    pub fn capacities(&self) -> Capacities {
        self.storage.capacities()
    }

    /// Whether this engine’s Storage includes body pools.
    #[must_use]
    pub fn has_body_pools(&self) -> bool {
        self.storage.capacities().has_body_pools()
    }

    /// Acquire one RX datagram slot, or `None` if the pool is saturated.
    pub fn acquire_rx(&mut self) -> Option<SlotId> {
        self.storage.rx_datagram().acquire()
    }

    /// Release an RX datagram slot.
    pub fn release_rx(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage.rx_datagram().release(id)
    }

    /// Advance the RX datagram rotating cursor.
    pub fn rotate_rx(&mut self) {
        self.storage.rx_datagram().rotate();
    }

    /// Acquire one TX datagram slot, or `None` if the pool is saturated.
    pub fn acquire_tx(&mut self) -> Option<SlotId> {
        self.storage.tx_datagram().acquire()
    }

    /// Release a TX datagram slot.
    pub fn release_tx(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage.tx_datagram().release(id)
    }

    /// Advance the TX datagram rotating cursor.
    pub fn rotate_tx(&mut self) {
        self.storage.tx_datagram().rotate();
    }

    /// Acquire one RX body slot. `None` if body pools are absent or saturated.
    pub fn acquire_rx_body(&mut self) -> Option<SlotId> {
        self.storage.rx_body()?.acquire()
    }

    /// Release an RX body slot.
    pub fn release_rx_body(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage
            .rx_body()
            .ok_or(SlotError::InvalidSlot)?
            .release(id)
    }

    /// Advance the RX body rotating cursor. No-op when body pools are absent.
    pub fn rotate_rx_body(&mut self) {
        if let Some(pool) = self.storage.rx_body() {
            pool.rotate();
        }
    }

    /// Acquire one TX body slot. `None` if body pools are absent or saturated.
    pub fn acquire_tx_body(&mut self) -> Option<SlotId> {
        self.storage.tx_body()?.acquire()
    }

    /// Release a TX body slot.
    pub fn release_tx_body(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage
            .tx_body()
            .ok_or(SlotError::InvalidSlot)?
            .release(id)
    }

    /// Advance the TX body rotating cursor. No-op when body pools are absent.
    pub fn rotate_tx_body(&mut self) {
        if let Some(pool) = self.storage.tx_body() {
            pool.rotate();
        }
    }

    /// Acquire one dedup entry.
    pub fn acquire_dedup(&mut self) -> Option<SlotId> {
        self.storage.dedup().acquire()
    }

    /// Release a dedup entry.
    pub fn release_dedup(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage.dedup().release(id)
    }

    /// Advance the dedup rotating cursor.
    pub fn rotate_dedup(&mut self) {
        self.storage.dedup().rotate();
    }

    /// Acquire one observe entry.
    pub fn acquire_observe(&mut self) -> Option<SlotId> {
        self.storage.observe().acquire()
    }

    /// Release an observe entry.
    pub fn release_observe(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.storage.observe().release(id)
    }

    /// Advance the observe rotating cursor.
    pub fn rotate_observe(&mut self) {
        self.storage.observe().rotate();
    }

    /// Occupied RX datagram slots.
    pub fn rx_occupied(&mut self) -> usize {
        self.storage.rx_datagram().occupied_count()
    }

    /// Occupied TX datagram slots.
    pub fn tx_occupied(&mut self) -> usize {
        self.storage.tx_datagram().occupied_count()
    }

    /// RX datagram cursor (next acquire search start).
    pub fn rx_cursor(&mut self) -> usize {
        self.storage.rx_datagram().cursor()
    }

    /// TX datagram cursor (next acquire search start).
    pub fn tx_cursor(&mut self) -> usize {
        self.storage.tx_datagram().cursor()
    }

    /// RX body cursor, if body pools exist.
    pub fn rx_body_cursor(&mut self) -> Option<usize> {
        self.storage.rx_body().map(|pool| pool.cursor())
    }
}
