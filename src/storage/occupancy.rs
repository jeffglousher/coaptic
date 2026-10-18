//! Rotating occupancy used by every pool and table.
//!
//! Acquire starts at the cursor and wraps. A successful acquire leaves the cursor
//! on the following index so the next pass does not restart at zero.
//!
//! [`Occupancy::occupied_count`] is a live counter (increment acquire,
//! decrement release) so idle [`super::Engine::progress`] can skip empty
//! tables in O(1). Datagram and body pools pin an occupied slot while
//! [`super::Access`] is live. [`release`] returns [`SlotError::Pinned`]
//! for that id. Rotate does not evict, so it does not consult the pin bit.

use super::slot::{SlotError, SlotId};

pub(crate) fn acquire(
    occupied: &mut [bool],
    pinned: &mut [bool],
    cursor: &mut usize,
) -> Option<SlotId> {
    let n = occupied.len();
    if n == 0 {
        return None;
    }
    // Search two contiguous ranges in cursor order. In particular, a runtime
    // capacity must not require an integer division for each occupied slot.
    let start = *cursor;
    let idx = if !occupied[start] {
        start
    } else {
        first_free(&occupied[start..])
            .map(|offset| start + offset)
            .or_else(|| first_free(&occupied[..start]))?
    };
    occupied[idx] = true;
    if let Some(pin) = pinned.get_mut(idx) {
        *pin = false;
    }
    *cursor = if idx + 1 == n { 0 } else { idx + 1 };
    Some(SlotId::from_index(idx))
}

#[inline]
fn first_free(occupied: &[bool]) -> Option<usize> {
    const CHUNK: usize = 32;
    if occupied.len() < CHUNK {
        return occupied.iter().position(|&used| !used);
    }
    let (chunks, tail) = occupied.as_chunks::<CHUNK>();
    for (index, chunk) in chunks.iter().enumerate() {
        if *chunk != [true; CHUNK] {
            return chunk
                .iter()
                .position(|&used| !used)
                .map(|i| index * CHUNK + i);
        }
    }
    tail.iter()
        .position(|&used| !used)
        .map(|i| occupied.len() - tail.len() + i)
}

pub(crate) fn release(occupied: &mut [bool], pinned: &[bool], id: SlotId) -> Result<(), SlotError> {
    let Some(slot) = occupied.get_mut(id.index()) else {
        return Err(SlotError::InvalidSlot);
    };
    if !*slot {
        return Err(SlotError::NotOccupied);
    }
    if pinned.get(id.index()).copied().unwrap_or(false) {
        return Err(SlotError::Pinned);
    }
    *slot = false;
    Ok(())
}

pub(crate) fn try_pin(occupied: &[bool], pinned: &mut [bool], id: SlotId) -> Result<(), SlotError> {
    let Some(&occ) = occupied.get(id.index()) else {
        return Err(SlotError::InvalidSlot);
    };
    if !occ {
        return Err(SlotError::NotOccupied);
    }
    let pin = pinned.get_mut(id.index()).ok_or(SlotError::InvalidSlot)?;
    if *pin {
        return Err(SlotError::Pinned);
    }
    *pin = true;
    Ok(())
}

#[cfg(test)]
pub(crate) fn unpin(pinned: &mut [bool], id: SlotId) -> Result<(), SlotError> {
    let pin = pinned.get_mut(id.index()).ok_or(SlotError::InvalidSlot)?;
    *pin = false;
    Ok(())
}

pub(crate) fn is_pinned(pinned: &[bool], id: SlotId) -> bool {
    pinned.get(id.index()).copied().unwrap_or(false)
}

#[cfg(feature = "alloc")]
pub(crate) fn advance(cursor: &mut usize, n: usize, steps: usize) {
    if n == 0 {
        return;
    }
    *cursor = (*cursor + (steps % n)) % n;
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
    pinned: [bool; N],
    cursor: usize,
    count: usize,
}

impl<const N: usize> Occupancy<N> {
    pub(crate) const fn new() -> Self {
        Self {
            occupied: [false; N],
            pinned: [false; N],
            cursor: 0,
            count: 0,
        }
    }

    pub(crate) fn acquire(&mut self) -> Option<SlotId> {
        if self.count == N {
            return None;
        }
        let id = acquire(&mut self.occupied, &mut self.pinned, &mut self.cursor)?;
        self.count += 1;
        debug_assert_eq!(self.count, occupied_count(&self.occupied));
        Some(id)
    }

