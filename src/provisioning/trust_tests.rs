extern crate std;

use super::super::{Error, Initiator, Responder};
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreError {
    Before,
    LostAcknowledgment,
    Conflict,
}

#[derive(Clone, Copy, Default)]
enum Failure {
    #[default]
    None,
    Before,
    After,
}

struct Store {
    anchor: TrustAnchor,
    record: Option<TrustRecord>,
    failure: Failure,
    attempts: usize,
}

impl Store {
    fn new() -> Self {
        Self {
            anchor: TrustAnchor::genesis([0x42; 32]).unwrap(),
            record: None,
            failure: Failure::None,
            attempts: 0,
        }
    }

    fn commit(
        &mut self,
        expected: &TrustAnchor,
        record: &TrustRecord,
        proposed: &TrustAnchor,
    ) -> Result<(), StoreError> {
        self.attempts += 1;
        if *expected != self.anchor {
            return Err(StoreError::Conflict);
        }
        if matches!(self.failure, Failure::Before) {
            return Err(StoreError::Before);
        }
        assert_eq!(record.anchor(), *proposed);
        assert_eq!(expected.parts().0, proposed.parts().0);
        assert_eq!(expected.parts().1.checked_add(1), Some(proposed.parts().1));
        self.record = Some(record.clone());
        self.anchor = *proposed;
        if matches!(self.failure, Failure::After) {
            return Err(StoreError::LostAcknowledgment);
        }
        Ok(())
    }

    fn install(&mut self, local: &Identity, peer: PinnedPeer) -> PeerTrust {
        PeerTrust::install(local, peer, self.anchor, |expected, record, proposed| {
            self.commit(expected, record, proposed)
        })
        .unwrap()
    }

    fn restore(&self, local: &Identity) -> PeerTrust {
        PeerTrust::restore(local, self.record.as_ref().unwrap().clone(), self.anchor).unwrap()
    }
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |bytes| {
        bytes.fill(0);
        bytes[31] = value;
        true
    }
}

fn session(local: &Identity, peer: &Identity) -> Session {
    sessions(local, peer).0
}

fn sessions(local: &Identity, peer: &Identity) -> (Session, Session) {
    let (initiator, m1) = Initiator::start(local, peer.peer(), entropy(4)).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(peer, local.peer(), &m1, entropy(5)).unwrap();
    let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    let (responder, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
    let initiator = initiator.receive_message_4(&m4, |_| true).unwrap();
    (initiator, responder)
}

#[test]
fn session_handoff_rejects_a_different_local_key_or_credential_identifier() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    for other_local in [identity(3, 0), identity(1, 9)] {
        for ready in [
            session(&other_local, &peer),
            sessions(&peer, &other_local).1,
        ] {
            assert_eq!(*ready.local_principal(), other_local.peer().principal());
            assert_eq!(*ready.principal(), peer.peer().principal());
            assert!(matches!(
                trust.accept_session(&grant, ready),
                Err(TrustError::IdentityMismatch)
            ));
        }
    }
    assert!(trust.accept_session(&grant, session(&local, &peer)).is_ok());
    assert!(
        trust
            .accept_session(&grant, sessions(&peer, &local).1)
            .is_ok()
    );
}

#[test]
fn session_metadata_tracks_complete_local_and_peer_credentials_in_both_roles() {
    let local = identity(1, 7);
    let peer = identity(2, 9);
    let (initiator, responder) = sessions(&local, &peer);
    assert_eq!(*initiator.local_principal(), local.peer().principal());
    assert_eq!(*initiator.principal(), peer.peer().principal());
    assert_eq!(*responder.local_principal(), peer.peer().principal());
    assert_eq!(*responder.principal(), local.peer().principal());
    assert_ne!(initiator.local_principal(), initiator.principal());
    assert_eq!(initiator.local_principal(), responder.principal());
    assert_eq!(responder.local_principal(), initiator.principal());
}

#[test]
fn genesis_requires_nonzero_store_and_initialized_record_cannot_reinstall() {
    assert_eq!(
        TrustAnchor::genesis([0; 32]),
        Err(TrustError::StoreIdentity)
    );
    assert_eq!(
        TrustAnchor::from_parts([1; 32], 0, [1; 32]),
        Err(TrustError::InvalidRecord)
    );
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    store.install(&local, peer.peer());
    assert!(matches!(
        PeerTrust::install::<()>(&local, peer.peer(), store.anchor, |_, _, _| {
            panic!("initialized authority must not be rewritten")
        }),
        Err(TrustCommitError::State(TrustError::Initialized))
    ));
}

