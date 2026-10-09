extern crate std;

use super::super::{Initiator, Responder};
use super::*;

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |bytes| {
        bytes.fill(0);
        bytes[bytes.len() - 1] = value;
        true
    }
}

fn sessions(device: &Identity, authority: &Identity) -> (Session, Session) {
    let (initiator, m1) = Initiator::start(device, authority.peer(), entropy(4)).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(authority, device.peer(), &m1, entropy(5)).unwrap();
    let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    let (server, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
    let client = initiator.receive_message_4(&m4, |_| true).unwrap();
    (client, server)
}

fn server_endpoint() -> Endpoint {
    Endpoint::v4([192, 0, 2, 1], 5683)
}
fn device_endpoint() -> Endpoint {
    Endpoint::v4([192, 0, 2, 2], 5683)
}
fn genesis() -> TrustAnchor {
    TrustAnchor::genesis([9; 32]).unwrap()
}

fn policy(application: &Identity) -> EnrollmentPolicy {
    EnrollmentPolicy::new(application.peer(), [7; 32], [8; 32]).unwrap()
}

fn snapshot(
    device: &Identity,
    application: &Identity,
    generation: u64,
    enabled: bool,
) -> PolicySnapshot {
    let record = TrustRecord::from_parts(TrustRecordParts {
        version: 1,
        store_id: genesis().parts().0,
        generation,
        local_principal: *device.peer().principal().fingerprint(),
        peer_public_key: application.peer().public_key(),
        peer_kid: application.peer().kid(),
        enabled,
    })
    .unwrap();
    PolicySnapshot::new(record, [7; 32], [8; 32]).unwrap()
}

fn channels(device: &Identity, authority: &Identity) -> (AuthorityConnection, AuthorityResponder) {
    let (client, server) = sessions(device, authority);
    (
        AuthorityConnection::from_session(
            device,
            &BootstrapAuthority::new(authority.peer()),
            server_endpoint(),
            client,
        )
        .unwrap(),
        AuthorityResponder::from_session(authority, device_endpoint(), server).unwrap(),
    )
}

fn pending(
    device: &Identity,
    authority: &Identity,
    query: PolicyQuery,
    nonce: u8,
) -> (PendingPolicy, AuthorityResponder) {
    let (client, server) = channels(device, authority);
    (
        client
            .begin(query, MessageId::new(17), 0, 1000, entropy(nonce))
            .unwrap(),
        server,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreError {
    Refused,
    Unavailable,
    Ambiguous,
}

struct Fixture {
    snapshot: PolicySnapshot,
    calls: usize,
    error: Option<StoreError>,
}

impl OnlinePolicyAuthority for Fixture {
    type Error = StoreError;
    fn resolve(
        &mut self,
        request: &AuthenticatedPolicyRequest,
    ) -> Result<PolicySnapshot, StoreError> {
        self.calls += 1;
        assert_eq!(
            request.principal().fingerprint(),
            self.snapshot.record.local_principal()
        );
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(self.snapshot.clone())
    }
}

fn fixture(device: &Identity, application: &Identity, generation: u64, enabled: bool) -> Fixture {
    Fixture {
        snapshot: snapshot(device, application, generation, enabled),
        calls: 0,
        error: None,
    }
}

fn malicious_response(
    server: AuthorityResponder,
    pending: &PendingPolicy,
    payload: &[u8],
) -> std::vec::Vec<u8> {
    let outer = decode(pending.datagram(0).unwrap().1).unwrap();
    let mut context = server.channel.context;
    let mut scratch = [0; DATAGRAM_LEN];
    let (request, reference) = context.unprotect_request(&outer, &mut scratch).unwrap();
    let format = ContentFormat::OCTET_STREAM.encode();
    let options = [Opt::content_format(&format)];
    let message = Message::new(Type::Acknowledgement, Code::CONTENT, request.message_id())
        .with_token(request.token())
        .with_options(&options)
        .with_payload(payload);
    let mut bytes = [0; DATAGRAM_LEN];
    let len = context
        .protect_response(&message, reference, &mut bytes)
        .unwrap();
    bytes[..len].to_vec()
}

#[test]
fn authentic_authority_enrolls_exact_device_and_commissioning_policy() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let application = identity(3, 3);
    let (pending, server) = pending(
        &device,
        &authority,
        PolicyQuery::Enroll {
            expected: genesis(),
            policy: policy(&application),
        },
        6,
    );
    let mut store = fixture(&device, &application, 1, true);
    let reply = server
        .serve(
            device_endpoint(),
            pending.datagram(0).unwrap().1,
            &mut store,
        )
        .unwrap();
    let verified = pending
        .accept(&device, server_endpoint(), 0, reply.datagram().1)
        .unwrap();
    assert_eq!(store.calls, 1);
    assert_eq!(verified.snapshot().owner(), &[7; 32]);
    assert_eq!(verified.snapshot().resource_policy(), &[8; 32]);
    let mut mirrored = None;
    let trust = verified
        .persist_and_restore(&device, |snapshot| {
            mirrored = Some((snapshot.record.parts(), snapshot.record.anchor()));
            Ok::<_, StoreError>(())
        })
        .unwrap();
    assert_eq!(
        mirrored.unwrap().0.local_principal,
        *device.peer().principal().fingerprint()
    );
    assert_eq!(
        trust.grant(&device).unwrap().principal(),
        &application.peer().principal()
    );
}

#[test]
fn bootstrap_pin_and_actual_local_session_identity_are_both_required() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let attacker = identity(3, 3);
    let (client, _) = sessions(&device, &attacker);
    assert!(matches!(
        AuthorityConnection::from_session(
            &device,
            &BootstrapAuthority::new(authority.peer()),
            server_endpoint(),
            client
        ),
        Err(EnrollmentError::Authority)
    ));
    let (client, _) = sessions(&attacker, &authority);
    assert!(matches!(
        AuthorityConnection::from_session(
            &device,
            &BootstrapAuthority::new(authority.peer()),
            server_endpoint(),
            client
        ),
        Err(EnrollmentError::LocalIdentity)
    ));
    let (_, server) = sessions(&device, &authority);
    assert!(matches!(
        AuthorityResponder::from_session(&attacker, device_endpoint(), server),
        Err(EnrollmentError::LocalIdentity)
    ));
}

#[test]
fn entropy_failures_and_initialized_enrollment_produce_no_request() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    for succeeds in [false, true] {
        let (client, _) = channels(&device, &authority);
        assert!(matches!(
            client.begin(
                PolicyQuery::recover_initial_commit(genesis()).unwrap(),
                MessageId::new(1),
                0,
                1000,
                |bytes| {
                    bytes.fill(0);
                    succeeds
                }
            ),
            Err(EnrollmentError::Entropy)
        ));
    }
    let (client, _) = channels(&device, &authority);
    assert!(matches!(
        client.begin(
            PolicyQuery::Enroll {
                expected: snapshot(&device, &authority, 1, true).record.anchor(),
                policy: policy(&authority)
            },
            MessageId::new(1),
            0,
            1000,
            entropy(6)
        ),
        Err(EnrollmentError::Trust(TrustError::Initialized))
    ));
}

