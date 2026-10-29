//! HKDF-SHA-256 and AES-CCM-16-64-128. RustCrypto only; no hand-rolled AEAD.

use aes::Aes128Enc;
use ccm::aead::{AeadInOut, KeyInit, Tag, inout::InOutBuf};
use ccm::{
    Ccm,
    consts::{U8, U13},
};
use hkdf::Hkdf;
use sha2::Sha256;

use super::cbor;
use super::header::PartialIv;
use super::{Error, KEY_LEN, NONCE_LEN, TAG_LEN};

// CCM uses AES encryption for both sealing and opening; inverse round keys
// are unused. Keep the key schedule local rather than enlarging every context.
type Aes128Ccm = Ccm<Aes128Enc, U8, U13>;

pub(crate) fn hkdf_expand(
    master_secret: &[u8],
    master_salt: &[u8],
    id: &[u8],
    id_context: Option<&[u8]>,
    label: &str,
    out: &mut [u8],
) -> Result<(), Error> {
    let mut info = [0u8; 48];
    let n = cbor::encode_info(
        &mut info,
        id,
        id_context,
        label,
        u8::try_from(out.len()).map_err(|_| Error::Derive)?,
    )?;
    let hk = Hkdf::<Sha256>::new(Some(master_salt), master_secret);
    hk.expand(&info[..n], out).map_err(|_| Error::Derive)
}

pub(crate) fn nonce(common_iv: &[u8; NONCE_LEN], id_piv: &[u8], piv: PartialIv) -> [u8; NONCE_LEN] {
    let mut raw = [0u8; NONCE_LEN];
    raw[0] = id_piv.len() as u8;
    let id_pad = NONCE_LEN - 6;
    raw[1 + id_pad - id_piv.len()..1 + id_pad].copy_from_slice(id_piv);
    let piv_bytes = piv.as_bytes();
    raw[NONCE_LEN - piv_bytes.len()..].copy_from_slice(piv_bytes);
    for (r, iv) in raw.iter_mut().zip(common_iv.iter()) {
        *r ^= *iv;
    }
    raw
}

pub(crate) fn seal_in_place(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext_len: usize,
    buffer: &mut [u8],
) -> Result<usize, Error> {
    let n = plaintext_len
        .checked_add(TAG_LEN)
        .ok_or(Error::MessageLength)?;
    if buffer.len() < n {
        return Err(Error::BufferTooSmall);
    }
    let cipher = Aes128Ccm::new(key.into());
    let tag = cipher
        .encrypt_inout_detached(nonce.into(), aad, (&mut buffer[..plaintext_len]).into())
        .map_err(|_| Error::Encrypt)?;
    buffer[plaintext_len..n].copy_from_slice(&tag);
    Ok(n)
}

pub(crate) fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    if ciphertext.len() < TAG_LEN {
        return Err(Error::MessageLength);
    }
    let pt_len = ciphertext.len() - TAG_LEN;
    if out.len() < pt_len {
        return Err(Error::BufferTooSmall);
    }
    let tag =
        Tag::<Aes128Ccm>::try_from(&ciphertext[pt_len..]).map_err(|_| Error::MessageLength)?;
    let cipher = Aes128Ccm::new(key.into());
    let buffer = InOutBuf::new(&ciphertext[..pt_len], &mut out[..pt_len])
        .map_err(|_| Error::MessageLength)?;
    cipher
        .decrypt_inout_detached(nonce.into(), aad, buffer, &tag)
        .map_err(|_| Error::Decrypt)?;
    Ok(pt_len)
}

