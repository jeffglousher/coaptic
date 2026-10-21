//! Explicit no-touch PIV development approval. This is not FIDO2 user verification.
//!
//! A fresh challenge binds the full commissioning request and durable operator
//! revision. The private key stays on the token. Development approvals cannot
//! be passed to `ApprovedRequest::approve`, the production FIDO2 path.

use crate::{Error, Request, TIMEOUT};
use p256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

fn public_key(bytes: &[u8]) -> Result<VerifyingKey, Error> {
    if bytes.len() != 65 || bytes[0] != 4 {
        return Err(Error::Input);
    }
    VerifyingKey::from_sec1_bytes(bytes).map_err(|_| Error::Input)
}

impl DevelopmentOperator {
    fn validate(&self) -> Result<(), Error> {
        if self.version != 1
            || !matches!(
                self.profile.as_str(),
                "development-piv-no-touch" | "piv-touch-required"
            )
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

    pub fn requires_touch(&self) -> bool {
        self.profile == "piv-touch-required"
    }

    /// Fresh possession and policy check without issuing an approval or updating records.
    pub fn self_test(&self) -> Result<(), Error> {
        self.validate()?;
        let digest = challenge(self, b"self-test", &[0; 32])?;
        let signature = self.sign(&digest)?;
        self.verify(&digest, &signature)
    }

    /// Every signing operation checks the live key's policy and public key first.
    pub fn sign(&self, digest: &[u8; 32]) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut key = yubikey::YubiKey::open_by_serial(self.serial.into())
            .map_err(|_| Error::Authentication)?;
        let slot = yubikey::piv::SlotId::try_from(self.slot).map_err(|_| Error::Input)?;
        check_policy(&mut key, slot, &self.public_key, self.requires_touch())?;
        yubikey::piv::sign_data(&mut key, digest, yubikey::piv::AlgorithmId::EccP256, slot)
            .map(|bytes| bytes.to_vec())
            .map_err(|_| Error::Authentication)
    }

    /// Prove possession before registering a supplied, independently selected key.
    pub fn registration(serial: u32, slot: u8, key: &[u8]) -> Result<PendingRegistration, Error> {
        Self::registration_with_touch(serial, slot, key, false)
    }

