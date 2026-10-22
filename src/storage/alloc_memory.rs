//! [`AllocMemory`]: runtime-sized Storage, allocated once, then no growth.

use crate::message::{OwnedToken, Token};
use alloc::vec::Vec;

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
use super::SlotPool;
use super::Storage;
use super::block::{
    BlockKey, BlockProgress, BlockRole, BlockTransfer, BodyOps, OutgoingBlock,
    accept_incoming_role, block_offset, same_body_identity, start_incoming, store_incoming,
    write_range,
};
use super::exchange::ExchangeStore;
use super::memory::MemoryProfile;
use super::occupancy::HeapOccupancy;
use super::pending::{
    PendingMark, PendingRto, next_due_slot, outstanding_pending, record_admission,
};
use super::slot::{SlotError, SlotId};
use super::table::{DedupStore, ObserveStore};
use crate::error::{BlockTransferError, BuildError};
use crate::message::{BlockValue, MessageId};

/// `alloc` backend: runtime [`Capacities`], typed heap arrays, no later growth.
///
/// Each area is a private, fallibly reserved `Vec<T>` created at init. Lengths
/// never change after construction. This is not a carved byte slab. Use
/// [`Self::try_new`] to enforce an aggregate heap request budget.
/// Q-Block assembly retains its total length across payloads. A changed size
/// is refused before modifying bytes or progress, matching fixed body pools.
/// Due-CON selection uses one inline `usize` cursor, with no additional heap
/// array or change to [`Self::required_heap_bytes`].
pub struct AllocMemory<T: OwnedToken = Token> {
    retransmit_cursor: usize,
    rx: AllocDatagramPool,
    tx: AllocDatagramPool,
    dedup: AllocDedupTable,
    observe: AllocObserveTable<T>,
    exchange: AllocExchangeTable<T>,
    rx_body: Option<AllocBodyPool<T>>,
    tx_body: Option<AllocBodyPool<T>>,
}

impl<T: OwnedToken> AllocMemory<T> {
    /// Checked sum of heap array layout sizes on this target, without allocating.
    ///
    /// Includes both payloads and metadata. See [`Self::try_new`] for exclusions.
    pub fn required_heap_bytes(c: Capacities) -> Result<usize, BuildError> {
        c.validate_for_build(c.has_body_pools())?;
        let mut bytes = HeapSize(0);
        bytes.pool::<AllocSlot>(c.rx_datagram_slots, c.rx_datagram_bytes)?;
        bytes.pool::<AllocSlot>(c.tx_datagram_slots, c.tx_datagram_bytes)?;
        bytes.table::<Option<DedupEntry>>(c.dedup_entries)?;
        bytes.table::<Option<ObserveInterest<T>>>(c.observe_entries)?;
        bytes.table::<Option<ExchangeEntry<T>>>(c.tx_datagram_slots)?;
        if let Some((rx_slots, rx_bytes, tx_slots, tx_bytes)) = c.body_dims() {
            bytes.pool::<AllocBodySlot<T>>(rx_slots, rx_bytes)?;
            bytes.pool::<AllocBodySlot<T>>(tx_slots, tx_bytes)?;
        }
        Ok(bytes.0)
    }
    pub(crate) fn from_capacities(
        capacities: Capacities,
        block_wise: bool,
    ) -> Result<Self, BuildError> {
        capacities.validate_for_build(block_wise)?;
        Self::try_new(capacities, usize::MAX)
    }

    /// Allocate fixed-length pools after checking every array and the heap budget.
    ///
    /// `heap_limit` bounds the sum of requested array layouts, including payloads,
    /// slot metadata, protocol tables and occupancy flags. It excludes inline App
    /// state, stack/scratch, allocator bookkeeping/rounding, transport and application
    /// memory. Allocator refusal returns an error and drops all partial storage.
    /// No pool grows after construction. Body pools are selected by `capacities`.
    ///
    /// ```
    /// use coaptic::{profiles, storage::{AllocMemory, Capacities}};
    /// let capacities = Capacities::from_profile::<profiles::Constrained>();
    /// let storage = AllocMemory::<coaptic::message::Token>::try_new(capacities, 64 * 1024)?;
    /// // Pass storage to AppBuilder::bind_storage with the platform transport.
    /// # Ok::<(), coaptic::BuildError>(())
    /// ```
    pub fn try_new(capacities: Capacities, heap_limit: usize) -> Result<Self, BuildError> {
        let required = Self::required_heap_bytes(capacities)?;
        if required > heap_limit {
            return Err(BuildError::HeapBudgetExceeded {
                required,
                limit: heap_limit,
            });
        }
        let block_wise = capacities.has_body_pools();
        let (rx_body, tx_body) = if block_wise {
            let Some((rx_slots, rx_bytes, tx_slots, tx_bytes)) = capacities.body_dims() else {
                return Err(BuildError::MissingBodyPools);
            };
            (
                Some(AllocBodyPool::new(rx_slots, rx_bytes)?),
                Some(AllocBodyPool::new(tx_slots, tx_bytes)?),
            )
        } else {
            (None, None)
        };
        Ok(Self {
            retransmit_cursor: 0,
            rx: AllocDatagramPool::new(capacities.rx_datagram_slots, capacities.rx_datagram_bytes)?,
            tx: AllocDatagramPool::new(capacities.tx_datagram_slots, capacities.tx_datagram_bytes)?,
            dedup: AllocDedupTable::new(capacities.dedup_entries)?,
            observe: AllocObserveTable::new(capacities.observe_entries)?,
            exchange: AllocExchangeTable::new(capacities.tx_datagram_slots)?,
            rx_body,
            tx_body,
        })
    }

