use super::*;
use coaptic::provisioning::Identity;

const OLD: [u8; 32] = [3; 32];
const NEW: [u8; 32] = [8; 32];

fn identity(value: u8) -> Identity {
    let mut key = [0; 32];
    key[31] = value;
    Identity::from_private_key(key, value).unwrap()
}

fn entropy(out: &mut [u8]) -> bool {
    getrandom::fill(out).is_ok()
}

fn unpaired(identity: &Identity, store: u8) -> LocalPairing {
    LocalPairing::restore(
        identity,
        PairingRecord::unpaired(identity, [store; 32]).unwrap(),
    )
    .unwrap()
}

fn transferred() -> (Identity, Identity, LocalPairing, LocalPairing) {
    let device = identity(1);
    let host = identity(4);
    let mut remote = unpaired(&device, 7);
    let challenge = remote.begin(&device, 0, entropy).unwrap();
    remote
        .pair(
            &challenge,
            EnrollmentPolicy::new(identity(2).peer(), OLD, [5; 32]).unwrap(),
            1,
            |_, _| Ok::<_, Error>(()),
        )
        .unwrap();
    let challenge = remote.begin(&device, 2, entropy).unwrap();
    remote
        .transfer(
            &challenge,
            &OLD,
            EnrollmentPolicy::new(host.peer(), NEW, [9; 32]).unwrap(),
            3,
            |_, _| Ok::<_, Error>(()),
        )
        .unwrap();
    let local = unpaired(&host, 6);
    (device, host, remote, local)
}

#[cfg(feature = "webauthn-tests")]
#[test]
fn lost_device_reply_recovers_only_the_host_with_fresh_new_owner_approval() {
    use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
    // The device's write survived; its reply and all in-memory transaction state did not.
    let (device, host, record, mut local) = {
        let (device, host, remote, local) = transferred();
        (device, host, remote.checkpoint().clone(), local)
    };
    let remote = LocalPairing::restore(&device, record).unwrap();
    let before = remote.checkpoint().commitment();
    let recovery = PeerRecovery::new(
        device.peer(),
        remote.checkpoint().clone(),
        local.checkpoint(),
    )
    .unwrap();
    let challenge = local.begin(&host, 0, entropy).unwrap();
    assert!(recovery.request(&challenge, OLD).is_err());
    let request = recovery.request(&challenge, NEW).unwrap();
    let mut client = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let (registration, pending) = crate::begin_registration(NEW).unwrap();
    let proof = client
        .do_registration(crate::origin(), registration)
        .unwrap();
    let operator = pending.finish(&proof).unwrap();
    let (prompt, pending) = operator.begin(request).unwrap();
    let proof = client.do_authentication(crate::origin(), prompt).unwrap();
    let approval = pending.finish(&proof, |_, _| Ok(())).unwrap();
    let mut durable = local.checkpoint().clone();
    approval
        .commit_local(|request| {
            recovery.commit(
                request,
                &challenge,
                (&device.peer(), remote.checkpoint()),
                &mut local,
                1,
                |old, next| {
                    assert_eq!(old.commitment(), durable.commitment());
                    durable = next.clone();
                    Ok(())
                },
            )
        })
        .unwrap();
    assert_eq!(remote.checkpoint().commitment(), before);
    let restored = LocalPairing::restore(&host, durable).unwrap();
    assert_eq!(restored.checkpoint().revision(), 2);
    assert!(restored.permits(&device.peer().principal()));
    assert_eq!(restored.checkpoint().policy().unwrap().owner(), &NEW);
    assert_eq!(
        restored.checkpoint().policy().unwrap().resource_policy(),
        &[9; 32]
    );
    assert!(
        PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            restored.checkpoint()
        )
        .is_err()
    );
}

