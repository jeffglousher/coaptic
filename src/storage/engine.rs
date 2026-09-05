//! [`Engine`]: acquire / release / rotate against [`Storage`].

use super::Access;
use super::AccessMut;
use super::BodySlots;
use super::Capacities;
use super::DatagramSlots;
use super::DedupEntry;
use super::DedupKey;
use super::DedupSlots;
use super::Endpoint;
use super::ExchangeEntry;
use super::ExchangeKey;
use super::Exchanges;
use super::ObserveInterest;
use super::ObserveKey;
use super::ObserveSlots;
use super::PendingCon;
use super::PendingCons;
use super::Retransmit;
use super::SlotError;
use super::SlotId;
use super::Storage;
use super::block::{BlockKey, BlockProgress, BlockTransfer, OutgoingBlock};
use crate::error::{BlockTransferError, SlotMessageError};
use crate::message::{
    BlockValue, Code, Message, MessageId, Opt, ParsedMessage, Token, Type, encode_uint,
};

/// Protocol engine, generic over [`Storage`].
///
/// Storage engine: occupancy, acquire/release, rotating cursors, and Block /
/// Q-Block body-slot assembly when `S` implements [`BodySlots`]. BERT,
/// Observe notify, and missing-block recovery are not implemented.
///
/// When `S` implements [`DatagramSlots`], [`Self::decode_rx`] /
/// [`Self::encode_tx`] (and the TX/RX mirrors) call [`crate::message`]
/// against occupied datagram slots and record `set_len` on encode.
/// [`Self::write_rx`] copies bytes and sets the sidecar [`Endpoint`].
/// When `S` implements [`DedupSlots`], insert / lookup / remove store
/// [`DedupEntry`] values in the Dedup Table (O(n) in configured capacity).
/// When `S` implements [`PendingCons`], outgoing CON slots are marked
/// pending on the TX datagram sidecar (RTO included) and matched against
/// empty ACK/RST. [`Self::poll_retransmit`] returns due TX slots.
/// When `S` implements [`Exchanges`], outstanding CON/NON requests are
/// recorded by Token and remote [`Endpoint`] and taken on a matching
/// response. Empty ACK (code 0.00) is not a token-matching response;
/// a piggybacked ACK with a response code is. When `S` implements
/// [`ObserveSlots`], GET Observe register (0) / deregister (1) insert or
/// take [`ObserveInterest`] rows (Token + remote [`Endpoint`]). When `S`
/// implements [`BodySlots`], incoming Block1 / Block2 / Q-Block1 / Q-Block2
/// assemble into the Incoming Body Pool and outgoing Block1 / Block2 /
/// Q-Block1 / Q-Block2 slice the Outgoing Body Pool. Dedup,
/// pending CON, exchange matching, Observe interest, and body-slot
/// transfers are different identities. Temporary [`Access`] / [`AccessMut`]
/// pins an occupied datagram or body slot against release (`design.md`
/// §Application memory access). Optional format and
/// unrecognized-critical checks stay on [`ParsedMessage`]. This type does
/// not invent 4.02 / 4.08 / 2.31 / RST policy.
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

    /// Whether RX datagram `id` is pinned by a live [`Access`].
    #[must_use]
    pub fn rx_is_pinned(&mut self, id: SlotId) -> bool {
        self.storage.rx_datagram().is_pinned(id)
    }

    /// Whether TX datagram `id` is pinned by a live [`Access`] or [`AccessMut`].
    #[must_use]
    pub fn tx_is_pinned(&mut self, id: SlotId) -> bool {
        self.storage.tx_datagram().is_pinned(id)
    }

    /// Whether RX body `id` is pinned. `false` when body pools are absent.
    #[must_use]
    pub fn rx_body_is_pinned(&mut self, id: SlotId) -> bool {
        self.storage
            .rx_body()
            .map(|pool| pool.is_pinned(id))
            .unwrap_or(false)
    }

    /// Whether TX body `id` is pinned. `false` when body pools are absent.
    #[must_use]
    pub fn tx_body_is_pinned(&mut self, id: SlotId) -> bool {
        self.storage
            .tx_body()
            .map(|pool| pool.is_pinned(id))
            .unwrap_or(false)
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

    /// Read access to an occupied RX datagram. Pins `id` until dropped.
    ///
    /// Release of `id` returns [`SlotError::Pinned`] while the guard is live.
    /// See `design.md` §Application memory access.
    pub fn access_rx(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.storage.access_rx(id)
    }

    /// Read access to an occupied TX datagram. Pins `id` until dropped.
    pub fn access_tx(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.storage.access_tx(id)
    }

    /// Write access to an occupied TX datagram buffer. Pins `id` until dropped.
    pub fn access_tx_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.storage.access_tx_mut(id)
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
    /// Initializes RTO from [`crate::Transmission`] using caller `now_ms`
    /// and `jitter_ms` (no OS clock; `0` jitter is the ACK_TIMEOUT floor).
    /// Sets the TX sidecar endpoint. `None` when the slot is free or out of
    /// range, or when outstanding pending CONs to `endpoint` already equal
    /// [`crate::Transmission::NSTART`]. Idempotent for the same MID and
    /// endpoint on `id` (RTO is not reset). Does not use the Dedup Table.
    pub fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
        now_ms: u64,
        jitter_ms: u32,
    ) -> Option<SlotId> {
        self.storage
            .record_pending_con(id, endpoint, message_id, now_ms, jitter_ms)
    }

    /// Occupied TX slot pending for `message_id` and `endpoint`, if any. O(n).
    #[must_use]
    pub fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        self.storage.lookup_pending_con(message_id, endpoint)
    }

    /// Clear pending and RTO for the matching TX slot. The slot stays occupied.
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
    /// Match is Message ID plus `endpoint`. Clears pending and RTO. Returns
    /// the TX slot id; the caller releases it. `None` when the datagram is
    /// not an empty ACK/RST or no pending CON matches. Does not consult the
    /// Dedup Table.
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

    /// Next pending CON whose timeout is due at caller `now_ms`, if any.
    ///
    /// Scans pending TX slots in index order (O(n)). On
    /// [`Retransmit::Due`], doubles the timeout and increments attempts.
    /// On [`Retransmit::GiveUp`], clears pending. Does not send. See
    /// `knowledge/rfcs/rfc7252.txt` §4.2.
    pub fn poll_retransmit(&mut self, now_ms: u64) -> Option<Retransmit> {
        self.storage.poll_retransmit(now_ms)
    }
}

