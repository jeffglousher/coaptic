use super::{Error, lakers::ConnId};

/// A connection identifier in fixed storage, independent of credential identity.
///
/// Raw identifiers contain at most seven bytes. Local identifiers must remain
/// reserved while their handshake, security context or completion cache is live.
/// Neither an identifier nor a source address authenticates a peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionId {
    bytes: [u8; Self::CAPACITY],
    len: u8,
}

impl ConnectionId {
    /// Maximum raw identifier length, matching the bounded OSCORE profile.
    pub const CAPACITY: usize = 7;

    /// Largest positive compact identifier accepted by [`Self::new`].
    pub const MAX: u8 = 23;

    pub(super) const INITIATOR: Self = Self {
        bytes: [0; Self::CAPACITY],
        len: 1,
    };
    pub(super) const RESPONDER: Self = Self {
        bytes: [1, 0, 0, 0, 0, 0, 0],
        len: 1,
    };

    /// Validates a positive compact identifier for existing device profiles.
    pub const fn new(value: u8) -> Result<Self, Error> {
        if value <= Self::MAX {
            Ok(Self {
                bytes: [value, 0, 0, 0, 0, 0, 0],
                len: 1,
            })
        } else {
            Err(Error::Profile)
        }
    }

    /// Copies a raw identifier, including an empty or negative compact value.
    /// The caller reserves the entire byte string rather than its first byte.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > Self::CAPACITY {
            return Err(Error::Profile);
        }
        let mut id = Self {
            bytes: [0; Self::CAPACITY],
            len: bytes.len() as u8,
        };
        id.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(id)
    }

    /// Complete raw identifier used by the corresponding OSCORE context.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// Positive compact value, or `None` for other supported identifiers.
    #[must_use]
    pub const fn as_u8(self) -> Option<u8> {
        if self.len == 1 && self.bytes[0] <= Self::MAX {
            Some(self.bytes[0])
        } else {
            None
        }
    }

    pub(super) fn lakers(self) -> ConnId {
        ConnId::from_slice(self.as_bytes()).expect("validated bounded connection identifier")
    }

    pub(super) fn from_lakers(value: ConnId) -> Result<Self, Error> {
        let (id, len) = Self::decode_prefix(value.as_cbor())?;
        if len != value.as_cbor().len() {
            return Err(Error::Profile);
        }
        Ok(id)
    }

    pub(super) fn decode_prefix(bytes: &[u8]) -> Result<(Self, usize), Error> {
        match bytes {
            [value @ (0..=23 | 32..=55), ..] => Ok((Self::from_slice(&[*value])?, 1)),
            [header @ 64..=71, tail @ ..] => {
                let len = usize::from(header & 0x1f);
                let value = tail.get(..len).ok_or(Error::Parsing)?;
                if matches!(value, [0..=23 | 32..=55]) {
                    return Err(Error::Profile);
                }
                Ok((Self::from_slice(value)?, len + 1))
            }
            _ => Err(Error::Profile),
        }
    }
}
