//! [`DatagramPool`] and [`BodyPool`]: byte buffers, occupancy, rotating cursor.

use super::SlotPool;
use super::occupancy::Occupancy;
use super::slot::{Peer, SlotError, SlotId};

/// Pool of datagram slots (RX and TX are two pools of this type).
///
/// Each slot holds CoAP message bytes (UDP payload) plus sidecar [`Peer`]
/// metadata. The peer is not stored in the byte buffer.
pub struct DatagramPool<const SLOTS: usize, const BYTES: usize> {
    bytes: [[u8; BYTES]; SLOTS],
    lens: [usize; SLOTS],
    peers: [Peer; SLOTS],
    occ: Occupancy<SLOTS>,
}

impl<const SLOTS: usize, const BYTES: usize> DatagramPool<SLOTS, BYTES> {
    /// Empty pool. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [[0u8; BYTES]; SLOTS],
            lens: [0; SLOTS],
            peers: [Peer::PLACEHOLDER; SLOTS],
            occ: Occupancy::new(),
        }
    }

    /// Bytes available in each slot.
    #[must_use]
    pub const fn slot_bytes(&self) -> usize {
        BYTES
    }

    /// Filled payload in an occupied slot.
    #[must_use]
    pub fn payload(&self, id: SlotId) -> Option<&[u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        let idx = id.index();
        Some(&self.bytes[idx][..self.lens[idx]])
    }

    /// Writable slot buffer (full capacity). Caller sets the filled length.
    pub fn payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        Some(&mut self.bytes[id.index()])
    }

    /// Mark how many bytes in the slot are the current datagram.
    pub fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        if len > BYTES {
            return Err(SlotError::LengthExceedsSlot);
        }
        self.lens[id.index()] = len;
        Ok(())
    }

    /// Sidecar peer for an occupied slot.
    #[must_use]
    pub fn peer(&self, id: SlotId) -> Option<Peer> {
        self.occ.is_occupied(id).then(|| self.peers[id.index()])
    }

    /// Set sidecar peer metadata. Not written into the byte buffer.
    pub fn set_peer(&mut self, id: SlotId, peer: Peer) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.peers[id.index()] = peer;
        Ok(())
    }

    fn reset(&mut self, id: SlotId) {
        let idx = id.index();
        self.lens[idx] = 0;
        self.peers[idx] = Peer::PLACEHOLDER;
    }
}

impl<const SLOTS: usize, const BYTES: usize> Default for DatagramPool<SLOTS, BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize, const BYTES: usize> SlotPool for DatagramPool<SLOTS, BYTES> {
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

/// Pool of complete-body slots when block-wise is enabled (RX and TX are two pools).
///
/// Independent of datagram slot size. Capacity is in bytes and must be a
/// multiple of 1024 when constructed through the engine builder.
pub struct BodyPool<const SLOTS: usize, const BYTES: usize> {
    bytes: [[u8; BYTES]; SLOTS],
    lens: [usize; SLOTS],
    occ: Occupancy<SLOTS>,
}

impl<const SLOTS: usize, const BYTES: usize> BodyPool<SLOTS, BYTES> {
    /// Empty pool. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [[0u8; BYTES]; SLOTS],
            lens: [0; SLOTS],
            occ: Occupancy::new(),
        }
    }

    /// Complete-body bytes available in each slot.
    #[must_use]
    pub const fn slot_bytes(&self) -> usize {
        BYTES
    }

    /// Filled body bytes in an occupied slot.
    #[must_use]
    pub fn payload(&self, id: SlotId) -> Option<&[u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        let idx = id.index();
        Some(&self.bytes[idx][..self.lens[idx]])
    }

    /// Writable complete-body buffer (full capacity).
    pub fn payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        Some(&mut self.bytes[id.index()])
    }

    /// Mark how many bytes in the slot are the current body.
    pub fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        if len > BYTES {
            return Err(SlotError::LengthExceedsSlot);
        }
        self.lens[id.index()] = len;
        Ok(())
    }

    fn reset(&mut self, id: SlotId) {
        self.lens[id.index()] = 0;
    }
}

impl<const SLOTS: usize, const BYTES: usize> Default for BodyPool<SLOTS, BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize, const BYTES: usize> SlotPool for BodyPool<SLOTS, BYTES> {
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
