//! Temporary application [`Access`] to occupied datagram and body bytes.
//!
//! See `design.md` §Application memory access. While an [`Access`] or
//! [`AccessMut`] is live, the issuing pool pins that [`SlotId`] and
//! [`SlotPool::release`](super::SlotPool::release) returns
//! [`SlotError::Pinned`]. [`Drop`] clears the pin. Rotate only moves the
//! acquire cursor; it does not evict, so a pin does not need to block it.
//!
//! Assumption: access is exclusive. A second pin of the same slot is
//! [`SlotError::Pinned`]. Engine methods take `&mut self`, so two live
//! accesses from one engine are also rejected at compile time.

use core::fmt;
use core::ops::Deref;

use super::slot::{SlotError, SlotId};

/// Shared read of an occupied slot's filled bytes.
///
/// Holds a borrow of the payload and a pin flag on the pool occupancy word.
/// Not `Clone`: two guards would unpin twice.
#[must_use = "dropping Access unpins the slot"]
pub struct Access<'a> {
    id: SlotId,
    bytes: &'a [u8],
    pin: PinGuard<'a>,
}

/// Exclusive write of an occupied slot's byte buffer.
///
/// [`Self::bytes_mut`] is the full slot capacity. [`Self::payload`] is the filled
/// prefix. [`Self::set_len`] records how many bytes are live. Same pin
/// rule as [`Access`].
#[must_use = "dropping AccessMut unpins the slot"]
pub struct AccessMut<'a> {
    id: SlotId,
    bytes: &'a mut [u8],
    len: &'a mut usize,
    pin: PinGuard<'a>,
}

/// Clears one occupancy pin flag. Field (not a newtype wrapper around the
/// slot) so [`Access`] can also hold the byte borrow.
struct PinGuard<'a> {
    pinned: &'a mut bool,
}

impl Drop for PinGuard<'_> {
    fn drop(&mut self) {
        *self.pinned = false;
    }
}

impl<'a> Access<'a> {
    pub(crate) fn new(id: SlotId, bytes: &'a [u8], pinned: &'a mut bool) -> Self {
        Self {
            id,
            bytes,
            pin: PinGuard { pinned },
        }
    }

    /// Slot this access pins.
    #[must_use]
    pub const fn id(&self) -> SlotId {
        self.id
    }

    /// Filled payload. Same bytes as [`Deref`].
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.bytes
    }

    /// Always true while this guard is live.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        *self.pin.pinned
    }
}

impl Deref for Access<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.bytes
    }
}

impl fmt::Debug for Access<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Access")
            .field("id", &self.id)
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl<'a> AccessMut<'a> {
    pub(crate) fn new(
        id: SlotId,
        bytes: &'a mut [u8],
        len: &'a mut usize,
        pinned: &'a mut bool,
    ) -> Self {
        Self {
            id,
            bytes,
            len,
            pin: PinGuard { pinned },
        }
    }

    /// Slot this access pins.
    #[must_use]
    pub const fn id(&self) -> SlotId {
        self.id
    }

    /// Full slot capacity (not only the filled prefix).
    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        self.bytes
    }

    /// Filled payload prefix.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.bytes[..*self.len]
    }

    /// Mutable filled payload prefix.
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let n = *self.len;
        &mut self.bytes[..n]
    }

    /// Record how many bytes in the slot are the current payload.
    pub fn set_len(&mut self, len: usize) -> Result<(), SlotError> {
        if len > self.bytes.len() {
            return Err(SlotError::LengthExceedsSlot);
        }
        *self.len = len;
        Ok(())
    }

    /// Configured slot byte capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.bytes.len()
    }

    /// Always true while this guard is live.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        *self.pin.pinned
    }
}

impl fmt::Debug for AccessMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessMut")
            .field("id", &self.id)
            .field("len", self.len)
            .field("capacity", &self.bytes.len())
            .finish()
    }
}