    pub(crate) fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        release(&mut self.occupied, &self.pinned, id)?;
        self.count -= 1;
        debug_assert_eq!(self.count, occupied_count(&self.occupied));
        Ok(())
    }

    pub(crate) fn try_pin(&mut self, id: SlotId) -> Result<(), SlotError> {
        try_pin(&self.occupied, &mut self.pinned, id)
    }

    #[cfg(test)]
    pub(crate) fn unpin(&mut self, id: SlotId) -> Result<(), SlotError> {
        unpin(&mut self.pinned, id)
    }

    pub(crate) fn pin_flag_mut(&mut self, id: SlotId) -> Option<&mut bool> {
        self.pinned.get_mut(id.index())
    }

    pub(crate) fn is_pinned(&self, id: SlotId) -> bool {
        is_pinned(&self.pinned, id)
    }

    #[cfg(test)]
    pub(crate) fn rotate(&mut self) {
        self.advance(1);
    }

    pub(crate) fn advance(&mut self, steps: usize) {
        if N == 0 {
            return;
        }
        self.set_cursor(self.cursor + (steps % N));
    }

    pub(crate) fn set_cursor(&mut self, cursor: usize) {
        if N == 0 {
            return;
        }
        self.cursor = cursor % N;
    }

    pub(crate) const fn slot_count(&self) -> usize {
        N
    }

    pub(crate) const fn occupied_count(&self) -> usize {
        self.count
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
    occupied: alloc::vec::Vec<bool>,
    pinned: alloc::vec::Vec<bool>,
    cursor: usize,
    count: usize,
}

#[cfg(feature = "alloc")]
impl HeapOccupancy {
    pub(crate) fn new(n: usize) -> Result<Self, crate::error::BuildError> {
        Ok(Self {
            occupied: super::alloc_memory::try_array(n, || Ok(false))?,
            pinned: super::alloc_memory::try_array(n, || Ok(false))?,
            cursor: 0,
            count: 0,
        })
    }

    pub(crate) fn acquire(&mut self) -> Option<SlotId> {
        if self.count == self.occupied.len() {
            return None;
        }
        let id = acquire(&mut self.occupied, &mut self.pinned, &mut self.cursor)?;
        self.count += 1;
        debug_assert_eq!(self.count, occupied_count(&self.occupied));
        Some(id)
    }

    pub(crate) fn release(&mut self, id: SlotId) -> Result<(), SlotError> {
        release(&mut self.occupied, &self.pinned, id)?;
        self.count -= 1;
        debug_assert_eq!(self.count, occupied_count(&self.occupied));
        Ok(())
    }

    pub(crate) fn try_pin(&mut self, id: SlotId) -> Result<(), SlotError> {
        try_pin(&self.occupied, &mut self.pinned, id)
    }

    pub(crate) fn pin_flag_mut(&mut self, id: SlotId) -> Option<&mut bool> {
        self.pinned.get_mut(id.index())
    }

    pub(crate) fn is_pinned(&self, id: SlotId) -> bool {
        is_pinned(&self.pinned, id)
    }

    pub(crate) fn advance(&mut self, steps: usize) {
        advance(&mut self.cursor, self.occupied.len(), steps);
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.occupied.len()
    }

    pub(crate) fn occupied_count(&self) -> usize {
        self.count
    }

