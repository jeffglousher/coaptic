//! [`Engine`]: acquire / release / rotate against [`Storage`].

use super::Capacities;
use super::DatagramSlots;
use super::DedupEntry;
use super::DedupKey;
use super::DedupSlots;
use super::Endpoint;
use super::PendingCon;
use super::PendingCons;
use super::SlotError;
use super::SlotId;
use super::Storage;
use crate::error::SlotMessageError;
use crate::message::{Message, MessageId, ParsedMessage};

/// Protocol engine, generic over [`Storage`].
///
/// Storage engine: occupancy, acquire/release, and rotating cursors.
/// Protocol state machines are not implemented.
///
/// When `S` implements [`DatagramSlots`], [`Self::decode_rx`] /
/// [`Self::encode_tx`] (and the TX/RX mirrors) call [`crate::message`]
/// against occupied datagram slots and record `set_len` on encode.
/// [`Self::write_rx`] copies bytes and sets the sidecar [`Endpoint`].
/// When `S` implements [`DedupSlots`], insert / lookup / remove store
/// [`DedupEntry`] values in the Dedup Table (O(n) in configured capacity).
/// When `S` implements [`PendingCons`], outgoing CON slots are marked
/// pending on the TX datagram sidecar and matched against empty ACK/RST.
/// Dedup and pending CON are different identities. Optional format and
/// unrecognized-critical checks stay on [`ParsedMessage`].
/// This type does not invent 4.02 / RST policy.
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

impl<S: Storage + DatagramSlots> Engine<S> {
    /// Decode the occupied RX datagram with [`crate::message::decode`].
    ///
    /// Does not run [`ParsedMessage::check_rfc7252_options`] or
    /// [`ParsedMessage::check_rfc7252_formats`].
    pub fn decode_rx(&self, id: SlotId) -> Result<ParsedMessage<'_>, SlotMessageError> {
        decode_occupied(self.storage.rx_payload(id))
    }

    /// Decode the occupied TX datagram with [`crate::message::decode`].
    pub fn decode_tx(&self, id: SlotId) -> Result<ParsedMessage<'_>, SlotMessageError> {
        decode_occupied(self.storage.tx_payload(id))
    }

    /// Encode `msg` into an acquired RX slot and [`DatagramSlots::set_rx_len`].
    pub fn encode_rx(&mut self, id: SlotId, msg: &Message<'_>) -> Result<usize, SlotMessageError> {
        let n = encode_occupied(self.storage.rx_payload_mut(id), msg)?;
        self.storage.set_rx_len(id, n)?;
        Ok(n)
    }

    /// Encode `msg` into an acquired TX slot and [`DatagramSlots::set_tx_len`].
    pub fn encode_tx(&mut self, id: SlotId, msg: &Message<'_>) -> Result<usize, SlotMessageError> {
        let n = encode_occupied(self.storage.tx_payload_mut(id), msg)?;
        self.storage.set_tx_len(id, n)?;
        Ok(n)
    }

    /// Copy `bytes` into an acquired RX slot, record length, and set sidecar [`Endpoint`].
    pub fn write_rx(
        &mut self,
        id: SlotId,
        bytes: &[u8],
        endpoint: Endpoint,
    ) -> Result<usize, SlotError> {
        copy_into_slot(self.storage.rx_payload_mut(id), bytes)?;
        self.storage.set_rx_len(id, bytes.len())?;
        self.storage.set_rx_endpoint(id, endpoint)?;
        Ok(bytes.len())
    }

    /// Copy `bytes` into an acquired TX slot, record length, and set sidecar [`Endpoint`].
    pub fn write_tx(
        &mut self,
        id: SlotId,
        bytes: &[u8],
        endpoint: Endpoint,
    ) -> Result<usize, SlotError> {
        copy_into_slot(self.storage.tx_payload_mut(id), bytes)?;
        self.storage.set_tx_len(id, bytes.len())?;
        self.storage.set_tx_endpoint(id, endpoint)?;
        Ok(bytes.len())
    }

    /// Sidecar [`Endpoint`] for an occupied RX slot.
    #[must_use]
    pub fn rx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        self.storage.rx_endpoint(id)
    }

    /// Sidecar [`Endpoint`] for an occupied TX slot.
    #[must_use]
    pub fn tx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        self.storage.tx_endpoint(id)
    }

    /// Set RX sidecar [`Endpoint`]. Not written into the byte buffer.
    pub fn set_rx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        self.storage.set_rx_endpoint(id, endpoint)
    }

    /// Set TX sidecar [`Endpoint`]. Not written into the byte buffer.
    pub fn set_tx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        self.storage.set_tx_endpoint(id, endpoint)
    }
}

