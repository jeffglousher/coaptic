//! Explicit no-touch PIV development approval. This is not FIDO2 user verification.
//!
//! A fresh challenge binds the full commissioning request and durable operator
//! revision. The private key stays on the token. Development approvals cannot
//! be passed to `ApprovedRequest::approve`, the production FIDO2 path.

use crate::{Error, Request, TIMEOUT};
use openssl::{
    bn::BigNumContext,
    ec::{EcGroup, EcKey, EcPoint},
    ecdsa::EcdsaSig,
    nid::Nid,
    pkey::Public,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentOperator {
    version: u8,
    profile: String,
    serial: u32,
    slot: u8,
    #[serde(with = "serde_bytes")]
    public_key: Vec<u8>,
    revision: u64,
}

fn public_key(bytes: &[u8]) -> Result<EcKey<Public>, Error> {
    if bytes.len() != 65 || bytes[0] != 4 {
        return Err(Error::Input);
    }
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(|_| Error::Input)?;
    let mut context = BigNumContext::new().map_err(|_| Error::Input)?;
    let point = EcPoint::from_bytes(&group, bytes, &mut context).map_err(|_| Error::Input)?;
    let key = EcKey::from_public_key(&group, &point).map_err(|_| Error::Input)?;
    key.check_key().map_err(|_| Error::Input)?;
    Ok(key)
}

impl DevelopmentOperator {
    fn validate(&self) -> Result<(), Error> {
        if self.version != 1
            || self.profile != "development-piv-no-touch"
            || self.serial == 0
            || !(0x82..=0x95).contains(&self.slot)
        {
            return Err(Error::Input);
        }
        public_key(&self.public_key)?;
        Ok(())
    }

    pub fn owner(&self) -> Result<[u8; 32], Error> {
        self.validate()?;
        Ok(blake3::derive_key(
            "coaptic development PIV operator owner v1",
            &self.public_key,
        ))
    }

    pub fn serial(&self) -> u32 {
        self.serial
    }

    pub fn slot(&self) -> u8 {
        self.slot
    }

    /// Prove possession before registering a supplied, independently selected key.
    pub fn registration(serial: u32, slot: u8, key: &[u8]) -> Result<PendingRegistration, Error> {
        let operator = Self {
            version: 1,
            profile: "development-piv-no-touch".into(),
            serial,
            slot,
            public_key: key.to_vec(),
            revision: 0,
        };
        operator.validate()?;
        let digest = challenge(&operator, b"registration", &[0; 32])?;
        Ok(PendingRegistration {
            operator,
            digest,
            started: Instant::now(),
        })
    }

    pub fn begin(&self, request: Request) -> Result<PendingApproval, Error> {
        self.validate()?;
        if request.owner != self.owner()? || self.revision == u64::MAX {
            return Err(Error::Input);
        }
        let digest = challenge(self, b"commissioning", &request.commitment()?)?;
        Ok(PendingApproval {
            operator: self.clone(),
            request,
            digest,
            started: Instant::now(),
        })
    }

    fn verify(&self, digest: &[u8; 32], signature: &[u8]) -> Result<(), Error> {
        if signature.len() > 72 {
            return Err(Error::Authentication);
        }
        let signature = EcdsaSig::from_der(signature).map_err(|_| Error::Authentication)?;
        let key = public_key(&self.public_key)?;
        if !signature
            .verify(digest, &key)
            .map_err(|_| Error::Authentication)?
        {
            return Err(Error::Authentication);
        }
        Ok(())
    }
}

fn challenge(
    operator: &DevelopmentOperator,
    operation: &[u8],
    commitment: &[u8; 32],
) -> Result<[u8; 32], Error> {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::Authentication)?;
    if nonce == [0; 32] {
        return Err(Error::Authentication);
    }
    let mut transcript = Vec::from(&b"coaptic development PIV proof v1\0"[..]);
    transcript.extend_from_slice(operation);
    transcript.extend_from_slice(&operator.serial.to_be_bytes());
    transcript.push(operator.slot);
    transcript.extend_from_slice(&operator.public_key);
    transcript.extend_from_slice(&operator.revision.to_be_bytes());
    transcript.extend_from_slice(commitment);
    transcript.extend_from_slice(&nonce);
    Ok(openssl::sha::sha256(&transcript))
}