    /// Configured sizes of this allocation.
    #[must_use]
    pub fn capacities(&self) -> Capacities {
        Capacities {
            rx_datagram_slots: self.rx.occ.slot_count(),
            rx_datagram_bytes: self.rx.slot_bytes,
            tx_datagram_slots: self.tx.occ.slot_count(),
            tx_datagram_bytes: self.tx.slot_bytes,
            dedup_entries: self.dedup.occ.slot_count(),
            observe_entries: self.observe.occ.slot_count(),
            rx_body_slots: self.rx_body.as_ref().map(|p| p.occ.slot_count()),
            rx_body_bytes: self.rx_body.as_ref().map(|p| p.slot_bytes),
            tx_body_slots: self.tx_body.as_ref().map(|p| p.occ.slot_count()),
            tx_body_bytes: self.tx_body.as_ref().map(|p| p.slot_bytes),
        }
    }
}

// Use Layout rather than unchecked multiplication: each allocation must fit
// isize::MAX after alignment, even when its product fits usize.
struct HeapSize(usize);

impl HeapSize {
    fn array<T>(&mut self, count: usize, copies: usize) -> Result<(), BuildError> {
        let size = core::alloc::Layout::array::<T>(count)
            .map_err(|_| BuildError::CapacityOverflow)?
            .size();
        self.0 = size
            .checked_mul(copies)
            .and_then(|bytes| self.0.checked_add(bytes))
            .ok_or(BuildError::CapacityOverflow)?;
        Ok(())
    }

    fn table<T>(&mut self, count: usize) -> Result<(), BuildError> {
        self.array::<T>(count, 1)?;
        self.array::<bool>(count, 2)
    }

    fn pool<T>(&mut self, count: usize, bytes: usize) -> Result<(), BuildError> {
        self.table::<T>(count)?;
        self.array::<u8>(bytes, count)
    }
}

// Retain Vec storage: converting to Box<[T]> can perform an infallible shrink
// allocation. Reserve once, then initialize without exceeding the reservation.
pub(super) fn try_array<T>(
    count: usize,
    mut init: impl FnMut() -> Result<T, BuildError>,
) -> Result<Vec<T>, BuildError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| BuildError::AllocationFailed)?;
    for _ in 0..count {
        values.push(init()?);
    }
    Ok(values)
}

impl<T: OwnedToken> core::fmt::Debug for AllocMemory<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AllocMemory<T>")
            .field("capacities", &self.capacities())
            .finish_non_exhaustive()
    }
}

impl<T: OwnedToken> Storage for AllocMemory<T> {
    type Token = T;
    fn capacities(&self) -> Capacities {
        self.capacities()
    }

    fn rx_datagram(&mut self) -> &mut dyn SlotPool {
        &mut self.rx
    }

    fn tx_datagram(&mut self) -> &mut dyn SlotPool {
        &mut self.tx
    }

    fn dedup(&mut self) -> &mut dyn SlotPool {
        &mut self.dedup
    }

    fn observe(&mut self) -> &mut dyn SlotPool {
        &mut self.observe
    }

    fn rx_body(&mut self) -> Option<&mut dyn SlotPool> {
        self.rx_body.as_mut().map(|p| p as &mut dyn SlotPool)
    }

    fn tx_body(&mut self) -> Option<&mut dyn SlotPool> {
        self.tx_body.as_mut().map(|p| p as &mut dyn SlotPool)
    }
}

struct AllocSlot {
    buf: Vec<u8>,
    len: usize,
    endpoint: Option<Endpoint>,
    pending: Option<PendingMark>,
}

struct AllocDatagramPool {
    slots: Vec<AllocSlot>,
    slot_bytes: usize,
    occ: HeapOccupancy,
}

