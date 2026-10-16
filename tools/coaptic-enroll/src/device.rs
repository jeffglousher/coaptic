//! Persistent host device keys sealed to the current Windows user with DPAPI.
//!
//! This is a local custody boundary, not a protected monotonic trust anchor.
//! A restored disk image can restore an older record. No peer or permission is
//! installed here, and this store never restores EDHOC or OSCORE session keys.

use crate::{Error, PublicIdentity, store};
use coaptic::provisioning::Identity;
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, path::Path};
use windows::{
    Win32::{
        Security::Cryptography::{
            CRYPTOAPI_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
        System::Memory::LocalFree,
    },
    core::PCWSTR,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    public: PublicIdentity,
    #[serde(with = "serde_bytes")]
    sealed: Vec<u8>,
}

/// Generate only during an explicit first setup in a new private device directory.
/// Existing, missing-after-setup, or corrupt records must never trigger this call
/// automatically. An uncertain write requires recovery, never a new identity.
pub fn provision(directory: &Path, kid: u8) -> Result<Identity, Error> {
    Identity::provision(
        kid,
        |out| getrandom::fill(out).is_ok(),
        |scalar, peer| {
            let record = Record {
                version: 1,
                public: PublicIdentity::from_peer(peer),
                sealed: protect(scalar)?,
            };
            let bytes = store::encode(&record)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(directory.join("device.cbor"))
                .map_err(|_| Error::Persistence)?;
            store::write_and_verify(file, &bytes)
        },
    )
    .map_err(|_| Error::Persistence)
}

/// Restore the same device identity without a hardware-key tap or online service.
/// The directory must remain private to the current authenticated OS user.
pub fn restore(directory: &Path) -> Result<Identity, Error> {
    let record: Record = store::read(&directory.join("device.cbor"))?;
    if record.version != 1 {
        return Err(Error::Input);
    }
    let plain = unprotect(&record.sealed)?;
    let scalar: Zeroizing<[u8; 32]> =
        Zeroizing::new(plain.as_slice().try_into().map_err(|_| Error::Input)?);
    let peer = record.public.peer()?;
    let identity = Identity::from_private_key(*scalar, peer.kid()).map_err(|_| Error::Input)?;
    if identity.peer().principal() != peer.principal() {
        return Err(Error::Input);
    }
    Ok(identity)
}

struct Blob(CRYPTOAPI_BLOB);
impl Drop for Blob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: DPAPI owns this LocalAlloc allocation for cbData bytes.
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                LocalFree(self.0.pbData as isize);
            }
        }
    }
}

fn protect(input: &[u8]) -> Result<Vec<u8>, Error> {
    let source = CRYPTOAPI_BLOB {
        cbData: input.len().try_into().map_err(|_| Error::Input)?,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut raw = CRYPTOAPI_BLOB::default();
    // SAFETY: DPAPI only reads the input. Output ownership transfers on success.
    let ok = unsafe {
        CryptProtectData(
            &source,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut raw,
        )
    };
    if !ok.as_bool() {
        return Err(Error::Persistence);
    }
    let output = Blob(raw);
    if output.0.pbData.is_null() || output.0.cbData == 0 || output.0.cbData > 16384 {
        return Err(Error::Persistence);
    }
    // SAFETY: successful DPAPI call initialized cbData bytes.
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}

fn unprotect(input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let source = CRYPTOAPI_BLOB {
        cbData: input.len().try_into().map_err(|_| Error::Input)?,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut raw = CRYPTOAPI_BLOB::default();
    // SAFETY: DPAPI only reads the input and suppresses all interactive prompts.
    let ok = unsafe {
        CryptUnprotectData(
            &source,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut raw,
        )
    };
    if !ok.as_bool() {
        return Err(Error::Persistence);
    }
    let output = Blob(raw);
    if output.0.pbData.is_null() || output.0.cbData != 32 {
        return Err(Error::Input);
    }
    // SAFETY: successful DPAPI call initialized cbData bytes; all copies zeroize.
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_keys_restore_without_interaction_and_refuse_modified_ciphertext() {
        let scalar = [7; 32];
        let mut sealed = protect(&scalar).unwrap();
        assert_ne!(sealed, scalar);
        assert_eq!(unprotect(&sealed).unwrap().as_slice(), scalar);
        let last = sealed.len() - 1;
        sealed[last] ^= 0x80;
        assert!(unprotect(&sealed).is_err());
        assert!(unprotect(&[]).is_err());
        assert!(unprotect(&protect(&[]).unwrap()).is_err());
    }

    #[test]
    fn durable_device_identity_recovers_and_never_overwrites_an_existing_key() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "coaptic-device-{:032x}",
            u128::from_le_bytes(random)
        ));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("device.cbor"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        assert!(restore(&directory).is_err());
        let first = provision(&directory, 17).unwrap();
        let principal = first.peer().principal();
        drop(first);
        assert_eq!(restore(&directory).unwrap().peer().principal(), principal);
        assert!(provision(&directory, 18).is_err());
        assert_eq!(restore(&directory).unwrap().peer().principal(), principal);
        std::fs::write(directory.join("device.cbor"), [0xff]).unwrap();
        assert!(restore(&directory).is_err());
        assert!(provision(&directory, 17).is_err());
    }
}
