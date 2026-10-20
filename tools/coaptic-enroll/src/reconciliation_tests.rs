use super::*;
use coaptic::provisioning::{
    Identity, LocalPairing, OperationId, PairingChallenge, PairingRecord, Receipt,
};

fn fixture() -> (LocalPairing, PairingRecord, PairingChallenge, Receipt) {
    let device = Identity::from_private_key([1; 32], 1).unwrap();
    let host = Identity::from_private_key([2; 32], 2).unwrap();
    let mut state =
        LocalPairing::restore(&device, PairingRecord::unpaired(&device, [3; 32]).unwrap()).unwrap();
    let entropy = |bytes: &mut [u8]| {
        bytes.fill(9);
        true
    };
    let challenge = state.begin(&device, 0, entropy).unwrap();
    state
        .pair(
            &challenge,
            EnrollmentPolicy::new(host.peer(), [4; 32], pairing::permissions(true)).unwrap(),
            1,
            |_, _| Ok::<_, ()>(()),
        )
        .unwrap();
    let prior = state.checkpoint().clone();
    let challenge = state.begin(&device, 2, entropy).unwrap();
    state
        .revoke(&challenge, &[4; 32], 3, |_, _| Ok::<_, ()>(()))
        .unwrap();
    let challenge = state.begin(&device, 4, entropy).unwrap();
    (
        state,
        prior,
        challenge,
        Receipt::from_parts(OperationId::new([5; 16]), [6; 32], 1).unwrap(),
    )
}

#[test]
fn reconciliation_approval_binds_every_receipt_byte_and_prior_commitment() {
    let (state, prior, challenge, receipt) = fixture();
    let request =
        Request::for_receipt_reconciliation(&challenge, state.checkpoint(), &prior, receipt)
            .unwrap();
    let commitment = request.commitment().unwrap();
    request
        .matches_receipt_reconciliation(&challenge, &prior, receipt)
        .unwrap();
    for position in 0..56 {
        let mut changed = request.clone();
        changed.reconciliation.as_mut().unwrap().receipt[position] ^= 1;
        assert!(
            changed
                .matches_receipt_reconciliation(&challenge, &prior, receipt)
                .is_err()
        );
        if let Ok(value) = changed.commitment() {
            assert_ne!(value, commitment);
        }
    }
    for position in 0..32 {
        let mut changed = request.clone();
        changed.reconciliation.as_mut().unwrap().prior_commitment[position] ^= 1;
        assert!(
            changed
                .matches_receipt_reconciliation(&challenge, &prior, receipt)
                .is_err()
        );
        assert_ne!(changed.commitment().unwrap(), commitment);
    }
    let encoded = store::encode(&request).unwrap();
    let restored: Request = store::decode(&encoded).unwrap();
    assert_eq!(restored.commitment().unwrap(), commitment);
    assert_eq!(restored.reconciliation.unwrap().receipt().unwrap(), receipt);
}

#[test]
fn receipt_reconciliation_cannot_be_approved_as_a_grant_or_use_a_changed_challenge() {
    let (state, prior, challenge, receipt) = fixture();
    let request =
        Request::for_receipt_reconciliation(&challenge, state.checkpoint(), &prior, receipt)
            .unwrap();
    for action in [Action::Pair, Action::Revoke, Action::Transfer] {
        let mut changed = request.clone();
        changed.action = action;
        assert!(changed.commitment().is_err());
    }
    let mut changed = request.clone();
    changed.reconciliation = None;
    assert!(changed.commitment().is_err());
    let mut changed = request.clone();
    changed.nonce[0] ^= 1;
    assert!(
        changed
            .matches_receipt_reconciliation(&challenge, &prior, receipt)
            .is_err()
    );
    for field in [0, 1, 2] {
        let mut changed = request.clone();
        match field {
            0 => changed.owner[0] ^= 1,
            1 => changed.permissions[0] ^= 1,
            _ => {
                changed.peer = PublicIdentity::from_peer(
                    &Identity::from_private_key([8; 32], 8).unwrap().peer(),
                )
            }
        }
        assert!(
            changed
                .matches_receipt_reconciliation(&challenge, &prior, receipt)
                .is_err()
        );
    }
    assert!(Request::for_receipt_reconciliation(&challenge, &prior, &prior, receipt).is_err());
    let mut called = false;
    let mut local = state;
    assert!(
        pairing::apply(&request, &challenge, &mut local, 5, |_, _| {
            called = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!called);
    assert!(!local.checkpoint().enabled());
}

#[test]
fn public_checkpoint_is_immutable_and_invalid_archives_never_get_replaced() {
    let (_, prior, _, _) = fixture();
    let directory = std::env::temp_dir().join(format!(
        "coaptic-checkpoint-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = pairing::save_device_checkpoint(&directory, &prior).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert_eq!(
        pairing::read_device_checkpoint(&path).unwrap().commitment(),
        prior.commitment()
    );
    assert_eq!(
        pairing::save_device_checkpoint(&directory, &prior).unwrap(),
        path
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    for invalid in [
        vec![0; PairingRecord::MAX_ENCODED_LEN + 1],
        [original.as_slice(), &[0]].concat(),
        b"invalid".to_vec(),
    ] {
        std::fs::write(&path, &invalid).unwrap();
        assert!(pairing::read_device_checkpoint(&path).is_err());
        assert!(pairing::save_device_checkpoint(&directory, &prior).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), invalid);
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
