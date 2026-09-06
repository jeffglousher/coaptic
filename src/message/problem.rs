//! RFC 9290 concise problem details (CBOR map).
//!
//! Wire format: `knowledge/rfcs/rfc9290.txt`. This module does not restate it.
//! Default CoAP Content-Format is [`ContentFormat::PROBLEM_DETAILS`] (257).

use crate::message::{Code, ContentFormat};

/// Standard Problem Detail key: title.
const KEY_TITLE: i8 = -1;
/// Standard Problem Detail key: detail.
const KEY_DETAIL: i8 = -2;
/// Standard Problem Detail key: response-code.
const KEY_RESPONSE_CODE: i8 = -4;

const MAJOR_UINT: u8 = 0;
const MAJOR_NINT: u8 = 1;
const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_ARRAY: u8 = 4;
const MAJOR_MAP: u8 = 5;
const MAJOR_TAG: u8 = 6;
const MAJOR_SIMPLE: u8 = 7;

/// Failure to encode or decode a concise problem-details CBOR map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProblemError {
    /// Output buffer cannot hold the encoded map.
    BufferTooSmall,
    /// Bytes are not a non-empty CBOR map of problem details.
    Invalid,
}

impl core::fmt::Display for ProblemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BufferTooSmall => f.write_str("problem-details encode buffer is too small"),
            Self::Invalid => f.write_str("invalid concise problem-details CBOR"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProblemError {}

/// RFC 9290 concise problem details: `response-code` plus optional title/detail.
///
/// Encodes as a CBOR map with Standard Problem Detail keys. The map is
/// non-empty: [`Self::new`] always includes `response-code` (−4), matching
/// the CoAP code on the message. Title (−1) and detail (−2) are optional
/// unadorned text. Default Content-Format is
/// [`ContentFormat::PROBLEM_DETAILS`]. On the App face, build these with
/// [`crate::Response::problem`].
///
/// ```
/// use coaptic::message::ProblemDetails;
/// use coaptic::{Code, ContentFormat};
///
/// let details = ProblemDetails::new(Code::NOT_FOUND).title("Not Found");
/// let mut buf = [0u8; 32];
/// let n = details.encode(&mut buf).expect("fits");
/// let parsed = ProblemDetails::decode(&buf[..n]).expect("cbor");
/// assert_eq!(parsed.response_code(), Some(Code::NOT_FOUND));
/// assert_eq!(parsed.title_text(), Some("Not Found"));
/// assert_eq!(ContentFormat::PROBLEM_DETAILS.get(), 257);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProblemDetails<'a> {
    code: Option<Code>,
    title: Option<&'a str>,
    detail: Option<&'a str>,
}

impl ProblemDetails<'static> {
    /// Problem details for `code`. Encodes `{ -4: code }` until title/detail
    /// are set.
    #[must_use]
    pub const fn new(code: Code) -> Self {
        Self {
            code: Some(code),
            title: None,
            detail: None,
        }
    }
}

impl<'a> ProblemDetails<'a> {
    /// CoAP Content-Format for this payload (`application/concise-problem-details+cbor`).
    pub const CONTENT_FORMAT: ContentFormat = ContentFormat::PROBLEM_DETAILS;

    /// Set the short title (key −1).
    #[must_use]
    pub const fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Set the occurrence-specific detail (key −2).
    #[must_use]
    pub const fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    /// `response-code` (−4), if present.
    #[must_use]
    pub const fn response_code(self) -> Option<Code> {
        self.code
    }

