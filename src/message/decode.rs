//! Parse a CoAP datagram (`&[u8]`) into a borrowed view.

use crate::error::ParseError;

use super::option::{OptionNumber, Options, read_option};
use super::{Code, Header, MessageId, OwnedToken, Token, Type};

/// Zero-copy view of a decoded CoAP datagram.
///
/// Options and payload borrow `buf` from [`decode`]. No allocator is used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedMessage<'a, T: OwnedToken = Token> {
    header: Header,
    token: T,
    options: &'a [u8],
    payload: &'a [u8],
}

/// Offsets and owned metadata from a fully validated datagram. App may use this
/// to borrow the identical retained RX bytes after releasing its scratch borrow.
/// This is not a validator for another packet, even one of the same length.
#[cfg(feature = "oscore")]
#[derive(Clone, Copy)]
pub(crate) struct MessageLayout<T: OwnedToken> {
    header: Header,
    token: T,
    wire_len: usize,
    options_len: usize,
    payload_len: usize,
}

#[cfg(feature = "oscore")]
impl<T: OwnedToken> MessageLayout<T> {
    /// Reborrow the exact bytes validated when this layout was obtained.
    /// The caller must preserve their contents; length checks only bound ranges.
    pub(crate) fn bind(self, wire: &[u8]) -> Option<ParsedMessage<'_, T>> {
        if wire.len() != self.wire_len {
            return None;
        }
        let payload_start = wire.len().checked_sub(self.payload_len)?;
        let options_end = payload_start.checked_sub(usize::from(self.payload_len != 0))?;
        let options_start = options_end.checked_sub(self.options_len)?;
        Some(ParsedMessage {
            header: self.header,
            token: self.token,
            options: &wire[options_start..options_end],
            payload: &wire[payload_start..],
        })
    }
}

impl<'a, T: OwnedToken> ParsedMessage<'a, T> {
    /// Detach ranges from the original, fully validated wire of `wire_len` bytes.
    #[cfg(feature = "oscore")]
    pub(crate) fn layout(self, wire_len: usize) -> MessageLayout<T> {
        MessageLayout {
            header: self.header,
            token: self.token,
            wire_len,
            options_len: self.options.len(),
            payload_len: self.payload.len(),
        }
    }

    /// Encoded header, token (including its length extension), and options.
    pub(crate) fn encoded_header_len(&self) -> Option<usize> {
        let len = self.token.len();
        let (_, _, extended) = super::extended::encode_length(len).ok()?;
        (4 + extended + len).checked_add(self.options.len())
    }

