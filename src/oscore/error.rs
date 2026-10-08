//! Structured OSCORE failures.
//!
//! App inbound mapping (feature `oscore`, context attached):
//! - AEAD / replay / OSCORE-option processing failures are a silent drop
//!   (no unprotected 4.00 / 4.02 that would distinguish decrypt).
//! - A non-empty message with no OSCORE option is [`Error::Unprotected`].
//!   Requests become unprotected 4.01; responses do not complete a Call.

use crate::error::{EncodeError, ParseError};

/// Failure of derive, protect, or unprotect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Master Secret is empty or longer than this slice stores.
    MasterSecret,
    /// Master Salt is longer than this slice stores.
    MasterSalt,
    /// Sender ID, Recipient ID, or ID Context is too long for AES-CCM-16-64-128.
    Id,
    /// Sender and Recipient IDs are identical (nonces would collide).
    IdCollision,
    /// HKDF expand failed (output length).
    Derive,
    /// Output buffer is shorter than the protected or inner datagram.
    BufferTooSmall,
    /// Plaintext or ciphertext is empty or exceeds the scratch used here.
    MessageLength,
    /// Authenticated Inner Code has the wrong request/response direction.
    MessageCode,
    /// OSCORE option flags or field lengths are reserved / truncated.
    Header,
    /// `kid` does not match this context's Recipient ID (or ID Context).
    Context,
    /// Non-empty message has no OSCORE option while a context is attached.
    Unprotected,
    /// The live Token→[`crate::oscore::RequestRef`] table
    /// ([`crate::oscore::LIVE_REQUESTS`]) is full.
    Saturated,
    /// Partial IV is missing on a request, or too long.
    PartialIv,
    /// Sender Sequence Number is exhausted (5-byte Partial IV).
    SequenceExhausted,
    /// Attempt to move the sender sequence below its current value.
    SequenceRollback,
    /// The guarded sender has no durably granted sequence remaining.
    SequenceUnreserved,
    /// Partial IV is a replay (or left of the window).
    Replay,
    /// Persisted replay checkpoint exceeds the five-byte sequence space.
    ReplayState,
    /// Restoring this checkpoint would weaken live replay protection.
    ReplayRollback,
    /// AEAD open failed (wrong key, nonce, AAD, or ciphertext).
    Decrypt,
    /// AEAD seal failed.
    Encrypt,
    /// Inner or outer CoAP encode failed.
    Encode(EncodeError),
    /// Inner or outer CoAP decode failed.
    Parse(ParseError),
    /// Class E / U option list exceeded the fixed builder used here.
    Options,
    /// Outer Block-wise over OSCORE (proxy hop-by-hop) is not in this slice.
    /// Inner Block-wise (fragment then protect) is supported on the App path.
    /// Protected timed recovery requires a retained authenticated request reference;
    /// manually seeded advanced bodies without one are refused.
    Unsupported,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MasterSecret => f.write_str("master secret length is not supported"),
            Self::MasterSalt => f.write_str("master salt is too long"),
            Self::Id => f.write_str("sender, recipient, or ID context is too long"),
            Self::IdCollision => f.write_str("sender and recipient IDs must differ"),
            Self::Derive => f.write_str("HKDF expand failed"),
            Self::BufferTooSmall => f.write_str("OSCORE buffer is too small"),
            Self::MessageLength => f.write_str("OSCORE plaintext or ciphertext length is invalid"),
            Self::MessageCode => f.write_str("OSCORE inner code has the wrong message direction"),
            Self::Header => f.write_str("OSCORE option is malformed"),
            Self::Context => f.write_str("security context not found"),
            Self::Unprotected => f.write_str("message is not OSCORE-protected"),
            Self::Saturated => f.write_str("OSCORE request-binding table is saturated"),
            Self::PartialIv => f.write_str("Partial IV is missing or invalid"),
            Self::SequenceExhausted => f.write_str("sender sequence number is exhausted"),
            Self::SequenceRollback => f.write_str("sender sequence number cannot move backwards"),
            Self::SequenceUnreserved => {
                f.write_str("sender sequence number is not durably reserved")
            }
            Self::Replay => f.write_str("OSCORE replay"),
            Self::ReplayState => f.write_str("invalid replay checkpoint"),
            Self::ReplayRollback => f.write_str("replay checkpoint would weaken protection"),
            Self::Decrypt => f.write_str("OSCORE decryption failed"),
            Self::Encrypt => f.write_str("OSCORE encryption failed"),
            Self::Encode(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "{e}"),
            Self::Options => f.write_str("OSCORE option list is full"),
            Self::Unsupported => f.write_str("OSCORE operation is not supported in this slice"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(e) => Some(e),
            Self::Parse(e) => Some(e),
            _ => None,
        }
    }
}

impl From<EncodeError> for Error {
    fn from(e: EncodeError) -> Self {
        Self::Encode(e)
    }
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

/// Failure to extend a guarded sender reservation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SenderReservationError<E> {
    /// Enable guarding with [`super::SecurityContext::restore_sender_reservation`] first.
    NotEnabled,
    /// A reservation must grant at least one additional sequence.
    InvalidSize,
    /// The requested range overflows or exceeds the five-byte sequence space.
    SequenceExhausted,
    /// The persistence callback did not confirm a durable commit.
    Persistence(E),
}

impl<E: core::fmt::Display> core::fmt::Display for SenderReservationError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotEnabled => f.write_str("sender reservation guarding is not enabled"),
            Self::InvalidSize => f.write_str("sender reservation size must be positive"),
            Self::SequenceExhausted => f.write_str("sender reservation exceeds sequence space"),
            Self::Persistence(error) => write!(f, "sender reservation persistence failed: {error}"),
        }
    }
}

#[cfg(feature = "std")]
impl<E: std::error::Error + 'static> std::error::Error for SenderReservationError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            _ => None,
        }
    }
}