    /// Title (−1), if present.
    #[must_use]
    pub const fn title_text(self) -> Option<&'a str> {
        self.title
    }

    /// Detail (−2), if present.
    #[must_use]
    pub const fn detail_text(self) -> Option<&'a str> {
        self.detail
    }

    /// Encode the CBOR map into `buf`. Keys are in deterministic order
    /// (title, detail, response-code).
    ///
    /// # Errors
    ///
    /// [`ProblemError::BufferTooSmall`] when `buf` cannot hold the map.
    /// [`ProblemError::Invalid`] when no field is set (empty map).
    pub fn encode(self, buf: &mut [u8]) -> Result<usize, ProblemError> {
        let pairs = usize::from(self.title.is_some())
            + usize::from(self.detail.is_some())
            + usize::from(self.code.is_some());
        if pairs == 0 {
            return Err(ProblemError::Invalid);
        }
        let mut at = 0usize;
        put_head(buf, &mut at, MAJOR_MAP, pairs as u64)?;
        if let Some(title) = self.title {
            put_nint(buf, &mut at, KEY_TITLE)?;
            put_text(buf, &mut at, title)?;
        }
        if let Some(detail) = self.detail {
            put_nint(buf, &mut at, KEY_DETAIL)?;
            put_text(buf, &mut at, detail)?;
        }
        if let Some(code) = self.code {
            put_nint(buf, &mut at, KEY_RESPONSE_CODE)?;
            put_uint(buf, &mut at, u64::from(code.as_raw()))?;
        }
        Ok(at)
    }

    /// Decode a concise problem-details CBOR map. Unknown keys are skipped.
    ///
    /// # Errors
    ///
    /// [`ProblemError::Invalid`] when the bytes are not a non-empty definite
    /// CBOR map, or a value for a known key has the wrong type.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ProblemError> {
        let mut at = 0usize;
        let (major, n) = read_head(bytes, &mut at)?;
        if major != MAJOR_MAP || n == 0 {
            return Err(ProblemError::Invalid);
        }
        let mut out = Self {
            code: None,
            title: None,
            detail: None,
        };
        for _ in 0..n {
            let key = read_int(bytes, &mut at)?;
            match key {
                -1 => out.title = Some(read_text(bytes, &mut at)?),
                -2 => out.detail = Some(read_text(bytes, &mut at)?),
                -4 => {
                    let raw = read_uint(bytes, &mut at)?;
                    if raw > u64::from(u8::MAX) {
                        return Err(ProblemError::Invalid);
                    }
                    out.code = Some(Code::from_raw(raw as u8));
                }
                _ => skip_item(bytes, &mut at)?,
            }
        }
        if at != bytes.len() {
            return Err(ProblemError::Invalid);
        }
        if out.title.is_none() && out.detail.is_none() && out.code.is_none() {
            return Err(ProblemError::Invalid);
        }
        Ok(out)
    }
}

fn put(buf: &mut [u8], at: &mut usize, byte: u8) -> Result<(), ProblemError> {
    if *at >= buf.len() {
        return Err(ProblemError::BufferTooSmall);
    }
    buf[*at] = byte;
    *at += 1;
    Ok(())
}

fn put_slice(buf: &mut [u8], at: &mut usize, bytes: &[u8]) -> Result<(), ProblemError> {
    if buf.len().saturating_sub(*at) < bytes.len() {
        return Err(ProblemError::BufferTooSmall);
    }
    buf[*at..*at + bytes.len()].copy_from_slice(bytes);
    *at += bytes.len();
    Ok(())
}

fn put_head(buf: &mut [u8], at: &mut usize, major: u8, n: u64) -> Result<(), ProblemError> {
    if n <= 23 {
        put(buf, at, (major << 5) | (n as u8))
    } else if n <= 255 {
        put(buf, at, (major << 5) | 24)?;
        put(buf, at, n as u8)
    } else if n <= u64::from(u16::MAX) {
        put(buf, at, (major << 5) | 25)?;
        put(buf, at, (n >> 8) as u8)?;
        put(buf, at, n as u8)
    } else if n <= u64::from(u32::MAX) {
        put(buf, at, (major << 5) | 26)?;
        for shift in [24, 16, 8, 0] {
            put(buf, at, (n >> shift) as u8)?;
        }
        Ok(())
    } else {
        put(buf, at, (major << 5) | 27)?;
        for shift in [56, 48, 40, 32, 24, 16, 8, 0] {
            put(buf, at, (n >> shift) as u8)?;
        }
        Ok(())
    }
}

fn put_uint(buf: &mut [u8], at: &mut usize, n: u64) -> Result<(), ProblemError> {
    put_head(buf, at, MAJOR_UINT, n)
}

fn put_nint(buf: &mut [u8], at: &mut usize, n: i8) -> Result<(), ProblemError> {
    debug_assert!(n < 0);
    put_head(
        buf,
        at,
        MAJOR_NINT,
        u64::from((-1_i16 - i16::from(n)) as u8),
    )
}

fn put_text(buf: &mut [u8], at: &mut usize, text: &str) -> Result<(), ProblemError> {
    put_head(buf, at, MAJOR_TEXT, text.len() as u64)?;
    put_slice(buf, at, text.as_bytes())
}

fn need(bytes: &[u8], at: usize, n: usize) -> Result<(), ProblemError> {
    if bytes.len().saturating_sub(at) < n {
        Err(ProblemError::Invalid)
    } else {
        Ok(())
    }
}

