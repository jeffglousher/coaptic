//! Rotating occupancy used by every pool and table.
//!
//! Acquire starts at the cursor and wraps. A successful acquire leaves the cursor
//! on the following index so the next pass does not restart at zero.

use crate::slot::{SlotError, SlotId};

pub(crate) fn acquire(occupied: &mut [bool], cursor: &mut usize) -> Option<SlotId> {
    let n = occupied.len();
    if n == 0 {
        return None;
    }
    for offset in 0..n {
        let idx = (*cursor + offset) % n;
        if !occupied[idx] {
            occupied[idx] = true;
            *cursor = (idx + 1) % n;
            return Some(SlotId::from_index(idx));
        }
    }
    None
}

pub(crate) fn release(occupied: &mut [bool], id: SlotId) -> Result<(), SlotError> {
    let Some(slot) = occupied.get_mut(id.index()) else {
        return Err(SlotError::InvalidSlot);
    };
    if !*slot {
        return Err(SlotError::NotOccupied);
    }
    *slot = false;
    Ok(())
}

pub(crate) fn rotate(cursor: &mut usize, n: usize) {
    if n == 0 {
        return;
    }
    *cursor = (*cursor + 1) % n;
}

pub(crate) fn occupied_count(occupied: &[bool]) -> usize {
    occupied.iter().filter(|occupied| **occupied).count()
}

pub(crate) fn is_occupied(occupied: &[bool], id: SlotId) -> bool {
    occupied.get(id.index()).copied().unwrap_or(false)
}

/// Compile-time occupancy flags plus a rotating cursor.
pub(crate) struct Occupancy<const N: usize> {
    occupied: [bool; N],
    cursor: usize,
}

impl<const N: usize> Occupancy<N> {
    pub(crate) const fn new() -> Self {
        Self {
            occupied: [false; N],
            cursor: 0,
        }
    }

    pub(crate) fn acquire(&mut self) -> Option<SlotId> {
        acquire(&mut self.occupied, &mut self.cursor)
    }

    pub(crate) fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        release(&mut self.occupied, id)
    }

    pub(crate) fn rotate(&mut self) {
        rotate(&mut self.cursor, N);
    }

    pub(crate) const fn slot_count(&self) -> usize {
        N
    }

    pub(crate) fn occupied_count(&self) -> usize {
        occupied_count(&self.occupied)
    }

    pub(crate) fn is_occupied(&self, id: SlotId) -> bool {
        is_occupied(&self.occupied, id)
    }

    pub(crate) const fn cursor(&self) -> usize {
        self.cursor
    }
}

/// Runtime occupancy for the `alloc` backend. Flags are allocated once.
#[cfg(feature = "alloc")]
pub(crate) struct HeapOccupancy {
    occupied: alloc::boxed::Box<[bool]>,
    cursor: usize,
}

#[cfg(feature = "alloc")]
impl HeapOccupancy {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            occupied: alloc::vec![false; n].into_boxed_slice(),
            cursor: 0,
        }
    }

    pub(crate) fn acquire(&mut self) -> Option<SlotId> {
        acquire(&mut self.occupied, &mut self.cursor)
    }

    pub(crate) fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        release(&mut self.occupied, id)
    }

    pub(crate) fn rotate(&mut self) {
        rotate(&mut self.cursor, self.occupied.len());
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.occupied.len()
    }

    pub(crate) fn occupied_count(&self) -> usize {
        occupied_count(&self.occupied)
    }

    pub(crate) fn is_occupied(&self, id: SlotId) -> bool {
        is_occupied(&self.occupied, id)
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }
}
