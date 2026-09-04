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
    /// Uri-Path (11).
    pub const URI_PATH: Self = Self(11);
    /// Content-Format (12).
    pub const CONTENT_FORMAT: Self = Self(12);
    /// Max-Age (14).
    pub const MAX_AGE: Self = Self(14);
    /// Uri-Query (15).
    pub const URI_QUERY: Self = Self(15);
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
    /// Other registered numbers (Observe, Block, Q-Block, …) parse as opaque
    /// options and are not included here.
    #[must_use]
    pub const fn is_rfc7252(self) -> bool {
        matches!(
            self.0,
            1 | 3 | 4 | 5 | 7 | 8 | 11 | 12 | 14 | 15 | 17 | 20 | 35 | 39 | 60
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
    bytes: &'a [u8],
    i: usize,
    prev: u32,
}

impl<'a> Options<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            i: 0,
            prev: 0,
        }
    }
}

impl<'a> Iterator for Options<'a> {
    type Item = Opt<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.bytes.len() {
            return None;
        }
        read_option(self.bytes, &mut self.i, &mut self.prev).ok()
    }
}

pub(crate) fn read_option<'a>(
    bytes: &'a [u8],
    i: &mut usize,
    prev: &mut u32,
) -> Result<Opt<'a>, ParseError> {
    let head = *bytes.get(*i).ok_or(ParseError::TruncatedOption)?;
    *i += 1;

    let mut delta = u32::from(head >> 4);
    let mut len = u32::from(head & 0x0f);

    if delta == 15 {
        return Err(ParseError::ReservedOptionDelta);
    }
    if len == 15 {
        return Err(ParseError::ReservedOptionLength);
    }

    take_extended(bytes, i, &mut delta)?;
    take_extended(bytes, i, &mut len)?;

    let number = prev
        .checked_add(delta)
        .filter(|&n| n <= u32::from(u16::MAX))
        .ok_or(ParseError::OptionNumberOverflow)?;
    *prev = number;

    let start = *i;
    let value_len = usize::try_from(len).map_err(|_| ParseError::TruncatedOption)?;
    let end = start
        .checked_add(value_len)
        .ok_or(ParseError::TruncatedOption)?;
    if end > bytes.len() {
        return Err(ParseError::TruncatedOption);
    }
    *i = end;

    Ok(Opt::new(
        OptionNumber::new(number as u16),
        &bytes[start..end],
    ))
}

fn take_extended(bytes: &[u8], i: &mut usize, n: &mut u32) -> Result<(), ParseError> {
    match *n {
        13 => {
            let extra = bytes.get(*i).copied().ok_or(ParseError::TruncatedOption)?;
            *n = 13 + u32::from(extra);
            *i += 1;
            Ok(())
        }
        14 => {
            if *i + 1 >= bytes.len() {
                return Err(ParseError::TruncatedOption);
            }
            *n = 269 + u32::from(u16::from_be_bytes([bytes[*i], bytes[*i + 1]]));
            *i += 2;
            Ok(())
        }
        _ => Ok(()),
    }
}