/// Open a response, preparing its alternative binding only after authentication fails.
/// Both attempts use the same immutable ciphertext and private plaintext scratch;
/// the local cipher is dropped before the caller processes authenticated Inner bytes.
pub(super) fn open_response<F>(
    key: &[u8; KEY_LEN],
    aad: &mut Aad,
    nonce: &mut [u8; NONCE_LEN],
    ciphertext: &[u8],
    out: &mut [u8],
    prepare_fallback: F,
) -> Result<(usize, bool), Error>
where
    F: FnOnce(&mut Aad, &mut [u8; NONCE_LEN]) -> Result<(), Error>,
{
    if ciphertext.len() < TAG_LEN {
        return Err(Error::MessageLength);
    }
    let pt_len = ciphertext.len() - TAG_LEN;
    if out.len() < pt_len {
        return Err(Error::BufferTooSmall);
    }
    let tag =
        Tag::<Aes128Ccm>::try_from(&ciphertext[pt_len..]).map_err(|_| Error::MessageLength)?;
    let cipher = Aes128Ccm::new(key.into());
    let buffer = InOutBuf::new(&ciphertext[..pt_len], &mut out[..pt_len])
        .map_err(|_| Error::MessageLength)?;
    if cipher
        .decrypt_inout_detached((&*nonce).into(), aad.as_bytes(), buffer, &tag)
        .is_ok()
    {
        return Ok((pt_len, false));
    }
    // Only the raw AEAD failure reaches this continuation. Preflight and
    // authenticated semantic errors cannot trigger another binding attempt.
    prepare_fallback(aad, nonce)?;
    let buffer = InOutBuf::new(&ciphertext[..pt_len], &mut out[..pt_len])
        .map_err(|_| Error::MessageLength)?;
    cipher
        .decrypt_inout_detached((&*nonce).into(), aad.as_bytes(), buffer, &tag)
        .map_err(|_| Error::Decrypt)?;
    Ok((pt_len, true))
}

// Supported callers supply a validated request kid <= 7 bytes and Partial IV
// <= 5 bytes, with empty Class I here: prefix 11 + bstr header 1 + external
// array 19 = at most 31 encoded bytes. ID Context is separate. Revisit this
// bound if those supported inputs change; the general encoder is unchanged.
pub(crate) struct Aad {
    bytes: [u8; 32],
    len: usize,
}

