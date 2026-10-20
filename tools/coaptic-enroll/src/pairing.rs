//! Current-user sealed durable pairing under an exclusive process lock.
//! This supplies the normal local-storage profile, not physical rollback protection.

use crate::{Action, Error, Request, device, store::RecordStore};
use coaptic::provisioning::{
    EnrollmentPolicy, Identity, LocalPairing, PairingChallenge, PairingRecord,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
};

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
    let mut bytes = [0; PairingRecord::MAX_ENCODED_LEN];
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

/// Preserve the exact public device checkpoint before an administrative mutation.
/// The caller holds the host pairing lock and authenticates the local device.
/// Existing files are never replaced, including malformed or partial archives.
pub fn save_device_checkpoint(directory: &Path, record: &PairingRecord) -> Result<PathBuf, Error> {
    let digest: String = record
        .commitment()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = directory.join(format!("device-checkpoint-{digest}.cbor"));
    let mut bytes = [0; PairingRecord::MAX_ENCODED_LEN];
    let length = record.encode(&mut bytes).map_err(|_| Error::Input)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(file) => crate::store::write_and_verify(file, &bytes[..length])?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_checkpoint_bytes(&path)? != bytes[..length] {
                return Err(Error::Persistence);
            }
        }
        Err(_) => return Err(Error::Persistence),
    }
    Ok(path)
}

/// Read bounded public CBOR retained before revocation. This does not establish
/// authenticity; the device must authenticate its original Pending binding.
pub fn read_device_checkpoint(path: &Path) -> Result<PairingRecord, Error> {
    PairingRecord::decode(&read_checkpoint_bytes(path)?).map_err(|_| Error::Input)
}

fn read_checkpoint_bytes(path: &Path) -> Result<Vec<u8>, Error> {
    let metadata = path.symlink_metadata().map_err(|_| Error::Persistence)?;
    if !metadata.is_file() || metadata.len() > PairingRecord::MAX_ENCODED_LEN as u64 {
        return Err(Error::Input);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::Input);
        }
    }
    let mut input = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| Error::Persistence)?
        .take(PairingRecord::MAX_ENCODED_LEN as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|_| Error::Persistence)?;
    Ok(input)
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
        Action::Transfer => pairing
            .transfer(
                challenge,
                &request.owner,
                request.pairing_policy()?,
                now_ms,
                commit,
            )
            .map_err(|_| Error::Persistence),
        // Recovery requires two independent approvals through its dedicated sink.
        _ => Err(Error::Input),
    }
}

/// Named service capabilities chosen before operator approval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    Echo,
    #[default]
    Telemetry,
}
impl Access {
    pub fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "--read-only" => Some(Self::ReadOnly),
            "--echo-only" => Some(Self::Echo),
            _ => None,
        }
    }
    pub fn permissions(self) -> [u8; 32] {
        match self {
            Self::ReadOnly => permissions(false),
            Self::Echo => echo_permissions(),
            Self::Telemetry => permissions(true),
        }
    }
}

/// Named application policy enforced by the commissioning runtime's routes.
pub fn permissions(write: bool) -> [u8; 32] {
    blake3::derive_key(
        "coaptic local service permissions v1",
        if write {
            b"GET /identity;POST /echo;POST /telemetry".as_slice()
        } else {
            b"GET /identity".as_slice()
        },
    )
}

/// Previously approved echo-only permission; never grants durable telemetry.
pub fn echo_permissions() -> [u8; 32] {
    blake3::derive_key(
        "coaptic local service permissions v1",
        b"GET /identity;POST /echo",
    )
}

pub fn telemetry_resource() -> [u8; 32] {
    blake3::derive_key("coaptic local service resource v1", b"POST /telemetry")
}

