//! [`SlotId`] and sidecar peer metadata.

/// Identifier of one slot or table entry in a pool.
///
/// Values are issued by acquire. They are not stable across reuse after release.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SlotId(usize);

impl SlotId {
    pub(crate) const fn from_index(index: usize) -> Self {
        Self(index)
    }

    /// Zero-based index inside the issuing pool or table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Sidecar peer identity stored next to a datagram slot, not in its byte buffer.
///
/// A datagram slot holds CoAP message bytes (the UDP payload). Address and port
/// are metadata beside that buffer. This type is a placeholder until an
/// `Endpoint` type is designed; it carries no address bytes today.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Peer {
    _placeholder: (),
}

impl Peer {
    /// Placeholder peer. Replace when `Endpoint` exists.
    pub const PLACEHOLDER: Self = Self { _placeholder: () };
}

/// Failure when releasing or addressing a slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotError {
    /// The identifier is outside the pool or table.
    InvalidSlot,
    /// The slot is already free.
    NotOccupied,
    /// A fill length exceeded the slot byte capacity.
    LengthExceedsSlot,
}

impl core::fmt::Display for SlotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSlot => f.write_str("slot id is outside the pool"),
            Self::NotOccupied => f.write_str("slot is not occupied"),
            Self::LengthExceedsSlot => f.write_str("length exceeds slot bytes"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SlotError {}
