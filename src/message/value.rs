//! RFC 7252 option value formats: empty, opaque, uint, and string.
//!
//! These codecs operate on option *value* bytes after
//! [`crate::message::decode`] / before [`crate::message::encode`]. They do
//! not parse option headers. Wire format lives in
//! `knowledge/rfcs/rfc7252.txt`. This module does not restate it.

use core::fmt;

use crate::error::{ParseError, ValueError};

use super::decode::ParsedMessage;
use super::option::{Opt, OptionNumber, Options};

/// RFC 7252 default Max-Age (seconds) when the option is absent.
///
/// An empty Max-Age *value* is uint 0, which is not this default.
pub const MAX_AGE_DEFAULT: u32 = 60;

/// RFC 7252 §3.2 option value format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionValueFormat {
    /// Zero-length value.
    Empty,
    /// Opaque byte string.
    Opaque,
    /// Unsigned integer in network byte order.
    Uint,
    /// UTF-8 string.
    String,
}

/// Stack-encoded uint option value (0–4 bytes, leading zeros omitted).
///
/// Zero encodes as the empty value. Hold this while an [`Opt`] borrows it.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EncodedUint {
    buf: [u8; 4],
    len: u8,
}

impl EncodedUint {
    /// Encode `n` without leading zero bytes.
    #[must_use]
    pub const fn new(n: u32) -> Self {
        let b = n.to_be_bytes();
        let start = if b[0] != 0 {
            0
        } else if b[1] != 0 {
            1
        } else if b[2] != 0 {
            2
        } else if b[3] != 0 {
            3
        } else {
            4
        };
        let len = 4 - start;
        let mut buf = [0u8; 4];
        let mut i = 0;
        while i < len {
            buf[i] = b[start + i];
            i += 1;
        }
        Self {
            buf,
            len: len as u8,
        }
    }

    /// Encoded bytes (empty when `n` was 0).
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.buf.split_at(self.len as usize).0
    }

    /// Encoded length in bytes (0–4).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether this is the empty encoding of 0.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for EncodedUint {
    fn default() -> Self {
        Self::new(0)
    }
}

impl From<u16> for EncodedUint {
    fn from(n: u16) -> Self {
        Self::new(u32::from(n))
    }
}

impl From<u32> for EncodedUint {
    fn from(n: u32) -> Self {
        Self::new(n)
    }
}

impl From<ContentFormat> for EncodedUint {
    fn from(cf: ContentFormat) -> Self {
        cf.encode()
    }
}

impl fmt::Debug for EncodedUint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("EncodedUint")
            .field(&self.as_bytes())
            .finish()
    }
}

/// Encode a uint option value on the stack. Leading zeros are omitted.
#[must_use]
pub const fn encode_uint(n: u32) -> EncodedUint {
    EncodedUint::new(n)
}

/// Decode a uint option value.
///
/// Accepts any length, including leading zeros, as long as the integer fits
/// in `u32`. The empty value is 0.
pub fn decode_uint(bytes: &[u8]) -> Result<u32, ValueError> {
    let mut n = 0u32;
    for &b in bytes {
        n = n
            .checked_mul(256)
            .and_then(|n| n.checked_add(u32::from(b)))
            .ok_or(ValueError::UintOverflow)?;
    }
    Ok(n)
}

/// Decode a uint option value that must fit in `u16`.
pub fn decode_uint16(bytes: &[u8]) -> Result<u16, ValueError> {
    u16::try_from(decode_uint(bytes)?).map_err(|_| ValueError::UintOverflow)
}

/// UTF-8 view of a string option value. Does not allocate.
pub fn as_str(bytes: &[u8]) -> Result<&str, ValueError> {
    core::str::from_utf8(bytes).map_err(|_| ValueError::InvalidUtf8)
}

/// CoAP Content-Format identifier (Content-Format / Accept).
///
/// Initial RFC 7252 registry entries are associated constants. Other IDs use
/// [`ContentFormat::new`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentFormat(u16);

impl ContentFormat {
    /// `text/plain; charset=utf-8`.
    pub const TEXT_PLAIN: Self = Self(0);
    /// `application/link-format`.
    pub const LINK_FORMAT: Self = Self(40);
    /// `application/xml`.
    pub const XML: Self = Self(41);
    /// `application/octet-stream`.
    pub const OCTET_STREAM: Self = Self(42);
    /// `application/exi`.
    pub const EXI: Self = Self(47);
    /// `application/json`.
    pub const JSON: Self = Self(50);