/// Explicit first initialization, never a missing-journal recovery fallback.
pub fn initialize_receipts(directory: &Path, identity: &Identity) -> Result<(), Error> {
    use coaptic::provisioning::TrustAnchor;
    use coaptic_receipts::{Authority, Journal, MAX_ROWS, Policy};
    let authority = Authority::new(Policy {
        anchor: TrustAnchor::genesis(*identity.peer().principal().fingerprint())
            .map_err(|_| Error::Input)?,
        principal: identity.peer().principal(),
        resource: telemetry_resource(),
        enabled: false,
    });
    Journal::create(&directory.join("receipts.bin"), authority, MAX_ROWS)
        .map_err(|_| Error::Persistence)?;
    Ok(())
}

/// Confirm an existing journal is recoverable before activating telemetry.
/// Holds its exclusive ownership until dropped; never creates or resets it.
pub fn preflight_receipts(
    directory: &Path,
    identity: &Identity,
) -> Result<coaptic_receipts::Journal, Error> {
    use coaptic::provisioning::TrustAnchor;
    use coaptic_receipts::{Authority, Journal, Policy};
    Journal::open(
        &directory.join("receipts.bin"),
        Authority::new(Policy {
            anchor: TrustAnchor::genesis(*identity.peer().principal().fingerprint())
                .map_err(|_| Error::Input)?,
            principal: identity.peer().principal(),
            resource: telemetry_resource(),
            enabled: false,
        }),
    )
    .map_err(|_| Error::Persistence)
}

#[cfg(all(test, windows))]
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

    #[test]
    fn transfer_store_restores_new_owner_and_refuses_the_old_claim_after_restart() {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "coaptic-transfer-{:032x}",
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&directory).unwrap();
        let identity = Identity::from_private_key([1; 32], 1).unwrap();
        let old_peer = Identity::from_private_key([2; 32], 2).unwrap().peer();
        let new_peer = Identity::from_private_key([3; 32], 3).unwrap().peer();
        {
            let store = PairingFile::open(&directory).unwrap();
            store.initialize(&identity).unwrap();
            let mut state = store.restore(&identity).unwrap();
            let challenge = state
                .begin(&identity, 0, |out| getrandom::fill(out).is_ok())
                .unwrap();
            let policy =
                EnrollmentPolicy::new(old_peer.clone(), [4; 32], permissions(true)).unwrap();
            apply(
                &Request::for_pairing(&challenge, &policy, Action::Pair).unwrap(),
                &challenge,
                &mut state,
                1,
                |old, next| store.commit(old, next),
            )
            .unwrap();
            let challenge = state
                .begin(&identity, 2, |out| getrandom::fill(out).is_ok())
                .unwrap();
            let policy =
                EnrollmentPolicy::new(new_peer.clone(), [5; 32], permissions(false)).unwrap();
            let request = Request::for_transfer(&challenge, [4; 32], &policy).unwrap();
            apply(&request, &challenge, &mut state, 3, |old, next| {
                store.commit(old, next)
            })
            .unwrap();
        }
        {
            let store = PairingFile::open(&directory).unwrap();
            let mut state = store.restore(&identity).unwrap();
            assert_eq!(state.checkpoint().revision(), 3);
            assert_eq!(state.checkpoint().policy().unwrap().owner(), &[5; 32]);
            assert!(state.permits(&new_peer.principal()));
            assert!(!state.permits(&old_peer.principal()));
            let challenge = state
                .begin(&identity, 0, |out| getrandom::fill(out).is_ok())
                .unwrap();
            let policy = EnrollmentPolicy::new(old_peer, [4; 32], permissions(true)).unwrap();
            assert!(
                apply(
                    &Request::for_pairing(&challenge, &policy, Action::Pair).unwrap(),
                    &challenge,
                    &mut state,
                    1,
                    |_, _| panic!("old claim reached storage")
                )
                .is_err()
            );
        }
        std::fs::remove_file(directory.join("pairing.cbor")).unwrap();
        std::fs::remove_file(directory.join("pairing.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
