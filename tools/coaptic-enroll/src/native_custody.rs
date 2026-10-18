//! macOS Keychain / Linux Secret Service custody. No file-only fallback.
use crate::{Error, sealing};
use std::path::Path;
use zeroize::Zeroizing;

fn entry() -> Result<keyring::Entry, Error> {
    keyring::Entry::new("org.coaptic.enrollment", "record-sealing-key")
        .map_err(|_| Error::Persistence)
}

fn key() -> Result<Zeroizing<[u8; 32]>, Error> {
    let bytes = Zeroizing::new(entry()?.get_secret().map_err(|_| Error::Persistence)?);
    Ok(Zeroizing::new(
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::Persistence)?,
    ))
}

/// Explicit first-time initialization only. Missing keys with existing records require recovery.
/// The caller holds the enrollment directory lock throughout this operation.
pub fn initialize(directory: &Path) -> Result<(), Error> {
    let entry = entry()?;
    match entry.get_secret() {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() == 32 {
                return Ok(());
            }
            return Err(Error::Persistence);
        }
        Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err(Error::Persistence),
    }
    if directory.join("Devices").exists() {
        return Err(Error::Persistence);
    }
    let mut bytes = Zeroizing::new([0; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| Error::Persistence)?;
    entry
        .set_secret(bytes.as_ref())
        .map_err(|_| Error::Persistence)?;
    if key()?.as_ref() != bytes.as_ref() {
        return Err(Error::Persistence);
    }
    Ok(())
}

pub(crate) fn protect(input: &[u8]) -> Result<Vec<u8>, Error> {
    let key = key()?;
    sealing::seal(&key, input)
}

pub(crate) fn unseal(input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let key = key()?;
    sealing::open(&key, input)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn absent_native_key_never_falls_back_to_unsealed_records() {
        // Windows test dependency has only keyring's mock backend enabled.
        assert!(super::protect(b"record").is_err());
        assert!(super::unseal(b"record").is_err());
    }
}
