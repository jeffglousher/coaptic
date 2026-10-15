extern crate std;

use super::*;
use crate::Endpoint;
use crate::message::MessageId;
use crate::provisioning::{
    AuthorityConnection, AuthorityResponder, BootstrapAuthority, CipherSuite, ConnectionId,
    Identity, Initiator, PendingPolicy, PolicyServeError, Responder, VerifiedPolicy,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    Conflict,
    Before,
    After,
    Panic,
}

#[derive(Default)]
struct Disk {
    current: Option<EnrollmentRecord>,
    calls: usize,
    fault: Option<Fault>,
}

#[derive(Clone, Default)]
struct Store(Arc<Mutex<Disk>>);

impl EnrollmentStore for Store {
    type Error = Fault;
    fn current(&mut self) -> Result<EnrollmentAnchor, Fault> {
        self.0
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(EnrollmentRecord::anchor)
            .ok_or(Fault::Before)
    }
    fn commit(
        &mut self,
        expected: Option<&EnrollmentAnchor>,
        next: &EnrollmentRecord,
        anchor: &EnrollmentAnchor,
    ) -> Result<(), Fault> {
        let mut disk = self.0.lock().unwrap();
        disk.calls += 1;
        if disk.current.as_ref().map(EnrollmentRecord::anchor).as_ref() != expected {
            return Err(Fault::Conflict);
        }
        assert_eq!(*anchor, next.anchor());
        if disk.fault == Some(Fault::Before) {
            return Err(Fault::Before);
        }
        disk.current = Some(next.clone());
        if disk.fault == Some(Fault::After) {
            return Err(Fault::After);
        }
        if disk.fault == Some(Fault::Panic) {
            drop(disk);
            panic!("ambiguous publication");
        }
        Ok(())
    }
}

fn identity(value: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, value).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |out| {
        out.fill(0);
        let last = out.len() - 1;
        out[last] = value;
        true
    }
}

fn genesis() -> TrustAnchor {
    TrustAnchor::genesis([9; 32]).unwrap()
}
fn policy(owner: u8, permissions: u8, peer: u8) -> EnrollmentPolicy {
    EnrollmentPolicy::new(identity(peer).peer(), [owner; 32], [permissions; 32]).unwrap()
}
fn server_endpoint() -> Endpoint {
    Endpoint::v4([192, 0, 2, 1], 5683)
}
fn device_endpoint() -> Endpoint {
    Endpoint::v4([192, 0, 2, 2], 5683)
}

