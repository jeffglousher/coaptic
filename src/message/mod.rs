//! CoAP messages: header, token, options, and payload.
//!
//! [`decode`] and [`encode`] operate on a datagram buffer (the UDP payload).
//! They do not require [`crate::Engine`].
//!
//! [`empty_ack`] and [`empty_rst`] build the empty ACK / RST messages used to
//! confirm or reject a CON. [`ParsedMessage::is_empty_ack`] /
//! [`ParsedMessage::is_empty_rst`] detect them after decode.
//! [`Ids`] is a wrapping Message ID counter; [`Token::mint`] /
//! [`Token::mint_from`] copy caller entropy ([`TokenSource`]). The core
//! does not call an OS RNG. [`Message::con`] / [`Message::non`] and
//! [`Ids::request`] build a CON/NON skeleton (next MID + token) with no
//! [`crate::Engine`]. [`Transmission`] names RFC 7252 §4.8 defaults and
//! [`Transmission::initial_timeout_ms`] (caller jitter). Retransmit
//! scheduling lives on [`crate::PendingCon`].
//!
//! [`OptionsBuilder`] collects [`Opt`] values (including Table 4 and
//! Block / Q-Block helpers) in any order and yields a slice for
//! [`Message::with_options`].
//!
//! [`value`] codecs interpret option values as empty, opaque, uint, or
//! string. Observe (option 6) reuses the uint codec; it is not in RFC 7252
//! Table 4. Block1 / Block2 / Size2 (RFC 7959) and Q-Block1 / Q-Block2
//! (RFC 9177) also reuse uint; [`BlockValue`] is NUM/M/SZX. None of those
//! numbers are in Table 4. [`ParsedMessage::check_rfc7252_formats`] is
//! optional and separate from wire decode and from
//! [`ParsedMessage::check_rfc7252_options`].
//!
//! Wire format lives in `knowledge/rfcs/rfc7252.txt`. This module does not
//! restate it.

mod builder;
mod decode;
mod encode;
mod id;
mod option;
pub mod value;

#[cfg(test)]
mod tests;

pub use builder::OptionsBuilder;
pub use decode::{ParsedMessage, decode};
pub use encode::{Message, encode};
pub use id::{Ids, TokenSource};
pub use option::{Opt, OptionNumber, Options};
pub use value::{
    BlockOptions, BlockValue, ContentFormat, EncodedUint, OBSERVE_DEREGISTER, OBSERVE_REGISTER,
    OBSERVE_SEQUENCE_MASK, OpaqueOptions, OptionValueFormat, OptionsByNumber, StringOptions,
    decode_block, decode_observe, decode_uint, decode_uint16, encode_block, encode_observe,
    encode_uint,
};

pub use crate::error::{OptionsFull, ValueError};

use core::fmt;

/// CoAP message type (header T field).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Type {
    /// Confirmable (CON).
    Confirmable,
    /// Non-confirmable (NON).
    NonConfirmable,
    /// Acknowledgement (ACK).
    Acknowledgement,
    /// Reset (RST).
    Reset,
}

impl Type {
    /// Decode the 2-bit T field. `None` if `bits` is outside 0..=3.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Confirmable),
            1 => Some(Self::NonConfirmable),
            2 => Some(Self::Acknowledgement),
            3 => Some(Self::Reset),
            _ => None,
        }
    }

    /// Encode as the 2-bit T field.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        match self {
            Self::Confirmable => 0,
            Self::NonConfirmable => 1,
            Self::Acknowledgement => 2,
            Self::Reset => 3,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Confirmable => "CON",
            Self::NonConfirmable => "NON",
            Self::Acknowledgement => "ACK",
            Self::Reset => "RST",
        })
    }
}

/// 16-bit CoAP Message ID.
///
/// Sequence generation is [`Ids`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageId(u16);

impl MessageId {
    /// Wrap a raw Message ID.
    #[must_use]
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// Raw Message ID.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Wrapping add. [`Ids`] holds the counter when minting a stream of IDs.
    #[must_use]
    pub const fn wrapping_add(self, n: u16) -> Self {
        Self(self.0.wrapping_add(n))
    }
}

impl From<u16> for MessageId {
    fn from(id: u16) -> Self {
        Self(id)
    }
}

/// CoAP Token (0–8 bytes).
///
/// Construct with [`Token::new`] or [`Token::mint`]. Lengths above
/// [`Token::MAX_LEN`] are rejected. Minting uses caller entropy; the core
/// does not call an OS RNG.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Token {
    bytes: [u8; 8],
    len: u8,
}

