//! Current-user sealed durable pairing under an exclusive process lock.
//! This supplies the normal local-storage profile, not physical rollback protection.

use crate::{Action, Error, Request, device, store::RecordStore};
use coaptic::provisioning::{
    EnrollmentPolicy, Identity, LocalPairing, PairingChallenge, PairingRecord,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedRecord {
    version: u8,
    #[serde(with = "serde_bytes")]
    sealed: Vec<u8>,
}

pub struct PairingFile(RecordStore<SealedRecord>);

impl PairingFile {
    /// Directory must be private and contain the independently recovered identity.
    pub fn open(directory: &Path) -> Result<Self, Error> {
        Ok(Self(RecordStore::open_named(directory, "pairing")?))
    }

    /// Explicit initialization only. Never called as recovery from missing state.
    pub fn initialize(&self, identity: &Identity) -> Result<(), Error> {
        let mut store = [0; 32];
        getrandom::fill(&mut store).map_err(|_| Error::Persistence)?;
        let record = PairingRecord::unpaired(identity, store).map_err(|_| Error::Input)?;
        self.0.initialize(&seal(&record)?)
    }

    pub fn restore(&self, identity: &Identity) -> Result<LocalPairing, Error> {
        let record = open(&self.0.load()?)?;
        LocalPairing::restore(identity, record).map_err(|_| Error::Input)
    }

    pub fn commit(&self, expected: &PairingRecord, next: &PairingRecord) -> Result<(), Error> {
        let prior = self.0.load()?;
        let current = open(&prior)?;
        if current.commitment() != expected.commitment()
            || current.store() != next.store()
            || current.revision().checked_add(1) != Some(next.revision())
        {
            return Err(Error::Persistence);
        }
        self.0.commit(&prior, &seal(next)?)
    }
}

fn seal(record: &PairingRecord) -> Result<SealedRecord, Error> {
    let mut bytes = [0; 224];
    let len = record.encode(&mut bytes).map_err(|_| Error::Input)?;
    Ok(SealedRecord {
        version: 1,
        sealed: device::protect(&bytes[..len])?,
    })
}

fn open(record: &SealedRecord) -> Result<PairingRecord, Error> {
    if record.version != 1 {
        return Err(Error::Input);
    }
    PairingRecord::decode(&device::unseal(&record.sealed)?).map_err(|_| Error::Input)
}

/// Commit only the exact live challenge and the exact authenticated action.
pub fn apply(
    request: &Request,
    challenge: &PairingChallenge,
    pairing: &mut LocalPairing,
    now_ms: u64,
    commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), Error>,
) -> Result<(), Error> {
    request.matches_pairing(challenge)?;
    match request.action {
        Action::Pair => {
            let policy =
                EnrollmentPolicy::new(request.peer.peer()?, request.owner, request.permissions)
                    .map_err(|_| Error::Input)?;
            pairing
                .pair(challenge, policy, now_ms, commit)
                .map_err(|_| Error::Persistence)
        }
        Action::Revoke => {
            let prior = pairing.checkpoint().policy().ok_or(Error::Input)?;
            if prior.peer().principal() != request.peer.peer()?.principal()
                || prior.owner() != &request.owner
                || prior.resource_policy() != &request.permissions
            {
                return Err(Error::Input);
            }
            pairing
                .revoke(challenge, &request.owner, now_ms, commit)
                .map_err(|_| Error::Persistence)
        }
    }
}

/// Named application policy enforced by the commissioning runtime's routes.
pub fn permissions(write: bool) -> [u8; 32] {
    blake3::derive_key(
        "coaptic local service permissions v1",
        if write {
            b"GET /identity;POST /echo".as_slice()
        } else {
            b"GET /identity".as_slice()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_store_restores_pairing_and_persists_revocation_without_operator() {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "coaptic-pairing-{:032x}",
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&directory).unwrap();
        let identity = Identity::from_private_key([1; 32], 1).unwrap();
        let peer = Identity::from_private_key([2; 32], 2).unwrap().peer();
        {
            let store = PairingFile::open(&directory).unwrap();
            store.initialize(&identity).unwrap();
            assert!(store.initialize(&identity).is_err());
            let mut pairing = store.restore(&identity).unwrap();
            let challenge = pairing
                .begin(&identity, 0, |out| getrandom::fill(out).is_ok())
                .unwrap();
            let policy = EnrollmentPolicy::new(peer.clone(), [3; 32], permissions(true)).unwrap();
            let request = Request::for_pairing(&challenge, &policy, Action::Pair).unwrap();
            apply(&request, &challenge, &mut pairing, 1, |old, next| {
                store.commit(old, next)
            })
            .unwrap();
        }
        {
            let store = PairingFile::open(&directory).unwrap();
            let mut pairing = store.restore(&identity).unwrap();
            assert!(pairing.permits(&peer.principal()));
            let challenge = pairing
                .begin(&identity, 0, |out| getrandom::fill(out).is_ok())
                .unwrap();
            let request = Request::for_pairing(
                &challenge,
                pairing.checkpoint().policy().unwrap(),
                Action::Revoke,
            )
            .unwrap();
            apply(&request, &challenge, &mut pairing, 1, |old, next| {
                store.commit(old, next)
            })
            .unwrap();
        }
        {
            let store = PairingFile::open(&directory).unwrap();
            assert!(!store.restore(&identity).unwrap().permits(&peer.principal()));
            std::fs::write(directory.join("pairing.cbor"), [0xff]).unwrap();
            assert!(store.restore(&identity).is_err());
            assert!(store.initialize(&identity).is_err());
        }
        std::fs::remove_file(directory.join("pairing.cbor")).unwrap();
        std::fs::remove_file(directory.join("pairing.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