fn pending(device: &Identity, query: PolicyQuery) -> (PendingPolicy, AuthorityResponder) {
    let authority = identity(2);
    let (initiator, m1) = Initiator::start_with_suite(
        device,
        authority.peer(),
        ConnectionId::INITIATOR,
        CipherSuite::Suite3,
        entropy(4),
    )
    .unwrap();
    let (responder, m2) =
        Responder::receive_message_1(&authority, device.peer(), &m1, entropy(5)).unwrap();
    let (initiator, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    let (server, m4) = responder.receive_message_3(&m3, |_| true).unwrap();
    let client = initiator.receive_message_4(&m4, |_| true).unwrap();
    assert_eq!(client.cipher_suite(), CipherSuite::Suite3);
    let client = AuthorityConnection::from_session(
        device,
        &BootstrapAuthority::new(authority.peer()),
        server_endpoint(),
        client,
    )
    .unwrap();
    let server = AuthorityResponder::from_session(&authority, device_endpoint(), server).unwrap();
    (
        client
            .begin(query, MessageId::new(17), 0, 1000, entropy(7))
            .unwrap(),
        server,
    )
}

fn exchange(
    authority: &mut EnrollmentAuthority<Store>,
    device: &Identity,
    query: PolicyQuery,
) -> Result<VerifiedPolicy, PolicyServeError<ApprovalCommitError<Fault>>> {
    let (pending, server) = pending(device, query);
    let response = server.serve(device_endpoint(), pending.datagram(0).unwrap().1, authority)?;
    Ok(pending
        .accept(device, server_endpoint(), 1, response.datagram().1)
        .unwrap())
}

fn enroll_query(policy: EnrollmentPolicy) -> PolicyQuery {
    PolicyQuery::Enroll {
        expected: genesis(),
        policy,
    }
}

fn enrolled() -> (EnrollmentAuthority<Store>, Store, PolicySnapshot) {
    let store = Store::default();
    let mut authority =
        EnrollmentAuthority::register(identity(1).peer(), genesis(), store.clone()).unwrap();
    authority
        .approve(authority.checkpoint().anchor(), policy(7, 8, 3))
        .unwrap();
    let snapshot = exchange(&mut authority, &identity(1), enroll_query(policy(7, 8, 3)))
        .unwrap()
        .snapshot()
        .clone();
    (authority, store, snapshot)
}

#[test]
fn authenticated_device_cannot_self_approve_change_owner_permissions_or_service() {
    let store = Store::default();
    let mut authority =
        EnrollmentAuthority::register(identity(1).peer(), genesis(), store.clone()).unwrap();
    assert!(exchange(&mut authority, &identity(1), enroll_query(policy(7, 8, 3))).is_err());
    let approved = authority
        .approve(authority.checkpoint().anchor(), policy(7, 8, 3))
        .unwrap();
    for wrong in [policy(6, 8, 3), policy(7, 9, 3), policy(7, 8, 4)] {
        assert!(exchange(&mut authority, &identity(1), enroll_query(wrong)).is_err());
    }
    assert!(matches!(
        exchange(&mut authority, &identity(6), enroll_query(policy(7, 8, 3))),
        Err(PolicyServeError::Provider(ApprovalCommitError::State(
            ApprovalError::Device
        )))
    ));
    assert_eq!(store.0.lock().unwrap().calls, 2);
    assert_eq!(authority.checkpoint().anchor(), approved);
    let enrolled = exchange(&mut authority, &identity(1), enroll_query(policy(7, 8, 3))).unwrap();
    assert_eq!(enrolled.snapshot().record().anchor().parts().1, 1);
    assert_eq!(enrolled.snapshot().owner(), &[7; 32]);
    assert!(exchange(&mut authority, &identity(1), enroll_query(policy(7, 8, 3))).is_err());
    assert_eq!(store.0.lock().unwrap().calls, 3);
}

#[test]
fn revoked_approval_and_exclusive_owner_survive_restore_and_regrant_invalidates_old_policy() {
    let (mut authority, store, first) = enrolled();
    let before = authority.checkpoint().clone();
    let revoked = authority.revoke(before.anchor()).unwrap();
    let snapshot = exchange(&mut authority, &identity(1), PolicyQuery::refresh(&first)).unwrap();
    assert!(!snapshot.snapshot().record().enabled());
    assert_eq!(snapshot.snapshot().record().anchor().parts().1, 2);
    assert!(EnrollmentAuthority::restore(before, revoked, store.clone()).is_err());
    let current = store.0.lock().unwrap().current.clone().unwrap();
    let mut authority = EnrollmentAuthority::restore(current, revoked, store.clone()).unwrap();
    assert!(matches!(
        authority.approve(revoked, policy(6, 8, 3)),
        Err(ApprovalCommitError::State(ApprovalError::Owner))
    ));
    authority.approve(revoked, policy(7, 8, 3)).unwrap();
    let current = exchange(&mut authority, &identity(1), PolicyQuery::refresh(&first)).unwrap();
    assert!(current.snapshot().record().enabled());
    assert_eq!(current.snapshot().record().anchor().parts().1, 3);
    assert_ne!(current.snapshot().commitment(), first.commitment());
}

#[test]
fn uncertain_enrollment_blocks_all_use_until_fresh_restore_then_genesis_refresh_recovers() {
    for fault in [Fault::Before, Fault::After, Fault::Panic] {
        let store = Store::default();
        let mut authority =
            EnrollmentAuthority::register(identity(1).peer(), genesis(), store.clone()).unwrap();
        let approved = authority
            .approve(authority.checkpoint().anchor(), policy(7, 8, 3))
            .unwrap();
        store.0.lock().unwrap().fault = Some(fault);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            exchange(&mut authority, &identity(1), enroll_query(policy(7, 8, 3)))
        }));
        if fault == Fault::Panic {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert!(matches!(
            authority.revoke(approved),
            Err(ApprovalCommitError::State(ApprovalError::Blocked))
        ));
        assert!(
            exchange(
                &mut authority,
                &identity(1),
                PolicyQuery::recover_initial_commit(genesis()).unwrap()
            )
            .is_err()
        );
        let record = {
            let mut disk = store.0.lock().unwrap();
            disk.fault = None;
            disk.current.clone().unwrap()
        };
        let mut restored =
            EnrollmentAuthority::restore(record.clone(), record.anchor(), store.clone()).unwrap();
        let recovered = exchange(
            &mut restored,
            &identity(1),
            PolicyQuery::recover_initial_commit(genesis()).unwrap(),
        );
        if fault == Fault::Before {
            assert!(recovered.is_err());
        } else {
            assert_eq!(recovered.unwrap().snapshot().record().anchor().parts().1, 1);
        }
    }
}