impl Token {
    /// Empty token (TKL 0).
    pub const EMPTY: Self = Self {
        bytes: [0; 8],
        len: 0,
    };

    /// Maximum Token length in bytes (RFC 7252 TKL).
    pub const MAX_LEN: usize = 8;

    /// Copy `bytes` into a token. `None` if `bytes` is longer than [`Self::MAX_LEN`].
    #[must_use]
    pub fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > Self::MAX_LEN {
            return None;
        }
        let mut token = Self::EMPTY;
        token.bytes[..bytes.len()].copy_from_slice(bytes);
        token.len = bytes.len() as u8;
        Some(token)
    }

    /// Token length in bytes (0–8).
    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    /// Whether the token is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Token bytes, without padding.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

impl Default for Token {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Token").field(&self.as_bytes()).finish()
    }
}

/// CoAP Code (`class.detail`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Code(u8);

const fn pack_code(class: u8, detail: u8) -> u8 {
    (class << 5) | detail
}

impl Code {
    /// Empty message (0.00).
    pub const EMPTY: Self = Self(0);
    /// GET (0.01).
    pub const GET: Self = Self(1);
    /// POST (0.02).
    pub const POST: Self = Self(2);
    /// PUT (0.03).
    pub const PUT: Self = Self(3);
    /// DELETE (0.04).
    pub const DELETE: Self = Self(4);

    /// 2.01 Created.
    pub const CREATED: Self = Self(pack_code(2, 1));
    /// 2.02 Deleted.
    pub const DELETED: Self = Self(pack_code(2, 2));
    /// 2.03 Valid.
    pub const VALID: Self = Self(pack_code(2, 3));
    /// 2.04 Changed.
    pub const CHANGED: Self = Self(pack_code(2, 4));
    /// 2.05 Content.
    pub const CONTENT: Self = Self(pack_code(2, 5));

    /// 4.00 Bad Request.
    pub const BAD_REQUEST: Self = Self(pack_code(4, 0));
    /// 4.01 Unauthorized.
    pub const UNAUTHORIZED: Self = Self(pack_code(4, 1));
    /// 4.02 Bad Option.
    pub const BAD_OPTION: Self = Self(pack_code(4, 2));
    /// 4.03 Forbidden.
    pub const FORBIDDEN: Self = Self(pack_code(4, 3));
    /// 4.04 Not Found.
    pub const NOT_FOUND: Self = Self(pack_code(4, 4));
    /// 4.05 Method Not Allowed.
    pub const METHOD_NOT_ALLOWED: Self = Self(pack_code(4, 5));
    /// 4.06 Not Acceptable.
    pub const NOT_ACCEPTABLE: Self = Self(pack_code(4, 6));
    /// 4.12 Precondition Failed.
    pub const PRECONDITION_FAILED: Self = Self(pack_code(4, 12));
    /// 4.13 Request Entity Too Large.
    pub const REQUEST_ENTITY_TOO_LARGE: Self = Self(pack_code(4, 13));
    /// 4.15 Unsupported Content-Format.
    pub const UNSUPPORTED_CONTENT_FORMAT: Self = Self(pack_code(4, 15));

    /// 5.00 Internal Server Error.
    pub const INTERNAL_SERVER_ERROR: Self = Self(pack_code(5, 0));
    /// 5.01 Not Implemented.
    pub const NOT_IMPLEMENTED: Self = Self(pack_code(5, 1));
    /// 5.02 Bad Gateway.
    pub const BAD_GATEWAY: Self = Self(pack_code(5, 2));
    /// 5.03 Service Unavailable.
    pub const SERVICE_UNAVAILABLE: Self = Self(pack_code(5, 3));
    /// 5.04 Gateway Timeout.
    pub const GATEWAY_TIMEOUT: Self = Self(pack_code(5, 4));
    /// 5.05 Proxying Not Supported.
    pub const PROXYING_NOT_SUPPORTED: Self = Self(pack_code(5, 5));

    /// Wrap a raw code byte.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    /// Build from class (0–7) and detail (0–31).
    #[must_use]
    pub const fn from_class_detail(class: u8, detail: u8) -> Option<Self> {
        if class > 7 || detail > 31 {
            None
        } else {
            Some(Self(pack_code(class, detail)))
        }
    }

    /// Raw code byte.
    #[must_use]
    pub const fn as_raw(self) -> u8 {
        self.0
    }

