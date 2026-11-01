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

#[inline]
pub(crate) fn read_option<'a>(
    bytes: &'a [u8],
    i: &mut usize,
    prev: &mut u32,
) -> Result<Opt<'a>, ParseError> {
    let head = *bytes.get(*i).ok_or(ParseError::TruncatedOption)?;
    *i += 1;

    let delta = u32::from(head >> 4);
    let len = u32::from(head & 0x0f);
    let (delta, len) = if delta < 13 && len < 13 {
        (delta, len)
    } else {
        read_extended(bytes, i, delta, len)?
    };

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

// Keep extension handling out of the inline regular-header path. Reserved-field
// priority and cursor updates remain the same for malformed extended options.
#[inline(never)]
fn read_extended(
    bytes: &[u8],
    i: &mut usize,
    mut delta: u32,
    mut len: u32,
) -> Result<(u32, u32), ParseError> {
    if delta == 15 {
        return Err(ParseError::ReservedOptionDelta);
    }
    if len == 15 {
        return Err(ParseError::ReservedOptionLength);
    }
    take_extended(bytes, i, &mut delta)?;
    take_extended(bytes, i, &mut len)?;
    Ok((delta, len))
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
            *n = 269 + u32::from(super::decode_u16_be([bytes[*i], bytes[*i + 1]]));
            *i += 2;
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent pre-split parser, including its cursor/error commit order.
    fn legacy<'a>(bytes: &'a [u8], i: &mut usize, prev: &mut u32) -> Result<Opt<'a>, ParseError> {
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
        for n in [&mut delta, &mut len] {
            match *n {
                13 => {
                    let extra = bytes.get(*i).copied().ok_or(ParseError::TruncatedOption)?;
                    *n = 13 + u32::from(extra);
                    *i += 1;
                }
                14 => {
                    if *i + 1 >= bytes.len() {
                        return Err(ParseError::TruncatedOption);
                    }
                    *n = 269 + u32::from(super::super::decode_u16_be([bytes[*i], bytes[*i + 1]]));
                    *i += 2;
                }
                _ => {}
            }
        }
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

    fn compare(bytes: &[u8], cursor: usize, number: u32) {
        let (mut i, mut prev) = (cursor, number);
        let (mut old_i, mut old_prev) = (cursor, number);
        let actual = read_option(bytes, &mut i, &mut prev);
        let expected = legacy(bytes, &mut old_i, &mut old_prev);
        assert_eq!(actual, expected, "cursor={cursor}, number={number}");
        assert_eq!((i, prev), (old_i, old_prev));
        if let (Ok(actual), Ok(expected)) = (actual, expected) {
            assert!(core::ptr::eq(actual.value(), expected.value()));
        }
    }

    #[test]
    fn regular_parser_matches_legacy_for_every_header_and_short_prefix() {
        for head in 0..=u8::MAX {
            for tail in [0, 1, 12, 255] {
                let mut bytes = [tail; 48];
                for cursor in 0..=3 {
                    bytes[cursor] = head;
                    for end in cursor..=bytes.len() {
                        for prev in [0, 65_522, 65_523, 65_524, 65_535, 65_536, u32::MAX] {
                            compare(&bytes[..end], cursor, prev);
                        }
                    }
                    bytes[cursor] = tail;
                }
            }
        }
    }

    #[test]
    fn regular_parser_matches_legacy_at_extension_and_number_boundaries() {
        let mut bytes = [0x5a; 65_812];
        for head in [0x0d, 0x0e, 0xd0, 0xe0, 0xdd, 0xde, 0xed, 0xee] {
            bytes[0] = head;
            for extra in [0, 1, 254, 255] {
                bytes[1..5].fill(extra);
                for end in [0, 1, 2, 3, 4, 5, 12, 13, 14, 268, 269, 270, 65_804, 65_812] {
                    for prev in [0, 1, 65_266, 65_267, 65_534, 65_535, u32::MAX] {
                        compare(&bytes[..end], 0, prev);
                    }
                }
            }
        }
        for cursor in [bytes.len(), bytes.len() + 1, usize::MAX] {
            compare(&bytes, cursor, u32::MAX);
        }
    }

    #[test]
    fn regular_parser_preserves_failure_priority_and_partial_state() {
        for (bytes, number, error, cursor, next_number) in [
            (&[0xff][..], 0, ParseError::ReservedOptionDelta, 1, 0),
            (&[0xdf][..], 0, ParseError::ReservedOptionLength, 1, 0),
            (&[0xdd, 0][..], 0, ParseError::TruncatedOption, 2, 0),
            (
                &[0x11][..],
                65_535,
                ParseError::OptionNumberOverflow,
                1,
                65_535,
            ),
            (&[0x11][..], 1, ParseError::TruncatedOption, 1, 2),
            (&[0xd1, 0][..], 1, ParseError::TruncatedOption, 2, 14),
        ] {
            let (mut i, mut prev) = (0, number);
            assert_eq!(read_option(bytes, &mut i, &mut prev), Err(error));
            assert_eq!((i, prev), (cursor, next_number));
            compare(bytes, 0, number);
        }
    }

    #[test]
    fn regular_parser_mixed_options_roundtrip_through_public_decoder() {
        use crate::message::{Code, Message, MessageId, Token, Type, decode, encode};

        let medium = [0x31; 13];
        let long = [0x72; 270];
        let opts = [
            Opt::new(OptionNumber::new(11), b"bench"),
            Opt::new(OptionNumber::new(11), b""),
            Opt::new(OptionNumber::new(25), &[0x42]),
            Opt::new(OptionNumber::new(26), &medium),
            Opt::new(OptionNumber::new(300), &long),
        ];
        let msg = Message::new(Type::Confirmable, Code::GET, MessageId::new(42))
            .with_token(Token::new(&[0xa5, 0x5a]).unwrap())
            .with_options(&opts)
            .with_payload(b"\xffpayload");
        let mut wire = [0; 512];
        let n = encode(&msg, &mut wire).unwrap();
        let parsed = decode(&wire[..n]).unwrap();
        assert_eq!(parsed.token(), msg.token());
        assert_eq!(parsed.payload(), msg.payload());
        assert!(parsed.options().eq(opts));
        let mut again = [0; 512];
        let again_n = parsed.encode(&mut again).unwrap();
        assert_eq!(&again[..again_n], &wire[..n]);
    }
}
