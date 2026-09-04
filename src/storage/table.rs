//! [`DedupTable`] and [`ObserveTable`]: fixed entry slots plus rotating occupancy.

use super::SlotPool;
use super::occupancy::Occupancy;
use super::slot::{SlotError, SlotId};

/// Placeholder dedup history row. Protocol fields are not modeled yet.
#[derive(Clone, Copy, Debug, Default)]
struct DedupEntry {
    _opaque: (),
}

/// Placeholder observe interest row. Protocol fields are not modeled yet.
#[derive(Clone, Copy, Debug, Default)]
struct ObserveEntry {
    _opaque: (),
}

/// Dedup table: compact duplicate history slots.
pub struct DedupTable<const ENTRIES: usize> {
    _entries: [DedupEntry; ENTRIES],
    occ: Occupancy<ENTRIES>,
}

impl<const ENTRIES: usize> DedupTable<ENTRIES> {
    /// Empty table. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _entries: [DedupEntry { _opaque: () }; ENTRIES],
            occ: Occupancy::new(),
        }
    }
}

impl<const ENTRIES: usize> Default for DedupTable<ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ENTRIES: usize> SlotPool for DedupTable<ENTRIES> {
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

/// Observe interest table: long-lived relation slots, not notification bodies.
pub struct ObserveTable<const ENTRIES: usize> {
    _entries: [ObserveEntry; ENTRIES],
    occ: Occupancy<ENTRIES>,
}

impl<const ENTRIES: usize> ObserveTable<ENTRIES> {
    /// Empty table. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _entries: [ObserveEntry { _opaque: () }; ENTRIES],
            occ: Occupancy::new(),
        }
    }
}

impl<const ENTRIES: usize> Default for ObserveTable<ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ENTRIES: usize> SlotPool for ObserveTable<ENTRIES> {
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