fn read_head(bytes: &[u8], at: &mut usize) -> Result<(u8, u64), ProblemError> {
    need(bytes, *at, 1)?;
    let b = bytes[*at];
    *at += 1;
    let major = b >> 5;
    let ai = b & 0x1f;
    let n = match ai {
        0..=23 => u64::from(ai),
        24 => {
            need(bytes, *at, 1)?;
            let n = u64::from(bytes[*at]);
            *at += 1;
            n
        }
        25 => {
            need(bytes, *at, 2)?;
            let n = u64::from(u16::from_be_bytes([bytes[*at], bytes[*at + 1]]));
            *at += 2;
            n
        }
        26 => {
            need(bytes, *at, 4)?;
            let n = u64::from(u32::from_be_bytes([
                bytes[*at],
                bytes[*at + 1],
                bytes[*at + 2],
                bytes[*at + 3],
            ]));
            *at += 4;
            n
        }
        27 => {
            need(bytes, *at, 8)?;
            let n = u64::from_be_bytes([
                bytes[*at],
                bytes[*at + 1],
                bytes[*at + 2],
                bytes[*at + 3],
                bytes[*at + 4],
                bytes[*at + 5],
                bytes[*at + 6],
                bytes[*at + 7],
            ]);
            *at += 8;
            n
        }
        _ => return Err(ProblemError::Invalid),
    };
    Ok((major, n))
}

fn read_int(bytes: &[u8], at: &mut usize) -> Result<i64, ProblemError> {
    let (major, n) = read_head(bytes, at)?;
    match major {
        MAJOR_UINT if n <= i64::MAX as u64 => Ok(n as i64),
        MAJOR_NINT if n < i64::MAX as u64 => Ok(-1 - (n as i64)),
        _ => Err(ProblemError::Invalid),
    }
}

fn read_uint(bytes: &[u8], at: &mut usize) -> Result<u64, ProblemError> {
    let (major, n) = read_head(bytes, at)?;
    if major == MAJOR_UINT {
        Ok(n)
    } else {
        Err(ProblemError::Invalid)
    }
}

fn read_text<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a str, ProblemError> {
    let (major, n) = read_head(bytes, at)?;
    if major != MAJOR_TEXT {
        return Err(ProblemError::Invalid);
    }
    let len = usize::try_from(n).map_err(|_| ProblemError::Invalid)?;
    need(bytes, *at, len)?;
    let slice = &bytes[*at..*at + len];
    *at += len;
    core::str::from_utf8(slice).map_err(|_| ProblemError::Invalid)
}

