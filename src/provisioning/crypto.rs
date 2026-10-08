use core::fmt;

use super::lakers;

use lakers::{
    BufferCiphertext3, BufferPlaintext3, BytesCcmIvLen, BytesCcmKeyLen, BytesHashLen,
    BytesMaxBuffer, BytesMaxInfoBuffer, BytesP256ElemLen, CryptoTrait, EDHOCError, MAX_BUFFER_LEN,
};
use p256::elliptic_curve::{
    point::{AffineCoordinates, DecompressPoint},
    zeroize::Zeroize,
};
use sha2::{Digest, Sha256};

use super::Error;
use crate::oscore::aead;

pub(super) struct Crypto {
    ephemeral: Option<p256::SecretKey>,
}

impl Crypto {
    /// Obtain one ephemeral scalar using at most eight fallible entropy calls.
    /// Each successful call must fill the entire output with fresh cryptographic entropy.
    pub(super) fn fresh(mut entropy: impl FnMut(&mut [u8]) -> bool) -> Result<Self, Error> {
        let mut candidate = [0u8; 32];
        for _ in 0..8 {
            if !entropy(&mut candidate) {
                candidate.zeroize();
                return Err(Error::Entropy);
            }
            let scalar = p256::SecretKey::from_slice(&candidate);
            candidate.zeroize();
            if let Ok(ephemeral) = scalar {
                return Ok(Self {
                    ephemeral: Some(ephemeral),
                });
            }
        }
        Err(Error::InvalidKey)
    }
}

impl fmt::Debug for Crypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Crypto").finish_non_exhaustive()
    }
}

/// Validate a compact P-256 public key before admitting it into an EDHOC state.
pub(super) fn validate_public_x(public_key: &[u8; 32]) -> Result<(), Error> {
    Option::<p256::AffinePoint>::from(p256::AffinePoint::decompress(public_key.into(), 1.into()))
        .map(|_| ())
        .ok_or(Error::InvalidKey)
}

/// Derive the compact public key for a validated local private scalar.
#[cfg(test)]
pub(super) fn public_x_for_private(private_key: &[u8; 32]) -> Result<[u8; 32], Error> {
    let secret = p256::SecretKey::from_slice(private_key).map_err(|_| Error::InvalidKey)?;
    Ok(secret.public_key().as_affine().x().into())
}

impl CryptoTrait for Crypto {
    fn sha256_digest(&mut self, message: &BytesMaxBuffer, message_len: usize) -> BytesHashLen {
        Sha256::digest(
            message
                .get(..message_len)
                .expect("bounded EDHOC hash input"),
        )
        .into()
    }

    fn hkdf_expand(
        &mut self,
        prk: &BytesHashLen,
        info: &BytesMaxInfoBuffer,
        info_len: usize,
        length: usize,
    ) -> BytesMaxBuffer {
        let mut output = [0u8; MAX_BUFFER_LEN];
        let hkdf = hkdf::Hkdf::<Sha256>::from_prk(prk).expect("SHA-256 PRK length");
        hkdf.expand(
            info.get(..info_len).expect("bounded EDHOC KDF input"),
            output.get_mut(..length).expect("bounded EDHOC KDF output"),
        )
        .expect("bounded EDHOC KDF length");
        output
    }

    fn hkdf_extract(&mut self, salt: &BytesHashLen, ikm: &BytesP256ElemLen) -> BytesHashLen {
        hkdf::Hkdf::<Sha256>::extract(Some(salt), ikm).0.into()
    }

    fn aes_ccm_encrypt_tag_8(
        &mut self,
        key: &BytesCcmKeyLen,
        iv: &BytesCcmIvLen,
        ad: &[u8],
        plaintext: &BufferPlaintext3,
    ) -> BufferCiphertext3 {
        let mut output = BufferCiphertext3::new();
        let source = plaintext
            .content
            .get(..plaintext.len)
            .expect("bounded EDHOC plaintext");
        output.content[..source.len()].copy_from_slice(source);
        output.len = aead::seal_in_place(key, iv, ad, source.len(), &mut output.content)
            .expect("bounded EDHOC encryption input");
        output
    }

