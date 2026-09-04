//! [`AllocMemory`]: runtime-sized Storage, allocated once, then no growth.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::BodySlots;
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
use super::SlotPool;
use super::Storage;
use super::block::{
    BlockKey, BlockProgress, BlockRole, BlockTransfer, BodyOps, OutgoingBlock, write_range,
};
use super::capacities::Capacities;
use super::exchange::ExchangeStore;
use super::occupancy::HeapOccupancy;
use super::slot::{SlotError, SlotId};
use super::table::{DedupStore, ObserveStore};
use crate::error::{BlockTransferError, BuildError};
use crate::message::{BlockValue, MessageId};

/// `alloc` backend: runtime [`Capacities`], typed heap arrays, no later growth.
///
/// Each area is a `Box<[T]>` (and each byte slot a `Box<[u8]>` of the configured
/// length) created at init. This is not a carved byte slab.
pub struct AllocMemory {
    rx: AllocDatagramPool,
    tx: AllocDatagramPool,
    dedup: AllocDedupTable,
    observe: AllocObserveTable,
    exchange: AllocExchangeTable,
    rx_body: Option<AllocBodyPool>,
    tx_body: Option<AllocBodyPool>,
}

impl AllocMemory {
    pub(crate) fn from_capacities(
        capacities: Capacities,
        block_wise: bool,
    ) -> Result<Self, BuildError> {
        capacities.validate_for_build(block_wise)?;
        let (rx_body, tx_body) = if block_wise {
            (
                Some(AllocBodyPool::new(
                    capacities.rx_body_slots.expect("validated"),
                    capacities.rx_body_bytes.expect("validated"),
                )),
                Some(AllocBodyPool::new(
                    capacities.tx_body_slots.expect("validated"),
                    capacities.tx_body_bytes.expect("validated"),
                )),
            )
        } else {
            (None, None)
        };
        Ok(Self {
            rx: AllocDatagramPool::new(capacities.rx_datagram_slots, capacities.rx_datagram_bytes),
            tx: AllocDatagramPool::new(capacities.tx_datagram_slots, capacities.tx_datagram_bytes),
            dedup: AllocDedupTable::new(capacities.dedup_entries),
            observe: AllocObserveTable::new(capacities.observe_entries),
            exchange: AllocExchangeTable::new(capacities.tx_datagram_slots),
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

impl core::fmt::Debug for AllocMemory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AllocMemory")
            .field("capacities", &self.capacities())
            .finish_non_exhaustive()
    }
}

impl Storage for AllocMemory {
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
    buf: Box<[u8]>,
    len: usize,
    endpoint: Option<Endpoint>,
    pending_mid: Option<MessageId>,
}

struct AllocDatagramPool {
    slots: Box<[AllocSlot]>,
    slot_bytes: usize,
    occ: HeapOccupancy,
}

impl AllocDatagramPool {
    fn new(slots: usize, bytes: usize) -> Self {
        let mut v = Vec::with_capacity(slots);
        for _ in 0..slots {
            v.push(AllocSlot {
                buf: vec![0u8; bytes].into_boxed_slice(),
                len: 0,
                endpoint: None,
                pending_mid: None,
            });
        }
        Self {
            slots: v.into_boxed_slice(),
            slot_bytes: bytes,
            occ: HeapOccupancy::new(slots),
        }
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.len = 0;
            slot.endpoint = None;
            slot.pending_mid = None;
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
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.slots.get(id.index())?.pending_mid
    }

    fn set_pending_mid(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < self.occ.slot_count() {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.slots[id.index()].pending_mid = Some(message_id);
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
        self.slots[id.index()].pending_mid = None;
        Ok(())
    }

    fn lookup_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            if self.pending_mid(id) == Some(message_id) && self.endpoint(id) == Some(endpoint) {
                Some(id)
            } else {
                None
            }
        })
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

impl DatagramSlots for AllocMemory {
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
}

impl PendingCons for AllocMemory {
    fn record_pending_con(
        &mut self,
        id: SlotId,
        endpoint: Endpoint,
        message_id: MessageId,
    ) -> Option<SlotId> {
        self.tx.set_endpoint(id, endpoint).ok()?;
        self.tx.set_pending_mid(id, message_id).ok()?;
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
        Some(PendingCon::new(message_id, endpoint, id))
    }
}

impl Exchanges for AllocMemory {
    fn insert_exchange(&mut self, entry: ExchangeEntry) -> Option<SlotId> {
        self.exchange.insert(entry)
    }

    fn lookup_exchange(&self, key: ExchangeKey) -> Option<SlotId> {
        self.exchange.lookup(key)
    }

    fn take_exchange(&mut self, key: ExchangeKey) -> Option<ExchangeEntry> {
        self.exchange.take(key)
    }

    fn exchange_entry(&self, id: SlotId) -> Option<ExchangeEntry> {
        self.exchange.entry(id)
    }
}

impl DedupSlots for AllocMemory {
    fn insert_dedup(&mut self, entry: DedupEntry) -> Option<SlotId> {
        self.dedup.insert(entry)
    }

    fn lookup_dedup(&self, key: DedupKey) -> Option<SlotId> {
        self.dedup.lookup(key)
    }

    fn remove_dedup(&mut self, key: DedupKey) -> bool {
        self.dedup.remove(key)
    }

    fn dedup_entry(&self, id: SlotId) -> Option<DedupEntry> {
        self.dedup.entry(id)
    }
}

impl BodySlots for AllocMemory {
    fn rx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.rx_body.as_ref()?.payload(id)
    }

    fn tx_body_payload(&self, id: SlotId) -> Option<&[u8]> {
        self.tx_body.as_ref()?.payload(id)
    }

    fn rx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.rx_body.as_ref()?.transfer(id)
    }