impl<S: Storage + Exchanges> Engine<S> {
    /// Insert an outstanding request, or return the existing slot if the key is present.
    ///
    /// `None` when the table is full and the key is not already stored. Scans
    /// the TX-derived capacity (O(n)). Does not use Dedup or pending CON.
    pub fn insert_exchange(&mut self, entry: ExchangeEntry) -> Option<SlotId> {
        self.storage.insert_exchange(entry)
    }

    /// Occupied exchange slot matching `key`, if any. O(n) in capacity.
    #[must_use]
    pub fn lookup_exchange(&self, key: ExchangeKey) -> Option<SlotId> {
        self.storage.lookup_exchange(key)
    }

    /// Remove and return the outstanding request matching `key`, if any.
    ///
    /// The caller releases [`ExchangeEntry::tx_slot`] when it still holds that
    /// TX datagram. `None` on miss.
    pub fn take_exchange(&mut self, key: ExchangeKey) -> Option<ExchangeEntry> {
        self.storage.take_exchange(key)
    }

    /// Occupied exchange payload at `id`.
    #[must_use]
    pub fn exchange_entry(&self, id: SlotId) -> Option<ExchangeEntry> {
        self.storage.exchange_entry(id)
    }

    /// Record an outstanding CON/NON request from occupied TX `id`.
    ///
    /// Sets the TX sidecar endpoint. `Ok(None)` when the datagram is not a
    /// CON/NON request, or the table is full. Idempotent for the same Token
    /// and endpoint. Does not mark pending CON and does not use Dedup.
    pub fn record_request(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
    ) -> Result<Option<ExchangeEntry>, SlotMessageError>
    where
        S: DatagramSlots,
    {
        let (token, message_id, is_request) = {
            let parsed = decode_occupied(self.storage.tx_payload(id))?;
            (
                parsed.token(),
                parsed.message_id(),
                super::exchange::is_con_or_non_request(&parsed),
            )
        };
        if !is_request {
            return Ok(None);
        }
        self.storage.set_tx_endpoint(id, endpoint)?;
        let entry = ExchangeEntry::new(token, endpoint, message_id, id);
        let table_id = self.storage.insert_exchange(entry);
        Ok(table_id.and_then(|tid| self.storage.exchange_entry(tid)))
    }