impl<S: Storage + DedupSlots> Engine<S> {
    /// Insert a Dedup Table row, or return the existing slot if the key is present.
    ///
    /// `None` when the table is full and the key is not already stored. Scans
    /// the configured capacity (O(n)).
    pub fn insert_dedup(&mut self, entry: DedupEntry) -> Option<SlotId> {
        self.storage.insert_dedup(entry)
    }

    /// Occupied Dedup Table slot matching `key`, if any. O(n) in capacity.
    #[must_use]
    pub fn lookup_dedup(&self, key: DedupKey) -> Option<SlotId> {
        self.storage.lookup_dedup(key)
    }

    /// Release the Dedup Table slot matching `key`, if occupied. O(n) in capacity.
    pub fn remove_dedup(&mut self, key: DedupKey) -> bool {
        self.storage.remove_dedup(key)
    }

    /// Occupied Dedup Table payload at `id`.
    #[must_use]
    pub fn dedup_entry(&self, id: SlotId) -> Option<DedupEntry> {
        self.storage.dedup_entry(id)
    }
}

impl<S: Storage + PendingCons> Engine<S> {
    /// Record occupied TX `id` as a pending CON (`message_id` + `endpoint`).
    ///
    /// Sets the TX sidecar endpoint. `None` when the slot is free or out of
    /// range (TX pool saturation, or never acquired). Idempotent for the same
    /// MID and endpoint. Does not use the Dedup Table.
    pub fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<SlotId> {
        self.storage.record_pending_con(id, endpoint, message_id)
    }

    /// Occupied TX slot pending for `message_id` and `endpoint`, if any. O(n).
    #[must_use]
    pub fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        self.storage.lookup_pending_con(message_id, endpoint)
    }

    /// Clear pending for the matching TX slot. The slot stays occupied.
    ///
    /// The caller releases the returned slot. `None` on miss.
    pub fn take_pending_con(
        &mut self,
        message_id: MessageId,
        endpoint: Endpoint,
    ) -> Option<SlotId> {
        self.storage.take_pending_con(message_id, endpoint)
    }

    /// Pending CON view at TX `id`, if that slot is marked pending.
    #[must_use]
    pub fn pending_con(&self, id: SlotId) -> Option<PendingCon> {
        self.storage.pending_con(id)
    }

    /// If `parsed` is an empty ACK or RST, take the matching pending CON.
    ///
    /// Match is Message ID plus `endpoint`. Returns the TX slot id; the
    /// caller releases it. `None` when the datagram is not an empty ACK/RST
    /// or no pending CON matches. Does not consult the Dedup Table.
    pub fn match_empty_ack_rst(
        &mut self,
        parsed: &ParsedMessage<'_>,
        endpoint: Endpoint,
    ) -> Option<SlotId> {
        if !parsed.is_empty_ack_or_rst() {
            return None;
        }
        self.storage.take_pending_con(parsed.message_id(), endpoint)
    }

    /// Decode occupied RX `id` and [`Self::match_empty_ack_rst`] using its sidecar endpoint.
    pub fn match_empty_ack_rst_rx(&mut self, id: SlotId) -> Result<Option<SlotId>, SlotMessageError>
    where
        S: DatagramSlots,
    {
        let endpoint = self.storage.rx_endpoint(id).ok_or(SlotError::NotOccupied)?;
        let (message_id, is_empty) = {
            let parsed = decode_occupied(self.storage.rx_payload(id))?;
            (parsed.message_id(), parsed.is_empty_ack_or_rst())
        };
        if !is_empty {
            return Ok(None);
        }
        Ok(self.storage.take_pending_con(message_id, endpoint))
    }
}

fn decode_occupied(bytes: Option<&[u8]>) -> Result<ParsedMessage<'_>, SlotMessageError> {
    crate::message::decode(bytes.ok_or(SlotError::NotOccupied)?).map_err(SlotMessageError::Parse)
}

fn encode_occupied(buf: Option<&mut [u8]>, msg: &Message<'_>) -> Result<usize, SlotMessageError> {
    crate::message::encode(msg, buf.ok_or(SlotError::NotOccupied)?)
        .map_err(SlotMessageError::Encode)
}

fn copy_into_slot(buf: Option<&mut [u8]>, bytes: &[u8]) -> Result<(), SlotError> {
    let buf = buf.ok_or(SlotError::NotOccupied)?;
    if bytes.len() > buf.len() {
        return Err(SlotError::LengthExceedsSlot);
    }
    buf[..bytes.len()].copy_from_slice(bytes);
    Ok(())
}