    pub fn registration_with_touch(
        serial: u32,
        slot: u8,
        key: &[u8],
        touch: bool,
    ) -> Result<PendingRegistration, Error> {
        let operator = Self {
            version: 1,
            profile: if touch {
                "piv-touch-required"
            } else {
                "development-piv-no-touch"
            }
            .into(),
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
        let signature = Signature::from_der(signature).map_err(|_| Error::Authentication)?;
        public_key(&self.public_key)?
            .verify_prehash(digest, &signature)
            .map_err(|_| Error::Authentication)
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
    transcript.extend_from_slice(operator.profile.as_bytes());
    transcript.extend_from_slice(&operator.serial.to_be_bytes());
    transcript.push(operator.slot);
    transcript.extend_from_slice(&operator.public_key);
    transcript.extend_from_slice(&operator.revision.to_be_bytes());
    transcript.extend_from_slice(commitment);
    transcript.extend_from_slice(&nonce);
    Ok(Sha256::digest(&transcript).into())
}

pub struct PendingRegistration {
    operator: DevelopmentOperator,
    digest: [u8; 32],
    started: Instant,
}

impl PendingRegistration {
    pub fn sign(&self) -> Result<Vec<u8>, Error> {
        self.operator.sign(&self.digest)
    }
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

/// Inspect the selected physical key without signing or changing token state.
pub fn verify_policy(serial: u32, slot: u8, public: &[u8], touch: bool) -> Result<(), Error> {
    let mut key =
        yubikey::YubiKey::open_by_serial(serial.into()).map_err(|_| Error::Authentication)?;
    let slot = yubikey::piv::SlotId::try_from(slot).map_err(|_| Error::Input)?;
    check_policy(&mut key, slot, public, touch)
}

fn check_policy(
    key: &mut yubikey::YubiKey,
    slot: yubikey::piv::SlotId,
    public: &[u8],
    touch: bool,
) -> Result<(), Error> {
    use yubikey::{PinPolicy, TouchPolicy, piv};
    let expected = if touch {
        TouchPolicy::Always
    } else {
        TouchPolicy::Never
    };
    if let Ok(metadata) = piv::metadata(key, slot) {
        if metadata.policy != Some((PinPolicy::Never, expected))
            || metadata
                .public
                .as_ref()
                .and_then(|p| p.subject_public_key.as_bytes())
                != Some(public)
        {
            return Err(Error::Authentication);
        }
        return Ok(());
    }
    // Older firmware exposes policy through generated-key attestation, not metadata.
    // Trust is the explicitly selected local token, not a manufacturer certification claim.
    use x509_cert::der::{Decode, Encode};
    let der = piv::attest(key, slot).map_err(|_| Error::Authentication)?;
    let cert = x509_cert::Certificate::from_der(&der).map_err(|_| Error::Authentication)?;
    let issuer = yubikey::Certificate::read(key, piv::SlotId::Attestation)
        .map_err(|_| Error::Authentication)?
        .cert
        .to_der()
        .map_err(|_| Error::Authentication)?;
    let issuer = x509_cert::Certificate::from_der(&issuer).map_err(|_| Error::Authentication)?;
    verify_attestation(&cert, &issuer, public, touch)
}

fn verify_attestation(
    cert: &x509_cert::Certificate,
    issuer: &x509_cert::Certificate,
    public: &[u8],
    touch: bool,
) -> Result<(), Error> {
    if cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        != Some(public)
    {
        return Err(Error::Authentication);
    }
    let extensions = cert
        .tbs_certificate
        .extensions
        .as_ref()
        .ok_or(Error::Authentication)?;
    let mut policy = extensions
        .iter()
        .filter(|e| e.extn_id.to_string() == "1.3.6.1.4.1.41482.3.8");
    let expected = [1, if touch { 2 } else { 1 }];
    if policy.next().map(|e| e.extn_value.as_bytes()) != Some(expected.as_slice())
        || policy.next().is_some()
    {
        return Err(Error::Authentication);
    }
    verify_attestation_signature(cert, issuer)
}

// PIV attestation uses SHA-256 with either RSA PKCS#1 v1.5 or ECDSA.
// The issuer is read from the selected token; this does not assert a CA chain.
fn verify_attestation_signature(
    cert: &x509_cert::Certificate,
    issuer: &x509_cert::Certificate,
) -> Result<(), Error> {
    use p256::pkcs8::DecodePublicKey;
    use x509_cert::der::Encode;
    if cert.signature_algorithm != cert.tbs_certificate.signature {
        return Err(Error::Authentication);
    }
    let signed = cert
        .tbs_certificate
        .to_der()
        .map_err(|_| Error::Authentication)?;
    let digest = Sha256::digest(&signed);
    let signature = cert.signature.as_bytes().ok_or(Error::Authentication)?;
    let spki = issuer
        .tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|_| Error::Authentication)?;
    match cert.signature_algorithm.oid.to_string().as_str() {
        "1.2.840.113549.1.1.11" => {
            // SHA256-with-RSA permits conventional NULL or absent parameters.
            if cert
                .signature_algorithm
                .parameters
                .as_ref()
                .is_some_and(|p| p != &x509_cert::der::asn1::Any::null())
            {
                return Err(Error::Authentication);
            }
            rsa::RsaPublicKey::from_public_key_der(&spki)
                .map_err(|_| Error::Authentication)?
                .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, signature)
                .map_err(|_| Error::Authentication)
        }
        "1.2.840.10045.4.3.2" if cert.signature_algorithm.parameters.is_none() => {
            if let Ok(key) = VerifyingKey::from_public_key_der(&spki) {
                let signature =
                    Signature::from_der(signature).map_err(|_| Error::Authentication)?;
                key.verify_prehash(&digest, &signature)
                    .map_err(|_| Error::Authentication)
            } else {
                let key = p384::ecdsa::VerifyingKey::from_public_key_der(&spki)
                    .map_err(|_| Error::Authentication)?;
                let signature = p384::ecdsa::Signature::from_der(signature)
                    .map_err(|_| Error::Authentication)?;
                key.verify_prehash(&digest, &signature)
                    .map_err(|_| Error::Authentication)
            }
        }
        _ => Err(Error::Authentication),
    }
}

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
    use crate::PublicIdentity;
    #[cfg(feature = "webauthn")]
    use crate::store;
    use coaptic::provisioning::{EnrollmentAnchor, Identity};
    use p256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};

    fn sign_fixture(key: &SigningKey, digest: &[u8]) -> Vec<u8> {
        let signature: Signature = key.sign_prehash(digest).unwrap();
        signature.to_der().as_bytes().to_vec()
    }

    fn fixture() -> (DevelopmentOperator, SigningKey) {
        software_fixture(7, 0x82)
    }

    fn software_fixture(scalar: u8, slot: u8) -> (DevelopmentOperator, SigningKey) {
        let key = SigningKey::from_slice(&[scalar; 32]).unwrap();
        let public = key.verifying_key().to_encoded_point(false);
        let pending = DevelopmentOperator::registration(123, slot, public.as_bytes()).unwrap();
        let proof = sign_fixture(&key, pending.digest());
        (pending.finish(&proof).unwrap(), key)
    }

    fn software_approval(
        operator: &mut DevelopmentOperator,
        key: &SigningKey,
        request: Request,
    ) -> DevelopmentApproval {
        let pending = operator.begin(request).unwrap();
        let signature = sign_fixture(key, pending.digest());
        pending
            .finish(&signature, |old, next| {
                assert_eq!(old.owner().unwrap(), operator.owner().unwrap());
                assert_eq!(old.revision, operator.revision);
                assert_eq!(next.revision, old.revision + 1);
                *operator = next.clone();
                Ok(())
            })
            .unwrap()
    }

    #[test]
    fn software_three_role_backup_composes_proofs_persistence_and_consumption() {
        use coaptic::provisioning::{
            EnrollmentPolicy, LocalPairing, PairingCommitError, PairingError, PairingRecord,
            PairingUpdate,
        };

        fn apply_encoded(
            update: PairingUpdate,
            state: &mut LocalPairing,
            durable: &mut PairingRecord,
            now: u64,
        ) -> Result<(), PairingCommitError<()>> {
            let mut wire = [0; 512];
            let len = update.encode(&mut wire).unwrap();
            PairingUpdate::decode(&wire[..len])
                .unwrap()
                .apply(state, now, |old, next| {
                    assert_eq!(old.commitment(), durable.commitment());
                    let mut record = [0; PairingRecord::MAX_ENCODED_LEN];
                    let len = next.encode(&mut record).unwrap();
                    *durable = PairingRecord::decode(&record[..len]).unwrap();
                    Ok(())
                })
        }

        // All three private signing keys exist only in this software test. This
        // exercises neither live PIV policy/touch nor installed flash power loss.
        let (mut current, current_key) = fixture();
        let (mut backup, backup_key) = software_fixture(8, 0x83);
        let (mut destination, destination_key) = software_fixture(9, 0x84);
        let a = current.owner().unwrap();
        let b = backup.owner().unwrap();
        let c = destination.owner().unwrap();
        assert!(a != b && a != c && b != c);
        let device = Identity::from_private_key([1; 32], 1).unwrap();
        let original_peer = Identity::from_private_key([2; 32], 2).unwrap();
        let replacement_peer = Identity::from_private_key([3; 32], 3).unwrap();
        let mut durable = PairingRecord::unpaired(&device, [4; 32]).unwrap();
        let store = *durable.store();
        let local = *durable.local_principal();
        let mut state = LocalPairing::restore(&device, durable.clone()).unwrap();
        let policy = EnrollmentPolicy::new(original_peer.peer(), a, [5; 32]).unwrap();
        let challenge = state
            .begin(&device, 0, |bytes| {
                bytes.fill(1);
                true
            })
            .unwrap();
        apply_encoded(
            PairingUpdate::new(challenge, policy.clone(), false),
            &mut state,
            &mut durable,
            1,
        )
        .unwrap();

        let challenge = state
            .begin(&device, 2, |bytes| {
                bytes.fill(2);
                true
            })
            .unwrap();
        let configure = Request::for_recovery_configuration(&challenge, &policy, Some(b)).unwrap();
        let confirm = configure.recovery_cosignature().unwrap();
        assert_eq!(configure.action, crate::Action::ConfigureRecovery);
        assert_eq!(confirm.action, crate::Action::ConfirmRecoveryCredential);
        assert!(current.begin(confirm.clone()).is_err());
        assert!(backup.begin(configure.clone()).is_err());
        let approval = software_approval(&mut current, &current_key, configure);
        let cosigner = software_approval(&mut backup, &backup_key, confirm);
        approval
            .commit(|request| {
                let expected = request.recovery_cosignature()?.commitment()?;
                cosigner.commit(|proof| {
                    assert_eq!(proof.commitment()?, expected);
                    request.matches_pairing(&challenge)?;
                    apply_encoded(
                        PairingUpdate::set_recovery(
                            challenge.clone(),
                            request.pairing_policy()?,
                            Some(request.next_owner),
                        )
                        .map_err(|_| Error::Input)?,
                        &mut state,
                        &mut durable,
                        3,
                    )
                    .map_err(|_| Error::Persistence)
                })
            })
            .unwrap();
        state = LocalPairing::restore(&device, durable.clone()).unwrap();
        assert_eq!(state.checkpoint().recovery_owner(), Some(&b));
        assert_eq!(state.checkpoint().policy().unwrap().owner(), &a);

        let challenge = state
            .begin(&device, 4, |bytes| {
                bytes.fill(3);
                true
            })
            .unwrap();
        let next = EnrollmentPolicy::new(replacement_peer.peer(), c, [6; 32]).unwrap();
        let recovery = Request::for_owner_recovery(&challenge, b, &next).unwrap();
        let accept = recovery.recovery_cosignature().unwrap();
        assert_eq!(recovery.action, crate::Action::RecoverOwner);
        assert_eq!(accept.action, crate::Action::AcceptRecoveredOwnership);
        let before = durable.commitment();
        assert!(current.begin(recovery.clone()).is_err());
        assert!(backup.begin(accept.clone()).is_err());
        let pending = backup.begin(recovery.clone()).unwrap();
        let signature = sign_fixture(&backup_key, pending.digest());
        assert!(matches!(
            destination
                .begin(accept.clone())
                .unwrap()
                .finish(&signature, |_, _| panic!("cross-role proof persisted")),
            Err(Error::Authentication)
        ));
        let mut changed = recovery.clone();
        changed.next_owner = a;
        assert!(matches!(
            backup
                .begin(changed)
                .unwrap()
                .finish(&signature, |_, _| panic!(
                    "changed destination proof persisted"
                )),
            Err(Error::Authentication)
        ));
        assert_eq!(state.checkpoint().commitment(), before);
        assert_eq!(durable.commitment(), before);
        assert_eq!(
            (current.revision, backup.revision, destination.revision),
            (1, 1, 0)
        );
        drop(pending);
        let approval = software_approval(&mut backup, &backup_key, recovery);
        let cosigner = software_approval(&mut destination, &destination_key, accept);
        approval
            .commit(|request| {
                let expected = request.recovery_cosignature()?.commitment()?;
                cosigner.commit(|proof| {
                    assert_eq!(proof.commitment()?, expected);
                    request.matches_pairing(&challenge)?;
                    apply_encoded(
                        PairingUpdate::recover(
                            challenge.clone(),
                            request.owner,
                            request.pairing_policy()?,
                        )
                        .map_err(|_| Error::Input)?,
                        &mut state,
                        &mut durable,
                        5,
                    )
                    .map_err(|_| Error::Persistence)
                })
            })
            .unwrap();
        state = LocalPairing::restore(&device, durable.clone()).unwrap();
        assert_eq!(state.checkpoint().store(), &store);
        assert_eq!(state.checkpoint().local_principal(), &local);
        assert_eq!(state.checkpoint().policy().unwrap().owner(), &c);
        assert!(state.permits(&replacement_peer.peer().principal()));
        assert!(!state.permits(&original_peer.peer().principal()));
        assert_eq!(state.checkpoint().recovery_owner(), None);

        // Fresh, valid B and A signatures cannot reuse the consumed B authority.
        let challenge = state
            .begin(&device, 6, |bytes| {
                bytes.fill(4);
                true
            })
            .unwrap();
        let reuse = Request::for_owner_recovery(&challenge, b, &policy).unwrap();
        let approval = software_approval(&mut backup, &backup_key, reuse.clone());
        let cosigner = software_approval(
            &mut current,
            &current_key,
            reuse.recovery_cosignature().unwrap(),
        );
        let completed = durable.commitment();
        approval
            .commit(|request| {
                let expected = request.recovery_cosignature()?.commitment()?;
                cosigner.commit(|proof| {
                    assert_eq!(proof.commitment()?, expected);
                    request.matches_pairing(&challenge)?;
                    assert!(matches!(
                        apply_encoded(
                            PairingUpdate::recover(
                                challenge.clone(),
                                request.owner,
                                request.pairing_policy()?
                            )
                            .map_err(|_| Error::Input)?,
                            &mut state,
                            &mut durable,
                            7,
                        ),
                        Err(PairingCommitError::State(PairingError::Owner))
                    ));
                    Ok(())
                })
            })
            .unwrap();
        assert_eq!(state.checkpoint().commitment(), completed);
        assert_eq!(durable.commitment(), completed);
        assert_eq!(
            (current.revision, backup.revision, destination.revision),
            (2, 3, 1)
        );
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
        let signature = sign_fixture(&key, pending.digest());
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
        let signature = sign_fixture(&key, pending.digest());
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
        let signature = sign_fixture(&key, pending.digest());
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
        let signature = sign_fixture(&key, pending.digest());
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
        #[cfg(feature = "webauthn")]
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
    fn fixture_hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn verifies_openssl_attestation_fixtures_and_refuses_policy_or_signature_changes() {
        use x509_cert::der::{Decode, asn1::BitString};
        // Independently signed with Python cryptography/OpenSSL, using synthetic
        // RSA-2048, P-256 and P-384 issuer keys and the same P-256 subject key.
        for (algorithm, certificate, issuer_spki) in [
            (
                "RSA2048",
                concat!(
                    "308201ef3081d8a003020102020101300d06092a864886f70d01010b050030123110300e06035504030c0752534132303438",
                    "301e170d3230303130313030303030305a170d3430303130313030303030305a301a3118301606035504030c0f436f617074",
                    "696320666978747572653059301306072a8648ce3d020106082a8648ce3d030107034200043ed113b7883b4c590638379db0",
                    "c21cda16742ed0255048bf433391d374bc21d19099209accc4c8a224c843afa4f4c68a090d04da5e9889dae2f8eefce82a37",
                    "40a31430123010060a2b0601040182c40a030804020102300d06092a864886f70d01010b0500038201010072d84ee0c8ccc2",
                    "e59a1a12b8db1346fc53202b2863ae2cb29570077b59b7c2233836500a062da772e6c2adef62bd19132120c223aeacb5b96b",
                    "0e92ce73aa4f470311b214029a963929327013e1d3c494ff98df626657cd8fe6f2dce42be67ecfca354c5bee3e67f4500f67",
                    "6bc294ee05343b9c30336a4a0e627413a43ff970b197e196675308678bffcdb1a1e273cb1d0ccc24ca40c8d3f235a8931fa6",
                    "a9adbf9a6dcaba935452e2f28991cc2940608d91f5a2fd83bad27b769c1196d9e0b200661932ad7bd9f721e43aa9606e33b0",
                    "ec017e3f39735cf07d9983e605283d60745a0463c7e3c148f4c332c4612769760d7e96daf093c4e0347679ff55f482a283",
                ),
                concat!(
                    "30820122300d06092a864886f70d01010105000382010f003082010a0282010100c9578689cde6a6cff3c5114cd986aa349c",
                    "f13e2671bdd4b865a0b76a87f964a7f94945c8636d0e0b8d1666e34db51f784b84ad80bbf410d4a61b381c0cf759ba090f1b",
                    "0c19830c62e007c574384d2f899689826861221bf9a037bcf7001506d8b9112731c3f89326cfc770754695ab38b150d54fd8",
                    "cc07e683076ad3e2946ed269ee7b01a2fb08d6bfc19219b2d7ade886354210c1d4f8afb1ae7841d9cbf7a0e056c8ee4a906c",
                    "822dd7d1e95bf32d7da0165a361522811c29cb8d54a9335436d073e080fa7a91afe4e68594515634458939b8c7c1af208abc",
                    "e9a7bda2a49068d6703c33ce84b47944a8bc38eca69e8defd0c4ab298c4966937609bc998c4be30203010001",
                ),
            ),
            (
                "P256",
                concat!(
                    "3082012b3081d2a003020102020101300a06082a8648ce3d040302300f310d300b06035504030c0450323536301e170d3230",
                    "303130313030303030305a170d3430303130313030303030305a301a3118301606035504030c0f436f617074696320666978",
                    "747572653059301306072a8648ce3d020106082a8648ce3d030107034200043ed113b7883b4c590638379db0c21cda16742e",
                    "d0255048bf433391d374bc21d19099209accc4c8a224c843afa4f4c68a090d04da5e9889dae2f8eefce82a3740a314301230",
                    "10060a2b0601040182c40a030804020102300a06082a8648ce3d0403020348003045022100f1c8e731a61f2a16b76e6f294e",
                    "a1cc2bdf8f6d95f401829f35afc53e1ab1712f0220751f2bc7eff3ba6f515eed6c61727644402bde4208e1d671c063cfe2dd",
                    "ef95fa",
                ),
                concat!(
                    "3059301306072a8648ce3d020106082a8648ce3d03010703420004741dd5bda817d95e4626537320e5d55179983028b2f82c",
                    "99d500c5ee8624e3c40770b46a9c385fdc567383554887b1548eeb912c35ba5ca71995ff22cd4481d3",
                ),
            ),
            (
                "P384",
                concat!(
                    "3082014a3081d2a003020102020101300a06082a8648ce3d040302300f310d300b06035504030c0450333834301e170d3230",
                    "303130313030303030305a170d3430303130313030303030305a301a3118301606035504030c0f436f617074696320666978",
                    "747572653059301306072a8648ce3d020106082a8648ce3d030107034200043ed113b7883b4c590638379db0c21cda16742e",
                    "d0255048bf433391d374bc21d19099209accc4c8a224c843afa4f4c68a090d04da5e9889dae2f8eefce82a3740a314301230",
                    "10060a2b0601040182c40a030804020102300a06082a8648ce3d040302036700306402301d871a2fc6679d63add306f39c76",
                    "093a0752678917857ce242416d0f9d06df0a315c5d4f2f366e9c6fbc8de4d3257c910230073fdd3683dd9d87da5620b8fd70",
                    "d6ba120ad9b227ae15dbad9d57eecad517f6203cba74d23048c13417f71f62be2394",
                ),
                concat!(
                    "3076301006072a8648ce3d020106052b8104002203620004a567ba97b67aea5bafdaf5002ffcc6ab9632bff9f01f873f6267",
                    "bcd1f0f11c139ee5f441abd99f1baaf1ca1e3b5cbce7de1b38b3989f3318644e4147af164ecc5185595046932ec086329be0",
                    "57857d66776bcb8272218a7d6423a12736f429cc",
                ),
            ),
        ] {
            let cert = x509_cert::Certificate::from_der(&fixture_hex(certificate)).unwrap();
            let mut issuer = cert.clone();
            issuer.tbs_certificate.subject_public_key_info =
                x509_cert::spki::SubjectPublicKeyInfoOwned::from_der(&fixture_hex(issuer_spki))
                    .unwrap();
            let public = cert
                .tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_bytes()
                .unwrap();
            assert!(
                verify_attestation(&cert, &issuer, public, true).is_ok(),
                "{algorithm}"
            );
            assert!(verify_attestation(&cert, &issuer, public, false).is_err());
            assert!(verify_attestation(&cert, &issuer, &[4; 65], true).is_err());
            let mut changed = cert.clone();
            let extensions = changed.tbs_certificate.extensions.as_mut().unwrap();
            extensions.push(extensions[0].clone());
            assert!(verify_attestation(&changed, &issuer, public, true).is_err());
            let mut changed = cert.clone();
            let mut signature = changed.signature.as_bytes().unwrap().to_vec();
            *signature.last_mut().unwrap() ^= 1;
            changed.signature = BitString::from_bytes(&signature).unwrap();
            assert!(verify_attestation(&changed, &issuer, public, true).is_err());
            let mut changed = cert.clone();
            changed.signature_algorithm.oid = "1.2.840.113549.1.1.5".parse().unwrap();
            changed.tbs_certificate.signature = changed.signature_algorithm.clone();
            assert!(verify_attestation(&changed, &issuer, public, true).is_err());
        }
    }
    #[test]
    fn accepts_external_prehash_der_signatures_with_either_s_value() {
        // OpenSSL-generated ECDSA over the supplied digest (no second hash).
        let (operator, _) = fixture();
        for encoded in [
            "3044022077b20737a107a64c60dec3af092e573f1afe5b8b0ad7219f1f1ebae78ab47a3602206e57071ba062dec789bd1629ce687c2c8b4b8582b7ab5ec9370c73ce6cd8ceab",
            "3045022077b20737a107a64c60dec3af092e573f1afe5b8b0ad7219f1f1ebae78ab47a3602210091a8f8e35f9d21397642e9d6319783d3319b752aef6c3fbbbcad56f48f8a56a6",
        ] {
            let signature = fixture_hex(encoded);
            operator.verify(&[9; 32], &signature).unwrap();
            assert!(operator.verify(&[8; 32], &signature).is_err());
            let mut trailing = signature;
            trailing.push(0);
            assert!(operator.verify(&[9; 32], &trailing).is_err());
        }
    }
}
