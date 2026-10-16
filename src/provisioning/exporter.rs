use core::fmt;

use hkdf::Hkdf;
use p256::elliptic_curve::zeroize::Zeroizing;
use sha2::Sha256;

/// An EDHOC application-key derivation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExporterError {
    /// The requested output exceeds SHA-256 HKDF's 8160-byte limit.
    OutputTooLong,
}

/// Application-key exporter from an authenticated, authorized EDHOC exchange.
///
/// Obtain this through [`super::Session`], after the mandatory message-4
/// transition. No raw exporter secret is exposed. Storage is fixed and the
/// exporter is neither `Clone` nor `Copy`; its retained secret is zeroized on
/// drop. This does not erase copies made inside the handshake implementation.
///
/// Retain it only for the lifetime of the authenticated authorization binding.
/// The caller must drop it when that binding is revoked or replaced, together
/// with any derived keys. Never persist it across a fresh device/App lifetime.
pub struct Exporter {
    prk: Zeroizing<[u8; 32]>,
}

impl Exporter {
    /// Maximum output length for the supported SHA-256 EDHOC suites.
    pub const MAX_OUTPUT_LEN: usize = 255 * 32;

    pub(super) fn new(prk: [u8; 32]) -> Self {
        Self {
            prk: Zeroizing::new(prk),
        }
    }

    /// Derives application keying material into caller-owned storage.
    ///
    /// This is RFC 9528 section 4.2.1's `EDHOC_Exporter`. Use the application's
    /// registered exporter label and context. A `(label, context)` pair must
    /// never identify two different purposes; changing only the output length
    /// does not provide a separate purpose. Labels 0 and 1 with empty context
    /// are already used by this session's OSCORE derivation.
    ///
    /// The output length is part of the derivation, so a shorter output is not
    /// a prefix of a longer export. Repeating the same inputs re-derives the
    /// same key; callers remain responsible for its safe use and nonce policy.
    ///
    /// Context is borrowed without copying or allocating. Work is proportional
    /// to its length and the requested number of SHA-256 output blocks. An
    /// empty output is allowed. Oversized output fails without modifying it or
    /// consuming the exporter; subsequent valid calls remain usable.
    pub fn export(
        &self,
        label: u64,
        context: &[u8],
        output: &mut [u8],
    ) -> Result<(), ExporterError> {
        if output.len() > Self::MAX_OUTPUT_LEN {
            return Err(ExporterError::OutputTooLong);
        }
        let mut label_header = [0; 9];
        let mut context_header = [0; 9];
        let mut length_header = [0; 9];
        let info = [
            header(0, label, &mut label_header),
            header(0x40, context.len() as u64, &mut context_header),
            context,
            header(0, output.len() as u64, &mut length_header),
        ];
        Hkdf::<Sha256>::from_prk(self.prk.as_ref())
            .expect("SHA-256 exporter PRK length")
            .expand_multi_info(&info, output)
            .map_err(|_| ExporterError::OutputTooLong)
    }
}

// Preferred CBOR integer/string-length header, independent of host word size.
fn header(major: u8, value: u64, output: &mut [u8; 9]) -> &[u8] {
    let (additional, bytes) = match value {
        0..=23 => {
            output[0] = major | value as u8;
            return &output[..1];
        }
        24..=0xff => (24, 1),
        0x100..=0xffff => (25, 2),
        0x10000..=0xffffffff => (26, 4),
        _ => (27, 8),
    };
    output[0] = major | additional;
    output[1..=bytes].copy_from_slice(&value.to_be_bytes()[8 - bytes..]);
    &output[..=bytes]
}

impl fmt::Debug for Exporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exporter").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
