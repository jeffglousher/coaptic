//! [`AllocMemory`]: runtime-sized Storage, allocated once, then no growth.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::DatagramSlots;
use super::DedupEntry;
use super::DedupKey;
use super::DedupSlots;
use super::Endpoint;
use super::SlotPool;
use super::Storage;
use super::capacities::Capacities;
use super::occupancy::HeapOccupancy;
use super::slot::{SlotError, SlotId};
use super::table::DedupStore;
use crate::error::BuildError;

/// `alloc` backend: runtime [`Capacities`], typed heap arrays, no later growth.
///
/// Each area is a `Box<[T]>` (and each byte slot a `Box<[u8]>` of the configured
/// length) created at init. This is not a carved byte slab.
pub struct AllocMemory {
    rx: AllocDatagramPool,
    tx: AllocDatagramPool,
    dedup: AllocDedupTable,
    observe: AllocTable,
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
            observe: AllocTable::new(capacities.observe_entries),
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

struct AllocBodyPool {
    slots: Box<[AllocSlot]>,
    slot_bytes: usize,
    occ: HeapOccupancy,
}

impl AllocBodyPool {
    fn new(slots: usize, bytes: usize) -> Self {
        let mut v = Vec::with_capacity(slots);
        for _ in 0..slots {
            v.push(AllocSlot {
                buf: vec![0u8; bytes].into_boxed_slice(),
                len: 0,
                endpoint: None,
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
        }
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

struct AllocTable {
    occ: HeapOccupancy,
}

impl AllocTable {
    fn new(entries: usize) -> Self {
        Self {
            occ: HeapOccupancy::new(entries),
        }
    }
}

impl SlotPool for AllocTable {
    fn acquire(&mut self) -> Option<SlotId> {
        self.occ.acquire()
    }

    fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        self.occ.release(id)
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