#[test]
fn refresh_after_uncertain_first_commit_accepts_actual_current_record() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let (pending, server) = pending(
        &device,
        &authority,
        PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        6,
    );
    let mut store = fixture(&device, &authority, 3, true);
    let reply = server
        .serve(
            device_endpoint(),
            pending.datagram(0).unwrap().1,
            &mut store,
        )
        .unwrap();
    let verified = pending
        .accept(&device, server_endpoint(), 0, reply.datagram().1)
        .unwrap();
    assert_eq!(verified.snapshot().record().anchor().parts().1, 3);
}

#[test]
fn revoked_tombstone_restores_but_never_grants_application_admission() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let (pending, server) = pending(
        &device,
        &authority,
        PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        6,
    );
    let mut store = fixture(&device, &authority, 4, false);
    let reply = server
        .serve(
            device_endpoint(),
            pending.datagram(0).unwrap().1,
            &mut store,
        )
        .unwrap();
    let trust = pending
        .accept(&device, server_endpoint(), 0, reply.datagram().1)
        .unwrap()
        .persist_and_restore(&device, |_| Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(trust.grant(&device), Err(TrustError::Unauthorized));
}

#[test]
fn older_and_same_generation_changed_records_are_refused() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let application = identity(3, 3);
    let expected = snapshot(&device, &authority, 4, true).record.anchor();
    for current in [
        snapshot(&device, &authority, 3, true),
        snapshot(&device, &application, 4, true),
    ] {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::Refresh {
                expected,
                policy_commitment: snapshot(&device, &authority, 4, true).commitment(),
            },
            6,
        );
        let payload = encode_response(pending.nonce, pending.digest, &current);
        let reply = malicious_response(server, &pending, &payload);
        assert!(matches!(
            pending.accept(&device, server_endpoint(), 0, &reply),
            Err(EnrollmentError::Rollback)
        ));
    }
}

