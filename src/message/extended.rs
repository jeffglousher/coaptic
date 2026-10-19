//! RFC 8974 UDP token codecs with borrowed wire storage.
//!
//! This codec accepts extended syntax independently of application capacity.
//! [`Datagram::copy_token`] reports capacity separately from malformed input, so
//! a receiver can echo the original borrowed token in a refusal response. It
//! does not change the compact [`super::Token`] or enable extended App storage.
//! A server supporting this extension must not reset a valid over-capacity
//! request: RFC 8974 section 2.2.2 recommends 4.00 for permanent capacity limits
//! and 5.03 for temporary pressure, echoing the complete token.

use super::{Code, Header, MessageId, Opt, Options, Type};
use crate::error::{EncodeError, ParseError};

/// Maximum token length representable by RFC 8974.
pub const MAX_TOKEN_LEN: usize = 65_804;

/// Valid wire token cannot fit the selected owned storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityError {
    /// Bytes required by the token.
    pub required: usize,
    /// Bytes supported by the destination, bounded by the wire maximum.
    pub available: usize,
}

/// Explicitly budgeted owned token. `N` is the caller's byte capacity, not a
/// library-wide maximum. The existing eight-byte [`super::Token`] is unchanged.
/// This stores `N` bytes plus a length and target alignment; copies copy the
/// entire object. Use borrowed [`Datagram::token`] for stateless processing.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct Token<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Token<N> {
    /// Empty token, with no allocation.
    pub const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };

    /// Checked copy; rejects both the caller capacity and the wire maximum.
    pub fn new(bytes: &[u8]) -> Result<Self, CapacityError> {
        let available = N.min(MAX_TOKEN_LEN);
        if bytes.len() > available {
            return Err(CapacityError {
                required: bytes.len(),
                available,
            });
        }
        let mut token = Self::EMPTY;
        token.bytes[..bytes.len()].copy_from_slice(bytes);
        token.len = bytes.len();
        Ok(token)
    }

    /// Token bytes, excluding zeroed unused capacity.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Actual token byte count.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether this token is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for Token<N> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<const N: usize> core::fmt::Debug for Token<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Token").field(&self.as_bytes()).finish()
    }
}

impl<const N: usize> super::OwnedToken for Token<N> {
    const EMPTY: Self = Self::EMPTY;
    const MAX_LEN: usize = if N < MAX_TOKEN_LEN { N } else { MAX_TOKEN_LEN };
    fn try_zeroed(len: usize) -> Result<Self, CapacityError> {
        if len > Self::MAX_LEN {
            return Err(CapacityError {
                required: len,
                available: Self::MAX_LEN,
            });
        }
        let mut token = Self::EMPTY;
        token.len = len;
        Ok(token)
    }
    fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes[..self.len]
    }
    fn try_from_bytes(bytes: &[u8]) -> Result<Self, CapacityError> {
        Self::new(bytes)
    }
    fn as_bytes(&self) -> &[u8] {
        Token::as_bytes(self)
    }
}

/// Validated borrowed datagram, retaining the entire token without copying it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Datagram<'a> {
    header: Header,
    token: &'a [u8],
    options: &'a [u8],
    payload: &'a [u8],
}

impl<'a> Datagram<'a> {
    /// Header. Its token-length field is the encoded nibble, not decoded length.
    pub const fn header(self) -> Header {
        self.header
    }
    /// Message type.
    pub const fn ty(self) -> Type {
        self.header.ty()
    }
    /// Request or response code.
    pub const fn code(self) -> Code {
        self.header.code()
    }
    /// Message identifier.
    pub const fn message_id(self) -> MessageId {
        self.header.message_id()
    }
    /// Full borrowed token. Its slice length is the decoded token length.
    pub const fn token(self) -> &'a [u8] {
        self.token
    }
    /// Validated wire options using the shared message option parser.
    pub const fn options(self) -> Options<'a> {
        Options::new(self.options)
    }
    /// Payload following the marker, or an empty slice.
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }
    /// Copy into explicitly selected token storage. Capacity refusal does not
    /// discard this datagram or prevent echoing its borrowed token.
    pub fn copy_token<const N: usize>(self) -> Result<Token<N>, CapacityError> {
        Token::new(self.token)
    }
    /// Replace the borrowed token while retaining options and payload.
    /// Encoding validates the new token length and writes its matching header.
    pub fn with_token(self, token: &'a [u8]) -> Self {
        Self { token, ..self }
    }
    /// Re-encode using the shared canonical option and token encoder.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, EncodeError> {
        super::encode::encode_token_iter(
            output,
            self.ty(),
            self.code(),
            self.message_id(),
            self.token,
            self.options(),
            self.payload,
        )
    }
}

/// Decode RFC 8974 syntax without imposing application token capacity. All
/// options/payload validation is shared with [`super::decode`]. TKL 15 and
/// truncated extension/token fields are malformed, unlike [`CapacityError`].
pub fn decode(bytes: &[u8]) -> Result<Datagram<'_>, ParseError> {
    let parts = super::decode::decode_parts(bytes, true)?;
    Ok(Datagram {
        header: parts.header,
        token: parts.token,
        options: parts.options,
        payload: parts.payload,
    })
}

/// Borrowed message to serialize with an extended (or ordinary) token.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// Message type.
    pub ty: Type,
    /// CoAP request/response code.
    pub code: Code,
    /// Message identifier.
    pub id: MessageId,
    /// Token bytes; no copy or allocation is performed by this descriptor.
    pub token: &'a [u8],
    /// Options in ascending order, including equal-number repeatable options.
    pub options: &'a [Opt<'a>],
    /// Payload; no marker is emitted when this is empty.
    pub payload: &'a [u8],
}

impl Message<'_> {
    /// Serialize into caller storage. As with [`super::Message::encode`], a
    /// failure during options/payload writing can leave a partial output; the
    /// caller must transmit only a successful result's returned prefix.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, EncodeError> {
        super::encode::encode_token_iter(
            output,
            self.ty,
            self.code,
            self.id,
            self.token,
            self.options.iter().copied(),
            self.payload,
        )
    }
}

pub(super) fn decode_length(nibble: u8, rest: &[u8]) -> Result<(usize, usize), ParseError> {
    match nibble {
        0..=12 => Ok((usize::from(nibble), 0)),
        13 => Ok((
            13 + usize::from(*rest.first().ok_or(ParseError::TruncatedToken)?),
            1,
        )),
        14 => {
            let bytes = rest.get(..2).ok_or(ParseError::TruncatedToken)?;
            Ok((
                269 + usize::from(super::decode_u16_be([bytes[0], bytes[1]])),
                2,
            ))
        }
        _ => Err(ParseError::BadTokenLength),
    }
}

pub(super) fn encode_length(len: usize) -> Result<(u8, [u8; 2], usize), EncodeError> {
    let len = u32::try_from(len).map_err(|_| EncodeError::TokenTooLong)?;
    super::encode::ext_nibble(len).map_err(|_| EncodeError::TokenTooLong)
}
