//! [`DedupTable`] and [`ObserveTable`]: fixed entry slots plus rotating occupancy.

use super::SlotPool;
use super::endpoint::Endpoint;
use super::occupancy::Occupancy;
use super::slot::{SlotError, SlotId};
use crate::message::{MessageId, Token};

/// Lookup identity for one Dedup Table row.
///
/// Compact duplicate history is Message ID plus remote [`Endpoint`]. Token
/// matching is not this table. See `knowledge/rfcs/rfc7252.txt` §4.5 and
/// `design.md`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DedupKey {
    message_id: MessageId,
    endpoint: Endpoint,
}

impl DedupKey {
    /// Identity for one remote Message ID.
    #[must_use]
    pub const fn new(message_id: MessageId, endpoint: Endpoint) -> Self {
        Self {
            message_id,
            endpoint,
        }
    }

    /// Message ID.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.message_id
    }

    /// Remote endpoint that sent the message.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.endpoint
    }
}

/// Occupied Dedup Table payload.
///
/// Stored in the table slot itself. Timing and retransmission state are not
/// modeled here.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DedupEntry {
    key: DedupKey,
}

impl DedupEntry {
    /// History row for `message_id` from `endpoint`.
    #[must_use]
    pub const fn new(message_id: MessageId, endpoint: Endpoint) -> Self {
        Self {
            key: DedupKey::new(message_id, endpoint),
        }
    }

    /// Lookup identity.
    #[must_use]
    pub const fn key(self) -> DedupKey {
        self.key
    }

    /// Message ID.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.key.message_id()
    }

    /// Remote endpoint.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.key.endpoint()
    }
}

impl From<DedupKey> for DedupEntry {
    fn from(key: DedupKey) -> Self {
        Self { key }
    }
}

/// Typed insert / lookup / remove on a Dedup Table.
///
/// Each operation scans the configured entry count (O(n) in capacity). Capacity
/// is fixed at construction.
pub(crate) trait DedupStore {
    fn insert(&mut self, entry: DedupEntry) -> Option<SlotId>;
    fn lookup(&self, key: DedupKey) -> Option<SlotId>;
    fn remove(&mut self, key: DedupKey) -> bool;
    fn entry(&self, id: SlotId) -> Option<DedupEntry>;
}

/// Lookup identity for one Observe Interest Table row.
///
/// RFC 7641 keys the observer list by client endpoint and Token. Resource
/// path is not part of that key. This is not Dedup (Message ID + Endpoint),
/// not pending CON, and not [`super::ExchangeKey`] (same Token + Endpoint
/// pair, different table). See `knowledge/rfcs/rfc7641.txt` and `design.md`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObserveKey {
    token: Token,
    endpoint: Endpoint,
}

impl ObserveKey {
    /// Identity for one Token at `endpoint`.
    #[must_use]
    pub const fn new(token: Token, endpoint: Endpoint) -> Self {
        Self { token, endpoint }
    }

    /// Client Token.
    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    /// Client endpoint.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.endpoint
    }
}

/// Occupied Observe Interest Table payload.
///
/// Long-lived relation state only. Notification bodies are not stored here.
/// Timing, freshness, and fan-out are not modeled yet.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObserveInterest {
    key: ObserveKey,
}

impl ObserveInterest {
    /// Interest row for `token` at `endpoint`.
    #[must_use]
    pub const fn new(token: Token, endpoint: Endpoint) -> Self {
        Self {
            key: ObserveKey::new(token, endpoint),
        }
    }

    /// Lookup identity.
    #[must_use]
    pub const fn key(self) -> ObserveKey {
        self.key
    }

    /// Client Token.
    #[must_use]
    pub const fn token(self) -> Token {
        self.key.token()
    }

    /// Client endpoint.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.key.endpoint()
    }
}

impl From<ObserveKey> for ObserveInterest {
    fn from(key: ObserveKey) -> Self {
        Self { key }
    }
}

/// Typed insert / lookup / remove / take on an Observe Interest Table.
///
/// Each operation scans the configured entry count (O(n) in capacity).
pub(crate) trait ObserveStore {
    fn insert(&mut self, interest: ObserveInterest) -> Option<SlotId>;
    fn lookup(&self, key: ObserveKey) -> Option<SlotId>;
    fn remove(&mut self, key: ObserveKey) -> bool;
    fn take(&mut self, key: ObserveKey) -> Option<ObserveInterest>;
    fn entry(&self, id: SlotId) -> Option<ObserveInterest>;
}

/// Dedup table: compact duplicate history slots.
///
/// Insert, lookup, and remove scan every configured entry (O(n) in capacity).
/// Saturation returns `None` from insert; this type does not evict. [`Self::rotate`]
/// still advances the occupancy cursor used by acquire.
pub struct DedupTable<const ENTRIES: usize> {
    entries: [Option<DedupEntry>; ENTRIES],
    occ: Occupancy<ENTRIES>,
}

