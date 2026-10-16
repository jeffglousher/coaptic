use super::*;
use coaptic::provisioning::{EnrollmentRecord, EnrollmentStore, Identity, TrustAnchor};
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

#[derive(Default)]
struct MemoryStore {
    latest: Option<EnrollmentAnchor>,
}
impl EnrollmentStore for MemoryStore {
    type Error = ();
    fn current(&mut self) -> std::result::Result<EnrollmentAnchor, ()> {
        self.latest.ok_or(())
    }
    fn commit(
        &mut self,
        old: Option<&EnrollmentAnchor>,
        next: &EnrollmentRecord,
        anchor: &EnrollmentAnchor,
    ) -> std::result::Result<(), ()> {
        if old.copied() != self.latest || next.anchor() != *anchor {
            return Err(());
        }
        self.latest = Some(*anchor);
        Ok(())
    }
}

fn identity(value: u8) -> Identity {
    let mut secret = [0; 32];
    secret[31] = value;
    Identity::from_private_key(secret, value).unwrap()
}

fn authority() -> EnrollmentAuthority<MemoryStore> {
    EnrollmentAuthority::register(
        identity(1).peer(),
        TrustAnchor::genesis([7; 32]).unwrap(),
        MemoryStore::default(),
    )
    .unwrap()
}

fn request(authority: &EnrollmentAuthority<MemoryStore>) -> Request {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).unwrap();
    Request::new(
        &identity(1).peer(),
        &identity(2).peer(),
        [3; 32],
        [4; 32],
        authority.checkpoint().anchor(),
        nonce,
    )
    .unwrap()
}

fn operator() -> (Operator, WebauthnAuthenticator<SoftPasskey>) {
    // This explicitly simulates UV for protocol mutation tests, never hardware proof.
    let mut client = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let (challenge, pending) = begin_registration([3; 32]).unwrap();
    let proof = client.do_registration(origin(), challenge).unwrap();
    (pending.finish(&proof).unwrap(), client)
}

fn transfer_request() -> (
    coaptic::provisioning::LocalPairing,
    coaptic::provisioning::PairingChallenge,
    Request,
) {
    use coaptic::provisioning::{LocalPairing, PairingRecord};
    let device = identity(1);
    let mut state =
        LocalPairing::restore(&device, PairingRecord::unpaired(&device, [9; 32]).unwrap()).unwrap();
    let first = state
        .begin(&device, 0, |out| getrandom::fill(out).is_ok())
        .unwrap();
    state
        .pair(
            &first,
            EnrollmentPolicy::new(identity(2).peer(), [3; 32], [4; 32]).unwrap(),
            1,
            |_, _| Ok::<(), ()>(()),
        )
        .unwrap();
    let challenge = state
        .begin(&device, 2, |out| getrandom::fill(out).is_ok())
        .unwrap();
    let policy = EnrollmentPolicy::new(identity(4).peer(), [8; 32], [9; 32]).unwrap();
    let request = Request::for_transfer(&challenge, [3; 32], &policy).unwrap();
    (state, challenge, request)
}