impl AllocDatagramPool {
    fn new(slots: usize, bytes: usize) -> Result<Self, BuildError> {
        let slots_storage = try_array(slots, || {
            Ok(AllocSlot {
                buf: try_array(bytes, || Ok(0))?,
                len: 0,
                endpoint: None,
                pending: None,
            })
        })?;
        Ok(Self {
            slots: slots_storage,
            slot_bytes: bytes,
            occ: HeapOccupancy::new(slots)?,
        })
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.len = 0;
            slot.endpoint = None;
            slot.pending = None;
        }
    }

    fn payload(&self, id: SlotId) -> Option<&[u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        let slot = self.slots.get(id.index())?;
        Some(&slot.buf[..slot.len])
    }

    fn payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        Some(self.slots.get_mut(id.index())?.buf.as_mut())
    }

    fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        if len > self.slot_bytes {
            return Err(SlotError::LengthExceedsSlot);
        }
        self.slots[id.index()].len = len;
        Ok(())
    }

    fn endpoint(&self, id: SlotId) -> Option<Endpoint> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.slots.get(id.index())?.endpoint
    }

    fn set_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.slots[id.index()].endpoint = Some(endpoint);
        Ok(())
    }

    fn pending_mid(&self, id: SlotId) -> Option<MessageId> {
        self.pending_mark(id).map(|m| m.message_id)
    }

    fn pending_rto(&self, id: SlotId) -> Option<PendingRto> {
        self.pending_mark(id).map(|m| m.rto)
    }

    fn pending_mark(&self, id: SlotId) -> Option<PendingMark> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.slots.get(id.index())?.pending
    }

    fn set_pending_mid(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        self.set_pending(id, message_id, PendingRto::new(0, 0))
    }

    fn set_pending(
        &mut self,
        id: SlotId,
        message_id: MessageId,
        rto: PendingRto,
    ) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.slots[id.index()].pending = Some(PendingMark::new(message_id, rto));
        Ok(())
    }

    fn clear_pending_mid(&mut self, id: SlotId) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.slots[id.index()].pending = None;
        Ok(())
    }

    fn lookup_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            if self.pending_mid(id) == Some(message_id)
                && self.endpoint(id) == Some(endpoint)
                && self.pending_rto(id).is_some_and(PendingRto::is_started)
            {
                Some(id)
            } else {
                None
            }
        })
    }

    fn access(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.occ.try_pin(id)?;
        let idx = id.index();
        let pin = self.occ.pin_flag_mut(id).ok_or(SlotError::InvalidSlot)?;
        let slot = self.slots.get(idx).ok_or(SlotError::InvalidSlot)?;
        let bytes = &slot.buf[..slot.len];
        Ok(Access::new(id, bytes, pin))
    }

    fn access_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.occ.try_pin(id)?;
        let idx = id.index();
        let pin = self.occ.pin_flag_mut(id).ok_or(SlotError::InvalidSlot)?;
        let slot = self.slots.get_mut(idx).ok_or(SlotError::InvalidSlot)?;
        Ok(AccessMut::new(id, slot.buf.as_mut(), &mut slot.len, pin))
    }
}

impl SlotPool for AllocDatagramPool {
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

    fn advance(&mut self, steps: usize) {
        self.occ.advance(steps);
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

    fn is_pinned(&self, id: SlotId) -> bool {
        self.occ.is_pinned(id)
    }

    fn cursor(&self) -> usize {
        self.occ.cursor()
    }
}

impl<T: OwnedToken> DatagramSlots for AllocMemory<T> {
    type RxScratch = super::DatagramScratch<{ super::profiles::Default::RX_DATAGRAM_BYTES }>;
    type TxScratch = super::DatagramScratch<{ super::profiles::Default::TX_DATAGRAM_BYTES }>;

    fn rx_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.rx.payload(id)
    }

    fn tx_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.tx.payload(id)
    }

    fn rx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        self.rx.payload_mut(id)
    }

    fn tx_payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        self.tx.payload_mut(id)
    }

    fn set_rx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        self.rx.set_len(id, len)
    }

    fn set_tx_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        self.tx.set_len(id, len)
    }

    fn rx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        self.rx.endpoint(id)
    }

    fn tx_endpoint(&self, id: SlotId) -> Option<Endpoint> {
        self.tx.endpoint(id)
    }

    fn set_rx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        self.rx.set_endpoint(id, endpoint)
    }

    fn set_tx_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        self.tx.set_endpoint(id, endpoint)
    }

    fn tx_pending_mid(&self, id: SlotId) -> Option<MessageId> {
        self.tx.pending_mid(id)
    }

    fn set_tx_pending(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        self.tx.set_pending_mid(id, message_id)
    }

    fn clear_tx_pending(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.tx.clear_pending_mid(id)
    }

    fn lookup_tx_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        self.tx.lookup_pending(message_id, endpoint)
    }

    fn access_rx(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.rx.access(id)
    }

    fn access_tx(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.tx.access(id)
    }

    fn access_tx_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.tx.access_mut(id)
    }
}

impl<T: OwnedToken> PendingCons for AllocMemory<T> {
    fn replace_pending_con(
        &mut self,
        expected: PendingCon,
        bytes: &[u8],
    ) -> Result<PendingCon, super::PendingReplaceError> {
        let id = expected.tx_slot();
        let replacement = super::pending::validate_replacement(
            expected,
            self.pending_con(id),
            self.tx.payload(id),
            bytes,
            self.tx.slot_bytes,
            self.tx.occ.is_pinned(id),
        )?;
        // Validation proves this slot exists and all lengths fit; commit cannot fail.
        let slot = &mut self.tx.slots[id.index()];
        slot.buf[..bytes.len()].copy_from_slice(bytes);
        slot.len = bytes.len();
        slot.pending = Some(PendingMark::new(
            replacement.message_id(),
            replacement.rto(),
        ));
        Ok(replacement)
    }

    fn reserve_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<PendingCon> {
        if !self.tx.is_occupied(id) {
            return None;
        }
        if let Some(current) = self.pending_con(id) {
            return (!current.rto().is_started()
                && current.message_id() == message_id
                && current.endpoint() == endpoint)
                .then_some(current);
        }
        let outstanding =
            outstanding_pending(self.tx.slot_count(), endpoint, |sid| self.pending_con(sid));
        record_admission(None, message_id, endpoint, outstanding)?;
        self.tx.set_endpoint(id, endpoint).ok()?;
        self.tx
            .set_pending(id, message_id, PendingRto::reserved())
            .ok()?;
        self.pending_con(id)
    }

    fn commit_pending_con(
        &mut self,
        expected: PendingCon,
        now_ms: u64,
        jitter_ms: u32,
    ) -> Option<PendingCon> {
        if expected.rto().is_started() || self.pending_con(expected.tx_slot()) != Some(expected) {
            return None;
        }
        self.tx
            .set_pending(
                expected.tx_slot(),
                expected.message_id(),
                PendingRto::new(now_ms, jitter_ms),
            )
            .ok()?;
        self.pending_con(expected.tx_slot())
    }