fn skip_item(bytes: &[u8], at: &mut usize) -> Result<(), ProblemError> {
    let (major, n) = read_head(bytes, at)?;
    match major {
        MAJOR_UINT | MAJOR_NINT | MAJOR_SIMPLE => Ok(()),
        MAJOR_BYTES | MAJOR_TEXT => {
            let len = usize::try_from(n).map_err(|_| ProblemError::Invalid)?;
            need(bytes, *at, len)?;
            *at += len;
            Ok(())
        }
        MAJOR_ARRAY => {
            for _ in 0..n {
                skip_item(bytes, at)?;
            }
            Ok(())
        }
        MAJOR_MAP => {
            for _ in 0..n {
                skip_item(bytes, at)?;
                skip_item(bytes, at)?;
            }
            Ok(())
        }
        MAJOR_TAG => skip_item(bytes, at),
        _ => Err(ProblemError::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::{ProblemDetails, ProblemError};
    use crate::message::Code;

    /// `{ -4: 132 }` — 4.04 Not Found, response-code only (RFC 9290).
    const NOT_FOUND_CODE_ONLY: &[u8] = &[0xa1, 0x23, 0x18, 0x84];

    /// `{ -1: "Not Found", -4: 132 }`.
    const NOT_FOUND_TITLED: &[u8] = &[
        0xa2, 0x20, 0x69, b'N', b'o', b't', b' ', b'F', b'o', b'u', b'n', b'd', 0x23, 0x18, 0x84,
    ];

    /// Figure 3 standard keys only: title, detail, response-code 128 (4.00).
    const FIGURE3_STANDARD: &[u8] = &[
        0xa3, 0x20, 0x72, b't', b'i', b't', b'l', b'e', b' ', b'o', b'f', b' ', b't', b'h', b'e',
        b' ', b'e', b'r', b'r', b'o', b'r', 0x21, 0x78, 0x24, b'd', b'e', b't', b'a', b'i', b'l',
        b'e', b'd', b' ', b'i', b'n', b'f', b'o', b'r', b'm', b'a', b't', b'i', b'o', b'n', b' ',
        b'a', b'b', b'o', b'u', b't', b' ', b't', b'h', b'e', b' ', b'e', b'r', b'r', b'o', b'r',
        0x23, 0x18, 0x80,
    ];

    #[test]
    fn encode_not_found_matches_known_cbor() {
        let mut buf = [0u8; 16];
        let n = ProblemDetails::new(Code::NOT_FOUND)
            .encode(&mut buf)
            .expect("encode");
        assert_eq!(&buf[..n], NOT_FOUND_CODE_ONLY);
        assert_eq!(Code::NOT_FOUND.as_raw(), 132);
    }

    #[test]
    fn encode_titled_not_found_matches_known_cbor() {
        let mut buf = [0u8; 32];
        let n = ProblemDetails::new(Code::NOT_FOUND)
            .title("Not Found")
            .encode(&mut buf)
            .expect("encode");
        assert_eq!(&buf[..n], NOT_FOUND_TITLED);
    }

    #[test]
    fn encode_figure3_standard_keys_matches_known_cbor() {
        let mut buf = [0u8; 80];
        let n = ProblemDetails::new(Code::BAD_REQUEST)
            .title("title of the error")
            .detail("detailed information about the error")
            .encode(&mut buf)
            .expect("encode");
        assert_eq!(&buf[..n], FIGURE3_STANDARD);
        assert_eq!(Code::BAD_REQUEST.as_raw(), 128);
    }

    #[test]
    fn decode_round_trips_fields() {
        let mut buf = [0u8; 80];
        let n = ProblemDetails::new(Code::METHOD_NOT_ALLOWED)
            .title("Method Not Allowed")
            .detail("POST on a GET-only path")
            .encode(&mut buf)
            .expect("encode");
        let parsed = ProblemDetails::decode(&buf[..n]).expect("decode");
        assert_eq!(parsed.response_code(), Some(Code::METHOD_NOT_ALLOWED));
        assert_eq!(parsed.title_text(), Some("Method Not Allowed"));
        assert_eq!(parsed.detail_text(), Some("POST on a GET-only path"));
    }

    #[test]
    fn decode_skips_unknown_key() {
        // `{ -4: 132, -99: 1 }` — unknown standard key ignored.
        let bytes = [0xa2, 0x23, 0x18, 0x84, 0x38, 0x62, 0x01];
        let parsed = ProblemDetails::decode(&bytes).expect("decode");
        assert_eq!(parsed.response_code(), Some(Code::NOT_FOUND));
        assert!(parsed.title_text().is_none());
    }

    #[test]
    fn decode_rejects_empty_or_non_map() {
        assert_eq!(ProblemDetails::decode(&[]), Err(ProblemError::Invalid));
        assert_eq!(ProblemDetails::decode(&[0xa0]), Err(ProblemError::Invalid));
        assert_eq!(ProblemDetails::decode(&[0x01]), Err(ProblemError::Invalid));
    }

    #[test]
    fn encode_empty_is_invalid_and_tiny_buf_fails() {
        let empty = ProblemDetails {
            code: None,
            title: None,
            detail: None,
        };
        let mut buf = [0u8; 16];
        assert_eq!(empty.encode(&mut buf), Err(ProblemError::Invalid));
        assert_eq!(
            ProblemDetails::new(Code::NOT_FOUND).encode(&mut []),
            Err(ProblemError::BufferTooSmall)
        );
    }

    #[test]
    fn long_text_uses_one_byte_length() {
        let title = "Request Entity Incomplete";
        assert_eq!(title.len(), 25);
        let mut buf = [0u8; 48];
        let n = ProblemDetails::new(Code::REQUEST_ENTITY_INCOMPLETE)
            .title(title)
            .encode(&mut buf)
            .expect("encode");
        assert_eq!(buf[0], 0xa2);
        assert_eq!(buf[1], 0x20);
        assert_eq!(&buf[2..4], &[0x78, 25]);
        let parsed = ProblemDetails::decode(&buf[..n]).expect("decode");
        assert_eq!(
            parsed.response_code(),
            Some(Code::REQUEST_ENTITY_INCOMPLETE)
        );
        assert_eq!(parsed.title_text(), Some(title));
        assert_eq!(Code::REQUEST_ENTITY_INCOMPLETE.as_raw(), 136);
    }
}
