//! Authenticated record envelope for native credential-store adapters.
use crate::Error;
use aes_gcm::{Aes256Gcm, KeyInit, aead::AeadInPlace};
use zeroize::Zeroizing;

const HEADER: &[u8] = b"Coaptic sealed record\x01";
const LIMIT: usize = 16384;

pub(crate) fn seal(key: &[u8; 32], input: &[u8]) -> Result<Vec<u8>, Error> {
    if input.len() > LIMIT {
        return Err(Error::Input);
    }
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).map_err(|_| Error::Persistence)?;
    seal_with_nonce(key, input, &nonce)
}

fn seal_with_nonce(key: &[u8; 32], input: &[u8], nonce: &[u8; 12]) -> Result<Vec<u8>, Error> {
    let cipher = Aes256Gcm::new(key.into());
    let mut ciphertext = Zeroizing::new(input.to_vec());
    let tag = cipher
        .encrypt_in_place_detached(nonce.into(), HEADER, &mut ciphertext)
        .map_err(|_| Error::Persistence)?;
    let mut output = Vec::with_capacity(HEADER.len() + 28 + ciphertext.len());
    output.extend_from_slice(HEADER);
    output.extend_from_slice(nonce);
    output.extend_from_slice(&tag);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

pub(crate) fn open(key: &[u8; 32], input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let start = HEADER.len();
    if input.len() < start + 28 || input.len() > LIMIT + start + 28 || !input.starts_with(HEADER) {
        return Err(Error::Input);
    }
    let cipher = Aes256Gcm::new(key.into());
    let nonce: [u8; 12] = input[start..start + 12]
        .try_into()
        .map_err(|_| Error::Input)?;
    let tag: [u8; 16] = input[start + 12..start + 28]
        .try_into()
        .map_err(|_| Error::Input)?;
    let mut plaintext = Zeroizing::new(input[start + 28..].to_vec());
    cipher
        .decrypt_in_place_detached(&nonce.into(), HEADER, &mut plaintext, &tag.into())
        .map_err(|_| Error::Persistence)?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_backend_erases_expanded_key_on_drop() {
        // Check the exact backend re-exported by aes-gcm, so dependency/version
        // changes cannot silently lose the key-schedule cleanup feature.
        fn requires_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        requires_zeroize_on_drop::<aes_gcm::aes::Aes256>();
    }
    #[test]
    fn refuses_modified_records_and_wrong_key() {
        let key = [7; 32];
        let sealed = seal(&key, b"private record").unwrap();
        assert_eq!(open(&key, &sealed).unwrap().as_slice(), b"private record");
        assert!(open(&[8; 32], &sealed).is_err());
        for index in 0..sealed.len() {
            let mut changed = sealed.clone();
            changed[index] ^= 1;
            assert!(open(&key, &changed).is_err());
        }
    }
    #[test]
    fn preserves_openssl_record_envelope_bytes() {
        // Generated independently with Python cryptography's AESGCM/OpenSSL.
        // Existing records place the detached tag before ciphertext.
        for (plaintext, envelope) in [
            (
                b"".as_slice(),
                &[
                    0x43, 0x6f, 0x61, 0x70, 0x74, 0x69, 0x63, 0x20, 0x73, 0x65, 0x61, 0x6c, 0x65,
                    0x64, 0x20, 0x72, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x01, 0x08, 0x08, 0x08, 0x08,
                    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x70, 0x9d, 0xb3, 0xda, 0x9a,
                    0xc3, 0xc2, 0xa3, 0xc8, 0xad, 0x6a, 0x88, 0x71, 0xe0, 0x91, 0xe4,
                ][..],
            ),
            (
                b"private record".as_slice(),
                &[
                    0x43, 0x6f, 0x61, 0x70, 0x74, 0x69, 0x63, 0x20, 0x73, 0x65, 0x61, 0x6c, 0x65,
                    0x64, 0x20, 0x72, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x01, 0x08, 0x08, 0x08, 0x08,
                    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x3d, 0x1b, 0x9f, 0x07, 0xbc,
                    0xb9, 0xb8, 0x11, 0x15, 0xe3, 0xd7, 0xe2, 0x8f, 0x08, 0xaf, 0x6d, 0x4c, 0x32,
                    0x37, 0xe8, 0x96, 0x4f, 0x83, 0xdd, 0xac, 0x5b, 0x17, 0x12, 0x8e, 0x11,
                ][..],
            ),
        ] {
            assert_eq!(
                seal_with_nonce(&[7; 32], plaintext, &[8; 12]).unwrap(),
                envelope
            );
            assert_eq!(open(&[7; 32], envelope).unwrap().as_slice(), plaintext);
        }
        assert!(seal(&[7; 32], &vec![0; LIMIT + 1]).is_err());
        assert!(open(&[7; 32], &vec![0; LIMIT + HEADER.len() + 29]).is_err());
    }
}