pub struct PendingRegistration {
    operator: DevelopmentOperator,
    digest: [u8; 32],
    started: Instant,
}

impl PendingRegistration {
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn finish(self, signature: &[u8]) -> Result<DevelopmentOperator, Error> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        self.operator.verify(&self.digest, signature)?;
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        Ok(self.operator)
    }
}

pub struct PendingApproval {
    operator: DevelopmentOperator,
    request: Request,
    digest: [u8; 32],
    started: Instant,
}

impl PendingApproval {
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Persist the exact compare-and-swap before exposing the single-use approval.
    pub fn finish(
        self,
        signature: &[u8],
        persist: impl FnOnce(&DevelopmentOperator, &DevelopmentOperator) -> Result<(), ()>,
    ) -> Result<DevelopmentApproval, Error> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        self.operator.verify(&self.digest, signature)?;
        let mut next = self.operator.clone();
        next.revision = next.revision.checked_add(1).ok_or(Error::Counter)?;
        persist(&self.operator, &next).map_err(|_| Error::Persistence)?;
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        Ok(DevelopmentApproval {
            request: self.request,
            started: self.started,
        })
    }
}

/// Only an explicitly selected development commissioning sink may consume this.
/// It is neither serializable nor a production FIDO2 `ApprovedRequest`.
pub struct DevelopmentApproval {
    request: Request,
    started: Instant,
}

impl DevelopmentApproval {
    pub fn commit<T>(self, commit: impl FnOnce(&Request) -> Result<T, Error>) -> Result<T, Error> {
        if self.started.elapsed() >= TIMEOUT {
            return Err(Error::Expired);
        }
        commit(&self.request)
    }
}

