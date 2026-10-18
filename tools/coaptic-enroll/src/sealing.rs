//! Authenticated record envelope for native credential-store adapters.
use crate::Error;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};
use zeroize::Zeroizing;

const HEADER: &[u8] = b"Coaptic sealed record\x01";
const LIMIT: usize = 16384;

pub(crate) fn seal(key: &[u8; 32], input: &[u8]) -> Result<Vec<u8>, Error> {
    if input.len() > LIMIT {
        return Err(Error::Input);
    }
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).map_err(|_| Error::Persistence)?;
    let mut tag = [0; 16];
    let ciphertext = encrypt_aead(
        Cipher::aes_256_gcm(),
        key,
        Some(&nonce),
        HEADER,
        input,
        &mut tag,
    )
    .map_err(|_| Error::Persistence)?;
    let mut output = Vec::with_capacity(HEADER.len() + 28 + ciphertext.len());
    output.extend_from_slice(HEADER);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&tag);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

pub(crate) fn open(key: &[u8; 32], input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let start = HEADER.len();
    if input.len() < start + 28 || input.len() > LIMIT + start + 28 || !input.starts_with(HEADER) {
        return Err(Error::Input);
    }
    decrypt_aead(
        Cipher::aes_256_gcm(),
        key,
        Some(&input[start..start + 12]),
        HEADER,
        &input[start + 28..],
        &input[start + 12..start + 28],
    )
    .map(Zeroizing::new)
    .map_err(|_| Error::Persistence)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
