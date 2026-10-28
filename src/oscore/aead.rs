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
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

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