#[cfg(windows)]
pub fn sign(serial: u32, slot: u8, digest: &[u8; 32]) -> Result<Vec<u8>, Error> {
    use yubikey::{YubiKey, piv};
    let mut key = YubiKey::open_by_serial(serial.into()).map_err(|_| Error::Authentication)?;
    let slot = piv::SlotId::try_from(slot).map_err(|_| Error::Input)?;
    // No PIN attempt, touch-policy change, credential replacement or fallback.
    piv::sign_data(&mut key, digest, piv::AlgorithmId::EccP256, slot)
        .map(|value| value.to_vec())
        .map_err(|_| Error::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PublicIdentity, store};
    use coaptic::provisioning::{EnrollmentAnchor, Identity};
    use openssl::{ec::PointConversionForm, pkey::Private};

    fn fixture() -> (DevelopmentOperator, EcKey<Private>) {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = EcKey::generate(&group).unwrap();
        let public = key
            .public_key()
            .to_bytes(
                &group,
                PointConversionForm::UNCOMPRESSED,
                &mut BigNumContext::new().unwrap(),
            )
            .unwrap();
        let pending = DevelopmentOperator::registration(123, 0x82, &public).unwrap();
        let signature = EcdsaSig::sign(pending.digest(), &key)
            .unwrap()
            .to_der()
            .unwrap();
        (pending.finish(&signature).unwrap(), key)
    }

    fn request(operator: &DevelopmentOperator) -> Request {
        let device = Identity::from_private_key([1; 32], 1).unwrap().peer();
        let peer = Identity::from_private_key([2; 32], 2).unwrap().peer();
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).unwrap();
        Request::new(
            &device,
            &peer,
            operator.owner().unwrap(),
            [3; 32],
            EnrollmentAnchor::from_parts([4; 32], 1, [5; 32]).unwrap(),
            nonce,
        )
        .unwrap()
    }

    #[test]
    fn fresh_hardware_proof_requires_durable_cas_and_consumes_exact_request() {
        let (operator, key) = fixture();
        let original = request(&operator);
        let commitment = original.commitment().unwrap();
        let pending = operator.begin(original).unwrap();
        let signature = EcdsaSig::sign(pending.digest(), &key)
            .unwrap()
            .to_der()
            .unwrap();
        let approval = pending
            .finish(&signature, |old, next| {
                assert_eq!(old.revision + 1, next.revision);
                Ok(())
            })
            .unwrap();
        approval
            .commit(|request| {
                assert_eq!(request.commitment()?, commitment);
                Ok(())
            })
            .unwrap();
        let pending = operator.begin(request(&operator)).unwrap();
        let signature = EcdsaSig::sign(pending.digest(), &key)
            .unwrap()
            .to_der()
            .unwrap();
        assert!(matches!(
            pending.finish(&signature, |_, _| Err(())),
            Err(Error::Persistence)
        ));
    }

    #[test]
    fn transfer_development_proof_covers_both_owners_and_refuses_rebinding() {
        use coaptic::provisioning::{EnrollmentPolicy, LocalPairing, PairingRecord};
        let (operator, key) = fixture();
        let device = Identity::from_private_key([1; 32], 1).unwrap();
        let peer = Identity::from_private_key([2; 32], 2).unwrap();
        let mut state =
            LocalPairing::restore(&device, PairingRecord::unpaired(&device, [3; 32]).unwrap())
                .unwrap();
        let first = state
            .begin(&device, 0, |out| getrandom::fill(out).is_ok())
            .unwrap();
        state
            .pair(
                &first,
                EnrollmentPolicy::new(peer.peer(), operator.owner().unwrap(), [4; 32]).unwrap(),
                1,
                |_, _| Ok::<(), ()>(()),
            )
            .unwrap();
        let challenge = state
            .begin(&device, 2, |out| getrandom::fill(out).is_ok())
            .unwrap();
        let destination = EnrollmentPolicy::new(peer.peer(), [9; 32], [5; 32]).unwrap();
        let request =
            Request::for_transfer(&challenge, operator.owner().unwrap(), &destination).unwrap();
        let pending = operator.begin(request.clone()).unwrap();
        let signature = EcdsaSig::sign(pending.digest(), &key)
            .unwrap()
            .to_der()
            .unwrap();
        let mut changed = request;
        changed.next_owner = [8; 32];
        assert!(matches!(
            operator
                .begin(changed)
                .unwrap()
                .finish(&signature, |_, _| panic!("rebound transfer persisted")),
            Err(Error::Authentication)
        ));
        let approval = pending.finish(&signature, |_, _| Ok(())).unwrap();
        approval
            .commit(|request| {
                request.matches_pairing(&challenge)?;
                state
                    .transfer(
                        &challenge,
                        &request.owner,
                        request.pairing_policy()?,
                        3,
                        |_, _| Ok::<(), ()>(()),
                    )
                    .map_err(|_| Error::Persistence)
            })
            .unwrap();
        assert_eq!(state.checkpoint().policy().unwrap().owner(), &[9; 32]);
    }

    #[test]
    fn refuses_replay_changed_request_and_profile_confusion() {
        let (operator, key) = fixture();
        let pending = operator.begin(request(&operator)).unwrap();
        let signature = EcdsaSig::sign(pending.digest(), &key)
            .unwrap()
            .to_der()
            .unwrap();
        let mut changed = request(&operator);
        changed.permissions[0] ^= 1;
        let different = operator.begin(changed).unwrap();
        assert!(matches!(
            different.finish(&signature, |_, _| panic!()),
            Err(Error::Authentication)
        ));
        let mut changed = request(&operator);
        changed.action = crate::Action::Revoke;
        assert!(matches!(
            operator
                .begin(changed)
                .unwrap()
                .finish(&signature, |_, _| panic!()),
            Err(Error::Authentication)
        ));
        let mut invalid = operator.clone();
        invalid.profile = "fido2".into();
        assert!(matches!(
            invalid.begin(request(&operator)),
            Err(Error::Input)
        ));
        assert!(store::decode::<crate::Operator>(&store::encode(&operator).unwrap()).is_err());
        let mut wrong_owner = request(&operator);
        wrong_owner.owner = [7; 32];
        assert!(matches!(operator.begin(wrong_owner), Err(Error::Input)));
        let peer = Identity::from_private_key([8; 32], 8).unwrap().peer();
        let mut changed = request(&operator);
        changed.device = PublicIdentity::from_peer(&peer);
        assert!(matches!(
            operator
                .begin(changed)
                .unwrap()
                .finish(&signature, |_, _| panic!()),
            Err(Error::Authentication)
        ));
    }
}
