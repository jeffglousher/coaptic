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
    Request::new(
        &identity(1).peer(),
        &identity(2).peer(),
        [3; 32],
        [4; 32],
        authority.checkpoint().anchor(),
        [5; 32],
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