    /// Wrap a raw Content-Format ID.
    #[must_use]
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// Raw Content-Format ID.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Encode as a uint option value.
    #[must_use]
    pub const fn encode(self) -> EncodedUint {
        EncodedUint::new(self.0 as u32)
    }
}

impl From<u16> for ContentFormat {
    fn from(id: u16) -> Self {
        Self(id)
    }
}

impl From<ContentFormat> for u16 {
    fn from(cf: ContentFormat) -> Self {
        cf.0
    }
}

/// Options with a given number, in wire order.
#[derive(Clone, Debug)]
pub struct OptionsByNumber<'a> {
    inner: Options<'a>,
    number: OptionNumber,
}

impl<'a> Iterator for OptionsByNumber<'a> {
    type Item = Opt<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.find(|opt| opt.number() == self.number)
    }
}

/// UTF-8 views of string option values with a given number.
#[derive(Clone, Debug)]
pub struct StringOptions<'a> {
    inner: OptionsByNumber<'a>,
}

impl<'a> Iterator for StringOptions<'a> {
    type Item = Result<&'a str, ValueError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(Opt::as_str)
    }
}

/// Opaque values of options with a given number.
#[derive(Clone, Debug)]
pub struct OpaqueOptions<'a> {
    inner: OptionsByNumber<'a>,
}

impl<'a> Iterator for OpaqueOptions<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(Opt::value)
    }
}

impl<'a> Opt<'a> {
    /// Empty option value.
    #[must_use]
    pub const fn empty(number: OptionNumber) -> Self {
        Self::new(number, &[])
    }

    /// Opaque option value.
    #[must_use]
    pub const fn opaque(number: OptionNumber, value: &'a [u8]) -> Self {
        Self::new(number, value)
    }

    /// String option value (UTF-8 bytes).
    #[must_use]
    pub const fn string(number: OptionNumber, value: &'a str) -> Self {
        Self::new(number, value.as_bytes())
    }

    /// Uint option value. `encoded` must outlive the [`Opt`].
    #[must_use]
    pub const fn uint(number: OptionNumber, encoded: &'a EncodedUint) -> Self {
        Self::new(number, encoded.as_bytes())
    }

    /// If-Match (opaque, empty means any).
    #[must_use]
    pub const fn if_match(etag: &'a [u8]) -> Self {
        Self::opaque(OptionNumber::IF_MATCH, etag)
    }

    /// Uri-Host (string).
    #[must_use]
    pub const fn uri_host(host: &'a str) -> Self {
        Self::string(OptionNumber::URI_HOST, host)
    }

    /// ETag (opaque).
    #[must_use]
    pub const fn etag(tag: &'a [u8]) -> Self {
        Self::opaque(OptionNumber::ETAG, tag)
    }

    /// If-None-Match (empty).
    #[must_use]
    pub const fn if_none_match() -> Self {
        Self::empty(OptionNumber::IF_NONE_MATCH)
    }

    /// Uri-Port (uint).
    #[must_use]
    pub const fn uri_port(encoded: &'a EncodedUint) -> Self {
        Self::uint(OptionNumber::URI_PORT, encoded)
    }

    /// Location-Path segment (string).
    #[must_use]
    pub const fn location_path(segment: &'a str) -> Self {
        Self::string(OptionNumber::LOCATION_PATH, segment)
    }

    /// Uri-Path segment (string).
    #[must_use]
    pub const fn uri_path(segment: &'a str) -> Self {
        Self::string(OptionNumber::URI_PATH, segment)
    }

    /// Content-Format (uint).
    #[must_use]
    pub const fn content_format(encoded: &'a EncodedUint) -> Self {
        Self::uint(OptionNumber::CONTENT_FORMAT, encoded)
    }

    /// Max-Age (uint).
    #[must_use]
    pub const fn max_age(encoded: &'a EncodedUint) -> Self {
        Self::uint(OptionNumber::MAX_AGE, encoded)
    }

    /// Uri-Query (string).
    #[must_use]
    pub const fn uri_query(query: &'a str) -> Self {
        Self::string(OptionNumber::URI_QUERY, query)
    }