    /// If `parsed` is a response for `endpoint`, take the matching exchange.
    ///
    /// Match is Token plus `endpoint` ([`ExchangeKey`]). A piggybacked ACK
    /// also requires the request Message ID. Empty ACK/RST are not responses.
    /// Returns the entry; the caller releases the TX slot. `None` on miss.
    /// Does not consult Dedup or pending CON.
    pub fn match_response(
        &mut self,
        parsed: &ParsedMessage<'_>,
        endpoint: Endpoint,
    ) -> Option<ExchangeEntry> {
        self.take_matching_response(
            parsed.token(),
            parsed.ty(),
            parsed.code(),
            parsed.message_id(),
            endpoint,
        )
    }

    /// Decode occupied RX `id` and [`Self::match_response`] using its sidecar endpoint.
    pub fn match_response_rx(
        &mut self,
        id: SlotId,
    ) -> Result<Option<ExchangeEntry>, SlotMessageError>
    where
        S: DatagramSlots,
    {
        let endpoint = self.storage.rx_endpoint(id).ok_or(SlotError::NotOccupied)?;
        let (token, ty, code, message_id) = {
            let parsed = decode_occupied(self.storage.rx_payload(id))?;
            (
                parsed.token(),
                parsed.ty(),
                parsed.code(),
                parsed.message_id(),
            )
        };
        Ok(self.take_matching_response(token, ty, code, message_id, endpoint))
    }

    fn take_matching_response(
        &mut self,
        token: Token,
        ty: crate::message::Type,
        code: crate::message::Code,
        message_id: MessageId,
        endpoint: Endpoint,
    ) -> Option<ExchangeEntry> {
        if !code.is_response()
            || !matches!(
                ty,
                crate::message::Type::Confirmable
                    | crate::message::Type::NonConfirmable
                    | crate::message::Type::Acknowledgement
            )
        {
            return None;
        }
        let key = ExchangeKey::new(token, endpoint);
        let id = self.storage.lookup_exchange(key)?;
        let entry = self.storage.exchange_entry(id)?;
        let mid_ok = match ty {
            crate::message::Type::Acknowledgement => message_id == entry.message_id(),
            crate::message::Type::Confirmable | crate::message::Type::NonConfirmable => true,
            crate::message::Type::Reset => false,
        };
        if !mid_ok {
            return None;
        }
        self.storage.take_exchange(key)
    }
}

impl<S: Storage + ObserveSlots> Engine<S> {
    /// Insert an Observe interest, or return the existing slot if the key is present.
    ///
    /// `None` when the table is full and the key is not already stored. Scans
    /// the configured capacity (O(n)). Does not use Dedup, pending CON, or
    /// exchange matching.
    pub fn insert_observe(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        self.storage.insert_observe(interest)
    }

    /// Occupied Observe slot matching `key`, if any. O(n) in capacity.
    #[must_use]
    pub fn lookup_observe(&self, key: ObserveKey) -> Option<SlotId> {
        self.storage.lookup_observe(key)
    }

    /// Release the Observe slot matching `key`, if occupied. O(n) in capacity.
    pub fn remove_observe(&mut self, key: ObserveKey) -> bool {
        self.storage.remove_observe(key)
    }

    /// Remove and return the Observe interest matching `key`, if any.
    pub fn take_observe(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        self.storage.take_observe(key)
    }

    /// Occupied Observe payload at `id`.
    #[must_use]
    pub fn observe_interest(&self, id: SlotId) -> Option<ObserveInterest> {
        self.storage.observe_interest(id)
    }

    /// If `parsed` is a GET with Observe 0 (register), insert Token + Endpoint.
    ///
    /// Idempotent for the same Token and endpoint. `None` when the datagram
    /// is not a register GET, or the table is full. Does not invent 4.02 /
    /// RST policy.
    pub fn register_observe(
        &mut self,
        parsed: &ParsedMessage<'_>,
        endpoint: Endpoint,
    ) -> Option<SlotId> {
        if !parsed.is_observe_register() {
            return None;
        }
        self.storage
            .insert_observe(ObserveInterest::new(parsed.token(), endpoint))
    }