#[test]
fn stale_management_and_policy_replicas_cannot_overwrite_current_approval() {
    let (mut authority, store, first) = enrolled();
    let old = authority.checkpoint().clone();
    let mut stale = EnrollmentAuthority::restore(old.clone(), old.anchor(), store.clone()).unwrap();
    let mut stale_refresh =
        EnrollmentAuthority::restore(old.clone(), old.anchor(), store.clone()).unwrap();
    let current = authority.approve(old.anchor(), policy(7, 9, 3)).unwrap();
    assert!(matches!(
        authority.revoke(old.anchor()),
        Err(ApprovalCommitError::State(ApprovalError::Anchor))
    ));
    assert_eq!(store.0.lock().unwrap().calls, 4);
    assert!(matches!(
        exchange(
            &mut stale_refresh,
            &identity(1),
            PolicyQuery::refresh(&first)
        ),
        Err(PolicyServeError::Provider(ApprovalCommitError::State(
            ApprovalError::Anchor
        )))
    ));
    assert!(matches!(
        stale_refresh.revoke(old.anchor()),
        Err(ApprovalCommitError::State(ApprovalError::Blocked))
    ));
    assert!(matches!(
        stale.revoke(old.anchor()),
        Err(ApprovalCommitError::Store(Fault::Conflict))
    ));
    assert!(matches!(
        stale.approve(old.anchor(), policy(7, 8, 3)),
        Err(ApprovalCommitError::State(ApprovalError::Blocked))
    ));
    assert_eq!(
        store.0.lock().unwrap().current.as_ref().unwrap().anchor(),
        current
    );
    let refreshed = exchange(&mut authority, &identity(1), PolicyQuery::refresh(&first)).unwrap();
    assert_eq!(refreshed.snapshot().resource_policy(), &[9; 32]);
}

#[test]
fn persisted_record_invariants_and_complete_anchor_bindings_refuse_substitution() {
    let (authority, store, _) = enrolled();
    let record = authority.checkpoint();
    let expected = record.anchor();
    for case in 0..9 {
        let mut parts = record.parts();
        match case {
            0 => parts.version = 2,
            1 => parts.revision = 0,
            2 => parts.genesis = TrustAnchor::from_parts([9; 32], 1, [1; 32]).unwrap(),
            3 => parts.approved = false,
            4 => parts.device = identity(6).peer(),
            5 => parts.policy = None,
            6 => parts.policy = Some(policy(6, 8, 3)),
            7 => parts.policy = Some(policy(7, 9, 3)),
            _ => parts.revision += 1,
        }
        if let Ok(changed) = EnrollmentRecord::from_parts(parts) {
            assert!(EnrollmentAuthority::restore(changed, expected, store.clone()).is_err());
        }
    }
    let mut parts = record.parts();
    parts.revision = u64::MAX;
    let record = EnrollmentRecord::from_parts(parts).unwrap();
    let mut exhausted =
        EnrollmentAuthority::restore(record.clone(), record.anchor(), store.clone()).unwrap();
    let before = store.0.lock().unwrap().calls;
    assert!(matches!(
        exhausted.revoke(record.anchor()),
        Err(ApprovalCommitError::State(ApprovalError::Exhausted))
    ));
    assert_eq!(store.0.lock().unwrap().calls, before);
}