impl Aad {
    pub(crate) fn new(request_kid: &[u8], request_piv: &[u8]) -> Result<Self, Error> {
        let mut bytes = [0u8; 32];
        let len = cbor::encode_aad(&mut bytes, request_kid, request_piv, &[])?;
        Ok(Self { bytes, len })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub(super) fn replace_request(
        &mut self,
        request_kid: &[u8],
        request_piv: &[u8],
    ) -> Result<(), Error> {
        let len = cbor::encode_aad(&mut self.bytes, request_kid, request_piv, &[])?;
        self.len = len;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn response_continuation_matches_one_shot_with_same_scratch() {
        let key = [0x91; KEY_LEN];
        let lengths = [0, 1, 15, 16, 17, 63, 64, 65, 1280];
        for (case, length) in lengths.into_iter().enumerate() {
            let mut nonce = [0x92; NONCE_LEN];
            nonce[0] = case as u8;
            let mut wrong_nonce = nonce;
            wrong_nonce[1] ^= 1;
            let aad = Aad::new(&[7], &[9]).unwrap();
            let mut ciphertext = [0x5a; 1280 + TAG_LEN];
            let n = seal_in_place(&key, &nonce, aad.as_bytes(), length, &mut ciphertext).unwrap();
            let saved = ciphertext;
            // Reopening the same ciphertext is safe. Only the encryption above
            // consumes a nonce; deterministic key construction is not a reset.
            for first in 0..3 {
                for second in 0..3 {
                    let mut aad = Aad::new(&[if first == 1 { 8 } else { 7 }], &[9]).unwrap();
                    let mut first_nonce = if first == 2 { wrong_nonce } else { nonce };
                    let mut actual = [0xcc; 1281];
                    let mut expected = actual;
                    let initial = open(
                        &key,
                        &first_nonce,
                        aad.as_bytes(),
                        &ciphertext[..n],
                        &mut expected,
                    );
                    let expected_result = match initial {
                        Ok(len) => Ok((len, false)),
                        Err(Error::Decrypt) => {
                            let fallback_aad =
                                Aad::new(&[if second == 1 { 8 } else { 7 }], &[9]).unwrap();
                            open(
                                &key,
                                if second == 2 { &wrong_nonce } else { &nonce },
                                fallback_aad.as_bytes(),
                                &ciphertext[..n],
                                &mut expected,
                            )
                            .map(|len| (len, true))
                        }
                        Err(error) => Err(error),
                    };
                    let mut calls = 0;
                    let result = open_response(
                        &key,
                        &mut aad,
                        &mut first_nonce,
                        &ciphertext[..n],
                        &mut actual,
                        |aad, fallback_nonce| {
                            calls += 1;
                            aad.replace_request(&[if second == 1 { 8 } else { 7 }], &[9])?;
                            *fallback_nonce = if second == 2 { wrong_nonce } else { nonce };
                            Ok(())
                        },
                    );
                    assert_eq!(result, expected_result);
                    assert_eq!(calls, usize::from(first != 0));
                    assert_eq!(actual, expected);
                    assert!(actual[length..].iter().all(|byte| *byte == 0xcc));
                    if result.is_err() {
                        assert!(actual[..length].iter().all(|byte| *byte == 0));
                    } else {
                        assert_eq!(&actual[..length], &[0x5a; 1280][..length]);
                    }
                    assert_eq!(ciphertext, saved);
                    // Recover with the very same scratch after success or two
                    // authentication failures, always rereading immutable ct.
                    aad.replace_request(&[7], &[9]).unwrap();
                    first_nonce = nonce;
                    assert_eq!(
                        open_response(
                            &key,
                            &mut aad,
                            &mut first_nonce,
                            &ciphertext[..n],
                            &mut actual,
                            |_, _| panic!("successful authentication must stop"),
                        ),
                        Ok((length, false))
                    );
                    assert_eq!(&actual[..length], &[0x5a; 1280][..length]);
                    assert!(actual[length..].iter().all(|byte| *byte == 0xcc));
                }
            }
        }
    }

    #[test]
    fn response_continuation_only_follows_raw_authentication_failure() {
        let key = [0x93; KEY_LEN];
        let nonce = [0x94; NONCE_LEN];
        let mut aad = Aad::new(&[7], &[9]).unwrap();
        let mut ciphertext = [0x5a; 17 + TAG_LEN];
        let n = seal_in_place(&key, &nonce, aad.as_bytes(), 17, &mut ciphertext).unwrap();
        // Tag extraction is exact and both InOut slices have pt_len, so their
        // refusals cannot be constructed after these valid-slice preflights.
        for bad_tag in [false, true] {
            let mut ct = ciphertext;
            if bad_tag {
                ct[n - 1] ^= 1;
            }
            for (ct_len, capacity, error) in [
                (TAG_LEN - 1, 0, Error::MessageLength),
                (TAG_LEN - 1, 18, Error::MessageLength),
                (n, 0, Error::BufferTooSmall),
                (n, 16, Error::BufferTooSmall),
            ] {
                let mut actual = [0xcc; 18];
                let mut expected = actual;
                let mut calls = 0;
                let mut current_nonce = nonce;
                assert_eq!(
                    open_response(
                        &key,
                        &mut aad,
                        &mut current_nonce,
                        &ct[..ct_len],
                        &mut actual[..capacity],
                        |_, _| {
                            calls += 1;
                            Err(Error::Decrypt)
                        },
                    ),
                    Err(error)
                );
                assert_eq!(
                    open(
                        &key,
                        &nonce,
                        aad.as_bytes(),
                        &ct[..ct_len],
                        &mut expected[..capacity]
                    ),
                    Err(error)
                );
                assert_eq!(calls, 0);
                assert_eq!(actual, expected);
                assert_eq!(actual, [0xcc; 18]);
            }
        }
        let mut corrupt = ciphertext;
        corrupt[n - 1] ^= 1;
        for refill_error in [false, true] {
            let mut scratch = [0xcc; 18];
            let mut calls = 0;
            let mut current_nonce = nonce;
            let old_len = aad.len;
            let result = open_response(
                &key,
                &mut aad,
                &mut current_nonce,
                &corrupt,
                &mut scratch,
                |aad, _| {
                    calls += 1;
                    if refill_error {
                        aad.replace_request(&[0; 9], &[0; 5])?;
                    }
                    // The absent fallback exits before any refill in production.
                    Err(Error::Decrypt)
                },
            );
            assert_eq!(
                result,
                Err(if refill_error {
                    Error::BufferTooSmall
                } else {
                    Error::Decrypt
                })
            );
            assert_eq!(calls, 1);
            assert_eq!(aad.len, old_len);
            assert_eq!(current_nonce, nonce);
            assert_eq!(&scratch[..17], &[0; 17]);
            assert_eq!(scratch[17], 0xcc);
            assert_eq!(ciphertext[n - 1] ^ corrupt[n - 1], 1);
            aad.replace_request(&[7], &[9]).unwrap();
            let mut recovery_calls = 0;
            assert_eq!(
                open_response(
                    &key,
                    &mut aad,
                    &mut current_nonce,
                    &ciphertext,
                    &mut scratch,
                    |_, _| {
                        recovery_calls += 1;
                        Err(Error::Decrypt)
                    },
                ),
                Ok((17, false))
            );
            assert_eq!(recovery_calls, 0);
            assert_eq!(&scratch[..17], &[0x5a; 17]);
            assert_eq!(scratch[17], 0xcc);
        }
        // These callback counters establish lazy preparation, not AEAD counts:
        // FnOnce and the helper's two explicit decrypt sites bound attempts.
    }

    #[test]
    fn aad_refill_reuses_backing_and_commits_length_only_on_success() {
        let long_kid = [0x71; 7];
        let long_piv = [0x81; 5];
        let mut aad = Aad::new(&long_kid, &long_piv).unwrap();
        let backing = aad.bytes.as_ptr();
        for (kid, piv) in [
            (&[][..], &[1][..]),
            (&long_kid[..], &long_piv[..]),
            (&[2][..], &[3, 4][..]),
            (&long_kid[..], &long_piv[..]),
        ] {
            aad.replace_request(kid, piv).unwrap();
            let fresh = Aad::new(kid, piv).unwrap();
            assert_eq!(aad.as_bytes(), fresh.as_bytes());
            assert_eq!(aad.bytes.as_ptr(), backing);
        }
        let old_len = aad.len;
        assert_eq!(
            aad.replace_request(&[0; 9], &long_piv),
            Err(Error::BufferTooSmall)
        );
        assert_eq!(aad.len, old_len);
        // A failed refill may partially mutate private bytes. The helper exits
        // immediately; no stale as_bytes view is used after that failure.
    }

    fn assert_aad_matches_reference(kid: &[u8], piv: &[u8]) -> usize {
        let mut reference = [0xa5; 64];
        let len = cbor::encode_aad(&mut reference, kid, piv, &[]).unwrap();
        let aad = Aad::new(kid, piv).unwrap();
        assert_eq!(aad.as_bytes(), &reference[..len]);
        assert!(reference[len..].iter().all(|byte| *byte == 0xa5));
        len
    }

    #[test]
    fn aad_matches_reference_for_every_supported_kid_and_piv_length() {
        let kid = [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
        let piv = [0x81, 0x82, 0x83, 0x84, 0x85];
        assert_eq!(kid.len(), super::super::MAX_ID_LEN);
        assert_eq!(piv.len(), super::super::MAX_PIV_LEN);
        let mut maximum = 0;
        for kid_len in 0..=kid.len() {
            for piv_len in 1..=piv.len() {
                let partial_iv = PartialIv::from_bytes(&piv[..piv_len]).unwrap();
                let len = assert_aad_matches_reference(&kid[..kid_len], partial_iv.as_bytes());
                assert_eq!(len, 19 + kid_len + piv_len);
                maximum = maximum.max(len);
            }
        }
        assert_eq!(maximum, 31);
        for seq in [0, (1u64 << 40) - 1] {
            let piv = PartialIv::from_seq(seq).unwrap();
            assert_eq!(
                piv.as_bytes(),
                if seq == 0 { &[0][..] } else { &[0xff; 5][..] }
            );
            assert_aad_matches_reference(&kid, piv.as_bytes());
        }
    }

    #[test]
    fn aad_backing_and_layout_are_smaller_than_legacy_storage() {
        struct LegacyAad {
            bytes: [u8; 64],
            len: usize,
        }
        let mut legacy = LegacyAad {
            bytes: [0xa5; 64],
            len: 0,
        };
        legacy.len = cbor::encode_aad(&mut legacy.bytes, &[], &[0], &[]).unwrap();
        let aad = Aad::new(&[], &[0]).unwrap();
        assert_eq!(aad.as_bytes(), &legacy.bytes[..legacy.len]);
        assert_eq!(legacy.bytes.len() - aad.bytes.len(), 32);
        let actual = core::mem::size_of::<Aad>();
        let previous = core::mem::size_of::<LegacyAad>();
        assert_eq!(
            core::mem::align_of::<Aad>(),
            core::mem::align_of::<LegacyAad>()
        );
        assert_eq!(previous - actual, 32);
        std::println!(
            "AAD object bytes: current={actual} legacy={previous}; not a stack-frame measurement"
        );
    }

    #[test]
    fn aad_bound_does_not_claim_arbitrary_private_input_equivalence() {
        // A nine-byte kid cannot inhabit RequestRef or match a stored recipient.
        let kid = [0x31; 9];
        let piv = [0x72; 5];
        let mut general = [0xa5; 64];
        assert_eq!(cbor::encode_aad(&mut general, &kid, &piv, &[]), Ok(33));
        assert!(matches!(Aad::new(&kid, &piv), Err(Error::BufferTooSmall)));
        // Adding Class I would also require revisiting the local capacity.
        assert_eq!(
            cbor::encode_aad(&mut general, &kid[..7], &piv, &[1, 2]),
            Ok(33)
        );
    }

    #[test]
    fn separate_input_authenticates_without_modifying_ciphertext_or_output_tail() {
        let key = [0x31; KEY_LEN];
        let nonce = [0x27; NONCE_LEN];
        for length in [0, 1, 15, 16, 17, 63, 64, 65, 1280] {
            let mut sealed = [0x5a; 1280 + TAG_LEN];
            let n = seal_in_place(&key, &nonce, b"aad", length, &mut sealed).unwrap();
            let original = sealed;
            let mut opened = [0xcc; 1281];
            assert_eq!(
                open(&key, &nonce, b"aad", &sealed[..n], &mut opened),
                Ok(length)
            );
            assert_eq!(&opened[..length], &[0x5a; 1280][..length]);
            assert!(opened[length..].iter().all(|byte| *byte == 0xcc));
            assert_eq!(sealed, original);
            assert_eq!(
                open(&key, &nonce, b"wrong", &sealed[..n], &mut opened),
                Err(Error::Decrypt)
            );
            assert!(opened[..length].iter().all(|byte| *byte == 0));
            assert!(opened[length..].iter().all(|byte| *byte == 0xcc));
            assert_eq!(sealed, original);
        }
    }

    #[test]
    fn in_place_capacity_checks_preserve_plaintext_and_exact_fit_authenticates() {
        let key = [0u8; KEY_LEN];
        let nonce = [0u8; NONCE_LEN];
        let mut short = [0x31u8; TAG_LEN];
        assert_eq!(
            seal_in_place(&key, &nonce, b"fixture", 1, &mut short),
            Err(Error::BufferTooSmall)
        );
        assert_eq!(short, [0x31; TAG_LEN]);
        assert_eq!(
            seal_in_place(&key, &nonce, b"fixture", usize::MAX, &mut short),
            Err(Error::MessageLength)
        );
        assert_eq!(short, [0x31; TAG_LEN]);
        let mut exact = [0u8; TAG_LEN + 1];
        exact[0] = 0x31;
        assert_eq!(
            seal_in_place(&key, &nonce, b"fixture", 1, &mut exact),
            Ok(exact.len())
        );
        let mut plain = [0u8; 1];
        assert_eq!(open(&key, &nonce, b"fixture", &exact, &mut plain), Ok(1));
        assert_eq!(plain, [0x31]);
        exact[0] ^= 1;
        assert_eq!(
            open(&key, &nonce, b"fixture", &exact, &mut plain),
            Err(Error::Decrypt)
        );
    }
}
