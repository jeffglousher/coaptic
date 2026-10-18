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

#[cfg(test)]
mod native_tests {
    use super::*;

    /// Run only in an isolated CI credential store, never against user custody.
    #[test]
    #[ignore = "requires an isolated, unlocked native credential store"]
    fn native_custody_roundtrip_and_missing_key_refusal() {
        assert_eq!(
            std::env::var("COAPTIC_NATIVE_CUSTODY_TEST").as_deref(),
            Ok("1")
        );
        let entry = entry().unwrap();
        assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
        let directory =
            std::env::temp_dir().join(format!("coaptic-native-custody-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = super::entry()
                    .and_then(|entry| entry.delete_credential().map_err(|_| Error::Persistence));
                let _ = std::fs::remove_dir(self.0.join("Devices"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        initialize(&directory).unwrap();
        let sealed = protect(b"native custody roundtrip").unwrap();
        // Load the key again from the OS store for each operation.
        assert_eq!(
            unseal(&sealed).unwrap().as_slice(),
            b"native custody roundtrip"
        );
        initialize(&directory).unwrap();
        assert!(unseal(&sealed).is_ok());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(unseal(&tampered).is_err());
        std::fs::create_dir(directory.join("Devices")).unwrap();
        entry.delete_credential().unwrap();
        assert!(unseal(&sealed).is_err());
        assert!(protect(b"new record").is_err());
        assert!(initialize(&directory).is_err());
        assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
    }
}