    /// Borrow the encoded options of the validated packet.
    pub(crate) const fn options_bytes(&self) -> &'a [u8] {
        self.options
    }

    /// Reopen the App's immutable, previously validated header-only encoding.
    /// Every byte following the token belongs to its retained option sequence.
    pub(crate) fn from_retained_header(bytes: &'a [u8]) -> Result<Self, TokenDecodeError> {
        let (header, token, start) =
            decode_prefix(bytes, true).map_err(TokenDecodeError::Malformed)?;
        Ok(Self {
            header,
            token: T::try_from_bytes(token).map_err(TokenDecodeError::Capacity)?,
            options: &bytes[start..],
            payload: &[],
        })
    }

    /// Parsed header.
    #[must_use]
    pub const fn header(self) -> Header {
        self.header
    }

    /// Message type.
    #[must_use]
    pub const fn ty(self) -> Type {
        self.header.ty()
    }

    /// Code.
    #[must_use]
    pub const fn code(self) -> Code {
        self.header.code()
    }

    /// Message ID.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        self.header.message_id()
    }

    /// Token.
    #[must_use]
    pub const fn token(self) -> T {
        self.token
    }

    /// Payload bytes after the 0xFF marker, or empty if the marker is absent.
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// Empty message (code 0.00). Successful [`decode`] already rejected extra bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.code().is_empty()
    }

    /// Empty ACK.
    #[must_use]
    pub const fn is_empty_ack(self) -> bool {
        matches!(self.ty(), Type::Acknowledgement) && self.is_empty()
    }

    /// Empty RST.
    #[must_use]
    pub const fn is_empty_rst(self) -> bool {
        matches!(self.ty(), Type::Reset) && self.is_empty()
    }

    /// Empty ACK or empty RST (the types that confirm or reject a pending CON).
    #[must_use]
    pub const fn is_empty_ack_or_rst(self) -> bool {
        self.is_empty_ack() || self.is_empty_rst()
    }

    /// Options in the message, in wire order.
    #[must_use]
    pub const fn options(self) -> Options<'a> {
        Options::new(self.options)
    }

    /// First critical option whose number is not defined in RFC 7252.
    ///
    /// Decode itself accepts any well-formed option as opaque bytes. This
    /// report is for a caller that wants Table 4 checking; it is not a
    /// 4.02 / RST decision. Block / OSCORE / … are reported here even
    /// though [`Self::unknown_critical`] treats them as known.
    #[must_use]
    pub fn unrecognized_critical(self) -> Option<OptionNumber> {
        self.options().find_map(|opt| {
            let n = opt.number();
            (n.is_critical() && !n.is_rfc7252()).then_some(n)
        })
    }

    /// First critical option whose number is not in [`OptionNumber::is_known`].
    ///
    /// [`decode`] does not call this. [`crate::App`] uses it for 4.02
    /// before handlers. Distinct from [`Self::unrecognized_critical`]
    /// (RFC 7252 Table 4 only).
    #[must_use]
    pub fn unknown_critical(self) -> Option<OptionNumber> {
        self.options().find_map(|opt| {
            let n = opt.number();
            (n.is_critical() && !n.is_known()).then_some(n)
        })
    }

    /// Fail if [`Self::unrecognized_critical`] finds an option.
    ///
    /// [`decode`] does not call this. Distinct from
    /// [`Self::check_rfc7252_formats`].
    pub fn check_rfc7252_options(self) -> Result<(), ParseError> {
        match self.unrecognized_critical() {
            Some(n) => Err(ParseError::UnrecognizedCritical(n)),
            None => Ok(()),
        }
    }

    /// Serialize this view into `buf`. Canonical option encoding is used.
    pub fn encode(self, buf: &mut [u8]) -> Result<usize, crate::error::EncodeError> {
        super::encode::encode_iter(
            buf,
            self.ty(),
            self.code(),
            self.message_id(),
            self.token(),
            self.options(),
            self.payload(),
        )
    }
}

/// Parse one CoAP datagram (UDP payload). Does not require [`crate::storage::Engine`].
///
/// Options are accepted as opaque number + value. Optional checks:
/// [`ParsedMessage::check_rfc7252_options`] (Table 4 unrecognized
/// critical), [`ParsedMessage::unknown_critical`] (implemented stack),
/// and [`ParsedMessage::check_rfc7252_formats`] (known option, wrong
/// format).
pub fn decode(buf: &[u8]) -> Result<ParsedMessage<'_>, ParseError> {
    let parts = decode_parts(buf, false)?;
    Ok(ParsedMessage {
        header: parts.header,
        token: Token::from_checked(parts.token),
        options: parts.options,
        payload: parts.payload,
    })
}

/// Syntax errors and valid tokens exceeding selected retained capacity are
/// separate outcomes. Capacity refusal must not be treated as a malformed UDP
/// request when extended-token support is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenDecodeError {
    /// Invalid/truncated wire syntax.
    Malformed(ParseError),
    /// Valid datagram, but the selected representation cannot retain its token.
    Capacity(super::extended::CapacityError),
}

/// Decode RFC 8974 syntax and retain the token in caller-selected owned storage.
/// On [`TokenDecodeError::Capacity`], [`super::extended::decode`] can still borrow
/// the full token from the same input to construct a refusal response.
pub fn decode_with_token<T: OwnedToken>(
    buf: &[u8],
) -> Result<ParsedMessage<'_, T>, TokenDecodeError> {
    super::extended::decode(buf)
        .map_err(TokenDecodeError::Malformed)?
        .retain_token()
        .map_err(TokenDecodeError::Capacity)
}