#[test]
fn installation_commits_complete_record_before_grant_and_roundtrips_public_parts() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let trust = store.install(&local, peer.peer());
    assert_eq!(store.attempts, 1);
    let parts = trust.checkpoint().parts();
    assert_eq!(parts.version, 1);
    assert_eq!(parts.store_id, [0x42; 32]);
    assert_eq!(parts.generation, 1);
    assert_eq!(
        parts.local_principal,
        *local.peer().principal().fingerprint()
    );
    assert_eq!(parts.peer_public_key, peer.peer().public_key());
    assert_eq!(parts.peer_kid, peer.peer().kid());
    assert!(parts.enabled);
    let record = TrustRecord::from_parts(parts).unwrap();
    assert_eq!(record.parts(), parts);
    assert_eq!(record.anchor(), store.anchor);
    let restored = PeerTrust::restore(&local, record, store.anchor).unwrap();
    let grant = restored.grant(&local).unwrap();
    assert_eq!(grant.anchor(), store.anchor);
    assert!(restored.permits(&grant, &peer.peer().principal()));
    assert!(!restored.permits(&grant, &local.peer().principal()));
}

#[test]
fn compressed_pin_exports_the_same_canonical_public_persistence_fields() {
    let peer = identity(2, 1).peer();
    let point = peer.public_key();
    let mut compressed = [0; 33];
    compressed[0] = 2 | (point[64] & 1);
    compressed[1..].copy_from_slice(&point[1..33]);
    let imported = PinnedPeer::from_public_key(&compressed, peer.kid()).unwrap();
    assert_eq!(imported.public_key(), point);
    assert_eq!(imported.kid(), peer.kid());
    assert_eq!(imported.principal(), peer.principal());
    let local = identity(1, 0);
    let mut first = Store::new();
    let mut second = Store::new();
    let first = first.install(&local, peer);
    let second = second.install(&local, imported);
    assert_eq!(first.checkpoint().parts(), second.checkpoint().parts());
    assert_eq!(first.checkpoint().anchor(), second.checkpoint().anchor());
}

#[test]
fn failed_installation_exposes_no_policy_and_lost_ack_recovers_actual_record() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    for failure in [Failure::Before, Failure::After] {
        let mut store = Store::new();
        let genesis = store.anchor;
        store.failure = failure;
        let result = PeerTrust::install(
            &local,
            peer.peer(),
            genesis,
            |expected, record, proposed| store.commit(expected, record, proposed),
        );
        assert!(matches!(result, Err(TrustCommitError::Persistence(_))));
        if matches!(failure, Failure::Before) {
            assert!(store.record.is_none());
            assert_eq!(store.anchor, genesis);
        } else {
            assert_ne!(store.anchor, genesis);
            let restored = store.restore(&local);
            assert!(restored.grant(&local).is_ok());
            assert!(matches!(
                PeerTrust::install(
                    &local,
                    peer.peer(),
                    genesis,
                    |expected, record, proposed| { store.commit(expected, record, proposed) }
                ),
                Err(TrustCommitError::Persistence(StoreError::Conflict))
            ));
        }
    }
}

#[test]
fn invalid_record_representation_fails_before_becoming_policy() {
    let local = identity(1, 0);
    let mut store = Store::new();
    let trust = store.install(&local, identity(2, 1).peer());
    let base = trust.checkpoint().parts();
    for change in 0..5 {
        let mut parts = base;
        match change {
            0 => parts.version = 0,
            1 => parts.version = 2,
            2 => parts.store_id = [0; 32],
            3 => parts.generation = 0,
            _ => parts.peer_public_key = [0; 65],
        }
        assert!(TrustRecord::from_parts(parts).is_err());
    }
}

