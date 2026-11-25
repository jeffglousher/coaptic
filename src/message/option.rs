//! Option numbers and a zero-copy option iterator.

use crate::error::ParseError;

/// One CoAP option: number plus value bytes in the datagram.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Opt<'a> {
    number: OptionNumber,
    value: &'a [u8],
}

impl<'a> Opt<'a> {
    /// Pair an option number with its value slice.
    #[must_use]
    pub const fn new(number: OptionNumber, value: &'a [u8]) -> Self {
        Self { number, value }
    }

    /// Option number.
    #[must_use]
    pub const fn number(self) -> OptionNumber {
        self.number
    }

    /// Option value bytes (opaque).
    #[must_use]
    pub const fn value(self) -> &'a [u8] {
        self.value
    }
}

/// CoAP option number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OptionNumber(u16);

impl OptionNumber {
    /// If-Match (1).
    pub const IF_MATCH: Self = Self(1);
    /// Uri-Host (3).
    pub const URI_HOST: Self = Self(3);
    /// ETag (4).
    pub const ETAG: Self = Self(4);
    /// If-None-Match (5).
    pub const IF_NONE_MATCH: Self = Self(5);
    /// Observe (6). RFC 7641; not in RFC 7252 Table 4.
    pub const OBSERVE: Self = Self(6);
    /// Uri-Port (7).
    pub const URI_PORT: Self = Self(7);
    /// Location-Path (8).
    pub const LOCATION_PATH: Self = Self(8);
    /// OSCORE (9). RFC 8613; not in RFC 7252 Table 4.
    pub const OSCORE: Self = Self(9);
    /// Uri-Path (11).
    pub const URI_PATH: Self = Self(11);
    /// Content-Format (12).
    pub const CONTENT_FORMAT: Self = Self(12);
    /// Max-Age (14).
    pub const MAX_AGE: Self = Self(14);
    /// Uri-Query (15).
    pub const URI_QUERY: Self = Self(15);
    /// Hop-Limit (16). RFC 8768; not in RFC 7252 Table 4.
    pub const HOP_LIMIT: Self = Self(16);
    /// Accept (17).
    pub const ACCEPT: Self = Self(17);
    /// Q-Block1 (19). RFC 9177; not in RFC 7252 Table 4.
    pub const Q_BLOCK1: Self = Self(19);
    /// Location-Query (20).
    pub const LOCATION_QUERY: Self = Self(20);
    /// Block2 (23). RFC 7959; not in RFC 7252 Table 4.
    pub const BLOCK2: Self = Self(23);
    /// Block1 (27). RFC 7959; not in RFC 7252 Table 4.
    pub const BLOCK1: Self = Self(27);
    /// Size2 (28). RFC 7959; not in RFC 7252 Table 4.
    pub const SIZE2: Self = Self(28);
    /// Q-Block2 (31). RFC 9177; not in RFC 7252 Table 4.
    pub const Q_BLOCK2: Self = Self(31);
    /// Proxy-Uri (35).
    pub const PROXY_URI: Self = Self(35);
    /// Proxy-Scheme (39).
    pub const PROXY_SCHEME: Self = Self(39);
    /// Size1 (60).
    pub const SIZE1: Self = Self(60);
    /// Echo (252). RFC 9175; not in RFC 7252 Table 4.
    pub const ECHO: Self = Self(252);
    /// No-Response (258). RFC 7967; not in RFC 7252 Table 4.
    pub const NO_RESPONSE: Self = Self(258);
    /// Request-Tag (292). RFC 9175; not in RFC 7252 Table 4.
    pub const REQUEST_TAG: Self = Self(292);

    /// Wrap a raw option number.
    #[must_use]
    pub const fn new(n: u16) -> Self {
        Self(n)
    }

    /// Raw option number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether this number is defined in RFC 7252 (Table 4).
    ///
    /// Other registered numbers (Observe, Block, Q-Block, Echo, Hop-Limit,
    /// No-Response, …) parse as opaque options and are not included here.
    /// See [`Self::is_known`] for the implemented stack.
    #[must_use]
    pub const fn is_rfc7252(self) -> bool {
        matches!(
            self.0,
            1 | 3 | 4 | 5 | 7 | 8 | 11 | 12 | 14 | 15 | 17 | 20 | 35 | 39 | 60
        )
    }