    fn cancel_pending_con(&mut self, expected: PendingCon) -> bool {
        if expected.rto().is_started() || self.pending_con(expected.tx_slot()) != Some(expected) {
            return false;
        }
        self.tx.clear_pending_mid(expected.tx_slot()).is_ok()
    }

    fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
        now_ms: u64,
        jitter_ms: u32,
    ) -> Option<SlotId> {
        if !self.tx.is_occupied(id) {
            return None;
        }
        let existing = self.pending_con(id);
        if existing.is_some_and(|pending| !pending.rto().is_started()) {
            return None;
        }
        let outstanding =
            outstanding_pending(self.tx.slot_count(), endpoint, |sid| self.pending_con(sid));
        if !record_admission(existing, message_id, endpoint, outstanding)? {
            return Some(id);
        }
        self.tx.set_endpoint(id, endpoint).ok()?;
        self.tx
            .set_pending(id, message_id, PendingRto::new(now_ms, jitter_ms))
            .ok()?;
        Some(id)
    }

    fn lookup_pending_con(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        self.tx.lookup_pending(message_id, endpoint)
    }

    fn take_pending_con(&mut self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        let id = self.tx.lookup_pending(message_id, endpoint)?;
        self.tx.clear_pending_mid(id).ok()?;
        Some(id)
    }

    fn pending_con(&self, id: SlotId) -> Option<PendingCon> {
        let message_id = self.tx.pending_mid(id)?;
        let endpoint = self.tx.endpoint(id)?;
        let rto = self.tx.pending_rto(id)?;
        Some(PendingCon::new(message_id, endpoint, id).with_rto(rto))
    }

    fn peek_retransmit(&self, now_ms: u64) -> Option<PendingCon> {
        let due = next_due_slot(self.tx.slot_count(), self.retransmit_cursor, now_ms, |id| {
            self.pending_con(id)
        })?;
        self.pending_con(due.id)
    }

    fn commit_retransmit(&mut self, expected: PendingCon, now_ms: u64) -> Option<Retransmit> {
        if self.pending_con(expected.tx_slot()) != Some(expected) {
            return None;
        }
        let due = super::pending::due_snapshot(expected, now_ms)?;
        match due.mark {
            Some(mark) => {
                self.tx
                    .set_pending(due.id, mark.message_id, mark.rto)
                    .ok()?;
            }
            None => {
                self.tx.clear_pending_mid(due.id).ok()?;
            }
        }
        self.retransmit_cursor = due.id.index() + 1;
        Some(due.event)
    }

    fn poll_retransmit(&mut self, now_ms: u64) -> Option<Retransmit> {
        let expected = self.peek_retransmit(now_ms)?;
        self.commit_retransmit(expected, now_ms)
    }
}

impl<T: OwnedToken> Exchanges for AllocMemory<T> {
    fn insert_exchange(&mut self, entry: ExchangeEntry<T>) -> Option<SlotId> {
        self.exchange.insert(entry)
    }

    fn lookup_exchange(&self, key: ExchangeKey<T>) -> Option<SlotId> {
        self.exchange.lookup(key)
    }

    fn take_exchange(&mut self, key: ExchangeKey<T>) -> Option<ExchangeEntry<T>> {
        self.exchange.take(key)
    }

    fn exchange_entry(&self, id: SlotId) -> Option<ExchangeEntry<T>> {
        self.exchange.entry(id)
    }
}

impl<T: OwnedToken> DedupSlots for AllocMemory<T> {
    fn insert_dedup(&mut self, entry: DedupEntry) -> Option<SlotId> {
        self.dedup.insert(entry)
    }

    fn lookup_dedup(&self, key: DedupKey) -> Option<SlotId> {
        self.dedup.lookup(key)
    }

    fn remove_dedup(&mut self, key: DedupKey) -> bool {
        self.dedup.remove(key)
    }

    fn dedup_entry_ref(&self, id: SlotId) -> Option<&DedupEntry> {
        self.dedup.entry_ref(id)
    }
}

impl<T: OwnedToken> BodySlots for AllocMemory<T> {
    #[allow(clippy::too_many_arguments)]
    fn replace_observe_body(
        &mut self,
        pending: super::PendingCon,
        original: ObserveInterest<Self::Token>,
        replacement: ObserveInterest<Self::Token>,
        bytes: &[u8],
        previous: Option<SlotId>,
        transfer: BlockTransfer<Self::Token>,
        payload: &[u8],
    ) -> Result<(), super::PendingReplaceError>
    where
        Self: Storage + PendingCons + ObserveSlots,
    {
        super::table::replace_owned_observe_body(
            self,
            pending,
            original,
            replacement,
            bytes,
            previous,
            transfer,
            payload,
        )
    }