#[test]
fn anchor_digest_commits_every_public_field_and_complete_credential() {
    let local = identity(1, 0);
    let mut store = Store::new();
    let trust = store.install(&local, identity(2, 1).peer());
    let base = trust.checkpoint().parts();
    for change in 0..6 {
        let mut parts = base;
        match change {
            0 => parts.store_id[31] ^= 1,
            1 => parts.generation += 1,
            2 => parts.local_principal[31] ^= 1,
            3 => parts.peer_public_key = identity(3, 1).peer().public_key(),
            4 => parts.peer_kid ^= 1,
            _ => parts.enabled = false,
        }
        let record = TrustRecord::from_parts(parts).unwrap();
        assert_ne!(record.anchor().parts().2, store.anchor.parts().2);
        assert!(matches!(
            PeerTrust::restore(&local, record, store.anchor),
            Err(TrustError::AnchorMismatch)
        ));
    }
}

#[test]
fn restore_refuses_stale_uncommitted_newer_cross_store_and_equivocated_records() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let original = trust.checkpoint().clone();
    trust
        .replace(
            &local,
            identity(3, 2).peer(),
            true,
            |expected, record, proposed| store.commit(expected, record, proposed),
        )
        .unwrap();
    assert!(matches!(
        PeerTrust::restore(&local, original, store.anchor),
        Err(TrustError::AnchorMismatch)
    ));
    let base = trust.checkpoint().parts();
    for change in 0..3 {
        let mut parts = base;
        match change {
            0 => parts.generation += 1,
            1 => parts.store_id[0] ^= 1,
            _ => parts.peer_kid ^= 1,
        }
        assert!(matches!(
            PeerTrust::restore(
                &local,
                TrustRecord::from_parts(parts).unwrap(),
                store.anchor
            ),
            Err(TrustError::AnchorMismatch)
        ));
    }
    assert!(matches!(
        PeerTrust::restore(
            &local,
            trust.checkpoint().clone(),
            TrustAnchor::genesis([0x42; 32]).unwrap()
        ),
        Err(TrustError::AnchorMismatch)
    ));
}

#[test]
fn local_full_credential_binding_refuses_changed_key_and_same_key_different_kid() {
    let local = identity(1, 0);
    let mut store = Store::new();
    let trust = store.install(&local, identity(2, 1).peer());
    for other in [identity(3, 0), identity(1, 2)] {
        assert_eq!(trust.grant(&other), Err(TrustError::IdentityMismatch));
        assert!(matches!(
            PeerTrust::restore(&other, trust.checkpoint().clone(), store.anchor),
            Err(TrustError::IdentityMismatch)
        ));
    }
}

#[test]
fn self_pin_install_and_replace_refuse_before_storage_and_do_not_disable_valid_grant() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    assert!(matches!(
        PeerTrust::install::<()>(&local, local.peer(), store.anchor, |_, _, _| {
            panic!("self pin must not be committed")
        }),
        Err(TrustCommitError::State(TrustError::SelfPeer))
    ));
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    assert!(matches!(
        trust.replace::<()>(&local, local.peer(), true, |_, _, _| {
            panic!("self pin replacement must not be committed")
        }),
        Err(TrustCommitError::State(TrustError::SelfPeer))
    ));
    assert!(!trust.is_blocked());
    assert!(trust.permits(&grant, &peer.peer().principal()));
}

#[test]
fn failed_write_blocks_previous_authorization_until_verified_restore() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let original = trust.checkpoint().parts();
    store.failure = Failure::Before;
    assert!(matches!(
        trust.revoke(|expected, record, proposed| store.commit(expected, record, proposed)),
        Err(TrustCommitError::Persistence(StoreError::Before))
    ));
    assert!(trust.is_blocked());
    assert_eq!(trust.checkpoint().parts(), original);
    assert_eq!(trust.grant(&local), Err(TrustError::Blocked));
    assert!(!trust.permits(&grant, &peer.peer().principal()));
    assert!(matches!(
        trust.revoke::<()>(|_, _, _| panic!("blocked policy must not retry blindly")),
        Err(TrustCommitError::State(TrustError::Blocked))
    ));
    let restored = store.restore(&local);
    assert!(restored.permits(&grant, &peer.peer().principal()));
}

#[test]
fn lost_acknowledgment_restores_revocation_and_cannot_reopen_stale_grant() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let stale = trust.checkpoint().clone();
    store.failure = Failure::After;
    assert!(matches!(
        trust.revoke(|expected, record, proposed| store.commit(expected, record, proposed)),
        Err(TrustCommitError::Persistence(
            StoreError::LostAcknowledgment
        ))
    ));
    assert!(!trust.permits(&grant, &peer.peer().principal()));
    assert!(matches!(
        PeerTrust::restore(&local, stale, store.anchor),
        Err(TrustError::AnchorMismatch)
    ));
    let restored = store.restore(&local);
    assert!(!restored.checkpoint().enabled());
    assert_eq!(restored.grant(&local), Err(TrustError::Unauthorized));
    assert!(!restored.permits(&grant, &peer.peer().principal()));
}