impl<'a> super::extended::Datagram<'a> {
    pub(crate) fn retain_token<T: OwnedToken>(
        self,
    ) -> Result<ParsedMessage<'a, T>, super::extended::CapacityError> {
        Ok(self.with_owned_token(T::try_from_bytes(self.token())?))
    }

    // OSCORE excludes the outer token from authentication. This projection is
    // only for authenticating a refusal; the reply echoes the original token.
    #[cfg(feature = "oscore")]
    pub(crate) fn for_capacity_refusal<T: OwnedToken>(self) -> ParsedMessage<'a, T> {
        self.with_owned_token(T::EMPTY)
    }

    fn with_owned_token<T: OwnedToken>(self, token: T) -> ParsedMessage<'a, T> {
        ParsedMessage {
            header: self.header(),
            token,
            options: self.options,
            payload: self.payload(),
        }
    }
}

pub(super) struct Parts<'a> {
    pub header: Header,
    pub token: &'a [u8],
    pub options: &'a [u8],
    pub payload: &'a [u8],
}

pub(super) fn decode_parts(buf: &[u8], extended: bool) -> Result<Parts<'_>, ParseError> {
    let (header, token, options_start) = decode_prefix(buf, extended)?;
    let (options, payload) = split_options_and_payload(buf, options_start)?;
    Ok(Parts {
        header,
        token,
        options,
        payload,
    })
}

fn decode_prefix(buf: &[u8], extended: bool) -> Result<(Header, &[u8], usize), ParseError> {
    if buf.len() < 4 {
        return Err(ParseError::TruncatedHeader);
    }

    let version = buf[0] >> 6;
    if version != 1 {
        return Err(ParseError::UnsupportedVersion);
    }

    let ty = match (buf[0] >> 4) & 0b11 {
        0 => Type::Confirmable,
        1 => Type::NonConfirmable,
        2 => Type::Acknowledgement,
        _ => Type::Reset,
    };
    let tkl = buf[0] & 0x0f;
    if !extended && tkl > 8 {
        return Err(ParseError::BadTokenLength);
    }

    let code = Code::from_raw(buf[1]);
    let id = MessageId::new(u16::from_be_bytes([buf[2], buf[3]]));

    let rest = &buf[4..];
    let (tkl_usize, prefix) = if extended {
        super::extended::decode_length(tkl, rest)?
    } else {
        (usize::from(tkl), 0)
    };
    // Keep every validation cursor relative to the original datagram so a
    // bounded backend need not reconstruct bounds across nested subslices.
    let token_start = 4usize
        .checked_add(prefix)
        .ok_or(ParseError::TruncatedToken)?;
    let options_start = token_start
        .checked_add(tkl_usize)
        .ok_or(ParseError::TruncatedToken)?;
    let token_bytes = buf
        .get(token_start..options_start)
        .ok_or(ParseError::TruncatedToken)?;

    if code.is_empty() && (tkl != 0 || options_start != buf.len()) {
        return Err(ParseError::EmptyMessageNotEmpty);
    }
    Ok((
        Header::new(version, ty, tkl, code, id),
        token_bytes,
        options_start,
    ))
}

#[cfg(feature = "oscore")]
pub(crate) fn decode_options_payload(bytes: &[u8]) -> Result<(Options<'_>, &[u8]), ParseError> {
    let (options, payload) = split_options_and_payload(bytes, 0)?;
    Ok((Options::new(options), payload))
}

fn split_options_and_payload(bytes: &[u8], start: usize) -> Result<(&[u8], &[u8]), ParseError> {
    let options = bytes.get(start..).ok_or(ParseError::TruncatedOption)?;
    let mut remaining = options;
    let mut previous = 0;
    while let Some((&head, mut rest)) = remaining.split_first() {
        if head == 0xff {
            if rest.is_empty() {
                return Err(ParseError::PayloadMarkerWithoutPayload);
            }
            return Ok((&options[..options.len() - remaining.len()], rest));
        }
        read_option(head, &mut rest, &mut previous)?;
        remaining = rest;
    }
    Ok((options, &[]))
}
