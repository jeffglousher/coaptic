//! Builder, parse, and encode errors.

use crate::message::OptionNumber;

/// Failure to construct an [`Engine`](crate::Engine).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildError {
    /// Builder sizes do not match the [`Storage`](crate::Storage) being moved in.
    SizeMismatch,
    /// Enabled body slot bytes are not a multiple of 1024 (max Block/Q-Block SZX).
    BodyBytesNotMultipleOf1024,
    /// `.block_wise(false)` but storage or the builder includes body pools.
    UnexpectedBodyPools,
    /// `.block_wise(true)` but storage has no body pools.
    MissingBodyPools,
    /// Enabled body slot count or bytes is zero. Zero is not “disabled.”
    ZeroBodyCapacity,
    /// Some body capacity fields are set and others are not.
    IncompleteBodyCapacities,
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SizeMismatch => f.write_str("storage sizes do not match the builder"),
            Self::BodyBytesNotMultipleOf1024 => {
                f.write_str("body slot bytes must be a multiple of 1024")
            }
            Self::UnexpectedBodyPools => {
                f.write_str("body pools are present but block-wise is disabled")
            }
            Self::MissingBodyPools => {
                f.write_str("block-wise is enabled but storage has no body pools")
            }
            Self::ZeroBodyCapacity => {
                f.write_str("enabled body capacity must not be zero slots or zero bytes")
            }
            Self::IncompleteBodyCapacities => {
                f.write_str("body capacity fields must be all set or all absent")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BuildError {}

/// Failure to decode a CoAP datagram.
///
/// Wire-format failures are distinct from [`Self::UnrecognizedCritical`] and
/// [`Self::BadOptionFormat`], which are structured reports for optional
/// RFC 7252 checks. [`crate::message::decode`] does not invent 4.02 / RST
/// policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// Buffer shorter than the 4-byte header.
    TruncatedHeader,
    /// Version field is not 1.
    UnsupportedVersion,
    /// Token length nibble is 9–15.
    BadTokenLength,
    /// Buffer ends before the token bytes given by TKL.
    TruncatedToken,
    /// Option header, extended field, or value overruns the buffer.
    TruncatedOption,
    /// Option delta nibble is 15 but the byte is not the payload marker.
    ReservedOptionDelta,
    /// Option length nibble is 15.
    ReservedOptionLength,
    /// Running option number exceeded 65535.
    OptionNumberOverflow,
    /// Empty message (code 0.00) has a token, options, or payload.
    EmptyMessageNotEmpty,
    /// Payload marker 0xFF with no following payload bytes.
    PayloadMarkerWithoutPayload,
    /// Critical option number is not among the RFC 7252 options.
    ///
    /// Returned by [`crate::message::ParsedMessage::check_rfc7252_options`],
    /// not by [`crate::message::decode`]. The caller chooses the protocol
    /// response.
    UnrecognizedCritical(OptionNumber),
    /// A known RFC 7252 option has a value that does not match its Table 4
    /// format.
    ///
    /// Returned by [`crate::message::ParsedMessage::check_rfc7252_formats`],
    /// not by [`crate::message::decode`] or
    /// [`crate::message::ParsedMessage::check_rfc7252_options`]. The caller
    /// chooses the protocol response.
    BadOptionFormat(OptionNumber),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TruncatedHeader => f.write_str("truncated CoAP header"),
            Self::UnsupportedVersion => f.write_str("unsupported CoAP version"),
            Self::BadTokenLength => f.write_str("token length is 9-15"),
            Self::TruncatedToken => f.write_str("truncated token"),
            Self::TruncatedOption => f.write_str("truncated option"),
            Self::ReservedOptionDelta => f.write_str("reserved option delta 15"),
            Self::ReservedOptionLength => f.write_str("reserved option length 15"),
            Self::OptionNumberOverflow => f.write_str("option number overflow"),
            Self::EmptyMessageNotEmpty => f.write_str("empty message is not empty"),
            Self::PayloadMarkerWithoutPayload => f.write_str("payload marker without payload"),
            Self::UnrecognizedCritical(n) => {
                write!(f, "unrecognized critical option {}", n.get())
            }
            Self::BadOptionFormat(n) => {
                write!(f, "RFC 7252 option {} has the wrong format", n.get())
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Failure to encode a CoAP datagram.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// Output buffer is shorter than the encoded message.
    BufferTooSmall,
    /// Empty message (code 0.00) has a token, options, or payload.
    EmptyMessageNotEmpty,
    /// Option numbers are not in non-decreasing order.
    OptionsNotAscending,
    /// Option value cannot be encoded (length above the 14-extended maximum).
    OptionValueTooLong,
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BufferTooSmall => f.write_str("encode buffer is too small"),
            Self::EmptyMessageNotEmpty => f.write_str("empty message is not empty"),
            Self::OptionsNotAscending => f.write_str("option numbers are not ascending"),
            Self::OptionValueTooLong => f.write_str("option value is too long"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}

/// Failure to decode an option value as empty, opaque, uint, or string.
///
/// Distinct from [`ParseError`]: wire decode keeps option values opaque.
/// See `knowledge/rfcs/rfc7252.txt` §3.2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueError {
    /// Bytes are not a UTF-8 string option value.
    InvalidUtf8,
    /// Integer does not fit the target width.
    UintOverflow,
}

impl core::fmt::Display for ValueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUtf8 => f.write_str("option value is not UTF-8"),
            Self::UintOverflow => f.write_str("uint option value does not fit the target"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValueError {}