    fn aes_ccm_decrypt_tag_8(
        &mut self,
        key: &BytesCcmKeyLen,
        iv: &BytesCcmIvLen,
        ad: &[u8],
        ciphertext: &BufferCiphertext3,
    ) -> Result<BufferPlaintext3, EDHOCError> {
        let source = ciphertext
            .content
            .get(..ciphertext.len)
            .ok_or(EDHOCError::ParsingError)?;
        let mut output = BufferPlaintext3::new();
        output.len = aead::open(key, iv, ad, source, &mut output.content).map_err(|error| {
            if error == crate::oscore::Error::Decrypt {
                EDHOCError::MacVerificationFailed
            } else {
                EDHOCError::ParsingError
            }
        })?;
        Ok(output)
    }

    fn p256_ecdh(
        &mut self,
        private_key: &BytesP256ElemLen,
        public_key: &BytesP256ElemLen,
    ) -> BytesP256ElemLen {
        let secret =
            p256::SecretKey::from_slice(private_key).expect("validated EDHOC private scalar");
        let public = p256::AffinePoint::decompress(public_key.into(), 1.into())
            .expect("validated EDHOC public key");
        (*p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), public).raw_secret_bytes()).into()
    }

    fn get_random_byte(&mut self) -> u8 {
        panic!("EDHOC profile supplies explicit connection identifiers")
    }

    fn p256_generate_key_pair(&mut self) -> (BytesP256ElemLen, BytesP256ElemLen) {
        let secret = self
            .ephemeral
            .take()
            .expect("one ephemeral key generation per EDHOC exchange");
        (
            secret.to_bytes().into(),
            secret.public_key().as_affine().x().into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(value: u8) -> [u8; 32] {
        let mut scalar = [0u8; 32];
        scalar[31] = value;
        scalar
    }

    fn backend(value: u8) -> Crypto {
        Crypto::fresh(|output| {
            output.copy_from_slice(&scalar(value));
            true
        })
        .unwrap()
    }

    #[test]
    fn entropy_failure_and_invalid_scalars_stop_with_bounded_attempts() {
        let mut calls = 0;
        assert!(matches!(
            Crypto::fresh(|_| {
                calls += 1;
                false
            }),
            Err(Error::Entropy)
        ));
        assert_eq!(calls, 1);
        calls = 0;
        assert!(matches!(
            Crypto::fresh(|output| {
                calls += 1;
                output.fill(0);
                true
            }),
            Err(Error::InvalidKey)
        ));
        assert_eq!(calls, 8);
        calls = 0;
        let mut crypto = Crypto::fresh(|output| {
            calls += 1;
            output.fill(0);
            if calls == 3 {
                output[31] = 1;
            }
            true
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(crypto.p256_generate_key_pair().0, scalar(1));
    }

    #[test]
    fn compact_public_key_validation_and_real_ecdh_agree() {
        let public_one = public_x_for_private(&scalar(1)).unwrap();
        let public_two = public_x_for_private(&scalar(2)).unwrap();
        let expected = [
            0x7c, 0xf2, 0x7b, 0x18, 0x8d, 0x03, 0x4f, 0x7e, 0x8a, 0x52, 0x38, 0x03, 0x04, 0xb5,
            0x1a, 0xc3, 0xc0, 0x89, 0x69, 0xe2, 0x77, 0xf2, 0x1b, 0x35, 0xa6, 0x0b, 0x48, 0xfc,
            0x47, 0x66, 0x99, 0x78,
        ];
        assert_eq!(public_two, expected);
        assert!(validate_public_x(&public_one).is_ok());
        assert!(matches!(
            validate_public_x(&[0xff; 32]),
            Err(Error::InvalidKey)
        ));
        assert!(matches!(
            public_x_for_private(&[0; 32]),
            Err(Error::InvalidKey)
        ));
        assert!(matches!(
            public_x_for_private(&[0xff; 32]),
            Err(Error::InvalidKey)
        ));
        let mut crypto = backend(3);
        assert_eq!(crypto.p256_ecdh(&scalar(1), &public_two), expected);
        assert_eq!(crypto.p256_ecdh(&scalar(2), &public_one), expected);
    }

    #[test]
    fn sha256_and_hkdf_expand_match_published_vectors() {
        let mut crypto = backend(1);
        let expected_hash = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(
            crypto.sha256_digest(&[0xff; MAX_BUFFER_LEN], 0),
            expected_hash
        );
        let prk = [
            0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf, 0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b,
            0xba, 0x63, 0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31, 0x22, 0xec, 0x84, 0x4a,
            0xd7, 0xc2, 0xb3, 0xe5,
        ];
        let mut info: BytesMaxInfoBuffer = [0u8; lakers::MAX_INFO_LEN];
        info[..10].copy_from_slice(&[0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9]);
        let expected = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        let output = crypto.hkdf_expand(&prk, &info, 10, expected.len());
        assert_eq!(output[..expected.len()], expected);
        assert!(output[expected.len()..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn aes_ccm_matches_peer_vector_and_rejects_corruption_and_short_inputs() {
        let key = [
            0x36, 0x8f, 0x35, 0xa1, 0xf8, 0x0e, 0xaa, 0xac, 0xd6, 0xbb, 0x13, 0x66, 0x09, 0x38,
            0x97, 0x27,
        ];
        let iv = [
            0x84, 0x2a, 0x84, 0x45, 0x84, 0x75, 0x02, 0xea, 0x77, 0x36, 0x3a, 0x16, 0xb6,
        ];
        let aad = [
            0x34, 0x39, 0x6d, 0xfc, 0xfa, 0x6f, 0x74, 0x2a, 0xea, 0x70, 0x40, 0x97, 0x6b, 0xd5,
            0x96, 0x49, 0x7a, 0x7a, 0x6f, 0xa4, 0xfb, 0x85, 0xee, 0x8e, 0x4c, 0xa3, 0x94, 0xd0,
            0x20, 0x95, 0xb7, 0xbf,
        ];
        let plaintext = [
            0x1c, 0xcc, 0xd5, 0x58, 0x25, 0x31, 0x6a, 0x94, 0xc5, 0x97, 0x9e, 0x04, 0x93, 0x10,
            0xd1, 0xd7, 0x17, 0xcd, 0xfb, 0x76, 0x24, 0x28, 0x9d, 0xac,
        ];
        let expected = [
            0x1a, 0x58, 0x09, 0x4f, 0x0e, 0x8c, 0x60, 0x35, 0xa5, 0x58, 0x4b, 0xfa, 0x8d, 0x10,
            0x09, 0xc5, 0xf7, 0x8f, 0xd2, 0xca, 0x48, 0x7f, 0xf2, 0x22, 0xf6, 0xd1, 0xd8, 0x97,
            0xd6, 0x05, 0x16, 0x18,
        ];
        let mut crypto = backend(1);
        let plaintext_buffer = BufferPlaintext3::new_from_slice(&plaintext).unwrap();
        let ciphertext = crypto.aes_ccm_encrypt_tag_8(&key, &iv, &aad, &plaintext_buffer);
        assert_eq!(ciphertext.as_slice(), expected);
        assert_eq!(
            crypto
                .aes_ccm_decrypt_tag_8(&key, &iv, &aad, &ciphertext)
                .unwrap()
                .as_slice(),
            plaintext
        );
        let mut corrupted = ciphertext;
        corrupted.content[0] ^= 1;
        assert!(matches!(
            crypto.aes_ccm_decrypt_tag_8(&key, &iv, &aad, &corrupted),
            Err(EDHOCError::MacVerificationFailed)
        ));
        for length in 0..8 {
            let mut short = BufferCiphertext3::new();
            short.len = length;
            assert!(matches!(
                crypto.aes_ccm_decrypt_tag_8(&key, &iv, &aad, &short),
                Err(EDHOCError::ParsingError)
            ));
        }
    }
}
