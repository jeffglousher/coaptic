extern crate std;

use super::*;
use std::cell::RefCell;

fn identity(value: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, value).unwrap()
}

fn policy(peer: u8, owner: u8) -> EnrollmentPolicy {
    EnrollmentPolicy::new(identity(peer).peer(), [owner; 32], [owner + 1; 32]).unwrap()
}

fn entropy(bytes: &mut [u8]) -> bool {
    bytes.fill(7);
    true
}

fn commit(_: &PairingRecord, _: &PairingRecord) -> Result<(), u8> {
    Ok(())
}

fn paired() -> (Identity, LocalPairing) {
    let device = identity(1);
    let mut state =
        LocalPairing::restore(&device, PairingRecord::unpaired(&device, [9; 32]).unwrap()).unwrap();
    let challenge = state.begin(&device, 0, entropy).unwrap();
    state.pair(&challenge, policy(2, 4), 1, commit).unwrap();
    (device, state)
}

#[test]
fn local_transfer_is_explicit_current_owner_bound_and_survives_restart() {
    for revoked in [false, true] {
        let (device, mut state) = paired();
        if revoked {
            let challenge = state.begin(&device, 2, entropy).unwrap();
            state.revoke(&challenge, &[4; 32], 3, commit).unwrap();
        }
        let before = state.checkpoint().clone();
        let challenge = state.begin(&device, 10, entropy).unwrap();
        let update = PairingUpdate::transfer(challenge.clone(), [4; 32], policy(3, 5)).unwrap();
        assert!(!update.is_revocation());
        assert_eq!(update.current_owner(), Some(&[4; 32]));
        let mut bytes = [0; 384];
        let len = update.encode(&mut bytes).unwrap();
        assert_eq!(&bytes[..3], &[0x88, 1, 2]);
        let update = PairingUpdate::decode(&bytes[..len]).unwrap();
        update
            .apply(&mut state, 11, |old, next| {
                assert_eq!(old.commitment(), before.commitment());
                assert_eq!(next.revision(), old.revision() + 1);
                assert_eq!(next.local_principal(), old.local_principal());
                assert_eq!(next.store(), old.store());
                commit(old, next)
            })
            .unwrap();
        assert!(!state.permits(&identity(2).peer().principal()));
        assert!(state.permits(&identity(3).peer().principal()));
        let len = state.checkpoint().encode(&mut bytes).unwrap();
        let mut restored =
            LocalPairing::restore(&device, PairingRecord::decode(&bytes[..len]).unwrap()).unwrap();
        assert_eq!(restored.checkpoint().policy().unwrap().owner(), &[5; 32]);
        assert_eq!(
            restored.checkpoint().policy().unwrap().resource_policy(),
            &[6; 32]
        );
        assert_eq!(restored.checkpoint().revision(), before.revision() + 1);
        assert!(
            update
                .apply(&mut restored, 12, |_, _| -> Result<(), u8> {
                    panic!("restored state has no live challenge")
                })
                .is_err()
        );
        let fresh = restored.begin(&device, 20, entropy).unwrap();
        assert!(matches!(
            restored.pair(&fresh, policy(2, 4), 21, commit),
            Err(PairingCommitError::State(PairingError::Owner))
        ));
        let fresh = restored.begin(&device, 22, entropy).unwrap();
        assert!(matches!(
            restored.transfer(&fresh, &[4; 32], policy(2, 6), 23, commit),
            Err(PairingCommitError::State(PairingError::Owner))
        ));
        let fresh = restored.begin(&device, 24, entropy).unwrap();
        restored
            .transfer(&fresh, &[5; 32], policy(2, 6), 25, commit)
            .unwrap();
        assert!(restored.permits(&identity(2).peer().principal()));
    }
}