#[test]
fn recovery_refuses_uncommitted_revoked_or_differently_bound_device_state() {
    let (device, host, mut remote, local) = transferred();
    let absent = PairingRecord::unpaired(&device, [1; 32]).unwrap();
    assert!(PeerRecovery::new(device.peer(), absent, local.checkpoint()).is_err());
    assert!(
        PeerRecovery::new(
            identity(5).peer(),
            remote.checkpoint().clone(),
            local.checkpoint()
        )
        .is_err()
    );
    assert!(
        PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            unpaired(&identity(5), 6).checkpoint()
        )
        .is_err()
    );
    let challenge = remote.begin(&device, 4, entropy).unwrap();
    remote
        .revoke(&challenge, &NEW, 5, |_, _| Ok::<_, Error>(()))
        .unwrap();
    assert!(
        PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            local.checkpoint()
        )
        .is_err()
    );
    let challenge = remote.begin(&device, 6, entropy).unwrap();
    remote
        .pair(
            &challenge,
            EnrollmentPolicy::new(host.peer(), NEW, [9; 32]).unwrap(),
            7,
            |_, _| Ok::<_, Error>(()),
        )
        .unwrap();
    let mut claimed_host = unpaired(&host, 6);
    let challenge = claimed_host.begin(&host, 0, entropy).unwrap();
    claimed_host
        .pair(
            &challenge,
            EnrollmentPolicy::new(device.peer(), OLD, [9; 32]).unwrap(),
            1,
            |_, _| Ok::<_, Error>(()),
        )
        .unwrap();
    let challenge = claimed_host.begin(&host, 2, entropy).unwrap();
    claimed_host
        .revoke(&challenge, &OLD, 3, |_, _| Ok::<_, Error>(()))
        .unwrap();
    assert!(
        PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            claimed_host.checkpoint()
        )
        .is_err()
    );
}

#[test]
fn changed_snapshot_request_or_host_refuses_before_storage() {
    for field in 0..15 {
        let (device, host, mut remote, mut local) = transferred();
        let recovery = PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            local.checkpoint(),
        )
        .unwrap();
        let challenge = local.begin(&host, 0, entropy).unwrap();
        let mut request = recovery.request(&challenge, NEW).unwrap();
        let mut observed_device = device.peer();
        match field {
            0 => request.action = Action::Revoke,
            1 => request.owner = OLD,
            2 => request.permissions = [10; 32],
            3 => request.peer = crate::PublicIdentity::from_peer(&identity(5).peer()),
            4 => request.nonce[0] ^= 1,
            5 => request.revision += 1,
            6 => request.record_digest[0] ^= 1,
            7 => request.store[0] ^= 1,
            8 => request.device = crate::PublicIdentity::from_peer(&identity(5).peer()),
            9 => request.next_owner = OLD,
            10 => observed_device = identity(5).peer(),
            11..=13 => {
                let challenge = remote.begin(&device, 4, entropy).unwrap();
                if field == 11 {
                    remote
                        .revoke(&challenge, &NEW, 5, |_, _| Ok::<_, Error>(()))
                        .unwrap();
                } else {
                    // Both a changed policy and a newer identical policy invalidate the snapshot.
                    let permissions = if field == 12 { [10; 32] } else { [9; 32] };
                    remote
                        .pair(
                            &challenge,
                            EnrollmentPolicy::new(host.peer(), NEW, permissions).unwrap(),
                            5,
                            |_, _| Ok::<_, Error>(()),
                        )
                        .unwrap();
                }
            }
            _ => {
                local
                    .pair(
                        &challenge,
                        EnrollmentPolicy::new(device.peer(), NEW, [9; 32]).unwrap(),
                        1,
                        |_, _| Ok::<_, Error>(()),
                    )
                    .unwrap();
            }
        }
        let before = local.checkpoint().commitment();
        assert!(
            recovery
                .commit(
                    &request,
                    &challenge,
                    (&observed_device, remote.checkpoint()),
                    &mut local,
                    2,
                    |_, _| panic!("changed recovery reached storage")
                )
                .is_err(),
            "field {field}"
        );
        assert_eq!(local.checkpoint().commitment(), before);
    }
}

