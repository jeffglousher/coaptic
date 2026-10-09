//! HKDF-SHA-256 and AES-CCM-16-64-128. RustCrypto only; no hand-rolled AEAD.

use aes::Aes128;
use ccm::aead::{AeadInOut, KeyInit, Tag};
use ccm::{
    Ccm,
    consts::{U8, U13},
};
use hkdf::Hkdf;
use sha2::Sha256;

use super::cbor;
use super::header::PartialIv;
use super::{Error, KEY_LEN, NONCE_LEN, TAG_LEN};

type Aes128Ccm = Ccm<Aes128, U8, U13>;

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
    out[..pt_len].copy_from_slice(&ciphertext[..pt_len]);
    let tag =
        Tag::<Aes128Ccm>::try_from(&ciphertext[pt_len..]).map_err(|_| Error::MessageLength)?;
    let cipher = Aes128Ccm::new(key.into());
    cipher
        .decrypt_inout_detached(nonce.into(), aad, (&mut out[..pt_len]).into(), &tag)
        .map_err(|_| Error::Decrypt)?;
    Ok(pt_len)
}

pub(crate) struct Aad {
    bytes: [u8; 64],
    len: usize,
}

impl Aad {
    pub(crate) fn new(request_kid: &[u8], request_piv: &[u8]) -> Result<Self, Error> {
        let mut bytes = [0u8; 64];
        let len = cbor::encode_aad(&mut bytes, request_kid, request_piv, &[])?;
        Ok(Self { bytes, len })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