#[test]
fn equal_generation_requires_the_exact_anchor_and_newer_generation_allows_rotation() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let application = identity(3, 3);
    let original = snapshot(&device, &authority, 4, true);
    for current in [original.clone(), snapshot(&device, &application, 5, true)] {
        let (pending, server) = pending(&device, &authority, PolicyQuery::refresh(&original), 6);
        let payload = encode_response(pending.nonce, pending.digest, &current);
        let reply = malicious_response(server, &pending, &payload);
        assert!(
            pending
                .accept(&device, server_endpoint(), 0, &reply)
                .is_ok()
        );
    }
}

#[test]
fn foreign_device_store_owner_permissions_and_pin_are_refused() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let other = identity(3, 3);
    for field in 0..5 {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::Enroll {
                expected: genesis(),
                policy: policy(&authority),
            },
            6,
        );
        let mut current = snapshot(&device, &authority, 1, true);
        let mut parts = current.record.parts();
        match field {
            0 => parts.local_principal = *other.peer().principal().fingerprint(),
            1 => parts.store_id = [22; 32],
            2 => current.owner = [22; 32],
            3 => current.resource_policy = [22; 32],
            4 => {
                parts.peer_public_key = other.peer().public_key();
                parts.peer_kid = other.peer().kid();
            }
            _ => unreachable!(),
        }
        current.record = TrustRecord::from_parts(parts).unwrap();
        let payload = encode_response(pending.nonce, pending.digest, &current);
        let reply = malicious_response(server, &pending, &payload);
        assert!(matches!(
            pending.accept(&device, server_endpoint(), 0, &reply),
            Err(EnrollmentError::Binding)
        ));
    }
}

#[test]
fn self_peer_policy_cannot_produce_usable_trust() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let same_key_other_kid = identity(1, 8);
    let (pending, server) = pending(
        &device,
        &authority,
        PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        6,
    );
    let payload = encode_response(
        pending.nonce,
        pending.digest,
        &snapshot(&device, &same_key_other_kid, 1, true),
    );
    let reply = malicious_response(server, &pending, &payload);
    assert!(matches!(
        pending.accept(&device, server_endpoint(), 0, &reply),
        Err(EnrollmentError::Trust(TrustError::SelfPeer))
    ));
}

#[test]
fn response_nonce_digest_and_trailing_fields_are_checked_after_authentication() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    for field in 0..4 {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            6,
        );
        let mut payload = encode_response(
            pending.nonce,
            pending.digest,
            &snapshot(&device, &authority, 1, true),
        )
        .to_vec();
        match field {
            0 => payload[4] ^= 1,
            1 => payload[36] ^= 1,
            2 => payload.push(0),
            3 => {
                payload.pop();
            }
            _ => unreachable!(),
        }
        let reply = malicious_response(server, &pending, &payload);
        assert!(matches!(
            pending.accept(&device, server_endpoint(), 0, &reply),
            Err(EnrollmentError::Binding | EnrollmentError::Message)
        ));
    }
}