    fn rx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.rx_body.as_ref()?.payload(id)
    }

    fn tx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.tx_body.as_ref()?.payload(id)
    }

    fn rx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer<T>> {
        self.rx_body.as_ref()?.transfer(id)
    }

    fn set_rx_body_transfer(
        &mut self,
        id: SlotId,
        transfer: BlockTransfer<T>,
    ) -> Result<(), SlotError> {
        self.rx_body
            .as_mut()
            .ok_or(SlotError::InvalidSlot)?
            .set_transfer(id, transfer)
    }

    fn tx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer<T>> {
        self.tx_body.as_ref()?.transfer(id)
    }

    fn set_tx_body_transfer(
        &mut self,
        id: SlotId,
        transfer: BlockTransfer<T>,
    ) -> Result<(), SlotError> {
        self.tx_body
            .as_mut()
            .ok_or(SlotError::InvalidSlot)?
            .set_transfer(id, transfer)
    }

    fn lookup_rx_body(&self, key: BlockKey<T>) -> Option<SlotId> {
        self.rx_body.as_ref()?.lookup(key)
    }

    fn lookup_tx_body(&self, key: BlockKey<T>) -> Option<SlotId> {
        self.tx_body.as_ref()?.lookup(key)
    }

    fn admit_block1(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .admit_incoming(key, BlockRole::IncomingBlock1, block, payload, size1)
    }

    fn write_block1(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .write_incoming(id, BlockRole::IncomingBlock1, block, payload)
    }

    fn apply_block1(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .apply_incoming(key, BlockRole::IncomingBlock1, block, payload, size1)
    }

    fn admit_block2(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .admit_incoming(key, BlockRole::IncomingBlock2, block, payload, size2)
    }

    fn write_block2(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .write_incoming(id, BlockRole::IncomingBlock2, block, payload)
    }

    fn apply_block2(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .apply_incoming(key, BlockRole::IncomingBlock2, block, payload, size2)
    }

    fn admit_q_block1(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .admit_incoming(key, BlockRole::IncomingQBlock1, block, payload, size1)
    }

    fn write_q_block1(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .write_incoming(id, BlockRole::IncomingQBlock1, block, payload)
    }

    fn apply_q_block1(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size1: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .apply_incoming(key, BlockRole::IncomingQBlock1, block, payload, size1)
    }

    fn admit_q_block2(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .admit_incoming(key, BlockRole::IncomingQBlock2, block, payload, size2)
    }

    fn write_q_block2(
        &mut self,
        id: SlotId,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .write_incoming(id, BlockRole::IncomingQBlock2, block, payload)
    }

    fn apply_q_block2(
        &mut self,
        key: BlockKey<T>,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .apply_incoming(key, BlockRole::IncomingQBlock2, block, payload, size2)
    }

    fn start_block1(
        &mut self,
        key: BlockKey<T>,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .start_outgoing(key, BlockRole::OutgoingBlock1, body, szx)
    }

    fn next_block1(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_outgoing(id, BlockRole::OutgoingBlock1)
    }

    fn adopt_smaller_block1(&mut self, id: SlotId, szx: u8) -> Result<(), BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .adopt_smaller_outgoing_szx(id, BlockRole::OutgoingBlock1, szx)
    }

    fn next_bert1(
        &mut self,
        id: SlotId,
        max_payload: usize,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_bert_outgoing(id, BlockRole::OutgoingBlock1, max_payload)
    }

    fn start_block2(
        &mut self,
        key: BlockKey<T>,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .start_outgoing(key, BlockRole::OutgoingBlock2, body, szx)
    }

    fn next_block2(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_outgoing(id, BlockRole::OutgoingBlock2)
    }

    fn next_bert2(
        &mut self,
        id: SlotId,
        max_payload: usize,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_bert_outgoing(id, BlockRole::OutgoingBlock2, max_payload)
    }

    fn start_q_block1(
        &mut self,
        key: BlockKey<T>,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .start_outgoing(key, BlockRole::OutgoingQBlock1, body, szx)
    }

    fn next_q_block1(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_outgoing(id, BlockRole::OutgoingQBlock1)
    }

    fn pace_q_block1(&mut self, id: SlotId) -> Result<(), BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .pace_q_block1(id)
    }

    fn ack_q_block1(&mut self, id: SlotId, num: u32) -> Result<BlockProgress, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .ack_outgoing(id, BlockRole::OutgoingQBlock1, num)
    }

    fn start_q_block2(
        &mut self,
        key: BlockKey<T>,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .start_outgoing(key, BlockRole::OutgoingQBlock2, body, szx)
    }

    fn next_q_block2(&mut self, id: SlotId) -> Result<OutgoingBlock, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .next_outgoing(id, BlockRole::OutgoingQBlock2)
    }

    fn ack_q_block2(&mut self, id: SlotId, num: u32) -> Result<BlockProgress, BlockTransferError> {
        self.tx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .ack_outgoing(id, BlockRole::OutgoingQBlock2, num)
    }

    fn access_rx_body(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.rx_body
            .as_mut()
            .ok_or(SlotError::InvalidSlot)?
            .access(id)
    }

    fn access_tx_body_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.tx_body
            .as_mut()
            .ok_or(SlotError::InvalidSlot)?
            .access_mut(id)
    }
}

impl<T: OwnedToken> ObserveSlots for AllocMemory<T> {
    fn replace_observe_con(
        &mut self,
        pending: PendingCon,
        original: ObserveInterest<T>,
        replacement: ObserveInterest<T>,
        bytes: &[u8],
    ) -> Result<(), super::PendingReplaceError>
    where
        Self: PendingCons,
    {
        let id = super::table::validate_observe_replacement(
            self,
            pending,
            original,
            replacement,
            bytes,
        )?;
        self.replace_pending_con(pending, bytes)?;
        // ObserveStore is private: the owned table cannot fail an occupied-row
        // write while this exclusive borrow retains the validated row.
        self.observe
            .set_entry(id, replacement)
            .expect("validated occupied Observe row");
        Ok(())
    }
    fn insert_observe(&mut self, interest: ObserveInterest<T>) -> Option<SlotId> {
        self.observe.insert(interest)
    }

