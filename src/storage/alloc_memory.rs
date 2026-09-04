//! [`AllocMemory`]: runtime-sized Storage, allocated once, then no growth.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::SlotPool;
use super::Storage;
use super::capacities::Capacities;
use super::occupancy::HeapOccupancy;
use super::slot::{Peer, SlotError, SlotId};
use crate::error::BuildError;

/// `alloc` backend: runtime [`Capacities`], typed heap arrays, no later growth.
///
/// Each area is a `Box<[T]>` (and each byte slot a `Box<[u8]>` of the configured
/// length) created at init. This is not a carved byte slab.
pub struct AllocMemory {
    rx: AllocDatagramPool,
    tx: AllocDatagramPool,
    dedup: AllocTable,
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
            dedup: AllocTable::new(capacities.dedup_entries),
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
    #[allow(dead_code)]
    buf: Box<[u8]>,
    #[allow(dead_code)]
    len: usize,
    #[allow(dead_code)]
    peer: Peer,
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
                peer: Peer::PLACEHOLDER,
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
            slot.peer = Peer::PLACEHOLDER;
        }
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
                peer: Peer::PLACEHOLDER,
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