#[test]
fn replay_of_previous_session_reply_fails_even_with_repeated_challenge() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let (old, server) = pending(
        &device,
        &authority,
        PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        6,
    );
    let mut store = fixture(&device, &authority, 1, true);
    let reply = server
        .serve(device_endpoint(), old.datagram(0).unwrap().1, &mut store)
        .unwrap();
    let old_bytes = reply.datagram().1.to_vec();
    assert!(
        old.accept(&device, server_endpoint(), 0, &old_bytes)
            .is_ok()
    );
    let (client, _) = sessions_with_entropy(&device, &authority, 8, 9);
    let fresh = AuthorityConnection::from_session(
        &device,
        &BootstrapAuthority::new(authority.peer()),
        server_endpoint(),
        client,
    )
    .unwrap()
    .begin(
        PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        MessageId::new(17),
        0,
        1000,
        entropy(6),
    )
    .unwrap();
    assert!(matches!(
        fresh.accept(&device, server_endpoint(), 0, &old_bytes),
        Err(EnrollmentError::Oscore(_))
    ));
}

fn sessions_with_entropy(
    device: &Identity,
    authority: &Identity,
    first: u8,
    second: u8,
) -> (Session, Session) {
    let (initiator, m1) = Initiator::start(device, authority.peer(), entropy(first)).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(authority, device.peer(), &m1, entropy(second)).unwrap();
    let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    let (server, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
    (initiator.receive_message_4(&m4, |_| true).unwrap(), server)
}

#[test]
fn plaintext_ciphertext_corruption_wrong_endpoint_and_wrong_identity_fail_closed() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let other = identity(3, 3);
    for field in 0..4 {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            6,
        );
        let mut store = fixture(&device, &authority, 1, true);
        let reply = server
            .serve(
                device_endpoint(),
                pending.datagram(0).unwrap().1,
                &mut store,
            )
            .unwrap();
        let mut bytes = reply.datagram().1.to_vec();
        let endpoint = if field == 2 {
            device_endpoint()
        } else {
            server_endpoint()
        };
        if field == 0 {
            bytes = std::vec![0x68, 0x45, 0, 17, 0, 0, 0, 0, 0, 0, 0, 6];
        }
        if field == 1 {
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
        }
        let identity = if field == 3 { &other } else { &device };
        assert!(pending.accept(identity, endpoint, 0, &bytes).is_err());
    }
}

#[test]
fn response_token_message_id_and_type_cannot_redirect_admission() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    for field in 0..3 {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            6,
        );
        let mut store = fixture(&device, &authority, 1, true);
        let reply = server
            .serve(
                device_endpoint(),
                pending.datagram(0).unwrap().1,
                &mut store,
            )
            .unwrap();
        let mut bytes = reply.datagram().1.to_vec();
        match field {
            0 => bytes[4] ^= 1,
            1 => bytes[3] ^= 1,
            2 => bytes[0] ^= 0x10,
            _ => unreachable!(),
        }
        assert!(matches!(
            pending.accept(&device, server_endpoint(), 0, &bytes),
            Err(EnrollmentError::Message)
        ));
    }
}

#[test]
fn provider_failure_or_ambiguous_commit_returns_no_protected_policy() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    for error in [
        StoreError::Refused,
        StoreError::Unavailable,
        StoreError::Ambiguous,
    ] {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            6,
        );
        let mut store = fixture(&device, &authority, 1, true);
        store.error = Some(error);
        assert!(
            matches!(server.serve(device_endpoint(), pending.datagram(0).unwrap().1, &mut store),
            Err(PolicyServeError::Provider(result)) if result == error)
        );
        assert_eq!(store.calls, 1);
    }
}

#[test]
fn mirror_failure_or_wrong_identity_never_returns_usable_trust() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let other = identity(3, 3);
    for wrong_identity in [false, true] {
        let (pending, server) = pending(
            &device,
            &authority,
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            6,
        );
        let mut store = fixture(&device, &authority, 1, true);
        let reply = server
            .serve(
                device_endpoint(),
                pending.datagram(0).unwrap().1,
                &mut store,
            )
            .unwrap();
        let verified = pending
            .accept(&device, server_endpoint(), 0, reply.datagram().1)
            .unwrap();
        let mut attempts = 0;
        let result =
            verified.persist_and_restore(if wrong_identity { &other } else { &device }, |_| {
                attempts += 1;
                Err(StoreError::Ambiguous)
            });
        assert!(matches!(
            result,
            Err(TrustCommitError::Persistence(StoreError::Ambiguous)
                | TrustCommitError::State(TrustError::IdentityMismatch))
        ));
        assert_eq!(attempts, usize::from(!wrong_identity));
    }
}