    /// Accept (uint Content-Format ID).
    #[must_use]
    pub const fn accept(encoded: &'a EncodedUint) -> Self {
        Self::uint(OptionNumber::ACCEPT, encoded)
    }

    /// Location-Query (string).
    #[must_use]
    pub const fn location_query(query: &'a str) -> Self {
        Self::string(OptionNumber::LOCATION_QUERY, query)
    }

    /// Proxy-Uri (string).
    #[must_use]
    pub const fn proxy_uri(uri: &'a str) -> Self {
        Self::string(OptionNumber::PROXY_URI, uri)
    }

    /// Proxy-Scheme (string).
    #[must_use]
    pub const fn proxy_scheme(scheme: &'a str) -> Self {
        Self::string(OptionNumber::PROXY_SCHEME, scheme)
    }

    /// Size1 (uint).
    #[must_use]
    pub const fn size1(encoded: &'a EncodedUint) -> Self {
        Self::uint(OptionNumber::SIZE1, encoded)
    }

    /// Whether the value is zero-length (present empty, not missing).
    #[must_use]
    pub const fn is_empty_value(self) -> bool {
        self.value().is_empty()
    }

    /// UTF-8 view of this option value.
    pub fn as_str(self) -> Result<&'a str, ValueError> {
        as_str(self.value())
    }

    /// Decode this option value as a uint that fits in `u32`.
    pub fn as_uint(self) -> Result<u32, ValueError> {
        decode_uint(self.value())
    }

    /// Decode this option value as a uint that fits in `u16`.
    pub fn as_uint16(self) -> Result<u16, ValueError> {
        decode_uint16(self.value())
    }

    /// Table 4 format for this option number, if it is an RFC 7252 option.
    #[must_use]
    pub const fn value_format(self) -> Option<OptionValueFormat> {
        option_value_format(self.number())
    }

    /// Fail if this is a known RFC 7252 option with the wrong format.
    ///
    /// Unknown numbers succeed. [`crate::message::decode`] does not call this.
    pub fn check_rfc7252_format(self) -> Result<(), ParseError> {
        check_format(self)
    }
}

impl OptionNumber {
    /// Table 4 format for this number, if it is an RFC 7252 option.
    #[must_use]
    pub const fn value_format(self) -> Option<OptionValueFormat> {
        option_value_format(self)
    }
}

/// Table 4 format for `number`, or `None` when the number is not RFC 7252.
#[must_use]
pub const fn option_value_format(number: OptionNumber) -> Option<OptionValueFormat> {
    match table4_spec(number) {
        Some((kind, _, _)) => Some(kind),
        None => None,
    }
}

const fn table4_spec(number: OptionNumber) -> Option<(OptionValueFormat, usize, usize)> {
    match number.get() {
        1 => Some((OptionValueFormat::Opaque, 0, 8)),
        3 => Some((OptionValueFormat::String, 1, 255)),
        4 => Some((OptionValueFormat::Opaque, 1, 8)),
        5 => Some((OptionValueFormat::Empty, 0, 0)),
        7 => Some((OptionValueFormat::Uint, 0, 2)),
        8 => Some((OptionValueFormat::String, 0, 255)),
        11 => Some((OptionValueFormat::String, 0, 255)),
        12 => Some((OptionValueFormat::Uint, 0, 2)),
        14 => Some((OptionValueFormat::Uint, 0, 4)),
        15 => Some((OptionValueFormat::String, 0, 255)),
        17 => Some((OptionValueFormat::Uint, 0, 2)),
        20 => Some((OptionValueFormat::String, 0, 255)),
        35 => Some((OptionValueFormat::String, 1, 1034)),
        39 => Some((OptionValueFormat::String, 1, 255)),
        60 => Some((OptionValueFormat::Uint, 0, 4)),
        _ => None,
    }
}

fn check_format(opt: Opt<'_>) -> Result<(), ParseError> {
    let Some((kind, min, max)) = table4_spec(opt.number()) else {
        return Ok(());
    };
    let len = opt.value().len();
    let in_range = len >= min && len <= max;
    let ok = match kind {
        OptionValueFormat::Empty => len == 0,
        OptionValueFormat::Opaque | OptionValueFormat::Uint => in_range,
        OptionValueFormat::String => in_range && as_str(opt.value()).is_ok(),
    };
    if ok {
        Ok(())
    } else {
        Err(ParseError::BadOptionFormat(opt.number()))
    }
}