    /// If `parsed` is a GET with Observe 1 (deregister), take Token + Endpoint.
    ///
    /// `None` when the datagram is not a deregister GET, or no row matches.
    pub fn deregister_observe(
        &mut self,
        parsed: &ParsedMessage<'_>,
        endpoint: Endpoint,
    ) -> Option<ObserveInterest> {
        if !parsed.is_observe_deregister() {
            return None;
        }
        self.storage
            .take_observe(ObserveKey::new(parsed.token(), endpoint))
    }

    /// Decode occupied RX `id` and [`Self::register_observe`] using its sidecar endpoint.
    pub fn register_observe_rx(&mut self, id: SlotId) -> Result<Option<SlotId>, SlotMessageError>
    where
        S: DatagramSlots,
    {
        let endpoint = self.storage.rx_endpoint(id).ok_or(SlotError::NotOccupied)?;
        let (token, is_register) = {
            let parsed = decode_occupied(self.storage.rx_payload(id))?;
            (parsed.token(), parsed.is_observe_register())
        };
        if !is_register {
            return Ok(None);
        }
        Ok(self
            .storage
            .insert_observe(ObserveInterest::new(token, endpoint)))
    }

    /// Decode occupied RX `id` and [`Self::deregister_observe`] using its sidecar endpoint.
    pub fn deregister_observe_rx(
        &mut self,
        id: SlotId,
    ) -> Result<Option<ObserveInterest>, SlotMessageError>
    where
        S: DatagramSlots,
    {
        let endpoint = self.storage.rx_endpoint(id).ok_or(SlotError::NotOccupied)?;
        let (token, is_deregister) = {
            let parsed = decode_occupied(self.storage.rx_payload(id))?;
            (parsed.token(), parsed.is_observe_deregister())
        };
        if !is_deregister {
            return Ok(None);
        }
        Ok(self.storage.take_observe(ObserveKey::new(token, endpoint)))
    }
}

impl<S: Storage + BodySlots> Engine<S> {
    /// Read access to an occupied incoming body. Pins `id` until dropped.
    pub fn access_rx_body(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.storage.access_rx_body(id)
    }