#[test]
fn malformed_or_cross_principal_request_never_reaches_authority_provider() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let other = identity(3, 3);
    for field in 0..3 {
        let (mut client, server) = channels(&device, &authority);
        let principal = if field == 0 {
            other.peer().principal()
        } else {
            device.peer().principal()
        };
        let mut payload = encode_request(
            principal,
            &PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            [6; 32],
        )
        .to_vec();
        if field == 1 {
            payload.push(0);
        }
        if field == 2 {
            payload[141] = 1;
        }
        let format = ContentFormat::OCTET_STREAM.encode();
        let options = [Opt::uri_path(POLICY_PATH), Opt::content_format(&format)];
        let message = Message::con(Code::POST, MessageId::new(1), Token::from_checked(&[1]))
            .with_options(&options)
            .with_payload(&payload);
        let mut bytes = [0; DATAGRAM_LEN];
        let len = client
            .context
            .protect_request(&message, &mut bytes)
            .unwrap();
        let mut store = fixture(&device, &authority, 1, true);
        assert!(matches!(
            server.serve(device_endpoint(), &bytes[..len], &mut store),
            Err(PolicyServeError::Exchange(_))
        ));
        assert_eq!(store.calls, 0);
    }
}

#[test]
fn invalid_server_policy_is_refused_before_response_encryption() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let (pending, server) = pending(
        &device,
        &authority,
        PolicyQuery::Enroll {
            expected: genesis(),
            policy: policy(&authority),
        },
        6,
    );
    let mut store = fixture(&device, &authority, 1, true);
    store.snapshot.owner = [99; 32];
    assert!(matches!(
        server.serve(
            device_endpoint(),
            pending.datagram(0).unwrap().1,
            &mut store
        ),
        Err(PolicyServeError::Exchange(EnrollmentError::Binding))
    ));
}

#[test]
fn same_generation_owner_or_resource_changes_are_rollback_refusals() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let original = snapshot(&device, &authority, 4, true);
    for owner_changes in [true, false] {
        let (pending, server) = pending(&device, &authority, PolicyQuery::refresh(&original), 6);
        let mut changed = original.clone();
        if owner_changes {
            changed.owner = [99; 32];
        } else {
            changed.resource_policy = [99; 32];
        }
        let payload = encode_response(pending.nonce, pending.digest, &changed);
        let reply = malicious_response(server, &pending, &payload);
        assert!(matches!(
            pending.accept(&device, server_endpoint(), 0, &reply),
            Err(EnrollmentError::Rollback)
        ));
    }
}

#[test]
fn refresh_commitment_is_bound_and_required_outside_initial_commit_recovery() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    let original = snapshot(&device, &authority, 4, true);
    for query in [
        PolicyQuery::Refresh {
            expected: genesis(),
            policy_commitment: [6; 32],
        },
        PolicyQuery::Refresh {
            expected: original.record.anchor(),
            policy_commitment: [0; 32],
        },
    ] {
        let (client, _) = channels(&device, &authority);
        assert!(matches!(
            client.begin(query, MessageId::new(1), 0, 1000, entropy(6)),
            Err(EnrollmentError::Binding)
        ));
    }
    assert!(PolicyQuery::recover_initial_commit(original.record.anchor()).is_err());
    let first = encode_request(
        device.peer().principal(),
        &PolicyQuery::refresh(&original),
        [6; 32],
    );
    let mut changed = original.clone();
    changed.resource_policy = [99; 32];
    let second = encode_request(
        device.peer().principal(),
        &PolicyQuery::refresh(&changed),
        [6; 32],
    );
    assert_ne!(
        blake3::derive_key(REQUEST_DOMAIN, &first),
        blake3::derive_key(REQUEST_DOMAIN, &second)
    );
}

