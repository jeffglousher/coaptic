//! [`DatagramPool`] and [`BodyPool`]: byte buffers, occupancy, rotating cursor.

use super::SlotPool;
use super::endpoint::Endpoint;
use super::occupancy::Occupancy;
use super::slot::{SlotError, SlotId};
use crate::error::SlotMessageError;
use crate::message::{Message, MessageId, ParsedMessage};

/// Pool of datagram slots (RX and TX are two pools of this type).
///
/// Each slot holds CoAP message bytes (UDP payload) plus sidecar [`Endpoint`]
/// metadata. On TX slots, an optional pending Message ID marks a CON waiting
/// for ACK/RST. Neither sidecar is stored in the byte buffer.
pub struct DatagramPool<const SLOTS: usize, const BYTES: usize> {
    bytes: [[u8; BYTES]; SLOTS],
    lens: [usize; SLOTS],
    endpoints: [Option<Endpoint>; SLOTS],
    pending_mids: [Option<MessageId>; SLOTS],
    occ: Occupancy<SLOTS>,
}

impl<const SLOTS: usize, const BYTES: usize> DatagramPool<SLOTS, BYTES> {
    /// Empty pool. Cursor starts at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [[0u8; BYTES]; SLOTS],
            lens: [0; SLOTS],
            endpoints: [None; SLOTS],
            pending_mids: [None; SLOTS],
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

    /// Decode the occupied slot's filled bytes with [`crate::message::decode`].
    ///
    /// Does not run format or unrecognized-critical checks.
    pub fn decode(&self, id: SlotId) -> Result<ParsedMessage<'_>, SlotMessageError> {
        crate::message::decode(self.payload(id).ok_or(SlotError::NotOccupied)?)
            .map_err(SlotMessageError::Parse)
    }

    /// Encode `msg` into the occupied slot and record the filled length.
    pub fn encode(&mut self, id: SlotId, msg: &Message<'_>) -> Result<usize, SlotMessageError> {
        let n = {
            let buf = self.payload_mut(id).ok_or(SlotError::NotOccupied)?;
            crate::message::encode(msg, buf).map_err(SlotMessageError::Encode)?
        };
        self.set_len(id, n)?;
        Ok(n)
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

    /// Copy `bytes` into an occupied slot, record length, and set sidecar [`Endpoint`].
    pub fn write(
        &mut self,
        id: SlotId,
        bytes: &[u8],
        endpoint: Endpoint,
    ) -> Result<usize, SlotError> {
        {
            let buf = self.payload_mut(id).ok_or(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            })?;
            if bytes.len() > buf.len() {
                return Err(SlotError::LengthExceedsSlot);
            }
            buf[..bytes.len()].copy_from_slice(bytes);
        }
        self.set_len(id, bytes.len())?;
        self.set_endpoint(id, endpoint)?;
        Ok(bytes.len())
    }

    /// Sidecar endpoint for an occupied slot. `None` if free or not yet set.
    #[must_use]
    pub fn endpoint(&self, id: SlotId) -> Option<Endpoint> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.endpoints[id.index()]
    }

    /// Set sidecar endpoint metadata. Not written into the byte buffer.
    pub fn set_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.endpoints[id.index()] = Some(endpoint);
        Ok(())
    }

    /// Pending CON Message ID for an occupied slot, if marked.
    #[must_use]
    pub fn pending_mid(&self, id: SlotId) -> Option<MessageId> {
        if !self.occ.is_occupied(id) {
            return None;
        }
        self.pending_mids.get(id.index()).copied().flatten()
    }

    /// Mark occupied `id` as a pending CON with `message_id`.
    pub fn set_pending_mid(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.pending_mids[id.index()] = Some(message_id);
        Ok(())
    }

    /// Clear the pending-CON mark. The slot stays occupied.
    pub fn clear_pending_mid(&mut self, id: SlotId) -> Result<(), SlotError> {
        if !self.occ.is_occupied(id) {
            return Err(if id.index() < SLOTS {
                SlotError::NotOccupied
            } else {
                SlotError::InvalidSlot
            });
        }
        self.pending_mids[id.index()] = None;
        Ok(())
    }

    /// Occupied slot whose pending MID and sidecar endpoint match, if any.
    ///
    /// Scans the configured slot count (O(n)).
    #[must_use]
    pub fn lookup_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        (0..SLOTS).find_map(|i| {
            let id = SlotId::from_index(i);
            if self.pending_mid(id) == Some(message_id) && self.endpoint(id) == Some(endpoint) {
                Some(id)
            } else {
                None
            }
        })
    }

    fn reset(&mut self, id: SlotId) {
        let idx = id.index();
        self.lens[idx] = 0;
        self.endpoints[idx] = None;
        self.pending_mids[idx] = None;
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

/// Byte and sidecar access for one datagram pool. Used by [`DatagramSlots`](super::DatagramSlots).
pub(crate) trait DatagramBytes {
    fn payload(&self, id: SlotId) -> Option<&[u8]>;
    fn payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]>;
    fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError>;
    fn endpoint(&self, id: SlotId) -> Option<Endpoint>;
    fn set_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError>;
    fn pending_mid(&self, id: SlotId) -> Option<MessageId>;
    fn set_pending_mid(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError>;
    fn clear_pending_mid(&mut self, id: SlotId) -> Result<(), SlotError>;
    fn lookup_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId>;
}

impl<const SLOTS: usize, const BYTES: usize> DatagramBytes for DatagramPool<SLOTS, BYTES> {
    fn payload(&self, id: SlotId) -> Option<&[u8]> {
        DatagramPool::payload(self, id)
    }

    fn payload_mut(&mut self, id: SlotId) -> Option<&mut [u8]> {
        DatagramPool::payload_mut(self, id)
    }

    fn set_len(&mut self, id: SlotId, len: usize) -> Result<(), SlotError> {
        DatagramPool::set_len(self, id, len)
    }

    fn endpoint(&self, id: SlotId) -> Option<Endpoint> {
        DatagramPool::endpoint(self, id)
    }

    fn set_endpoint(&mut self, id: SlotId, endpoint: Endpoint) -> Result<(), SlotError> {
        DatagramPool::set_endpoint(self, id, endpoint)
    }

    fn pending_mid(&self, id: SlotId) -> Option<MessageId> {
        DatagramPool::pending_mid(self, id)
    }

    fn set_pending_mid(&mut self, id: SlotId, message_id: MessageId) -> Result<(), SlotError> {
        DatagramPool::set_pending_mid(self, id, message_id)
    }

    fn clear_pending_mid(&mut self, id: SlotId) -> Result<(), SlotError> {
        DatagramPool::clear_pending_mid(self, id)
    }

    fn lookup_pending(&self, message_id: MessageId, endpoint: Endpoint) -> Option<SlotId> {
        DatagramPool::lookup_pending(self, message_id, endpoint)
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