#[test]
fn uncertain_host_write_blocks_until_verified_restore_without_repeating_device_update() {
    for write_completed in [false, true] {
        let (device, host, remote, mut local) = transferred();
        let remote_before = remote.checkpoint().commitment();
        let recovery = PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            local.checkpoint(),
        )
        .unwrap();
        let challenge = local.begin(&host, 0, entropy).unwrap();
        let request = recovery.request(&challenge, NEW).unwrap();
        let mut durable = local.checkpoint().clone();
        assert!(
            recovery
                .commit(
                    &request,
                    &challenge,
                    (&device.peer(), remote.checkpoint()),
                    &mut local,
                    1,
                    |_, next| {
                        if write_completed {
                            durable = next.clone();
                        }
                        Err(Error::Persistence)
                    }
                )
                .is_err()
        );
        assert!(!local.permits(&device.peer().principal()));
        assert!(local.begin(&host, 2, entropy).is_err());
        let restored = LocalPairing::restore(&host, durable).unwrap();
        assert_eq!(
            restored.permits(&device.peer().principal()),
            write_completed
        );
        assert_eq!(remote.checkpoint().commitment(), remote_before);
        if !write_completed {
            let recovery = PeerRecovery::new(
                device.peer(),
                remote.checkpoint().clone(),
                restored.checkpoint(),
            )
            .unwrap();
            let mut restored = restored;
            let fresh = restored.begin(&host, 3, entropy).unwrap();
            assert!(
                recovery
                    .commit(
                        &request,
                        &fresh,
                        (&device.peer(), remote.checkpoint()),
                        &mut restored,
                        4,
                        |_, _| panic!("old approval reused")
                    )
                    .is_err()
            );
            let fresh_request = recovery.request(&fresh, NEW).unwrap();
            recovery
                .commit(
                    &fresh_request,
                    &fresh,
                    (&device.peer(), remote.checkpoint()),
                    &mut restored,
                    4,
                    |_, _| Ok(()),
                )
                .unwrap();
        }
    }
}

#[test]
fn expired_recovery_approval_cannot_write_the_host() {
    let (device, host, remote, mut local) = transferred();
    let recovery = PeerRecovery::new(
        device.peer(),
        remote.checkpoint().clone(),
        local.checkpoint(),
    )
    .unwrap();
    let challenge = local.begin(&host, 0, entropy).unwrap();
    let request = recovery.request(&challenge, NEW).unwrap();
    assert!(
        recovery
            .commit(
                &request,
                &challenge,
                (&device.peer(), remote.checkpoint()),
                &mut local,
                60_000,
                |_, _| panic!("expired recovery reached storage")
            )
            .is_err()
    );
    assert!(local.checkpoint().policy().is_none());
}

#[cfg(windows)]
#[test]
fn recovered_host_survives_sealed_store_reopen() {
    use crate::pairing::PairingFile;
    let (device, host, remote, _) = transferred();
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "coaptic-peer-recovery-{:032x}",
        u128::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&directory).unwrap();
    {
        let store = PairingFile::open(&directory).unwrap();
        store.initialize(&host).unwrap();
        let mut local = store.restore(&host).unwrap();
        let recovery = PeerRecovery::new(
            device.peer(),
            remote.checkpoint().clone(),
            local.checkpoint(),
        )
        .unwrap();
        let challenge = local.begin(&host, 0, entropy).unwrap();
        let request = recovery.request(&challenge, NEW).unwrap();
        recovery
            .commit(
                &request,
                &challenge,
                (&device.peer(), remote.checkpoint()),
                &mut local,
                1,
                |old, next| store.commit(old, next),
            )
            .unwrap();
    }
    {
        let store = PairingFile::open(&directory).unwrap();
        let restored = store.restore(&host).unwrap();
        assert!(restored.permits(&device.peer().principal()));
        assert_eq!(restored.checkpoint().policy().unwrap().owner(), &NEW);
        assert!(
            PeerRecovery::new(
                device.peer(),
                remote.checkpoint().clone(),
                restored.checkpoint()
            )
            .is_err()
        );
    }
    std::fs::remove_file(directory.join("pairing.cbor")).unwrap();
    std::fs::remove_file(directory.join("pairing.lock")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