#[test]
fn authority_deadlines_bound_delayed_responses_and_backwards_clock_input() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    for (now, deadline) in [(5, 5), (5, 4), (0, 30_001), (u64::MAX, 0)] {
        let (client, _) = channels(&device, &authority);
        assert!(matches!(
            client.begin(
                PolicyQuery::recover_initial_commit(genesis()).unwrap(),
                MessageId::new(1),
                now,
                deadline,
                entropy(6)
            ),
            Err(EnrollmentError::Deadline)
        ));
    }
    for accepted_at in [9, 20, 21] {
        let (client, server) = channels(&device, &authority);
        let pending = client
            .begin(
                PolicyQuery::recover_initial_commit(genesis()).unwrap(),
                MessageId::new(1),
                10,
                20,
                entropy(6),
            )
            .unwrap();
        let mut store = fixture(&device, &authority, 1, true);
        let reply = server
            .serve(
                device_endpoint(),
                pending.datagram(10).unwrap().1,
                &mut store,
            )
            .unwrap();
        assert!(matches!(
            pending.accept(&device, server_endpoint(), accepted_at, reply.datagram().1),
            Err(EnrollmentError::Deadline)
        ));
    }
    let (client, server) = channels(&device, &authority);
    let pending = client
        .begin(
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            MessageId::new(1),
            10,
            20,
            entropy(6),
        )
        .unwrap();
    assert_eq!(pending.deadline_ms(), 20);
    let mut store = fixture(&device, &authority, 1, true);
    let reply = server
        .serve(
            device_endpoint(),
            pending.datagram(10).unwrap().1,
            &mut store,
        )
        .unwrap();
    let verified = pending
        .accept(&device, server_endpoint(), 19, reply.datagram().1)
        .unwrap();
    assert_eq!(verified.accepted_ms(), 19);
    assert_eq!(verified.deadline_ms(), 20);
    for rejected_at in [9, 20] {
        let (client, _) = channels(&device, &authority);
        let pending = client
            .begin(
                PolicyQuery::recover_initial_commit(genesis()).unwrap(),
                MessageId::new(1),
                10,
                20,
                entropy(6),
            )
            .unwrap();
        assert!(matches!(
            pending.datagram(rejected_at),
            Err(EnrollmentError::Deadline)
        ));
        assert!(matches!(
            pending.datagram(10),
            Err(EnrollmentError::Deadline)
        ));
    }
    let (client, server) = channels(&device, &authority);
    let pending = client
        .begin(
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
            MessageId::new(1),
            10,
            20,
            entropy(6),
        )
        .unwrap();
    let mut store = fixture(&device, &authority, 1, true);
    let reply = server
        .serve(
            device_endpoint(),
            pending.datagram(15).unwrap().1,
            &mut store,
        )
        .unwrap();
    assert!(matches!(
        pending.accept(&device, server_endpoint(), 14, reply.datagram().1),
        Err(EnrollmentError::Deadline)
    ));
}

#[test]
fn challenge_and_owner_shapes_remain_bounded_and_exact() {
    let device = identity(1, 1);
    let authority = identity(2, 2);
    assert!(EnrollmentPolicy::new(authority.peer(), [0; 32], [8; 32]).is_err());
    assert!(EnrollmentPolicy::new(authority.peer(), [7; 32], [0; 32]).is_err());
    let request = encode_request(
        device.peer().principal(),
        &PolicyQuery::Enroll {
            expected: genesis(),
            policy: policy(&authority),
        },
        [6; 32],
    );
    for length in 0..REQUEST_LEN {
        assert!(decode_request(&request[..length]).is_err());
    }
    let response = encode_response([6; 32], [7; 32], &snapshot(&device, &authority, 1, true));
    for length in 0..RESPONSE_LEN {
        assert!(decode_response(&response[..length]).is_err());
    }
    assert!(core::mem::size_of::<PendingPolicy>() <= 1536);
    assert!(core::mem::size_of::<ProtectedPolicyResponse>() < 420);
}