    fn lookup_observe(&self, key: ObserveKey<T>) -> Option<SlotId> {
        self.observe.lookup(key)
    }

    fn remove_observe(&mut self, key: ObserveKey<T>) -> bool {
        self.observe.remove(key)
    }

    fn take_observe(&mut self, key: ObserveKey<T>) -> Option<ObserveInterest<T>> {
        self.observe.take(key)
    }

    fn observe_interest(&self, id: SlotId) -> Option<ObserveInterest<T>> {
        self.observe.entry(id)
    }

    fn set_observe_interest(
        &mut self,
        id: SlotId,
        interest: ObserveInterest<T>,
    ) -> Result<(), SlotError> {
        self.observe.set_entry(id, interest)
    }
}

struct AllocBodySlot<T: OwnedToken = Token> {
    buf: Vec<u8>,
    len: usize,
    transfer: Option<BlockTransfer<T>>,
}

struct AllocBodyPool<T: OwnedToken = Token> {
    slots: Vec<AllocBodySlot<T>>,
    slot_bytes: usize,
    occ: HeapOccupancy,
}

impl<T: OwnedToken> AllocBodyPool<T> {
    fn new(slots: usize, bytes: usize) -> Result<Self, BuildError> {
        let slots_storage = try_array(slots, || {
            Ok(AllocBodySlot {
                buf: try_array(bytes, || Ok(0))?,
                len: 0,
                transfer: None,
            })
        })?;
        Ok(Self {
            slots: slots_storage,
            slot_bytes: bytes,
            occ: HeapOccupancy::new(slots)?,
        })
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.len = 0;
            slot.transfer = None;
        }
    }

    fn payload(&self, id: SlotId) -> Option<&[u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        let slot = self.slots.get(id.index())?;
        Some(&slot.buf[..slot.len])
    }

    fn access(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        self.occ.try_pin(id)?;
        let idx = id.index();
        let pin = self.occ.pin_flag_mut(id).ok_or(SlotError::InvalidSlot)?;
        let slot = self.slots.get(idx).ok_or(SlotError::InvalidSlot)?;
        let bytes = &slot.buf[..slot.len];
        Ok(Access::new(id, bytes, pin))
    }

    fn access_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        self.occ.try_pin(id)?;
        let idx = id.index();
        let pin = self.occ.pin_flag_mut(id).ok_or(SlotError::InvalidSlot)?;
        let slot = self.slots.get_mut(idx).ok_or(SlotError::InvalidSlot)?;
        Ok(AccessMut::new(id, slot.buf.as_mut(), &mut slot.len, pin))
    }

    fn transfer(&self, id: SlotId) -> Option<BlockTransfer<T>> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.slots.get(id.index())?.transfer
    }

    fn set_transfer(&mut self, id: SlotId, transfer: BlockTransfer<T>) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        let slot = self
            .slots
            .get_mut(id.index())
            .ok_or(SlotError::InvalidSlot)?;
        slot.transfer = Some(transfer);
        Ok(())
    }

    fn lookup(&self, key: BlockKey<T>) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.transfer(id) {
                Some(t) if t.key() == key => Some(id),
                _ => None,
            }
        })
    }

    /// Occupied body with the exact key and transfer role.
    #[must_use]
    fn lookup_role(&self, key: BlockKey<T>, role: BlockRole) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            self.transfer(id)
                .filter(|transfer| transfer.key() == key && transfer.role() == role)
                .map(|_| id)
        })
    }

    fn lookup_identity(&self, key: BlockKey<T>, role: BlockRole) -> Option<SlotId> {
        if key.identity().is_absent() {
            return None;
        }
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.transfer(id) {
                Some(t) if same_body_identity(t, key, role) => Some(id),
                _ => None,
            }
        })
    }

    fn admit_incoming(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        let transfer = start_incoming(
            key,
            role,
            block,
            payload.len(),
            self.slot_bytes,
            expected_len,
        )?;
        let id = self.acquire().ok_or(BlockTransferError::Saturated)?;
        let offset = if role.is_q_block() {
            block_offset(block.num(), usize::from(block.size()))?
        } else {
            0
        };
        let contiguous = role.is_q_block().then_some(transfer.filled());
        let write_err = {
            let slot = &mut self.slots[id.index()];
            store_incoming(&mut slot.buf, &mut slot.len, offset, payload, contiguous).err()
        };
        if let Some(e) = write_err {
            let _ = self.release(id);
            return Err(e);
        }
        self.slots[id.index()].transfer = Some(transfer);
        Ok(id)
    }

    fn write_incoming(
        &mut self,
        id: SlotId,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied.into()
            } else {
                SlotError::InvalidSlot.into()
            });
        }
        let slot_bytes = self.slot_bytes;
        let offset = {
            let transfer = self.slots[id.index()]
                .transfer
                .as_mut()
                .ok_or(BlockTransferError::NoTransfer)?;
            if transfer.role() != role {
                return Err(BlockTransferError::IdentityMismatch);
            }
            accept_incoming_role(transfer, block, payload.len(), slot_bytes)?
        };
        let contiguous = role.is_q_block().then_some(
            self.slots[id.index()]
                .transfer
                .ok_or(BlockTransferError::NoTransfer)?
                .filled(),
        );
        let slot = &mut self.slots[id.index()];
        store_incoming(&mut slot.buf, &mut slot.len, offset, payload, contiguous)?;
        let transfer = slot.transfer.ok_or(BlockTransferError::NoTransfer)?;
        Ok(BlockProgress::new(
            id,
            transfer.filled(),
            transfer.is_complete(),
        ))
    }

    fn apply_incoming(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        if let Some(id) = self
            .lookup_role(key, role)
            .or_else(|| self.lookup_identity(key, role))
        {
            let transfer = self.transfer(id).ok_or(BlockTransferError::NoTransfer)?;
            if role.is_q_block() && transfer.expected_len() != expected_len {
                return Err(BlockTransferError::LengthInconsistent);
            }
            return self.write_incoming(id, role, block, payload);
        }
        let id = self.admit_incoming(key, role, block, payload, expected_len)?;
        let transfer = self.transfer(id).ok_or(BlockTransferError::NoTransfer)?;
        Ok(BlockProgress::new(
            id,
            transfer.filled(),
            transfer.is_complete(),
        ))
    }

    fn start_outgoing(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        let transfer = if role.is_q_block() {
            BlockTransfer::outgoing_q(key, role, body.len(), szx, self.slot_bytes)?
        } else {
            BlockTransfer::outgoing(key, role, body.len(), szx, self.slot_bytes)?
        };
        let id = self.acquire().ok_or(BlockTransferError::Saturated)?;
        let write_err = {
            let slot = &mut self.slots[id.index()];
            write_range(&mut slot.buf, &mut slot.len, 0, body).err()
        };
        if let Some(e) = write_err {
            let _ = self.release(id);
            return Err(e);
        }
        self.slots[id.index()].transfer = Some(transfer);
        Ok(id)
    }

    fn next_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied.into()
            } else {
                SlotError::InvalidSlot.into()
            });
        }
        let transfer = self.slots[id.index()]
            .transfer
            .as_mut()
            .ok_or(BlockTransferError::NoTransfer)?;
        if transfer.role() != role {
            return Err(BlockTransferError::IdentityMismatch);
        }
        let (block, offset, len) = if role.is_q_block() {
            transfer.issue_q_outgoing()?
        } else {
            transfer.issue_outgoing()?
        };
        Ok(OutgoingBlock::new(
            id,
            block,
            offset,
            len,
            transfer.is_complete(),
        ))
    }

    fn adopt_smaller_outgoing_szx(
        &mut self,
        id: SlotId,
        role: BlockRole,
        szx: u8,
    ) -> Result<(), BlockTransferError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied.into()
            } else {
                SlotError::InvalidSlot.into()
            });
        }
        let transfer = self.slots[id.index()]
            .transfer
            .as_mut()
            .ok_or(BlockTransferError::NoTransfer)?;
        if transfer.role() != role {
            return Err(BlockTransferError::IdentityMismatch);
        }
        transfer.adopt_smaller_szx(szx)
    }

    fn next_bert_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
        max_payload: usize,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied.into()
            } else {
                SlotError::InvalidSlot.into()
            });
        }
        let transfer = self.slots[id.index()]
            .transfer
            .as_mut()
            .ok_or(BlockTransferError::NoTransfer)?;
        if transfer.role() != role {
            return Err(BlockTransferError::IdentityMismatch);
        }
        let (block, offset, len) = transfer.issue_bert_outgoing(max_payload)?;
        Ok(OutgoingBlock::new(
            id,
            block,
            offset,
            len,
            transfer.is_complete(),
        ))
    }

    fn ack_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
        num: u32,
    ) -> Result<BlockProgress, BlockTransferError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied.into()
            } else {
                SlotError::InvalidSlot.into()
            });
        }
        let transfer = self.slots[id.index()]
            .transfer
            .as_mut()
            .ok_or(BlockTransferError::NoTransfer)?;
        if transfer.role() != role {
            return Err(BlockTransferError::IdentityMismatch);
        }
        let (filled, complete) = transfer.ack_q_window(num)?;
        Ok(BlockProgress::new(id, filled, complete))
    }
}