    pub(crate) fn is_occupied(&self, id: SlotId) -> bool {
        is_occupied(&self.occupied, id)
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::Occupancy;
    use crate::storage::SlotError;
    use crate::storage::SlotId;

    #[test]
    fn acquire_matches_rotating_search_for_every_small_state() {
        // Exhaust the free/full and wrap cases, including stale pin flags.
        for n in 1..=8 {
            for mask in 0..(1usize << n) {
                for start in 0..n {
                    let mut occupied = [false; 8];
                    for (i, flag) in occupied[..n].iter_mut().enumerate() {
                        *flag = mask & (1 << i) != 0;
                    }
                    let before = occupied;
                    let mut pinned = [true; 8];
                    let mut cursor = start;
                    let expected = (0..n)
                        .map(|offset| (start + offset) % n)
                        .find(|&idx| !before[idx]);
                    let actual = super::acquire(&mut occupied[..n], &mut pinned[..n], &mut cursor);
                    assert_eq!(actual.map(SlotId::index), expected);
                    for i in 0..n {
                        assert_eq!(occupied[i], before[i] || expected == Some(i));
                        assert_eq!(pinned[i], expected != Some(i));
                    }
                    assert_eq!(cursor, expected.map_or(start, |idx| (idx + 1) % n));
                }
            }
        }
        let mut cursor = 0;
        assert_eq!(super::acquire(&mut [], &mut [], &mut cursor), None);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn chunk_search_preserves_order_at_every_boundary() {
        let mut flags = [true; 131];
        for n in 0..=flags.len() {
            assert_eq!(super::first_free(&flags[..n]), None);
            for hole in 0..n {
                flags[hole] = false;
                assert_eq!(super::first_free(&flags[..n]), Some(hole));
                if hole + 1 < n {
                    flags[hole + 1] = false;
                    assert_eq!(super::first_free(&flags[..n]), Some(hole));
                    flags[hole + 1] = true;
                }
                flags[hole] = true;
            }
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn heap_churn_wraps_and_preserves_pinned_refusal() {
        let mut occ = super::HeapOccupancy::new(131).expect("bounded occupancy");
        for i in 0..131 {
            assert_eq!(occ.acquire().map(SlotId::index), Some(i));
        }
        assert_eq!(occ.acquire(), None);
        let pinned = SlotId::from_index(64);
        occ.try_pin(pinned).expect("pin");
        assert_eq!(occ.release(pinned), Err(SlotError::Pinned));
        assert_eq!(occ.occupied_count(), 131);
        for i in [0, 31, 32, 63, 65, 127, 130] {
            occ.release(SlotId::from_index(i)).expect("release");
        }
        occ.advance(128);
        for i in [130, 0, 31, 32, 63, 65, 127] {
            assert_eq!(occ.acquire().map(SlotId::index), Some(i));
        }
        assert_eq!(occ.acquire(), None);
        assert!(occ.is_pinned(pinned));
        *occ.pin_flag_mut(pinned).expect("pin flag") = false;
        occ.release(pinned).expect("unpin then release");
        assert_eq!(occ.acquire(), Some(pinned));
        assert_eq!(occ.occupied_count(), 131);
        assert!(!occ.is_pinned(pinned));
    }

    #[test]
    fn pin_refuses_release() {
        let mut occ = Occupancy::<2>::new();
        let id = occ.acquire().expect("acquire");
        occ.try_pin(id).expect("pin");
        assert!(occ.is_pinned(id));
        assert_eq!(occ.release(id), Err(SlotError::Pinned));
        assert!(occ.is_occupied(id));
        occ.unpin(id).expect("unpin");
        assert!(!occ.is_pinned(id));
        occ.release(id).expect("release after unpin");
        assert!(!occ.is_occupied(id));
    }

    #[test]
    fn drop_style_unpin_allows_release() {
        let mut occ = Occupancy::<1>::new();
        let id = occ.acquire().expect("acquire");
        occ.try_pin(id).expect("pin");
        *occ.pin_flag_mut(id).expect("flag") = false;
        occ.release(id).expect("release after flag clear");
    }

    #[test]
    fn double_pin_is_exclusive() {
        let mut occ = Occupancy::<2>::new();
        let id = occ.acquire().expect("acquire");
        occ.try_pin(id).expect("first pin");
        assert_eq!(occ.try_pin(id), Err(SlotError::Pinned));
        occ.unpin(id).expect("unpin");
        occ.try_pin(id).expect("pin after unpin");
    }

    #[test]
    fn pin_requires_occupied() {
        let mut occ = Occupancy::<2>::new();
        let id = SlotId::from_index(0);
        assert_eq!(occ.try_pin(id), Err(SlotError::NotOccupied));
        assert_eq!(
            occ.try_pin(SlotId::from_index(4)),
            Err(SlotError::InvalidSlot)
        );
    }

    #[test]
    fn rotate_does_not_evict_pinned() {
        let mut occ = Occupancy::<2>::new();
        let id = occ.acquire().expect("acquire");
        occ.try_pin(id).expect("pin");
        let cursor = occ.cursor();
        occ.rotate();
        assert_ne!(occ.cursor(), cursor);
        assert!(occ.is_occupied(id));
        assert!(occ.is_pinned(id));
        assert_eq!(occ.release(id), Err(SlotError::Pinned));
    }

    #[test]
    fn advance_jumps_cursor_in_one_modulo() {
        let mut occ = Occupancy::<4>::new();
        occ.advance(3);
        assert_eq!(occ.cursor(), 3);
        occ.advance(2);
        assert_eq!(occ.cursor(), 1);
        occ.set_cursor(5);
        assert_eq!(occ.cursor(), 1);
        occ.advance(0);
        assert_eq!(occ.cursor(), 1);
    }

    #[test]
    fn occupied_count_tracks_acquire_release() {
        let mut occ = Occupancy::<2>::new();
        assert_eq!(occ.occupied_count(), 0);
        let a = occ.acquire().expect("a");
        assert_eq!(occ.occupied_count(), 1);
        let b = occ.acquire().expect("b");
        assert_eq!(occ.occupied_count(), 2);
        assert!(occ.acquire().is_none());
        assert_eq!(occ.occupied_count(), 2);
        occ.release(a).expect("release a");
        assert_eq!(occ.occupied_count(), 1);
        occ.release(b).expect("release b");
        assert_eq!(occ.occupied_count(), 0);
        assert_eq!(occ.release(a), Err(SlotError::NotOccupied));
        assert_eq!(occ.occupied_count(), 0);
    }

    #[test]
    fn occupied_count_unchanged_while_pinned() {
        let mut occ = Occupancy::<1>::new();
        let id = occ.acquire().expect("acquire");
        occ.try_pin(id).expect("pin");
        assert_eq!(occ.release(id), Err(SlotError::Pinned));
        assert_eq!(occ.occupied_count(), 1);
        occ.unpin(id).expect("unpin");
        occ.release(id).expect("release");
        assert_eq!(occ.occupied_count(), 0);
    }
}
