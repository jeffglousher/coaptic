//! Pairwise OSCORE (RFC 8613) for a caller-owned security context.
//!
//! Enable the `oscore` crate feature. You derive a [`SecurityContext`] from
//! a Master Secret, optional Master Salt, Sender ID, and Recipient ID, then
//! either call [`SecurityContext::protect_request`] /
//! [`SecurityContext::unprotect_request`] (and the response pair) yourself
//! or attach the context with [`crate::App::set_oscore`]. Engine slots do
//! not store keys. Sequence numbers and the replay window live on the
//! context you own.
//!
//! This slice implements AES-CCM-16-64-128 and HKDF-SHA-256 only (the RFC
//! 8613 mandatory algorithms). COSE is the compressed OSCORE option plus
//! ciphertext payload — not a general COSE library. AES-CCM comes from
//! RustCrypto (`aes` + `ccm`); it is not hand-rolled.
//!
//! # Caller contract
//!
//! 1. Provision the Master Secret (and optional salt / ID Context) out of
//!    band. This crate does not mint secrets or call an OS RNG.
//! 2. Give each endpoint distinct Sender / Recipient IDs (they are mirrors).
//! 3. Persist or advance [`SecurityContext::sender_seq`] so a reboot does
//!    not reuse Partial IVs. See `knowledge/rfcs/rfc8613.txt` Appendix B.
//! 4. Keep the context for the lifetime of the pairwise association. App
//!    holds it only if you call [`crate::App::set_oscore`].
//! 5. At most [`LIVE_REQUESTS`] (4) in-flight Token→[`RequestRef`] rows.
//!    A fifth distinct Token is [`Error::Saturated`] — no silent eviction.
//!
//! App with a context attached is fail-closed: a non-empty datagram without
//! an OSCORE option is rejected (unprotected 4.01 on a request; a plain
//! response does not complete a [`crate::Call`]). AEAD failure is a silent
//! drop (no unprotected 4.00 decrypt oracle). Empty ACK/RST stay
//! unprotected (RFC 7252 reliability).
//!
//! # What this slice does not do
//!
//! Group OSCORE, other AEAD/HKDF algorithms, EDHOC / ACE key establishment,
//! Observe or outer Block-wise over OSCORE, and first-party DTLS (still
//! harness `DatagramIo` only).
//!
//! Wire format lives in `knowledge/rfcs/rfc8613.txt`. This module does not
//! restate it.
//!
//! ```
//! # #[cfg(feature = "oscore")]
//! # {
//! use coaptic::message::{Message, MessageId, Opt, OptionsBuilder, Token, Type, decode};
//! use coaptic::oscore::{DeriveParams, SecurityContext};
//! use coaptic::Code;
//!
//! let secret = [
//!     0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
//!     0x0d, 0x0e, 0x0f, 0x10,
//! ];
//! let salt = [0x9e, 0x7c, 0xa9, 0x22, 0x23, 0x78, 0x63, 0x40];
//! let mut client = SecurityContext::derive(DeriveParams {
//!     master_secret: &secret,
//!     master_salt: &salt,
//!     sender_id: &[],
//!     recipient_id: &[0x01],
//!     id_context: &[],
//! })
//! .unwrap();
//! let mut server = SecurityContext::derive(DeriveParams {
//!     master_secret: &secret,
//!     master_salt: &salt,
//!     sender_id: &[0x01],
//!     recipient_id: &[],
//!     id_context: &[],
//! })
//! .unwrap();
//!
//! let mut path = OptionsBuilder::<1>::new();
//! path.push(Opt::uri_path("tv1")).unwrap();
//! let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
//!     .with_token(Token::from_checked(&[1]))
//!     .with_options(path.as_slice());
//!
//! let mut wire = [0u8; 128];
//! let n = client.protect_request(&req, &mut wire).unwrap();
//! let protected = decode(&wire[..n]).unwrap();
//!
//! let mut inner = [0u8; 128];
//! let (plain, request) = server.unprotect_request(&protected, &mut inner).unwrap();
//! assert_eq!(plain.code(), Code::GET);
//! # let _ = request;
//! # }
//! ```

mod aead;
mod cbor;
mod context;
mod error;
mod header;
mod protect;

#[cfg(test)]
mod tests;

pub use context::{DeriveParams, RequestRef, SecurityContext};
pub use error::Error;
pub use header::{OscoreHeader, PartialIv};
pub use protect::{
    OscoreContext, protect_request, protect_response, unprotect_request, unprotect_response,
};

/// AES-CCM-16-64-128 key length (bytes).
pub const KEY_LEN: usize = 16;
/// AES-CCM-16-64-128 nonce length (bytes).
pub const NONCE_LEN: usize = 13;
/// AES-CCM-16-64-128 tag length (bytes).
pub const TAG_LEN: usize = 8;
/// Maximum Sender / Recipient ID length for a 13-byte nonce (`nonce_len - 6`).
pub const MAX_ID_LEN: usize = 7;
/// Maximum ID Context stored on [`SecurityContext`] in this slice.
pub const MAX_ID_CONTEXT_LEN: usize = 16;
/// Partial IV is at most 5 bytes.
pub const MAX_PIV_LEN: usize = 5;
/// Default replay-window width (RFC 8613).
pub const REPLAY_WINDOW: u64 = 32;
/// COSE AEAD algorithm number for AES-CCM-16-64-128.
pub const AEAD_AES_CCM_16_64_128: i32 = 10;
/// OSCORE version in the AAD (this RFC).
pub const OSCORE_VERSION: u8 = 1;
/// In-flight Token→[`RequestRef`] bindings on [`SecurityContext`].
///
/// Client-side: [`protect_request`] remembers so a later
/// [`unprotect_response`] can look up the request Partial IV. Same cap as
/// the App client inbox. [`SecurityContext::remember`] returns
/// [`Error::Saturated`] when a fifth distinct Token arrives. The server
/// holds [`RequestRef`] on the inbound exchange, not in this table.
pub const LIVE_REQUESTS: usize = 4;