impl<T: OwnedToken> BodyOps<T> for AllocBodyPool<T> {
    fn payload(&self, id: SlotId) -> Option<&[u8]> {
        AllocBodyPool::payload(self, id)
    }

    fn transfer(&self, id: SlotId) -> Option<BlockTransfer<T>> {
        AllocBodyPool::transfer(self, id)
    }

    fn set_transfer(&mut self, id: SlotId, transfer: BlockTransfer<T>) -> Result<(), SlotError> {
        AllocBodyPool::set_transfer(self, id, transfer)
    }

    fn lookup(&self, key: BlockKey<T>) -> Option<SlotId> {
        AllocBodyPool::lookup(self, key)
    }

    fn admit_incoming(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        AllocBodyPool::admit_incoming(self, key, role, block, payload, expected_len)
    }

    fn write_incoming(
        &mut self,
        id: SlotId,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError> {
        AllocBodyPool::write_incoming(self, id, role, block, payload)
    }

    fn apply_incoming(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        AllocBodyPool::apply_incoming(self, key, role, block, payload, expected_len)
    }

    fn start_outgoing(
        &mut self,
        key: BlockKey<T>,
        role: BlockRole,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        AllocBodyPool::start_outgoing(self, key, role, body, szx)
    }

    fn next_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        AllocBodyPool::next_outgoing(self, id, role)
    }