    /// 3-bit class.
    #[must_use]
    pub const fn class(self) -> u8 {
        self.0 >> 5
    }

    /// 5-bit detail.
    #[must_use]
    pub const fn detail(self) -> u8 {
        self.0 & 0x1f
    }

    /// Code 0.00.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Request method (class 0, not empty).
    #[must_use]
    pub const fn is_request(self) -> bool {
        self.class() == 0 && !self.is_empty()
    }

    /// Response (class 2, 4, or 5).
    #[must_use]
    pub const fn is_response(self) -> bool {
        matches!(self.class(), 2 | 4 | 5)
    }

    /// Class 2.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.class() == 2
    }

    /// Class 4.
    #[must_use]
    pub const fn is_client_error(self) -> bool {
        self.class() == 4
    }

    /// Class 5.
    #[must_use]
    pub const fn is_server_error(self) -> bool {
        self.class() == 5
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.class(), self.detail())
    }
}

/// Parsed 4-byte CoAP header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    version: u8,
    ty: Type,
    tkl: u8,
    code: Code,
    id: MessageId,
}

impl Header {
    /// Version field. Successful [`decode`] yields 1.
    #[must_use]
    pub const fn version(self) -> u8 {
        self.version
    }

    /// Message type.
    #[must_use]
    pub const fn ty(self) -> Type {
        self.ty
    }

    /// Token length nibble (0–8 after a successful [`decode`]).
    #[must_use]
    pub const fn token_length(self) -> u8 {
        self.tkl
    }

    /// Code.
    #[must_use]
    pub const fn code(self) -> Code {
        self.code
    }

    /// Message ID.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.id
    }

    pub(crate) const fn new(version: u8, ty: Type, tkl: u8, code: Code, id: MessageId) -> Self {
        Self {
            version,
            ty,
            tkl,
            code,
            id,
        }
    }
}

/// Empty ACK for `id`. See [`Message::empty_ack`].
#[must_use]
pub const fn empty_ack(id: MessageId) -> Message<'static> {
    Message::empty_ack(id)
}

/// Empty RST for `id`. See [`Message::empty_rst`].
#[must_use]
pub const fn empty_rst(id: MessageId) -> Message<'static> {
    Message::empty_rst(id)
}

/// RFC 7252 §4.8 defaults plus the initial-RTO helper.
///
/// The core does not draw randomness. Callers pass jitter into
/// [`Self::initial_timeout_ms`] (or `0` for the ACK_TIMEOUT floor).
/// Scheduling is [`crate::PendingCon`] / [`crate::Engine::poll_retransmit`].
/// See `knowledge/rfcs/rfc7252.txt` §4.2 / §4.8.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transmission;

impl Transmission {
    /// `ACK_TIMEOUT` in milliseconds.
    pub const ACK_TIMEOUT_MS: u32 = 2_000;
    /// `ACK_RANDOM_FACTOR` numerator (`3 / 2` = 1.5).
    pub const ACK_RANDOM_FACTOR_NUM: u16 = 3;
    /// `ACK_RANDOM_FACTOR` denominator (`3 / 2` = 1.5).
    pub const ACK_RANDOM_FACTOR_DEN: u16 = 2;
    /// Extra milliseconds jitter may add (`ACK_TIMEOUT * (ACK_RANDOM_FACTOR - 1)`).
    pub const ACK_RANDOM_SPAN_MS: u32 = Self::ACK_TIMEOUT_MS / Self::ACK_RANDOM_FACTOR_DEN as u32
        * (Self::ACK_RANDOM_FACTOR_NUM - Self::ACK_RANDOM_FACTOR_DEN) as u32;
    /// `MAX_RETRANSMIT`.
    pub const MAX_RETRANSMIT: u8 = 4;
    /// `NSTART`.
    pub const NSTART: u8 = 1;

    /// Initial timeout: `ACK_TIMEOUT` plus `min(jitter_ms, ACK_RANDOM_SPAN_MS)`.
    ///
    /// `jitter_ms = 0` is the RFC minimum. The core does not call an RNG;
    /// the caller supplies jitter (or a deterministic value).
    #[must_use]
    pub const fn initial_timeout_ms(jitter_ms: u32) -> u32 {
        let extra = if jitter_ms > Self::ACK_RANDOM_SPAN_MS {
            Self::ACK_RANDOM_SPAN_MS
        } else {
            jitter_ms
        };
        Self::ACK_TIMEOUT_MS.saturating_add(extra)
    }
}
