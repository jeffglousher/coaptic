//! [`DedupTable`] and [`ObserveTable`]: fixed entry slots plus rotating occupancy.

use super::SlotPool;
use super::endpoint::Endpoint;
use super::occupancy::Occupancy;
use super::slot::{SlotError, SlotId};
use crate::message::{MessageId, OBSERVE_SEQUENCE_MASK, Token};

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

/// Colocated Max-Age / CON-wait deadline on an [`ObserveInterest`] row.
///
/// Absolute `due_ms` in the caller `now_ms` domain (same posture as
/// [`super::PendingRto`]). The core has no OS clock. See
/// `knowledge/rfcs/rfc7641.txt` and Max-Age in `knowledge/rfcs/rfc7252.txt`.
/// `design.md` §Bounded state-machine lifetime.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObserveLifetime {
    /// Freshness deadline from a caller-supplied Max-Age (typically client).
    MaxAge {
        /// Absolute millisecond time when this deadline fires.
        due_ms: u64,
    },
    /// CON notification waiting for an empty ACK (server client-OFF).
    Unacked {
        /// Absolute millisecond time when this deadline fires.
        due_ms: u64,
        /// Message ID of the outstanding CON notification.
        message_id: MessageId,
    },
}

impl ObserveLifetime {
    /// [`Self::MaxAge`] due at `now_ms + max_age_secs * 1000`.
    #[must_use]
    pub const fn max_age(now_ms: u64, max_age_secs: u32) -> Self {
        Self::MaxAge {
            due_ms: max_age_due_ms(now_ms, max_age_secs),
        }
    }

    /// [`Self::Unacked`] due at `now_ms + max_age_secs * 1000`.
    #[must_use]
    pub const fn unacked(now_ms: u64, max_age_secs: u32, message_id: MessageId) -> Self {
        Self::Unacked {
            due_ms: max_age_due_ms(now_ms, max_age_secs),
            message_id,
        }
    }

    /// Absolute millisecond time when this deadline fires (`now_ms` domain).
    #[must_use]
    pub const fn due_ms(self) -> u64 {
        match self {
            Self::MaxAge { due_ms } | Self::Unacked { due_ms, .. } => due_ms,
        }
    }

    /// Outstanding CON Message ID, if this is [`Self::Unacked`].
    #[must_use]
    pub const fn con_mid(self) -> Option<MessageId> {
        match self {
            Self::MaxAge { .. } => None,
            Self::Unacked { message_id, .. } => Some(message_id),
        }
    }

    /// Whether `now_ms` is at or past [`Self::due_ms`].
    #[must_use]
    pub const fn is_due(self, now_ms: u64) -> bool {
        now_ms >= self.due_ms()
    }
}

const fn max_age_due_ms(now_ms: u64, max_age_secs: u32) -> u64 {
    now_ms.saturating_add((max_age_secs as u64).saturating_mul(1_000))
}

/// One Observe interest whose colocated lifetime is due.
///
/// The row stays occupied. The caller drops it or stops notifying. The core
/// does not send RST or invent 4.02. See `knowledge/rfcs/rfc7641.txt`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObserveExpiry {
    /// [`ObserveLifetime::MaxAge`] elapsed.
    MaxAge(SlotId),
    /// [`ObserveLifetime::Unacked`] elapsed (CON notify without ACK).
    ClientOff(SlotId),
}

impl ObserveExpiry {
    /// Observe table slot that is due.
    #[must_use]
    pub const fn slot(self) -> SlotId {
        match self {
            Self::MaxAge(id) | Self::ClientOff(id) => id,
        }
    }
}

/// Occupied Observe Interest Table payload.
///
/// Relation identity plus small pending/coalescing state, the last
/// library-assigned 24-bit notification sequence, and optional Max-Age /
/// CON-wait lifetime. Notification bodies are not stored here. See
/// `design.md` §Observe Interest Table / Bounded state-machine lifetime and
/// `knowledge/rfcs/rfc7641.txt`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObserveInterest {
    key: ObserveKey,
    seq: u32,
    pending: bool,
    lifetime: Option<ObserveLifetime>,
}

impl ObserveInterest {
    /// Interest row for `token` at `endpoint`.
    ///
    /// Sequence starts at 0, is not pending, and has no lifetime.
    #[must_use]
    pub const fn new(token: Token, endpoint: Endpoint) -> Self {
        Self {
            key: ObserveKey::new(token, endpoint),
            seq: 0,
            pending: false,
            lifetime: None,
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

    /// Last library-assigned notification sequence (24-bit).
    ///
    /// Zero after register, before the first [`Self::take_due`]. See
    /// `knowledge/rfcs/rfc7641.txt`.
    #[must_use]
    pub const fn seq(self) -> u32 {
        self.seq
    }

    /// Whether a notification is waiting to be surfaced by progress.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        self.pending
    }

    /// Set the last-assigned sequence (masked to 24 bits).
    #[must_use]
    pub const fn with_seq(self, seq: u32) -> Self {
        Self {
            seq: seq & OBSERVE_SEQUENCE_MASK,
            ..self
        }
    }

    /// Colocated Max-Age / CON-wait lifetime, if set.
    #[must_use]
    pub const fn lifetime(self) -> Option<ObserveLifetime> {
        self.lifetime
    }

    /// Replace the colocated lifetime (or clear it with `None`).
    #[must_use]
    pub const fn with_lifetime(self, lifetime: Option<ObserveLifetime>) -> Self {
        Self { lifetime, ..self }
    }

    /// Whether `now_ms` is at or past a colocated lifetime deadline.
    #[must_use]
    pub const fn is_lifetime_due(self, now_ms: u64) -> bool {
        match self.lifetime {
            Some(life) => life.is_due(now_ms),
            None => false,
        }
    }

    /// If the colocated lifetime is due, clear it and pending, return it.
    ///
    /// Surfaces once. The row stays occupied. See
    /// `knowledge/rfcs/rfc7641.txt`.
    pub fn take_expired(&mut self, now_ms: u64) -> Option<ObserveLifetime> {
        let life = self.lifetime?;
        if !life.is_due(now_ms) {
            return None;
        }
        self.lifetime = None;
        self.pending = false;
        Some(life)
    }

    /// Mark this row as needing one notification.
    ///
    /// A second mark before [`Self::take_due`] still yields one work item.
    pub fn mark_due(&mut self) {
        self.pending = true;
    }

    /// If pending, assign the next 24-bit sequence, clear pending, return it.
    ///
    /// Wraps through [`OBSERVE_SEQUENCE_MASK`]. See
    /// `knowledge/rfcs/rfc7641.txt`.
    pub fn take_due(&mut self) -> Option<u32> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        self.seq = self.seq.wrapping_add(1) & OBSERVE_SEQUENCE_MASK;
        Some(self.seq)
    }
}

impl From<ObserveKey> for ObserveInterest {
    fn from(key: ObserveKey) -> Self {
        Self::new(key.token(), key.endpoint())
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
    fn set_entry(&mut self, id: SlotId, interest: ObserveInterest) -> Result<(), SlotError>;
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

    fn set_entry(&mut self, id: SlotId, interest: ObserveInterest) -> Result<(), SlotError> {
        ObserveTable::set_entry(self, id, interest)
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
