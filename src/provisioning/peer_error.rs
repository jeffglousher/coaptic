use super::lakers::CBORDecoder;

/// An unauthenticated EDHOC error received in the selected sequential profile.
/// Neither the code nor its diagnostics establish identity or policy authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerErrorCode {
    /// Reserved success code received on the wire; the session must abort.
    ReservedSuccess,
    /// Error code 1: unspecified error with a UTF-8 diagnostic.
    Unspecified,
    /// Error code 2: wrong selected cipher suite, replying to message 1 only.
    WrongCipherSuite,
    /// Error code 3: unknown referenced credential.
    UnknownCredential,
}

/// A borrowed view of an error retained in a fixed bootstrap exchange buffer.
///
/// Diagnostics are untrusted input. The caller chooses whether and how to log
/// them; this view does not allocate, reflect them on the wire, select a cipher
/// suite, change trust or trigger another handshake. It expires with its owner.
#[derive(Clone, Copy)]
pub struct PeerError<'a> {
    code: PeerErrorCode,
    info: &'a [u8],
}

impl<'a> PeerError<'a> {
    /// Maximum number of advertised suites accepted by this fixed profile.
    pub const SUITES_CAPACITY: usize = 9;

    /// Maximum diagnostic length accepted by the selected CBOR string decoder.
    pub const DIAGNOSTIC_CAPACITY: usize = 255;

    /// The error code; it is not authenticated by the handshake.
    #[must_use]
    pub const fn code(self) -> PeerErrorCode {
        self.code
    }

    /// Complete validated UTF-8 diagnostic for error code 1, without truncation.
    #[must_use]
    pub fn diagnostic(self) -> Option<&'a str> {
        (self.code == PeerErrorCode::Unspecified)
            .then(|| core::str::from_utf8(self.info).expect("validated diagnostic"))
    }

    /// Advertised signed eight-bit suite identifiers for error code 2.
    /// Other error codes produce an empty iterator. This is advisory metadata;
    /// the application must keep its configured security profile unchanged.
    pub fn cipher_suites(self) -> impl Iterator<Item = i8> + 'a {
        let mut decoder = CBORDecoder::new(self.info);
        let count = if self.code != PeerErrorCode::WrongCipherSuite {
            0
        } else if decoder.current().is_ok_and(|head| head >> 5 == 4) {
            decoder.array().expect("validated suite array")
        } else {
            1
        };
        (0..count).map(move |_| decoder.i8().expect("validated suite identifier"))
    }

    pub(super) fn parse(bytes: &'a [u8]) -> Option<Self> {
        let mut decoder = CBORDecoder::new(bytes);
        let code = match decoder.u8().ok()? {
            0 => PeerErrorCode::ReservedSuccess,
            1 => PeerErrorCode::Unspecified,
            2 => PeerErrorCode::WrongCipherSuite,
            3 => PeerErrorCode::UnknownCredential,
            _ => return None,
        };
        let start = decoder.position();
        let info = match code {
            // Reserved zero must abort even when the following information is
            // malformed or exceeds the selected decoder's bounds. Do not make
            // this mandatory termination depend on parsing an arbitrary item.
            PeerErrorCode::ReservedSuccess => {
                return Some(Self {
                    code,
                    info: &bytes[start..],
                });
            }
            PeerErrorCode::Unspecified => {
                let text = decoder.str().ok()?;
                core::str::from_utf8(text).ok()?;
                text
            }
            PeerErrorCode::WrongCipherSuite => {
                let count = if decoder.current().ok()? >> 5 == 4 {
                    let count = decoder.array().ok()?;
                    if count < 2 {
                        return None;
                    }
                    count
                } else {
                    1
                };
                if count > Self::SUITES_CAPACITY {
                    return None;
                }
                for _ in 0..count {
                    decoder.i8().ok()?;
                }
                &bytes[start..decoder.position()]
            }
            PeerErrorCode::UnknownCredential => {
                if decoder.read_slice(1).ok()? != [0xf5] {
                    return None;
                }
                &bytes[start..decoder.position()]
            }
        };
        decoder.ensure_finished().ok()?;
        Some(Self { code, info })
    }
}

impl core::fmt::Debug for PeerError<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PeerError")
            .field("code", &self.code)
            .field("info_length", &self.info.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn peer_error_views_preserve_diagnostics_and_suite_bounds() {
        let error = PeerError::parse(&[1, 0x62, b'o', b'k']).unwrap();
        assert_eq!(error.code(), PeerErrorCode::Unspecified);
        assert_eq!(error.diagnostic(), Some("ok"));
        assert_eq!(error.cipher_suites().count(), 0);
        for bytes in [
            &[2, 2][..],
            &[2, 0x82, 2, 0x20],
            &[2, 0x89, 0, 1, 2, 3, 4, 5, 6, 7, 8],
        ] {
            let error = PeerError::parse(bytes).unwrap();
            assert_eq!(error.code(), PeerErrorCode::WrongCipherSuite);
            assert!(error.diagnostic().is_none());
            assert!(error.cipher_suites().count() <= PeerError::SUITES_CAPACITY);
        }
        assert_eq!(
            PeerError::parse(&[2, 0x82, 2, 0x20])
                .unwrap()
                .cipher_suites()
                .collect::<std::vec::Vec<_>>(),
            [2, -1]
        );
        assert_eq!(
            PeerError::parse(&[0, 0xf6]).unwrap().code(),
            PeerErrorCode::ReservedSuccess
        );
        for bytes in [&[0][..], &[0, 0x19, 1, 0], &[0, 0x81], &[0, 0xf6, 0]] {
            assert_eq!(
                PeerError::parse(bytes).unwrap().code(),
                PeerErrorCode::ReservedSuccess
            );
        }
        assert_eq!(
            PeerError::parse(&[3, 0xf5]).unwrap().code(),
            PeerErrorCode::UnknownCredential
        );
    }

    #[test]
    fn peer_error_views_refuse_malformed_types_trailing_and_excess_suites() {
        for bytes in [
            &[][..],
            &[1],
            &[1, 0x41, 0],
            &[1, 0x61, 0xff],
            &[1, 0x60, 0],
            &[2, 0x80],
            &[2, 0x81, 2],
            &[2, 0x8a, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[2, 0x82, 2],
            &[2, 0x18, 0xff],
            &[3, 0xf4],
            &[3, 0xf5, 0],
            &[23, 0xf6],
        ] {
            assert!(PeerError::parse(bytes).is_none(), "{bytes:?}");
        }
    }

    #[test]
    fn peer_error_diagnostic_capacity_and_every_truncation_are_checked() {
        let mut bytes = std::vec![1, 0x78, 255];
        bytes.extend_from_slice(&[b'x'; PeerError::DIAGNOSTIC_CAPACITY]);
        assert_eq!(
            PeerError::parse(&bytes)
                .unwrap()
                .diagnostic()
                .unwrap()
                .len(),
            255
        );
        for len in 0..bytes.len() {
            assert!(PeerError::parse(&bytes[..len]).is_none());
        }
        let mut oversized = std::vec![1, 0x79, 1, 0];
        oversized.extend_from_slice(&[b'x'; 256]);
        assert!(PeerError::parse(&oversized).is_none());
    }
}