#[cfg(feature = "std")]
#[test]
fn udp_loopback_enrolls_only_the_exact_operator_approved_device_policy() {
    use crate::provisioning::CoapProvisioner;
    use crate::storage::{DatagramIo, UdpSocketIo};
    use std::{net::UdpSocket, time::Duration};
    let socket = || {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        UdpSocketIo::new(socket, [0; 385]).unwrap()
    };
    let mut ci = socket();
    let mut si = socket();
    let client_endpoint = ci.socket().local_addr().unwrap().into();
    let server_endpoint = si.socket().local_addr().unwrap().into();
    let device = identity(1);
    let server_identity = identity(2);
    let mut client = CoapProvisioner::start_with_suite(
        &device,
        server_identity.peer(),
        server_endpoint,
        ConnectionId::INITIATOR,
        CipherSuite::Suite3,
        0,
        entropy(4),
    )
    .unwrap();
    let mut server = CoapProvisioner::listen(&server_identity, device.peer(), client_endpoint, 0);
    let no_entropy = |_: &mut [u8]| panic!("reuse established handshake state");
    client.flush(&mut ci, 0).unwrap();
    server
        .poll(&mut si, 1, entropy(5), |_| panic!("M1 is not identity"))
        .unwrap();
    client.poll(&mut ci, 2, no_entropy, |_| true).unwrap();
    server.poll(&mut si, 3, no_entropy, |_| true).unwrap();
    client.poll(&mut ci, 4, no_entropy, |_| true).unwrap();
    let client = AuthorityConnection::from_session(
        &device,
        &BootstrapAuthority::new(server_identity.peer()),
        server_endpoint,
        client.take_session().unwrap(),
    )
    .unwrap();
    let server = AuthorityResponder::from_session(
        &server_identity,
        client_endpoint,
        server.take_session().unwrap(),
    )
    .unwrap();
    let store = Store::default();
    let mut authority =
        EnrollmentAuthority::register(device.peer(), genesis(), store.clone()).unwrap();
    authority
        .approve(authority.checkpoint().anchor(), policy(7, 8, 3))
        .unwrap();
    let pending = client
        .begin(
            enroll_query(policy(7, 8, 3)),
            MessageId::new(40),
            5,
            1000,
            entropy(7),
        )
        .unwrap();
    let (endpoint, bytes) = pending.datagram(5).unwrap();
    ci.send(endpoint, bytes).unwrap();
    let mut wire = [0; 384];
    let (len, endpoint) = si.recv(&mut wire).unwrap().unwrap();
    let response = server
        .serve(endpoint, &wire[..len], &mut authority)
        .unwrap();
    let (endpoint, bytes) = response.datagram();
    si.send(endpoint, bytes).unwrap();
    let (len, endpoint) = ci.recv(&mut wire).unwrap().unwrap();
    let verified = pending.accept(&device, endpoint, 6, &wire[..len]).unwrap();
    assert_eq!(verified.snapshot().owner(), &[7; 32]);
    assert_eq!(verified.snapshot().resource_policy(), &[8; 32]);
    assert_eq!(
        verified.snapshot().record().peer().principal(),
        identity(3).peer().principal()
    );
    assert_eq!(store.0.lock().unwrap().calls, 3);
    let mut saved = None;
    let trust = verified
        .persist_and_restore(&device, |snapshot| {
            saved = Some(snapshot.clone());
            Ok::<_, ()>(())
        })
        .unwrap();
    assert!(trust.grant(&device).is_ok());
    assert_eq!(saved.unwrap().record().anchor().parts().1, 1);
}
