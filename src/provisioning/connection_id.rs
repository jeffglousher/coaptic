use super::{Error, lakers::ConnId};

/// A compact connection identifier supported by the bounded EDHOC profile.
///
/// Values are limited to `0..=23`. Local IDs identify OSCORE recipients and
/// must remain reserved while their handshake, security context or completion
/// cache is live. Credential identifiers and [`super::Principal`] are separate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionId(u8);

impl ConnectionId {
    /// Largest connection identifier supported by this profile.
    pub const MAX: u8 = 23;

    pub(super) const INITIATOR: Self = Self(0);
    pub(super) const RESPONDER: Self = Self(1);

    /// Validates a caller-reserved compact connection identifier.
    pub const fn new(value: u8) -> Result<Self, Error> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(Error::Profile)
        }
    }

    /// Returns the one-byte identifier value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub(super) fn lakers(self) -> ConnId {
        ConnId::from_slice(&[self.0]).expect("validated compact connection identifier")
    }

    pub(super) fn from_lakers(value: ConnId) -> Result<Self, Error> {
        match value.as_cbor() {
            [byte] => Self::new(*byte),
            _ => Err(Error::Profile),
        }
    }
}