    /// Whether this number is in the implemented option set.
    ///
    /// RFC 7252 Table 4 plus Observe, OSCORE, Hop-Limit, Q-Block1, Block2,
    /// Block1, Size2, Q-Block2, Echo, No-Response, and Request-Tag.
    /// [`crate::App`] 4.02s a critical option that is not in this set.
    #[must_use]
    pub const fn is_known(self) -> bool {
        self.is_rfc7252()
            || matches!(
                self,
                Self::OBSERVE
                    | Self::OSCORE
                    | Self::HOP_LIMIT
                    | Self::Q_BLOCK1
                    | Self::BLOCK2
                    | Self::BLOCK1
                    | Self::SIZE2
                    | Self::Q_BLOCK2
                    | Self::ECHO
                    | Self::NO_RESPONSE
                    | Self::REQUEST_TAG
            )
    }

    /// Least-significant bit set: critical.
    #[must_use]
    pub const fn is_critical(self) -> bool {
        self.0 & 1 == 1
    }

    /// Bit 1 set: unsafe-to-forward.
    #[must_use]
    pub const fn is_unsafe(self) -> bool {
        self.0 & 2 == 2
    }

    /// NoCacheKey bit pattern on a safe-to-forward option.
    #[must_use]
    pub const fn is_no_cache_key(self) -> bool {
        self.0 & 0x1e == 0x1c
    }
}

impl From<u16> for OptionNumber {
    fn from(n: u16) -> Self {
        Self(n)
    }
}

/// Iterator over options in a decoded message.
///
/// Yields [`Opt`] values already validated by [`crate::message::decode`].
#[derive(Clone, Debug)]
pub struct Options<'a> {
    tail: OptionTail<'a>,
}

/// Position in one validated packet, including the base for its next delta.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OptionTail<'a> {
    bytes: &'a [u8],
    previous: u16,
}

impl<'a> OptionTail<'a> {
    pub(crate) fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn options(self) -> Options<'a> {
        Options { tail: self }
    }
}

impl<'a> Options<'a> {
    pub(crate) fn positioned(mut self) -> impl Iterator<Item = (OptionTail<'a>, Opt<'a>)> {
        core::iter::from_fn(move || {
            let tail = self.tail;
            self.next().map(|option| (tail, option))
        })
    }

    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self {
            tail: OptionTail { bytes, previous: 0 },
        }
    }
}

impl<'a> Iterator for Options<'a> {
    type Item = Opt<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (&head, mut rest) = self.tail.bytes.split_first()?;
        let opt = read_option(head, &mut rest, &mut self.tail.previous).ok()?;
        self.tail.bytes = rest;
        Some(opt)
    }
}

// The caller consumes the header while checking for the payload marker or
// end of input. Only extended fields consume bytes before the value.
pub(crate) fn read_option<'a>(
    head: u8,
    rest: &mut &'a [u8],
    previous: &mut u16,
) -> Result<Opt<'a>, ParseError> {
    let delta = head >> 4;
    let len = head & 15;
    if delta == 15 {
        return Err(ParseError::ReservedOptionDelta);
    }
    if len == 15 {
        return Err(ParseError::ReservedOptionLength);
    }
    let delta = extend(delta, rest)?;
    let len = extend(len, rest)?;
    let number = u16::try_from(u32::from(*previous) + delta)
        .map_err(|_| ParseError::OptionNumberOverflow)?;
    let len = usize::try_from(len).map_err(|_| ParseError::TruncatedOption)?;
    let (value, next) = rest
        .split_at_checked(len)
        .ok_or(ParseError::TruncatedOption)?;
    *previous = number;
    *rest = next;
    Ok(Opt::new(OptionNumber::new(number), value))
}

fn extend(field: u8, rest: &mut &[u8]) -> Result<u32, ParseError> {
    match field {
        13 => {
            let (&extra, next) = rest.split_first().ok_or(ParseError::TruncatedOption)?;
            *rest = next;
            Ok(13 + u32::from(extra))
        }
        14 => {
            let (extra, next) = rest
                .split_first_chunk::<2>()
                .ok_or(ParseError::TruncatedOption)?;
            *rest = next;
            Ok(269 + u32::from(super::decode_u16_be(*extra)))
        }
        _ => Ok(u32::from(field)),
    }
}