    /// Write access to an occupied outgoing body buffer. Pins `id` until dropped.
    pub fn access_tx_body_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.storage.access_tx_body_mut(id)
    }

    /// Filled incoming body bytes, if `id` is occupied.
    #[must_use]
    pub fn rx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.storage.rx_body_payload(id)
    }

    /// Filled outgoing body bytes, if `id` is occupied.
    #[must_use]
    pub fn tx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.storage.tx_body_payload(id)
    }

    /// Incoming Block1 / Block2 sidecar on `id`.
    #[must_use]
    pub fn rx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.storage.rx_body_transfer(id)
    }

    /// Outgoing Block1 / Block2 sidecar on `id`.
    #[must_use]
    pub fn tx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.storage.tx_body_transfer(id)
    }

    /// Incoming body slot matching `key`, if any.
    #[must_use]
    pub fn lookup_rx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.storage.lookup_rx_body(key)
    }

    /// Outgoing body slot matching `key`, if any.
    #[must_use]
    pub fn lookup_tx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.storage.lookup_tx_body(key)
    }

    /// Admit a new incoming Block1 body when the first block arrives.
    ///
    /// Acquires one Incoming Body Slot. Classic Block starts at NUM 0.
    /// `size1` is the Size1 hint when present. See `design.md` and
    /// `knowledge/rfcs/rfc7959.txt`.
    pub fn admit_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.admit_block1(key, block, payload, size1)
    }

    /// Write the next in-order incoming Block1 range into `id`.
    ///
    /// Rejects overlap, gap, SZX mismatch, overflow, and inconsistent
    /// Size1. Does not invent 4.08 policy.
    pub fn write_block1(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.write_block1(id, block, payload)
    }

    /// Admit or continue incoming Block1 for `key`.
    pub fn apply_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.apply_block1(key, block, payload, size1)
    }

    /// Decode occupied RX `id` and [`Self::apply_block1`] using Block1 + Token + endpoint.
    ///
    /// Copies the datagram payload into the body slot (datagram RX stays
    /// separate). Missing Block1 is [`BlockTransferError::MissingBlock`].
    pub fn apply_block1_rx(&mut self, id: SlotId) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.apply_incoming_rx(id, RxBlockOpt::Block1)
    }

    /// Admit a new incoming Block2 body when the first block arrives.
    ///
    /// Acquires one Incoming Body Slot. Classic Block starts at NUM 0.
    /// `size2` is the Size2 hint when present. See `design.md` and
    /// `knowledge/rfcs/rfc7959.txt`.
    pub fn admit_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.admit_block2(key, block, payload, size2)
    }

    /// Write the next in-order incoming Block2 range into `id`.
    ///
    /// Rejects overlap, gap, SZX mismatch, overflow, and inconsistent
    /// Size2. Does not invent 4.08 policy.
    pub fn write_block2(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.write_block2(id, block, payload)
    }

    /// Admit or continue incoming Block2 for `key`.
    pub fn apply_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.apply_block2(key, block, payload, size2)
    }

    /// Decode occupied RX `id` and [`Self::apply_block2`] using Block2 + Token + endpoint.
    ///
    /// Copies the datagram payload into the body slot (datagram RX stays
    /// separate). Missing Block2 is [`BlockTransferError::MissingBlock`].
    pub fn apply_block2_rx(&mut self, id: SlotId) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.apply_incoming_rx(id, RxBlockOpt::Block2)
    }

    /// Admit a new incoming Q-Block1 body when the first window-0 block arrives.
    ///
    /// The first datagram may be any NUM in `0..MAX_PAYLOADS`. `size1` is the
    /// Size1 hint when present. See `design.md` and
    /// `knowledge/rfcs/rfc9177.txt`.
    pub fn admit_q_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.admit_q_block1(key, block, payload, size1)
    }

    /// Write one incoming Q-Block1 range into `id`.
    ///
    /// Accepts out-of-order NUMs in the current window. Rejects duplicates
    /// and NUMs outside the window. Does not invent 4.08 policy.
    pub fn write_q_block1(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.write_q_block1(id, block, payload)
    }

    /// Admit or continue incoming Q-Block1 for `key`.
    pub fn apply_q_block1(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.apply_q_block1(key, block, payload, size1)
    }

    /// Decode occupied RX `id` and [`Self::apply_q_block1`] using Q-Block1 + Token + endpoint.
    ///
    /// Identity is Token + Endpoint (same as classic Block). RFC 9177 Q-Block1
    /// body match uses Request-Tag; that is not applied here. Missing Q-Block1
    /// is [`BlockTransferError::MissingBlock`].
    pub fn apply_q_block1_rx(&mut self, id: SlotId) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.apply_incoming_rx(id, RxBlockOpt::QBlock1)
    }

    /// Admit a new incoming Q-Block2 body when the first window-0 block arrives.
    ///
    /// The first datagram may be any NUM in `0..MAX_PAYLOADS`. `size2` is the
    /// Size2 hint when present. See `design.md` and
    /// `knowledge/rfcs/rfc9177.txt`.
    pub fn admit_q_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.admit_q_block2(key, block, payload, size2)
    }

    /// Write one incoming Q-Block2 range into `id`.
    ///
    /// Accepts out-of-order NUMs in the current window. Rejects duplicates
    /// and NUMs outside the window. Does not invent 4.08 policy.
    pub fn write_q_block2(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.write_q_block2(id, block, payload)
    }

    /// Admit or continue incoming Q-Block2 for `key`.
    pub fn apply_q_block2(
        &mut self,
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.apply_q_block2(key, block, payload, size2)
    }

    /// Decode occupied RX `id` and [`Self::apply_q_block2`] using Q-Block2 + Token + endpoint.
    ///
    /// Uses the first Q-Block2 option (repeatable recovery options are out of
    /// scope). Missing Q-Block2 is [`BlockTransferError::MissingBlock`].
    pub fn apply_q_block2_rx(&mut self, id: SlotId) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.apply_incoming_rx(id, RxBlockOpt::QBlock2)
    }

    /// Copy a complete body into an Outgoing Body Slot and start Block1.
    pub fn start_block1(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.start_block1(key, body, szx)
    }

    /// Issue the next in-order outgoing Block1 range. Bytes stay in the body slot.
    pub fn next_block1(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.storage.next_block1(id)
    }

    /// Issue the next Block1 and encode it into occupied TX `tx_id`.
    ///
    /// Token and remote endpoint come from the body-slot sidecar. The caller
    /// supplies type, code, and Message ID. Does not invent 2.31 / 4.08.
    pub fn encode_block1_tx(
        &mut self,
        body_id: SlotId,
        tx_id: SlotId,
        ty: Type,
        code: Code,
        message_id: MessageId,
    ) -> Result<OutgoingBlock, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let issued = self.storage.next_block1(body_id)?;
        self.finish_outgoing_tx(
            issued,
            body_id,
            tx_id,
            (ty, code, message_id),
            OutgoingBlockOpt::Block1,
        )
    }

    /// Copy a complete body into an Outgoing Body Slot and start Block2.
    pub fn start_block2(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.start_block2(key, body, szx)
    }

    /// Issue the next in-order outgoing Block2 range. Bytes stay in the body slot.
    pub fn next_block2(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.storage.next_block2(id)
    }

    /// Issue the next Block2 and encode it into occupied TX `tx_id`.
    ///
    /// Token and remote endpoint come from the body-slot sidecar. The caller
    /// supplies type, code, and Message ID. Does not invent 2.31 / 4.08.
    pub fn encode_block2_tx(
        &mut self,
        body_id: SlotId,
        tx_id: SlotId,
        ty: Type,
        code: Code,
        message_id: MessageId,
    ) -> Result<OutgoingBlock, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let issued = self.storage.next_block2(body_id)?;
        self.finish_outgoing_tx(
            issued,
            body_id,
            tx_id,
            (ty, code, message_id),
            OutgoingBlockOpt::Block2,
        )
    }

    /// Copy a complete body into an Outgoing Body Slot and start Q-Block1.
    pub fn start_q_block1(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.start_q_block1(key, body, szx)
    }

    /// Issue the next unsent outgoing Q-Block1 range in the current window.
    pub fn next_q_block1(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.storage.next_q_block1(id)
    }

    /// Advance an outgoing Q-Block1 window using the peer Continue NUM.
    ///
    /// `num` is the Q-Block1 NUM from the peer (RFC 9177 §4.3: all blocks
    /// through `num` received). Empty ACK is not a window ACK. Does not invent
    /// 2.31 / 4.08 policy.
    pub fn ack_q_block1(
        &mut self,
        id: SlotId,
        num: u32,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.ack_q_block1(id, num)
    }

    /// Issue the next Q-Block1 and encode it into occupied TX `tx_id`.
    ///
    /// Token and remote endpoint come from the body-slot sidecar. Size1 is
    /// encoded (RFC 9177 §4.6). Request-Tag is caller-owned. The caller
    /// supplies type, code, and Message ID.
    pub fn encode_q_block1_tx(
        &mut self,
        body_id: SlotId,
        tx_id: SlotId,
        ty: Type,
        code: Code,
        message_id: MessageId,
    ) -> Result<OutgoingBlock, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let issued = self.storage.next_q_block1(body_id)?;
        self.finish_outgoing_tx(
            issued,
            body_id,
            tx_id,
            (ty, code, message_id),
            OutgoingBlockOpt::QBlock1,
        )
    }

    /// Decode occupied RX `id` and [`Self::ack_q_block1`] using Q-Block1 NUM.
    ///
    /// Empty ACK (code 0.00) has no Q-Block1 and is [`BlockTransferError::MissingBlock`].
    /// Token need not match (RFC 9177 §6 may use a new Token per request);
    /// endpoint must match the body slot. Does not inspect the response code.
    pub fn ack_q_block1_rx(
        &mut self,
        body_id: SlotId,
        rx_id: SlotId,
    ) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.ack_outgoing_rx(body_id, rx_id, RxBlockOpt::QBlock1)
    }

    /// Copy a complete body into an Outgoing Body Slot and start Q-Block2.
    pub fn start_q_block2(
        &mut self,
        key: BlockKey,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.storage.start_q_block2(key, body, szx)
    }

    /// Issue the next unsent outgoing Q-Block2 range in the current window.
    pub fn next_q_block2(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.storage.next_q_block2(id)
    }

    /// Advance an outgoing Q-Block2 window using the peer Continue NUM.
    ///
    /// `num` is the Continue Q-Block2 NUM (RFC 9177 §4.4: `num % MAX_PAYLOADS
    /// == 0` and `num != 0`). Empty ACK is not a window ACK. Does not invent
    /// 2.31 / 4.08 policy.
    pub fn ack_q_block2(
        &mut self,
        id: SlotId,
        num: u32,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.storage.ack_q_block2(id, num)
    }

    /// Issue the next Q-Block2 and encode it into occupied TX `tx_id`.
    ///
    /// Token and remote endpoint come from the body-slot sidecar. Size2 is
    /// encoded (RFC 9177 §4.6). ETag is caller-owned. The caller supplies
    /// type, code, and Message ID.
    pub fn encode_q_block2_tx(
        &mut self,
        body_id: SlotId,
        tx_id: SlotId,
        ty: Type,
        code: Code,
        message_id: MessageId,
    ) -> Result<OutgoingBlock, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let issued = self.storage.next_q_block2(body_id)?;
        self.finish_outgoing_tx(
            issued,
            body_id,
            tx_id,
            (ty, code, message_id),
            OutgoingBlockOpt::QBlock2,
        )
    }

    /// Decode occupied RX `id` and [`Self::ack_q_block2`] using Q-Block2 NUM.
    ///
    /// Empty ACK (code 0.00) has no Q-Block2 and is [`BlockTransferError::MissingBlock`].
    /// A Continue request must have M set. Token need not match; endpoint must
    /// match the body slot. Does not invent 2.31 policy.
    pub fn ack_q_block2_rx(
        &mut self,
        body_id: SlotId,
        rx_id: SlotId,
    ) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        self.ack_outgoing_rx(body_id, rx_id, RxBlockOpt::QBlock2)
    }

    fn apply_incoming_rx(
        &mut self,
        id: SlotId,
        which: RxBlockOpt,
    ) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let endpoint = self.storage.rx_endpoint(id).ok_or(SlotError::NotOccupied)?;
        let mut tmp = [0u8; BlockValue::SIZE_MAX as usize];
        let (token, block, expected, n) = {
            let parsed =
                crate::message::decode(self.storage.rx_payload(id).ok_or(SlotError::NotOccupied)?)?;
            let block = match which.read_block(&parsed) {
                Some(Ok(b)) => b,
                Some(Err(e)) => return Err(e.into()),
                None => return Err(BlockTransferError::MissingBlock),
            };
            let expected = if which.uses_size1() {
                match parsed.size1() {
                    Some(Ok(n)) => Some(n),
                    Some(Err(e)) => return Err(e.into()),
                    None => None,
                }
            } else {
                match parsed.size2() {
                    Some(Ok(n)) => Some(n),
                    Some(Err(e)) => return Err(e.into()),
                    None => None,
                }
            };
            let payload = parsed.payload();
            if payload.len() > tmp.len() {
                return Err(BlockTransferError::PayloadLength);
            }
            tmp[..payload.len()].copy_from_slice(payload);
            (parsed.token(), block, expected, payload.len())
        };
        let key = BlockKey::new(token, endpoint);
        match which {
            RxBlockOpt::Block1 => self.apply_block1(key, block, &tmp[..n], expected),
            RxBlockOpt::Block2 => self.apply_block2(key, block, &tmp[..n], expected),
            RxBlockOpt::QBlock1 => self.apply_q_block1(key, block, &tmp[..n], expected),
            RxBlockOpt::QBlock2 => self.apply_q_block2(key, block, &tmp[..n], expected),
        }
    }

    fn finish_outgoing_tx(
        &mut self,
        issued: OutgoingBlock,
        body_id: SlotId,
        tx_id: SlotId,
        header: (Type, Code, MessageId),
        which: OutgoingBlockOpt,
    ) -> Result<OutgoingBlock, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let (ty, code, message_id) = header;
        let transfer = self
            .storage
            .tx_body_transfer(body_id)
            .ok_or(BlockTransferError::NoTransfer)?;
        let mut tmp = [0u8; BlockValue::SIZE_MAX as usize];
        let payload = self
            .storage
            .tx_body_payload(body_id)
            .ok_or(SlotError::NotOccupied)?;
        let end = issued
            .offset()
            .checked_add(issued.len())
            .ok_or(BlockTransferError::Overflow)?;
        if end > payload.len() || issued.len() > tmp.len() {
            return Err(BlockTransferError::Overflow);
        }
        tmp[..issued.len()].copy_from_slice(&payload[issued.offset()..end]);
        let encoded = issued.block().encode();
        let size_n = u32::try_from(transfer.filled()).map_err(|_| BlockTransferError::Overflow)?;
        let size = encode_uint(size_n);
        let q1 = [Opt::q_block1(&encoded), Opt::size1(&size)];
        let q2 = [Opt::size2(&size), Opt::q_block2(&encoded)];
        let b1 = [Opt::block1(&encoded)];
        let b2 = [Opt::block2(&encoded)];
        let opts: &[Opt<'_>] = match which {
            OutgoingBlockOpt::Block1 => &b1,
            OutgoingBlockOpt::Block2 => &b2,
            OutgoingBlockOpt::QBlock1 => &q1,
            OutgoingBlockOpt::QBlock2 => &q2,
        };
        let msg = Message::new(ty, code, message_id)
            .with_token(transfer.token())
            .with_options(opts)
            .with_payload(&tmp[..issued.len()]);
        let n = encode_occupied(self.storage.tx_payload_mut(tx_id), &msg).map_err(|e| match e {
            SlotMessageError::Slot(s) => BlockTransferError::Slot(s),
            SlotMessageError::Parse(p) => BlockTransferError::Parse(p),
            SlotMessageError::Encode(enc) => BlockTransferError::Encode(enc),
        })?;
        self.storage.set_tx_len(tx_id, n)?;
        self.storage.set_tx_endpoint(tx_id, transfer.endpoint())?;
        Ok(issued)
    }

    fn ack_outgoing_rx(
        &mut self,
        body_id: SlotId,
        rx_id: SlotId,
        which: RxBlockOpt,
    ) -> Result<BlockProgress, BlockTransferError>
    where
        S: DatagramSlots,
    {
        let endpoint = self
            .storage
            .rx_endpoint(rx_id)
            .ok_or(SlotError::NotOccupied)?;
        let num = {
            let parsed = crate::message::decode(
                self.storage
                    .rx_payload(rx_id)
                    .ok_or(SlotError::NotOccupied)?,
            )?;
            if parsed.is_empty() {
                return Err(BlockTransferError::MissingBlock);
            }
            let block = match which.read_block(&parsed) {
                Some(Ok(b)) => b,
                Some(Err(e)) => return Err(e.into()),
                None => return Err(BlockTransferError::MissingBlock),
            };
            if matches!(which, RxBlockOpt::QBlock2) && !block.more() {
                return Err(BlockTransferError::Gap);
            }
            block.num()
        };
        let transfer = self
            .storage
            .tx_body_transfer(body_id)
            .ok_or(BlockTransferError::NoTransfer)?;
        if transfer.endpoint() != endpoint {
            return Err(BlockTransferError::IdentityMismatch);
        }
        match which {
            RxBlockOpt::QBlock1 => self.ack_q_block1(body_id, num),
            RxBlockOpt::QBlock2 => self.ack_q_block2(body_id, num),
            RxBlockOpt::Block1 | RxBlockOpt::Block2 => Err(BlockTransferError::IdentityMismatch),
        }
    }
}

#[derive(Clone, Copy)]
enum OutgoingBlockOpt {
    Block1,
    Block2,
    QBlock1,
    QBlock2,
}

#[derive(Clone, Copy)]
enum RxBlockOpt {
    Block1,
    Block2,
    QBlock1,
    QBlock2,
}

impl RxBlockOpt {
    fn uses_size1(self) -> bool {
        matches!(self, Self::Block1 | Self::QBlock1)
    }

    fn read_block(
        self,
        parsed: &ParsedMessage<'_>,
    ) -> Option<Result<BlockValue, crate::error::ValueError>> {
        match self {
            Self::Block1 => parsed.block1(),
            Self::Block2 => parsed.block2(),
            Self::QBlock1 => parsed.q_block1(),
            Self::QBlock2 => parsed.q_block2().next(),
        }
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