#[test]
fn transfer_approval_authenticates_current_owner_and_commits_only_the_new_policy() {
    let (mut state, challenge, request) = transfer_request();
    let (operator, mut client) = operator();
    assert_eq!(request.owner, *operator.owner());
    assert_eq!(request.pairing_policy().unwrap().owner(), &[8; 32]);
    let (proof_challenge, pending) = operator.begin(request).unwrap();
    let proof = client.do_authentication(origin(), proof_challenge).unwrap();
    let approval = pending.finish(&proof, |_, _| Ok(())).unwrap();
    approval
        .commit_local(|request| {
            request.matches_pairing(&challenge)?;
            assert_eq!(request.action, Action::Transfer);
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
    assert_eq!(state.checkpoint().policy().unwrap().owner(), &[8; 32]);
    assert!(!state.permits(&identity(2).peer().principal()));
    assert!(state.permits(&identity(4).peer().principal()));
}

#[test]
fn transfer_destination_and_action_are_bound_and_legacy_pair_requests_stay_unchanged() {
    let (_, challenge, original) = transfer_request();
    let (operator, mut client) = operator();
    for field in 0..3 {
        let mut changed = original.clone();
        match field {
            0 => changed.next_owner = [7; 32],
            1 => changed.peer = PublicIdentity::from_peer(&identity(5).peer()),
            _ => changed.permissions = [6; 32],
        }
        assert_ne!(
            original.commitment().unwrap(),
            changed.commitment().unwrap()
        );
        let (a, _) = operator.begin(original.clone()).unwrap();
        let (_, b) = operator.begin(changed).unwrap();
        let proof = client.do_authentication(origin(), a).unwrap();
        assert!(matches!(
            b.finish(&proof, |_, _| panic!("changed transfer persisted")),
            Err(Error::Authentication)
        ));
    }
    for field in 0..4 {
        let mut invalid = original.clone();
        match field {
            0 => invalid.next_owner = [0; 32],
            1 => invalid.next_owner = invalid.owner,
            2 => invalid.action = Action::Pair,
            _ => invalid.owner = [2; 32],
        }
        assert!(operator.begin(invalid).is_err());
    }
    assert!(
        Request::for_pairing(
            &challenge,
            &original.pairing_policy().unwrap(),
            Action::Transfer
        )
        .is_err()
    );
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&original, &mut bytes).unwrap();
    let round_trip: Request = ciborium::de::from_reader(&bytes[..]).unwrap();
    assert_eq!(
        round_trip.commitment().unwrap(),
        original.commitment().unwrap()
    );
    let legacy = request(&authority());
    bytes.clear();
    ciborium::ser::into_writer(&legacy, &mut bytes).unwrap();
    assert!(
        !bytes
            .windows(b"next_owner".len())
            .any(|part| part == b"next_owner")
    );
    let decoded: Request = ciborium::de::from_reader(&bytes[..]).unwrap();
    assert_eq!(decoded.next_owner, [0; 32]);
    assert_eq!(decoded.commitment().unwrap(), legacy.commitment().unwrap());
}

#[test]
fn registration_profile_and_restored_counter_are_checked_before_prompting() {
    let (operator, _) = operator();
    for field in 0..3 {
        let mut credential: Credential = operator.key.clone().into();
        match field {
            0 => credential.user_verified = false,
            1 => credential.backup_eligible = true,
            _ => credential.backup_state = true,
        }
        assert!(matches!(
            credential_counter(&credential.into()),
            Err(Error::Authentication)
        ));
    }
    let mut mismatched = operator.clone();
    mismatched.counter += 1;
    assert!(matches!(
        mismatched.begin(request(&authority())),
        Err(Error::Input)
    ));
}

#[test]
fn valid_proof_commits_only_the_bound_device_policy() {
    let mut authority = authority();
    let (operator, mut client) = operator();
    let (challenge, pending) = operator.begin(request(&authority)).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    let mut persisted = false;
    let approval = pending
        .finish(&proof, |old, next| {
            assert_eq!(old.owner(), next.owner());
            persisted = true;
            Ok(())
        })
        .unwrap();
    assert!(persisted);
    let next = approval.approve(&mut authority).unwrap();
    assert_eq!(next, authority.checkpoint().anchor());
    assert!(authority.checkpoint().parts().approved);
}

#[test]
fn proof_for_another_transaction_cannot_be_rebound() {
    let authority = authority();
    let (operator, mut client) = operator();
    let original = request(&authority);
    let mut changed = original.clone();
    changed.permissions = [9; 32];
    assert_ne!(
        original.commitment().unwrap(),
        changed.commitment().unwrap()
    );
    let (challenge, _) = operator.begin(original).unwrap();
    let (_, pending) = operator.begin(changed).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    assert!(matches!(
        pending.finish(&proof, |_, _| panic!("wrong challenge reached persistence")),
        Err(Error::Authentication)
    ));
}

#[test]
fn invalid_signature_is_refused_before_persistence() {
    let (operator, mut client) = operator();
    let (challenge, pending) = operator.begin(request(&authority())).unwrap();
    let mut proof = client.do_authentication(origin(), challenge).unwrap();
    proof.response.signature = vec![0; 64].into();
    assert!(matches!(
        pending.finish(&proof, |_, _| panic!("invalid signature reached storage")),
        Err(Error::Authentication)
    ));
}

#[test]
fn persistence_failure_never_exposes_a_usable_approval() {
    let authority = authority();
    let (operator, mut client) = operator();
    let (challenge, pending) = operator.begin(request(&authority)).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    assert!(matches!(
        pending.finish(&proof, |_, _| Err(())),
        Err(Error::Persistence)
    ));
    let (challenge, pending) = operator.begin(request(&authority)).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    assert!(pending.finish(&proof, |_, _| Ok(())).is_ok());
}

#[test]
fn competing_management_change_invalidates_the_approved_request() {
    let mut authority = authority();
    let (operator, mut client) = operator();
    let (challenge, pending) = operator.begin(request(&authority)).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    let approval = pending.finish(&proof, |_, _| Ok(())).unwrap();
    authority.revoke(authority.checkpoint().anchor()).unwrap();
    assert!(matches!(
        approval.approve(&mut authority),
        Err(CommitError::Authority(_))
    ));
    assert!(!authority.checkpoint().parts().approved);
}

#[test]
fn different_device_and_expired_approval_cannot_change_trust() {
    for expired in [false, true] {
        let mut authority = authority();
        let (operator, mut client) = operator();
        let mut request = request(&authority);
        if !expired {
            request.device = PublicIdentity::from_peer(&identity(6).peer());
        }
        let (challenge, pending) = operator.begin(request).unwrap();
        let proof = client.do_authentication(origin(), challenge).unwrap();
        let mut approval = pending.finish(&proof, |_, _| Ok(())).unwrap();
        if expired {
            approval.started = Instant::now().checked_sub(TIMEOUT).unwrap();
        }
        assert!(matches!(
            approval.approve(&mut authority),
            Err(CommitError::Approval(_))
        ));
        assert!(!authority.checkpoint().parts().approved);
    }
}

#[test]
fn wrong_owner_and_expired_challenge_are_refused() {
    let authority = authority();
    let (operator, mut client) = operator();
    let mut wrong_owner = request(&authority);
    wrong_owner.owner = [8; 32];
    assert!(matches!(operator.begin(wrong_owner), Err(Error::Input)));
    let (challenge, mut pending) = operator.begin(request(&authority)).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    pending.started = Instant::now().checked_sub(TIMEOUT).unwrap();
    assert!(matches!(
        pending.finish(&proof, |_, _| panic!("expired challenge reached storage")),
        Err(Error::Expired)
    ));
}

#[test]
fn cbor_records_are_bounded_and_reject_trailing_bytes() {
    let request = request(&authority());
    let mut bytes = store::encode(&request).unwrap();
    let restored: Request = store::decode(&bytes).unwrap();
    assert_eq!(
        restored.commitment().unwrap(),
        request.commitment().unwrap()
    );
    bytes.push(0);
    assert!(store::decode::<Request>(&bytes).is_err());
    assert!(store::decode::<Request>(&vec![0; 16385]).is_err());
    let mut nested = vec![0x81; 40];
    nested.push(0xf6);
    assert!(store::decode::<ciborium::value::Value>(&nested).is_err());
}

#[test]
fn operator_file_recovery_preserves_counter_and_excludes_competing_writer() {
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let directory =
        std::env::temp_dir().join(format!("coaptic-enroll-{}", Uuid::from_bytes(random)));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for name in ["operator.cbor", "operator.lock"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let (operator, mut client) = operator();
    let store = store::OperatorStore::open(&directory).unwrap();
    assert!(store.load().is_err());
    store.initialize(&operator).unwrap();
    assert!(store.initialize(&operator).is_err());
    assert!(store::OperatorStore::open(&directory).is_err());
    let (challenge, pending) = store.load().unwrap().begin(request(&authority())).unwrap();
    let proof = client.do_authentication(origin(), challenge).unwrap();
    pending
        .finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))
        .unwrap();
    let committed = store.load().unwrap();
    assert!(committed.counter > operator.counter);
    assert!(store.commit(&operator, &committed).is_err());
    drop(store);
    let recovered = store::OperatorStore::open(&directory).unwrap();
    assert_eq!(
        store::encode(&recovered.load().unwrap()).unwrap(),
        store::encode(&committed).unwrap()
    );
    // A consumed assertion cannot authorize another fresh challenge after restart.
    let (_, pending) = recovered
        .load()
        .unwrap()
        .begin(request(&authority()))
        .unwrap();
    assert!(matches!(
        pending.finish(&proof, |_, _| panic!("replayed proof reached storage")),
        Err(Error::Authentication)
    ));
    std::fs::write(directory.join("operator.cbor"), [0xff]).unwrap();
    assert!(recovered.load().is_err());
    assert!(recovered.initialize(&operator).is_err());
}
