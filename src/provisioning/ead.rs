use core::fmt;

use super::Error;

// MAC_2 has the larger context: encoded CID, full reference, encoded TH,
// pinned credential and exact EAD. Keep its length below the one-byte KDF
// context encoding limit as well as the private context buffer capacity.
const _: () = assert!(8 + 4 + 34 + super::identity::CREDENTIAL_LEN + Ead::CAPACITY < 256);

/// Failure to construct a bounded EDHOC EAD field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EadError {
    /// The sequence is malformed or does not use preferred CBOR encoding.
    Malformed,
    /// The complete encoded sequence exceeds the fixed capacity.
    Capacity,
    /// The absolute label exceeds the RFC 9528 registry's 0..=65535 range.
    Label,
}

/// One borrowed EAD item. Debug omits its value.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EadItem<'a> {
    /// Signed registered label: negative is critical, zero is padding.
    pub label: i32,
    /// Opaque value; `None` differs from an empty byte string.
    pub value: Option<&'a [u8]>,
}

/// A complete fixed-capacity EAD sequence, including its encoded labels/values.
///
/// The 64-byte aggregate limit covers every item and CBOR header. Registered
/// labels through 65535, multiple items and repeated padding are supported.
/// Only definite, preferred CBOR is accepted. Bytes are retained exactly for
/// transcript/MAC processing; parsing does not normalize received data.
///
/// Applications must use registered labels and implement their specifications,
/// including repetition, criticality and stage-specific security policy.
#[derive(Clone, Eq, PartialEq)]
pub struct Ead {
    bytes: [u8; Self::CAPACITY],
    len: usize,
}

impl Ead {
    /// Maximum complete encoded EAD field length.
    pub const CAPACITY: usize = 64;

    /// Empty field; no EAD is added to the message.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [0; Self::CAPACITY],
            len: 0,
        }
    }

    /// Validate and copy an exact encoded EAD sequence. Empty means no field.
    pub fn from_encoded(encoded: &[u8]) -> Result<Self, EadError> {
        if encoded.len() > Self::CAPACITY {
            return Err(EadError::Capacity);
        }
        let mut remaining = encoded;
        while !remaining.is_empty() {
            take_item(&mut remaining)?;
        }
        let mut result = Self::new();
        result.bytes[..encoded.len()].copy_from_slice(encoded);
        result.len = encoded.len();
        Ok(result)
    }

    /// Append one registered item, preserving prior contents on failure.
    /// Supply caller-generated (pseudo)random value bytes for padding label 0.
    pub fn push(&mut self, label: i32, value: Option<&[u8]>) -> Result<(), EadError> {
        let magnitude = label.unsigned_abs();
        if magnitude > u16::MAX.into() {
            return Err(EadError::Label);
        }
        let argument = if label < 0 { magnitude - 1 } else { magnitude } as u16;
        let mut encoded_label = [0; 3];
        let label_bytes = encode_head(
            if label < 0 { 0x20 } else { 0 },
            argument,
            &mut encoded_label,
        );
        let mut encoded_length = [0; 3];
        let value_bytes = value.unwrap_or(&[]);
        if value_bytes.len() > Self::CAPACITY {
            return Err(EadError::Capacity);
        }
        let value_header = if value.is_some() {
            encode_head(0x40, value_bytes.len() as u16, &mut encoded_length)
        } else {
            &[]
        };
        let added = label_bytes.len() + value_header.len() + value_bytes.len();
        if added > Self::CAPACITY - self.len {
            return Err(EadError::Capacity);
        }
        for part in [label_bytes, value_header, value_bytes] {
            self.bytes[self.len..self.len + part.len()].copy_from_slice(part);
            self.len += part.len();
        }
        Ok(())
    }

    /// Exact complete EAD bytes. Treat values according to their confidentiality.
    #[must_use]
    pub fn as_encoded(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub(super) fn optional(&self) -> Option<Self> {
        (self.len != 0).then(|| self.clone())
    }

    pub(super) fn process(
        field: &Option<Self>,
        mut handler: impl FnMut(EadItem<'_>) -> bool,
    ) -> Result<(), Error> {
        let Some(field) = field else { return Ok(()) };
        let mut remaining = field.as_encoded();
        while !remaining.is_empty() {
            let item = take_item(&mut remaining).map_err(|_| Error::Parsing)?;
            // Padding must never reach the receiving security application.
            if item.label != 0 && !handler(item) && item.label < 0 {
                return Err(Error::Profile);
            }
        }
        Ok(())
    }
}

impl Default for Ead {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_head(major: u8, argument: u16, output: &mut [u8; 3]) -> &[u8] {
    match argument {
        0..=23 => {
            output[0] = major | argument as u8;
            &output[..1]
        }
        24..=255 => {
            output[..2].copy_from_slice(&[major | 24, argument as u8]);
            &output[..2]
        }
        _ => {
            output[0] = major | 25;
            output[1..].copy_from_slice(&argument.to_be_bytes());
            output
        }
    }
}

fn take_head(input: &mut &[u8]) -> Result<(u8, u64), EadError> {
    let (&first, tail) = input.split_first().ok_or(EadError::Malformed)?;
    let (value, consumed) = match first & 31 {
        value @ 0..=23 => (u64::from(value), 0),
        additional @ 24..=27 => {
            let count = 1usize << (additional - 24);
            let mut bytes = [0; 8];
            bytes[8 - count..].copy_from_slice(tail.get(..count).ok_or(EadError::Malformed)?);
            let value = u64::from_be_bytes(bytes);
            let minimum = match count {
                1 => 24,
                2 => 256,
                4 => 65536,
                _ => 1 << 32,
            };
            if value < minimum {
                return Err(EadError::Malformed);
            }
            (value, count)
        }
        _ => return Err(EadError::Malformed),
    };
    *input = &tail[consumed..];
    Ok((first >> 5, value))
}

fn take_item<'a>(input: &mut &'a [u8]) -> Result<EadItem<'a>, EadError> {
    let (major, argument) = take_head(input)?;
    let label = match major {
        0 if argument <= u64::from(u16::MAX) => argument as i32,
        1 if argument < u64::from(u16::MAX) => -1 - argument as i32,
        0..=1 => return Err(EadError::Label),
        _ => return Err(EadError::Malformed),
    };
    let value = if input.first().is_some_and(|byte| byte >> 5 == 2) {
        let (_, length) = take_head(input)?;
        let length = usize::try_from(length).map_err(|_| EadError::Malformed)?;
        let value = input.get(..length).ok_or(EadError::Malformed)?;
        *input = &input[length..];
        Some(value)
    } else {
        None
    };
    Ok(EadItem { label, value })
}

impl fmt::Debug for Ead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ead").field("len", &self.len).finish()
    }
}

impl fmt::Debug for EadItem<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EadItem")
            .field("label", &self.label)
            .field("value_len", &self.value.map(<[u8]>::len))
            .finish()
    }
}