impl<const ENTRIES: usize> DedupTable<ENTRIES> {
    /// Empty table. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [None; ENTRIES],
            occ: Occupancy::new(),
        }
    }

    /// Insert `entry`, or return the existing slot if the key is already present.
    ///
    /// Returns `None` when the table is full and the key is not already stored.
    /// Scans the configured capacity (O(n)).
    pub fn insert(&mut self, entry: DedupEntry) -> Option<SlotId> {
        if let Some(id) = self.lookup(entry.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(entry);
        Some(id)
    }

    /// Slot whose occupied payload matches `key`, if any.
    ///
    /// Scans the configured capacity (O(n)).
    #[must_use]
    pub fn lookup(&self, key: DedupKey) -> Option<SlotId> {
        (0..ENTRIES).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(entry) if entry.key() == key => Some(id),
                _ => None,
            }
        })
    }

    /// Release the slot matching `key`, if occupied.
    ///
    /// Scans the configured capacity (O(n)).
    pub fn remove(&mut self, key: DedupKey) -> bool {
        match self.lookup(key) {
            Some(id) => self.release(id).is_ok(),
            None => false,
        }
    }

    /// Occupied payload at `id`.
    #[must_use]
    pub fn entry(&self, id: SlotId) -> Option<DedupEntry> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }

    /// Write payload into an already-acquired slot.
    pub fn set_entry(&mut self, id: SlotId, entry: DedupEntry) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < ENTRIES {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.entries[id.index()] = Some(entry);
        Ok(())
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl<const ENTRIES: usize> Default for DedupTable<ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ENTRIES: usize> DedupStore for DedupTable<ENTRIES> {
    fn insert(&mut self, entry: DedupEntry) -> Option<SlotId> {
        DedupTable::insert(self, entry)
    }

    fn lookup(&self, key: DedupKey) -> Option<SlotId> {
        DedupTable::lookup(self, key)
    }

    fn remove(&mut self, key: DedupKey) -> bool {
        DedupTable::remove(self, key)
    }

    fn entry(&self, id: SlotId) -> Option<DedupEntry> {
        DedupTable::entry(self, id)
    }
}

impl<const ENTRIES: usize> SlotPool for DedupTable<ENTRIES> {
    fn acquire(&mut self) -> Option<SlotId> {
        let id = self.occ.acquire()?;
        self.reset(id);
        Some(id)
    }

    fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.occ.release(id)?;
        self.reset(id);
        Ok(())
    }

    fn rotate(&mut self) {
        self.occ.rotate();
    }

    fn slot_count(&self) -> usize {
        self.occ.slot_count()
    }

    fn occupied_count(&self) -> usize {
        self.occ.occupied_count()
    }

    fn is_occupied(&self, id: SlotId) -> bool {
        self.occ.is_occupied(id)
    }

    fn cursor(&self) -> usize {
        self.occ.cursor()
    }
}

/// Observe interest table: long-lived relation slots, not notification bodies.
///
/// Insert, lookup, remove, and take scan every configured entry (O(n) in
/// capacity). Saturation returns `None` from insert; this type does not evict.
/// [`Self::rotate`] still advances the occupancy cursor used by acquire.
pub struct ObserveTable<const ENTRIES: usize> {
    entries: [Option<ObserveInterest>; ENTRIES],
    occ: Occupancy<ENTRIES>,
}

impl<const ENTRIES: usize> ObserveTable<ENTRIES> {
    /// Empty table. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [None; ENTRIES],
            occ: Occupancy::new(),
        }
    }

    /// Insert `interest`, or return the existing slot if the key is already present.
    ///
    /// Returns `None` when the table is full and the key is not already stored.
    /// Scans the configured capacity (O(n)).
    pub fn insert(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        if let Some(id) = self.lookup(interest.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(interest);
        Some(id)
    }

    /// Slot whose occupied payload matches `key`, if any.
    ///
    /// Scans the configured capacity (O(n)).
    #[must_use]
    pub fn lookup(&self, key: ObserveKey) -> Option<SlotId> {
        (0..ENTRIES).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(interest) if interest.key() == key => Some(id),
                _ => None,
            }
        })
    }

    /// Release the slot matching `key`, if occupied.
    ///
    /// Scans the configured capacity (O(n)).
    pub fn remove(&mut self, key: ObserveKey) -> bool {
        match self.lookup(key) {
            Some(id) => self.release(id).is_ok(),
            None => false,
        }
    }

    /// Remove and return the occupied payload matching `key`, if any.
    ///
    /// Scans the configured capacity (O(n)).
    pub fn take(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        let id = self.lookup(key)?;
        let interest = self.entry(id)?;
        let _ = self.release(id);
        Some(interest)
    }

    /// Occupied payload at `id`.
    #[must_use]
    pub fn entry(&self, id: SlotId) -> Option<ObserveInterest> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }

    /// Write payload into an already-acquired slot.
    pub fn set_entry(&mut self, id: SlotId, interest: ObserveInterest) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < ENTRIES {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.entries[id.index()] = Some(interest);
        Ok(())
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl<const ENTRIES: usize> Default for ObserveTable<ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ENTRIES: usize> ObserveStore for ObserveTable<ENTRIES> {
    fn insert(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        ObserveTable::insert(self, interest)
    }

    fn lookup(&self, key: ObserveKey) -> Option<SlotId> {
        ObserveTable::lookup(self, key)
    }

    fn remove(&mut self, key: ObserveKey) -> bool {
        ObserveTable::remove(self, key)
    }

    fn take(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        ObserveTable::take(self, key)
    }

    fn entry(&self, id: SlotId) -> Option<ObserveInterest> {
        ObserveTable::entry(self, id)
    }
}

impl<const ENTRIES: usize> SlotPool for ObserveTable<ENTRIES> {
    fn acquire(&mut self) -> Option<SlotId> {
        let id = self.occ.acquire()?;
        self.reset(id);
        Some(id)
    }

    fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.occ.release(id)?;
        self.reset(id);
        Ok(())
    }

    fn rotate(&mut self) {
        self.occ.rotate();
    }

    fn slot_count(&self) -> usize {
        self.occ.slot_count()
    }

    fn occupied_count(&self) -> usize {
        self.occ.occupied_count()
    }

    fn is_occupied(&self, id: SlotId) -> bool {
        self.occ.is_occupied(id)
    }

    fn cursor(&self) -> usize {
        self.occ.cursor()
    }
}