#[test]
fn stale_replica_cas_conflict_blocks_losing_policy_and_preserves_winner() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut winner = store.install(&local, peer.peer());
    let mut loser = store.restore(&local);
    let old_grant = loser.grant(&local).unwrap();
    winner
        .revoke(|expected, record, proposed| store.commit(expected, record, proposed))
        .unwrap();
    let winning_anchor = store.anchor;
    assert!(matches!(
        loser.replace(
            &local,
            identity(3, 2).peer(),
            true,
            |expected, record, proposed| { store.commit(expected, record, proposed) }
        ),
        Err(TrustCommitError::Persistence(StoreError::Conflict))
    ));
    assert_eq!(store.anchor, winning_anchor);
    assert!(!loser.permits(&old_grant, &peer.peer().principal()));
    assert!(loser.is_blocked());
    assert_eq!(
        store.restore(&local).grant(&local),
        Err(TrustError::Unauthorized)
    );
}

#[test]
fn revocation_tombstone_regrant_same_credential_refuses_aba_grant() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let old = trust.grant(&local).unwrap();
    trust
        .revoke(|expected, record, proposed| store.commit(expected, record, proposed))
        .unwrap();
    assert!(!trust.checkpoint().enabled());
    assert_eq!(
        trust.checkpoint().peer().principal(),
        peer.peer().principal()
    );
    assert_eq!(
        store.restore(&local).grant(&local),
        Err(TrustError::Unauthorized)
    );
    trust
        .replace(&local, peer.peer(), true, |expected, record, proposed| {
            store.commit(expected, record, proposed)
        })
        .unwrap();
    let fresh = trust.grant(&local).unwrap();
    assert_eq!(old.principal(), fresh.principal());
    assert_ne!(old.anchor(), fresh.anchor());
    assert!(!trust.permits(&old, &peer.peer().principal()));
    assert!(trust.permits(&fresh, &peer.peer().principal()));
    assert!(matches!(
        trust.accept_session(&old, session(&local, &peer)),
        Err(TrustError::StaleGrant)
    ));
}

#[test]
fn peer_rotation_and_unchanged_replacement_invalidate_prior_grants() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let replacement = identity(3, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let old = trust.grant(&local).unwrap();
    trust
        .replace(
            &local,
            replacement.peer(),
            true,
            |expected, record, proposed| store.commit(expected, record, proposed),
        )
        .unwrap();
    let rotated = trust.grant(&local).unwrap();
    assert!(!trust.permits(&old, &peer.peer().principal()));
    assert!(!trust.permits(&rotated, &peer.peer().principal()));
    assert!(trust.permits(&rotated, &replacement.peer().principal()));
    assert!(
        trust
            .accept_session(&rotated, session(&local, &replacement))
            .is_ok()
    );
    trust
        .replace(
            &local,
            replacement.peer(),
            true,
            |expected, record, proposed| store.commit(expected, record, proposed),
        )
        .unwrap();
    assert!(!trust.permits(&rotated, &replacement.peer().principal()));
}