    fn tx_body_transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        self.tx_body.as_ref()?.transfer(id)
    }

    fn lookup_rx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.rx_body.as_ref()?.lookup(key)
    }

    fn lookup_tx_body(&self, key: BlockKey) -> Option<SlotId> {
        self.tx_body.as_ref()?.lookup(key)
    }

    fn admit_block1(
        &mut self,
        key: BlockKey,
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
        key: BlockKey,
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
        key: BlockKey,
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
        key: BlockKey,
        block: BlockValue,
        payload: &[u8],
        size2: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        self.rx_body
            .as_mut()
            .ok_or(BlockTransferError::NoBodyPools)?
            .apply_incoming(key, BlockRole::IncomingBlock2, block, payload, size2)
    }

    fn start_block1(
        &mut self,
        key: BlockKey,
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

    fn start_block2(
        &mut self,
        key: BlockKey,
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
}

impl ObserveSlots for AllocMemory {
    fn insert_observe(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        self.observe.insert(interest)
    }

    fn lookup_observe(&self, key: ObserveKey) -> Option<SlotId> {
        self.observe.lookup(key)
    }

    fn remove_observe(&mut self, key: ObserveKey) -> bool {
        self.observe.remove(key)
    }

    fn take_observe(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        self.observe.take(key)
    }

    fn observe_interest(&self, id: SlotId) -> Option<ObserveInterest> {
        self.observe.entry(id)
    }
}

struct AllocBodySlot {
    buf: Box<[u8]>,
    len: usize,
    transfer: Option<BlockTransfer>,
}

struct AllocBodyPool {
    slots: Box<[AllocBodySlot]>,
    slot_bytes: usize,
    occ: HeapOccupancy,
}

impl AllocBodyPool {
    fn new(slots: usize, bytes: usize) -> Self {
        let mut v = Vec::with_capacity(slots);
        for _ in 0..slots {
            v.push(AllocBodySlot {
                buf: vec![0u8; bytes].into_boxed_slice(),
                len: 0,
                transfer: None,
            });
        }
        Self {
            slots: v.into_boxed_slice(),
            slot_bytes: bytes,
            occ: HeapOccupancy::new(slots),
        }
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

    fn transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.slots.get(id.index())?.transfer
    }

    fn lookup(&self, key: BlockKey) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.transfer(id) {
                Some(t) if t.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn admit_incoming(
        &mut self,
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<SlotId, BlockTransferError> {
        let transfer = BlockTransfer::incoming(
            key,
            role,
            block,
            payload.len(),
            self.slot_bytes,
            expected_len,
        )?;
        let id = self.acquire().ok_or(BlockTransferError::Saturated)?;
        let write_err = {
            let slot = &mut self.slots[id.index()];
            write_range(&mut slot.buf, &mut slot.len, 0, payload).err()
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
            transfer.accept_incoming(block, payload.len(), slot_bytes)?
        };
        let slot = &mut self.slots[id.index()];
        write_range(&mut slot.buf, &mut slot.len, offset, payload)?;
        let transfer = slot.transfer.ok_or(BlockTransferError::NoTransfer)?;
        Ok(BlockProgress::new(
            id,
            transfer.filled(),
            transfer.is_complete(),
        ))
    }

    fn apply_incoming(
        &mut self,
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        if let Some(id) = self.lookup(key) {
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
        key: BlockKey,
        role: BlockRole,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError> {
        let transfer = BlockTransfer::outgoing(key, role, body.len(), szx, self.slot_bytes)?;
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
        let (block, offset, len) = transfer.issue_outgoing()?;
        Ok(OutgoingBlock::new(
            id,
            block,
            offset,
            len,
            transfer.is_complete(),
        ))
    }
}

impl BodyOps for AllocBodyPool {
    fn payload(&self, id: SlotId) -> Option<&[u8]> {
        AllocBodyPool::payload(self, id)
    }

    fn transfer(&self, id: SlotId) -> Option<BlockTransfer> {
        AllocBodyPool::transfer(self, id)
    }

    fn lookup(&self, key: BlockKey) -> Option<SlotId> {
        AllocBodyPool::lookup(self, key)
    }

    fn admit_incoming(
        &mut self,
        key: BlockKey,
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
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError> {
        AllocBodyPool::apply_incoming(self, key, role, block, payload, expected_len)
    }

    fn start_outgoing(
        &mut self,
        key: BlockKey,
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
}

impl SlotPool for AllocBodyPool {
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

struct AllocDedupTable {
    entries: Box<[Option<DedupEntry>]>,
    occ: HeapOccupancy,
}

impl AllocDedupTable {
    fn new(entries: usize) -> Self {
        Self {
            entries: vec![None; entries].into_boxed_slice(),
            occ: HeapOccupancy::new(entries),
        }
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
            match self.entry(id) {
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

    fn entry(&self, id: SlotId) -> Option<DedupEntry> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }
}

struct AllocExchangeTable {
    entries: Box<[Option<ExchangeEntry>]>,
    occ: HeapOccupancy,
}

impl AllocExchangeTable {
    fn new(entries: usize) -> Self {
        Self {
            entries: vec![None; entries].into_boxed_slice(),
            occ: HeapOccupancy::new(entries),
        }
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl ExchangeStore for AllocExchangeTable {
    fn insert(&mut self, entry: ExchangeEntry) -> Option<SlotId> {
        if let Some(id) = self.lookup(entry.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(entry);
        Some(id)
    }

    fn lookup(&self, key: ExchangeKey) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(entry) if entry.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn take(&mut self, key: ExchangeKey) -> Option<ExchangeEntry> {
        let id = self.lookup(key)?;
        let entry = self.entry(id)?;
        let _ = self.release(id);
        Some(entry)
    }

    fn entry(&self, id: SlotId) -> Option<ExchangeEntry> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }
}

impl SlotPool for AllocExchangeTable {
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

struct AllocObserveTable {
    entries: Box<[Option<ObserveInterest>]>,
    occ: HeapOccupancy,
}

impl AllocObserveTable {
    fn new(entries: usize) -> Self {
        Self {
            entries: vec![None; entries].into_boxed_slice(),
            occ: HeapOccupancy::new(entries),
        }
    }

    fn reset(&mut self, id: SlotId) {
        if let Some(slot) = self.entries.get_mut(id.index()) {
            *slot = None;
        }
    }
}

impl ObserveStore for AllocObserveTable {
    fn insert(&mut self, interest: ObserveInterest) -> Option<SlotId> {
        if let Some(id) = self.lookup(interest.key()) {
            return Some(id);
        }
        let id = self.occ.acquire()?;
        self.entries[id.index()] = Some(interest);
        Some(id)
    }

    fn lookup(&self, key: ObserveKey) -> Option<SlotId> {
        (0..self.occ.slot_count()).find_map(|i| {
            let id = SlotId::from_index(i);
            match self.entry(id) {
                Some(interest) if interest.key() == key => Some(id),
                _ => None,
            }
        })
    }

    fn remove(&mut self, key: ObserveKey) -> bool {
        match self.lookup(key) {
            Some(id) => self.release(id).is_ok(),
            None => false,
        }
    }

    fn take(&mut self, key: ObserveKey) -> Option<ObserveInterest> {
        let id = self.lookup(key)?;
        let interest = self.entry(id)?;
        let _ = self.release(id);
        Some(interest)
    }

    fn entry(&self, id: SlotId) -> Option<ObserveInterest> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.entries.get(id.index()).copied().flatten()
    }
}

impl SlotPool for AllocObserveTable {
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