    fn adopt_smaller_outgoing_szx(
        &mut self,
        id: SlotId,
        role: BlockRole,
        szx: u8,
    ) -> Result<(), BlockTransferError> {
        AllocBodyPool::adopt_smaller_outgoing_szx(self, id, role, szx)
    }

    fn next_bert_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
        max_payload: usize,
    ) -> Result<OutgoingBlock, BlockTransferError> {
        AllocBodyPool::next_bert_outgoing(self, id, role, max_payload)
    }

    fn ack_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
        num: u32,
    ) -> Result<BlockProgress, BlockTransferError> {
        AllocBodyPool::ack_outgoing(self, id, role, num)
    }

    fn access(&mut self, id: SlotId) -> Result<Access<'_>, SlotError> {
        AllocBodyPool::access(self, id)
    }

    fn access_mut(&mut self, id: SlotId) -> Result<AccessMut<'_>, SlotError> {
        AllocBodyPool::access_mut(self, id)
    }
}

impl<T: OwnedToken> SlotPool for AllocBodyPool<T> {
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

    fn advance(&mut self, steps: usize) {
        self.occ.advance(steps);
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

    fn is_pinned(&self, id: SlotId) -> bool {
        self.occ.is_pinned(id)
    }

    fn cursor(&self) -> usize {
        self.occ.cursor()
    }
}

struct AllocDedupTable {
    entries: Vec<Option<DedupEntry>>,
    occ: HeapOccupancy,
}

impl AllocDedupTable {
    fn new(entries: usize) -> Result<Self, BuildError> {
        Ok(Self {
            entries: try_array(entries, || Ok(None))?,
            occ: HeapOccupancy::new(entries)?,
        })
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl DedupStore for AllocDedupTable {
    fn insert(&mut self, entry: DedupEntry) -> Option<SlotId> {
        if let Some(id) = self.lookup(entry.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(entry);
        Some(id)
    }

    fn lookup(&self, key: DedupKey) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry_ref(id) {
                Some(entry) if entry.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn remove(&mut self, key: DedupKey) -> bool {
        match self.lookup(key) {
            Some(id) => self.release(id).is_ok(),
            None => false,
        }
    }

    fn entry_ref(&self, id: SlotId) -> Option<&DedupEntry> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).and_then(Option::as_ref)
    }
}

struct AllocExchangeTable<T: OwnedToken = Token> {
    entries: Vec<Option<ExchangeEntry<T>>>,
    occ: HeapOccupancy,
}

impl<T: OwnedToken> AllocExchangeTable<T> {
    fn new(entries: usize) -> Result<Self, BuildError> {
        Ok(Self {
            entries: try_array(entries, || Ok(None))?,
            occ: HeapOccupancy::new(entries)?,
        })
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl<T: OwnedToken> ExchangeStore<T> for AllocExchangeTable<T> {
    fn insert(&mut self, entry: ExchangeEntry<T>) -> Option<SlotId> {
        if let Some(id) = self.lookup(entry.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(entry);
        Some(id)
    }

    fn lookup(&self, key: ExchangeKey<T>) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(entry) if entry.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn take(&mut self, key: ExchangeKey<T>) -> Option<ExchangeEntry<T>> {
        let id = self.lookup(key)?;
        let entry = self.entry(id)?;
        let _ = self.release(id);
        Some(entry)
    }

    fn entry(&self, id: SlotId) -> Option<ExchangeEntry<T>> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }
}

impl<T: OwnedToken> SlotPool for AllocExchangeTable<T> {
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

    fn advance(&mut self, steps: usize) {
        self.occ.advance(steps);
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

impl SlotPool for AllocDedupTable {
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

    fn advance(&mut self, steps: usize) {
        self.occ.advance(steps);
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

struct AllocObserveTable<T: OwnedToken = Token> {
    entries: Vec<Option<ObserveInterest<T>>>,
    occ: HeapOccupancy,
}

impl<T: OwnedToken> AllocObserveTable<T> {
    fn new(entries: usize) -> Result<Self, BuildError> {
        Ok(Self {
            entries: try_array(entries, || Ok(None))?,
            occ: HeapOccupancy::new(entries)?,
        })
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl<T: OwnedToken> ObserveStore<T> for AllocObserveTable<T> {
    fn insert(&mut self, interest: ObserveInterest<T>) -> Option<SlotId> {
        if let Some(id) = self.lookup(interest.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(interest);
        Some(id)
    }

    fn lookup(&self, key: ObserveKey<T>) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(interest) if interest.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn remove(&mut self, key: ObserveKey<T>) -> bool {
        match self.lookup(key) {
            Some(id) => self.release(id).is_ok(),
            None => false,
        }
    }

    fn take(&mut self, key: ObserveKey<T>) -> Option<ObserveInterest<T>> {
        let id = self.lookup(key)?;
        let interest = self.entry(id)?;
        let _ = self.release(id);
        Some(interest)
    }

    fn entry(&self, id: SlotId) -> Option<ObserveInterest<T>> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }

    fn set_entry(&mut self, id: SlotId, interest: ObserveInterest<T>) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.entries[id.index()] = Some(interest);
        Ok(())
    }
}

impl<T: OwnedToken> SlotPool for AllocObserveTable<T> {
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

    fn advance(&mut self, steps: usize) {
        self.occ.advance(steps);
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