#[test]
fn local_transfer_refuses_wrong_owner_challenge_self_peer_and_exhaustion_before_storage() {
    for mutation in 0..10 {
        let (device, mut state) = paired();
        if mutation == 9 {
            state.record.revision = u64::MAX;
        }
        let before = state.record.commitment();
        let mut challenge = state.begin(&device, 10, entropy).unwrap();
        let mut destination = policy(3, 5);
        let mut owner = [4; 32];
        let mut now = 11;
        match mutation {
            0 => owner = [5; 32],
            1 => destination = policy(3, 4),
            2 => challenge.store[0] ^= 1,
            3 => challenge.digest[0] ^= 1,
            4 => challenge.revision += 1,
            5 => challenge.device = identity(6).peer(),
            6 => challenge.nonce[0] ^= 1,
            7 => now = 60_010,
            8 => destination = policy(1, 5),
            _ => {}
        }
        assert!(
            state
                .transfer(
                    &challenge,
                    &owner,
                    destination,
                    now,
                    |_, _| -> Result<(), u8> { panic!("invalid transfer reached storage") }
                )
                .is_err(),
            "mutation {mutation}"
        );
        assert_eq!(state.record.commitment(), before);
        assert!(state.permits(&identity(2).peer().principal()));
        assert!(
            state
                .transfer(&challenge, &[4; 32], policy(3, 5), 11, commit)
                .is_err()
        );
    }
    let device = identity(1);
    let mut state =
        LocalPairing::restore(&device, PairingRecord::unpaired(&device, [9; 32]).unwrap()).unwrap();
    let challenge = state.begin(&device, 10, entropy).unwrap();
    assert!(matches!(
        state.transfer(&challenge, &[4; 32], policy(3, 5), 11, commit),
        Err(PairingCommitError::State(PairingError::Owner))
    ));
}

#[test]
fn local_transfer_uncertain_commit_blocks_use_until_complete_record_recovery() {
    for fault in 0..3 {
        let (device, mut state) = paired();
        let disk = RefCell::new(state.record.clone());
        let challenge = state.begin(&device, 10, entropy).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.transfer(&challenge, &[4; 32], policy(3, 5), 11, |_, next| {
                if fault != 0 {
                    *disk.borrow_mut() = next.clone();
                }
                if fault == 2 {
                    panic!("uncertain publication");
                }
                Err::<(), _>(7)
            })
        }));
        if fault == 2 {
            assert!(result.is_err());
        } else {
            assert!(matches!(result.unwrap(), Err(PairingCommitError::Store(7))));
        }
        assert!(!state.permits(&identity(2).peer().principal()));
        assert!(!state.permits(&identity(3).peer().principal()));
        assert!(state.begin(&device, 12, entropy).is_err());
        let recovered = LocalPairing::restore(&device, disk.into_inner()).unwrap();
        assert_eq!(
            recovered.permits(&identity(2).peer().principal()),
            fault == 0
        );
        assert_eq!(
            recovered.permits(&identity(3).peer().principal()),
            fault != 0
        );
        assert_eq!(
            recovered.record.policy().unwrap().owner(),
            &[if fault == 0 { 4 } else { 5 }; 32]
        );
    }
}

#[test]
fn local_transfer_cbor_is_bounded_and_rejects_ambiguous_actions_or_truncation() {
    let (device, mut state) = paired();
    state.record.revision = u64::MAX - 1;
    let challenge = state.begin(&device, 10, entropy).unwrap();
    assert!(PairingUpdate::transfer(challenge.clone(), [0; 32], policy(3, 5)).is_err());
    assert!(PairingUpdate::transfer(challenge.clone(), [5; 32], policy(3, 5)).is_err());
    let update = PairingUpdate::transfer(challenge, [4; 32], policy(255, 5)).unwrap();
    let mut bytes = [0; 385];
    let len = update.encode(&mut bytes).unwrap();
    assert!(len <= 384);
    for end in 0..len {
        assert!(PairingUpdate::decode(&bytes[..end]).is_err());
    }
    bytes[len] = 0;
    assert!(PairingUpdate::decode(&bytes[..len + 1]).is_err());
    for (array, action) in [(0x87, 2), (0x88, 0), (0x88, 1), (0x88, 3)] {
        bytes[0] = array;
        bytes[2] = action;
        assert!(PairingUpdate::decode(&bytes[..len]).is_err());
    }
    bytes[0] = 0x88;
    bytes[2] = 2;
    let update = PairingUpdate::decode(&bytes[..len]).unwrap();
    let request = super::super::CommissioningRequest::Update {
        update,
        endpoint: crate::Endpoint::v4([192, 0, 2, 1], 5683),
    };
    let mut frame = [0; 512];
    let len = request.encode(&mut frame).unwrap();
    assert!(matches!(
        super::super::CommissioningRequest::decode(&frame[..len]),
        Ok(super::super::CommissioningRequest::Update { .. })
    ));
}