impl<'a> ParsedMessage<'a> {
    /// First option with `number`, if any.
    #[must_use]
    pub fn get_option(self, number: OptionNumber) -> Option<Opt<'a>> {
        self.options().find(|opt| opt.number() == number)
    }

    /// All options with `number`, in wire order.
    #[must_use]
    pub fn get_options(self, number: OptionNumber) -> OptionsByNumber<'a> {
        OptionsByNumber {
            inner: self.options(),
            number,
        }
    }

    /// Uri-Host, if present.
    #[must_use]
    pub fn uri_host(self) -> Option<Result<&'a str, ValueError>> {
        self.get_option(OptionNumber::URI_HOST).map(Opt::as_str)
    }

    /// Uri-Port, if present.
    #[must_use]
    pub fn uri_port(self) -> Option<Result<u16, ValueError>> {
        self.get_option(OptionNumber::URI_PORT).map(Opt::as_uint16)
    }

    /// Uri-Path segments in wire order.
    #[must_use]
    pub fn uri_path(self) -> StringOptions<'a> {
        StringOptions {
            inner: self.get_options(OptionNumber::URI_PATH),
        }
    }

    /// Uri-Query values in wire order.
    #[must_use]
    pub fn uri_query(self) -> StringOptions<'a> {
        StringOptions {
            inner: self.get_options(OptionNumber::URI_QUERY),
        }
    }

    /// Location-Path segments in wire order.
    #[must_use]
    pub fn location_path(self) -> StringOptions<'a> {
        StringOptions {
            inner: self.get_options(OptionNumber::LOCATION_PATH),
        }
    }

    /// Location-Query values in wire order.
    #[must_use]
    pub fn location_query(self) -> StringOptions<'a> {
        StringOptions {
            inner: self.get_options(OptionNumber::LOCATION_QUERY),
        }
    }

    /// Proxy-Uri, if present.
    #[must_use]
    pub fn proxy_uri(self) -> Option<Result<&'a str, ValueError>> {
        self.get_option(OptionNumber::PROXY_URI).map(Opt::as_str)
    }

    /// Proxy-Scheme, if present.
    #[must_use]
    pub fn proxy_scheme(self) -> Option<Result<&'a str, ValueError>> {
        self.get_option(OptionNumber::PROXY_SCHEME).map(Opt::as_str)
    }

    /// Content-Format, if present.
    #[must_use]
    pub fn content_format(self) -> Option<Result<ContentFormat, ValueError>> {
        self.get_option(OptionNumber::CONTENT_FORMAT)
            .map(|opt| opt.as_uint16().map(ContentFormat::new))
    }

    /// Accept, if present.
    #[must_use]
    pub fn accept(self) -> Option<Result<ContentFormat, ValueError>> {
        self.get_option(OptionNumber::ACCEPT)
            .map(|opt| opt.as_uint16().map(ContentFormat::new))
    }

    /// Max-Age, if present. Missing is `None`; empty value is `Some(Ok(0))`.
    ///
    /// Does not substitute [`MAX_AGE_DEFAULT`].
    #[must_use]
    pub fn max_age(self) -> Option<Result<u32, ValueError>> {
        self.get_option(OptionNumber::MAX_AGE).map(Opt::as_uint)
    }

    /// Size1, if present.
    #[must_use]
    pub fn size1(self) -> Option<Result<u32, ValueError>> {
        self.get_option(OptionNumber::SIZE1).map(Opt::as_uint)
    }

    /// Whether If-None-Match is present (empty vs missing).
    #[must_use]
    pub fn if_none_match(self) -> bool {
        self.get_option(OptionNumber::IF_NONE_MATCH).is_some()
    }

    /// If-Match values in wire order (empty slice means any).
    #[must_use]
    pub fn if_match(self) -> OpaqueOptions<'a> {
        OpaqueOptions {
            inner: self.get_options(OptionNumber::IF_MATCH),
        }
    }

    /// ETag values in wire order.
    #[must_use]
    pub fn etag(self) -> OpaqueOptions<'a> {
        OpaqueOptions {
            inner: self.get_options(OptionNumber::ETAG),
        }
    }

    /// First known RFC 7252 option whose value has the wrong format.
    #[must_use]
    pub fn bad_option_format(self) -> Option<OptionNumber> {
        self.options()
            .find_map(|opt| match opt.check_rfc7252_format() {
                Ok(()) => None,
                Err(_) => Some(opt.number()),
            })
    }

    /// Fail if [`Self::bad_option_format`] finds an option.
    ///
    /// [`crate::message::decode`] does not call this. Distinct from
    /// [`Self::check_rfc7252_options`].
    pub fn check_rfc7252_formats(self) -> Result<(), ParseError> {
        match self.bad_option_format() {
            Some(n) => Err(ParseError::BadOptionFormat(n)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::{
        ContentFormat, EncodedUint, OptionValueFormat, as_str, decode_uint, decode_uint16,
        encode_uint, option_value_format,
    };
    use crate::error::ValueError;
    use crate::message::OptionNumber;

    #[test]
    fn encode_uint_omits_leading_zeros() {
        assert_eq!(encode_uint(0).as_bytes(), &[] as &[u8]);
        assert_eq!(encode_uint(1).as_bytes(), &[1]);
        assert_eq!(encode_uint(255).as_bytes(), &[255]);
        assert_eq!(encode_uint(256).as_bytes(), &[1, 0]);
        assert_eq!(encode_uint(0x0100).as_bytes(), &[1, 0]);
        assert_eq!(encode_uint(0x00ff).as_bytes(), &[0xff]);
        assert_eq!(encode_uint(0x01020304).as_bytes(), &[1, 2, 3, 4]);
        assert!(encode_uint(0).is_empty());
        assert_eq!(encode_uint(1).len(), 1);
    }

    #[test]
    fn decode_uint_accepts_leading_zeros_and_empty() {
        assert_eq!(decode_uint(&[]).expect("empty"), 0);
        assert_eq!(decode_uint(&[1]).expect("1"), 1);
        assert_eq!(decode_uint(&[0, 1]).expect("leading zero"), 1);
        assert_eq!(decode_uint(&[0, 0, 0, 1]).expect("three zeros"), 1);
        assert_eq!(decode_uint(&[0, 0, 0, 0, 1]).expect("fits u32"), 1);
        assert_eq!(decode_uint(&[1, 0]).expect("256"), 256);
        assert_eq!(decode_uint(&[1, 2, 3, 4]).expect("full"), 0x0102_0304);
    }

    #[test]
    fn decode_uint_overflows_when_value_does_not_fit() {
        assert_eq!(decode_uint(&[1, 0, 0, 0, 0]), Err(ValueError::UintOverflow));
        assert_eq!(decode_uint16(&[1, 0, 0]), Err(ValueError::UintOverflow));
        assert_eq!(decode_uint16(&[0, 0, 0xff, 0xff]).expect("u16"), u16::MAX);
    }

    #[test]
    fn as_str_rejects_bad_utf8() {
        assert_eq!(as_str(b"temp").expect("ascii"), "temp");
        assert_eq!(as_str(&[0xff, 0xfe]), Err(ValueError::InvalidUtf8));
        assert_eq!(as_str(&[]).expect("empty string"), "");
    }

    #[test]
    fn content_format_encode_roundtrip() {
        let json = ContentFormat::JSON.encode();
        assert_eq!(json.as_bytes(), &[50]);
        assert_eq!(
            decode_uint16(json.as_bytes()).expect("id"),
            ContentFormat::JSON.get()
        );
        assert_eq!(ContentFormat::TEXT_PLAIN.encode().as_bytes(), &[] as &[u8]);
        assert_eq!(
            EncodedUint::from(ContentFormat::LINK_FORMAT).as_bytes(),
            &[40]
        );
    }

    #[test]
    fn table4_formats() {
        assert_eq!(
            option_value_format(OptionNumber::IF_NONE_MATCH),
            Some(OptionValueFormat::Empty)
        );
        assert_eq!(
            option_value_format(OptionNumber::IF_MATCH),
            Some(OptionValueFormat::Opaque)
        );
        assert_eq!(
            option_value_format(OptionNumber::URI_PATH),
            Some(OptionValueFormat::String)
        );
        assert_eq!(
            option_value_format(OptionNumber::CONTENT_FORMAT),
            Some(OptionValueFormat::Uint)
        );
        assert_eq!(option_value_format(OptionNumber::new(23)), None);
    }
}