#[test]
fn local_identity_rotation_binds_replacement_and_invalidates_old_grant() {
    let local = identity(1, 0);
    let replacement = identity(3, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let old = trust.grant(&local).unwrap();
    trust
        .replace(
            &replacement,
            peer.peer(),
            true,
            |expected, record, proposed| store.commit(expected, record, proposed),
        )
        .unwrap();
    assert_eq!(trust.grant(&local), Err(TrustError::IdentityMismatch));
    assert!(trust.grant(&replacement).is_ok());
    assert!(!trust.permits(&old, &peer.peer().principal()));
    assert!(matches!(
        PeerTrust::restore(&local, trust.checkpoint().clone(), store.anchor),
        Err(TrustError::IdentityMismatch)
    ));
    assert!(store.restore(&replacement).grant(&replacement).is_ok());
    let fresh = trust.grant(&replacement).unwrap();
    assert!(matches!(
        trust.accept_session(&fresh, session(&local, &peer)),
        Err(TrustError::IdentityMismatch)
    ));
    assert!(
        trust
            .accept_session(&fresh, session(&replacement, &peer))
            .is_ok()
    );
}

#[test]
fn generation_exhaustion_refuses_before_callback_without_weakening_authorization() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let trust = store.install(&local, peer.peer());
    let mut parts = trust.checkpoint().parts();
    parts.generation = u64::MAX;
    let record = TrustRecord::from_parts(parts).unwrap();
    let anchor = record.anchor();
    let mut trust = PeerTrust::restore(&local, record, anchor).unwrap();
    let grant = trust.grant(&local).unwrap();
    assert!(matches!(
        trust.revoke::<()>(|_, _, _| panic!("generation overflow must not persist")),
        Err(TrustCommitError::State(TrustError::GenerationExhausted))
    ));
    assert!(matches!(
        trust.replace::<()>(&local, peer.peer(), true, |_, _, _| {
            panic!("generation overflow must not persist")
        }),
        Err(TrustCommitError::State(TrustError::GenerationExhausted))
    ));
    assert!(!trust.is_blocked());
    assert!(trust.permits(&grant, &peer.peer().principal()));
}

#[test]
fn revocation_between_message2_and_message4_refuses_confirmation() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let (initiator, m1) = Initiator::start(&local, peer.peer(), entropy(4)).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(&peer, local.peer(), &m1, entropy(5)).unwrap();
    let (initiator, m3) = initiator
        .receive_message_2(&m2, |principal| trust.permits(&grant, principal))
        .unwrap();
    let (_, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
    trust
        .revoke(|expected, record, proposed| store.commit(expected, record, proposed))
        .unwrap();
    assert!(matches!(
        initiator.receive_message_4(&m4, |principal| trust.permits(&grant, principal)),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn responder_revocation_before_message3_refuses_session_and_final_confirmation() {
    let local = identity(2, 1);
    let peer = identity(1, 0);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let (initiator, m1) = Initiator::start(&peer, local.peer(), entropy(4)).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(&local, peer.peer(), &m1, entropy(5)).unwrap();
    let (_, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    trust
        .revoke(|expected, record, proposed| store.commit(expected, record, proposed))
        .unwrap();
    assert!(matches!(
        responder.receive_message_3(&m3, |principal| trust.permits(&grant, principal)),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn revocation_after_message4_before_handoff_refuses_ready_session() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let ready = session(&local, &peer);
    trust
        .revoke(|expected, record, proposed| store.commit(expected, record, proposed))
        .unwrap();
    assert!(matches!(
        trust.accept_session(&grant, ready),
        Err(TrustError::Unauthorized)
    ));
}

#[test]
fn session_handoff_compares_complete_authenticated_peer_principal() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let other = identity(3, 1);
    let mut store = Store::new();
    let trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    assert!(matches!(
        trust.accept_session(&grant, session(&local, &other)),
        Err(TrustError::Unauthorized)
    ));
    assert!(trust.accept_session(&grant, session(&local, &peer)).is_ok());
}

#[test]
fn panic_during_commit_also_leaves_policy_blocked() {
    let local = identity(1, 0);
    let peer = identity(2, 1);
    let mut store = Store::new();
    let mut trust = store.install(&local, peer.peer());
    let grant = trust.grant(&local).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<TrustAnchor, TrustCommitError<()>> =
            trust.revoke(|_, _, _| panic!("ambiguous backend outcome"));
    }));
    assert!(result.is_err());
    assert!(trust.is_blocked());
    assert!(!trust.permits(&grant, &peer.peer().principal()));
}

#[test]
fn fixed_policy_footprint_is_bounded() {
    assert!(core::mem::size_of::<PeerTrust>() <= 768);
    assert!(core::mem::size_of::<TrustRecord>() <= 640);
    assert!(core::mem::size_of::<TrustRecordParts>() <= 160);
    assert!(core::mem::size_of::<TrustGrant>() <= 112);
    assert!(core::mem::size_of::<TrustAnchor>() <= 80);
    std::println!(
        "trust fixed bytes: policy={} record={} parts={} grant={} anchor={}",
        core::mem::size_of::<PeerTrust>(),
        core::mem::size_of::<TrustRecord>(),
        core::mem::size_of::<TrustRecordParts>(),
        core::mem::size_of::<TrustGrant>(),
        core::mem::size_of::<TrustAnchor>(),
    );
}
