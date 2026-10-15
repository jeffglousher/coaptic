extern crate std;

use std::vec::Vec;

use super::*;
use crate::message::{Code, Opt, OptionNumber, Token, Type};
use crate::provisioning::{Error, PeerErrorCode};

const CLIENT: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const SERVER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const OTHER: Endpoint = Endpoint::v4([192, 0, 2, 3], 5683);

#[derive(Default)]
struct Io {
    sent: Vec<Vec<u8>>,
    destinations: Vec<Endpoint>,
    fail: bool,
    short: bool,
}

impl DatagramIo for Io {
    type Error = u8;

    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        panic!("recovery never receives application traffic")
    }

    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.sent.push(bytes.to_vec());
        self.destinations.push(endpoint);
        if self.fail {
            Err(9)
        } else if self.short {
            Ok(bytes.len() - 1)
        } else {
            Ok(bytes.len())
        }
    }
}

impl Io {
    fn last(&self) -> Vec<u8> {
        self.sent.last().unwrap().clone()
    }
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |out| {
        out.fill(0);
        if out.len() == 32 {
            out[31] = value;
        } else if out.len() == 22 {
            out[1] = value * 3;
            out[2] = value;
        }
        true
    }
}

fn no_entropy(_: &mut [u8]) -> bool {
    panic!("cached and subsequent transitions must not request entropy")
}

fn no_authorization(_: &Principal) -> bool {
    panic!("unverified traffic must not request authorization")
}

fn exchange<const A: usize, const B: usize>(
    client: &mut CoapRecovery<'_, A>,
    server: &mut CoapRecovery<'_, B>,
    now: u64,
    seed: u8,
) -> ([Vec<u8>; 4], Session, Session) {
    exchange_with_response_type(client, server, now, seed, Type::Acknowledgement)
}

fn exchange_with_response_type<const A: usize, const B: usize>(
    client: &mut CoapRecovery<'_, A>,
    server: &mut CoapRecovery<'_, B>,
    now: u64,
    seed: u8,
    response_type: Type,
) -> ([Vec<u8>; 4], Session, Session) {
    let mut a = Io::default();
    let mut b = Io::default();
    client.start(now, entropy(seed)).unwrap();
    client.flush(&mut a, now).unwrap();
    let m1 = a.last();
    server
        .ingest(&m1, CLIENT, now, entropy(seed + 1), |_| true)
        .unwrap();
    server.flush(&mut b, now).unwrap();
    let original_m2 = b.last();
    let parsed = decode(&original_m2).unwrap();
    let m2 = packet(
        &original_m2,
        response_type,
        parsed.message_id(),
        parsed.token(),
        parsed.payload(),
    );
    client
        .ingest(&m2, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a, now).unwrap();
    let m3 = a.last();
    server
        .ingest(&m3, CLIENT, now, no_entropy, |_| true)
        .unwrap();
    assert!(server.take_session().is_none());
    server.flush(&mut b, now).unwrap();
    let original_m4 = b.last();
    let parsed = decode(&original_m4).unwrap();
    let m4 = packet(
        &original_m4,
        response_type,
        parsed.message_id(),
        parsed.token(),
        parsed.payload(),
    );
    client
        .ingest(&m4, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a, now).unwrap();
    let client_session = client.take_session().unwrap();
    let server_session = server.take_session().unwrap();
    assert!(client.confirm_handoff());
    assert!(server.confirm_handoff());
    ([m1, m2, m3, m4], client_session, server_session)
}

fn packet(bytes: &[u8], ty: Type, mid: MessageId, token: Token, payload: &[u8]) -> Vec<u8> {
    let parsed = decode(bytes).unwrap();
    let options: Vec<_> = parsed.options().collect();
    let mut output = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::new(ty, parsed.code(), mid)
        .with_token(token)
        .with_options(&options)
        .with_payload(payload)
        .encode(&mut output)
        .unwrap();
    output[..len].to_vec()
}

fn critical_request(bytes: &[u8], mid: u16) -> Vec<u8> {
    let p = decode(bytes).unwrap();
    let mut options: Vec<_> = p.options().collect();
    options.push(Opt::new(OptionNumber::new(99), &[]));
    options.sort_by_key(|o| o.number());
    let mut out = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::con(Code::POST, MessageId::new(mid), p.token())
        .with_options(&options)
        .with_payload(p.payload())
        .encode(&mut out)
        .unwrap();
    out[..len].to_vec()
}

#[test]
fn initiator_error_survives_cancellation_preserves_active_session_and_blocks_a_new_con() {
    for cancel_before_send in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let wrong_reference = identity(2, 9);
        let mut client = CoapRecovery::<4>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<4>::new(&b, a.peer(), CLIENT, 0).unwrap();
        let (_, old_client, old_server) = exchange(&mut client, &mut server, 0, 3);
        let active = client.active_connection_id();
        let mut peer = CoapProvisioner::listen_with_id(
            &wrong_reference,
            a.peer(),
            CLIENT,
            ConnectionId::new(7).unwrap(),
            10,
        );
        let mut ci = Io::default();
        let mut si = Io::default();
        client.start(10, entropy(5)).unwrap();
        client.flush(&mut ci, 10).unwrap();
        let first = ci.last();
        let first_mid = decode(&first).unwrap().message_id();
        peer.ingest(&first, CLIENT, 10, entropy(6), no_authorization)
            .unwrap();
        peer.flush(&mut si, 10).unwrap();
        client
            .ingest(&si.last(), SERVER, 10, no_entropy, no_authorization)
            .unwrap();
        ci.fail = true;
        if !cancel_before_send {
            assert_eq!(
                client.flush(&mut ci, 11),
                Err(RecoveryError::Bootstrap(PollError::Io(9)))
            );
        }
        assert!(client.discard_candidate(11).unwrap());
        assert!(
            client.has_candidate(),
            "queued outgoing error must keep NSTART occupied"
        );
        assert_eq!(client.start(11, no_entropy), Err(RecoveryError::Busy));
        assert_eq!(
            client.flush(&mut ci, 11),
            Err(RecoveryError::Bootstrap(PollError::Io(9)))
        );
        let saved = ci.last();
        let p = decode(&saved).unwrap();
        assert_eq!(p.message_id(), first_mid.wrapping_add(1));
        assert_eq!(p.payload(), [7, 3, 0xf5]);
        let mut ack = [0; 32];
        let len = CoapMessage::new(Type::Acknowledgement, Code::CHANGED, p.message_id())
            .with_token(p.token())
            .encode(&mut ack)
            .unwrap();
        assert_eq!(
            client.ingest(&ack[..len], SERVER, 12, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        assert!(client.has_candidate());
        ci.fail = false;
        ci.short = true;
        assert_eq!(
            client.flush(&mut ci, 12),
            Err(RecoveryError::Bootstrap(PollError::ShortSend))
        );
        ci.short = false;
        client.flush(&mut ci, 13).unwrap();
        assert_eq!(ci.last(), saved);
        assert_eq!(client.next_deadline(), Some(2013));
        assert_eq!(
            client.ingest(&ack[..len], OTHER, 14, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        assert!(client.has_candidate());
        client.flush(&mut ci, 2013).unwrap();
        assert_eq!(ci.last(), saved);
        assert!(client.reserves_message_id(first_mid.wrapping_add(3), SERVER, 2013));
        peer.ingest(&saved, CLIENT, 2013, no_entropy, no_authorization)
            .unwrap();
        assert_eq!(
            peer.flush(&mut si, 2013),
            Err(PollError::PeerAbort(PeerErrorCode::UnknownCredential))
        );
        client
            .ingest(&si.last(), SERVER, 2014, no_entropy, no_authorization)
            .unwrap();
        assert!(!client.has_candidate());
        assert_eq!(client.active_connection_id(), active);
        assert!(client.take_session().is_none());
        let (_, fresh_client, fresh_server) = exchange(&mut client, &mut server, 2015, 7);
        assert_eq!(fresh_client.principal(), old_client.principal());
        let (old_client, _) = old_client.into_parts();
        let (old_server, _) = old_server.into_parts();
        assert_eq!(old_client.sender_key(), old_server.recipient_key());
        let (fresh_client, _) = fresh_client.into_parts();
        let (fresh_server, _) = fresh_server.into_parts();
        assert_eq!(fresh_client.sender_key(), fresh_server.recipient_key());
        assert_ne!(old_client.sender_key(), fresh_client.sender_key());
    }
}

#[test]
fn outbound_suite_error_recovery_retains_pending_reply_and_recovers_with_fresh_keys() {
    for cancel in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapRecovery::<4>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<4>::new(&b, a.peer(), CLIENT, 0).unwrap();
        let (_, old_client, old_server) = exchange(&mut client, &mut server, 0, 3);
        let active = server.active_connection_id();
        let mut ci = Io::default();
        let mut si = Io::default();
        client.start(10, entropy(5)).unwrap();
        client.flush(&mut ci, 10).unwrap();
        let m1 = ci.last();
        let p = decode(&m1).unwrap();
        let mut payload = p.payload().to_vec();
        payload[2] = 3;
        let wrong = packet(&m1, p.ty(), p.message_id(), p.token(), &payload);
        assert_eq!(
            server.ingest(&wrong, CLIENT, 11, no_entropy, no_authorization),
            Ok(Status::Progress)
        );
        assert!(server.has_candidate());
        assert_eq!(server.active_connection_id(), active);
        assert!(server.take_session().is_none());
        if cancel {
            assert_eq!(server.discard_candidate(12), Ok(true));
            assert!(!server.has_candidate());
        }
        si.fail = true;
        assert_eq!(
            server.flush(&mut si, 12),
            Err(RecoveryError::Bootstrap(PollError::Io(9)))
        );
        let saved = si.last();
        assert_eq!(decode(&saved).unwrap().payload(), [2, 2]);
        si.fail = false;
        si.short = true;
        assert_eq!(
            server.flush(&mut si, 13),
            Err(RecoveryError::Bootstrap(PollError::ShortSend))
        );
        si.short = false;
        let result = server.flush(&mut si, 14);
        if cancel {
            assert_eq!(result, Ok(Status::Progress));
        } else {
            assert_eq!(
                result,
                Err(RecoveryError::Bootstrap(PollError::Provisioning(
                    Error::Profile
                )))
            );
        }
        assert!(!server.has_candidate());
        assert_eq!(si.last(), saved);
        assert_eq!(server.active_connection_id(), active);
        assert_eq!(
            client.ingest(&saved, SERVER, 14, no_entropy, no_authorization),
            Ok(Status::Progress)
        );
        assert!(matches!(
            client.flush(&mut ci, 14),
            Err(RecoveryError::Bootstrap(PollError::PeerAbort(_)))
        ));
        let mut changed = wrong.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert_eq!(
            server.ingest(&changed, CLIENT, 15, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        assert_eq!(
            server.ingest(&wrong, CLIENT, 15, no_entropy, no_authorization),
            Ok(Status::Progress)
        );
        server.flush(&mut si, 15).unwrap();
        assert_eq!(si.last(), saved);
        let (_, fresh_client, fresh_server) = exchange(&mut client, &mut server, 16, 7);
        let (old_client, _) = old_client.into_parts();
        let (old_server, _) = old_server.into_parts();
        let (fresh_client, _) = fresh_client.into_parts();
        let (fresh_server, _) = fresh_server.into_parts();
        assert_ne!(old_client.sender_key(), fresh_client.sender_key());
        assert_ne!(old_server.sender_key(), fresh_server.sender_key());
        assert_eq!(fresh_client.sender_key(), fresh_server.recipient_key());
        assert_eq!(fresh_client.recipient_key(), fresh_server.sender_key());
        // Neither duplicate traffic nor the later handshake renews the refusal cache.
        // At its original expiry, this datagram may be admitted as a new attempt.
        let sends = si.sent.len();
        assert_eq!(
            server.ingest(
                &wrong,
                CLIENT,
                11 + EXCHANGE_LIFETIME_MS,
                no_entropy,
                no_authorization
            ),
            Ok(Status::Progress)
        );
        assert_eq!(
            server.flush(&mut si, 11 + EXCHANGE_LIFETIME_MS),
            Err(RecoveryError::Bootstrap(PollError::Provisioning(
                Error::Profile
            )))
        );
        assert_eq!(si.sent.len(), sends + 1);
    }
}

#[test]
fn outbound_suite_error_recovery_capacity_preserves_the_only_reply_until_expiry() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    let p = decode(&m1).unwrap();
    let mut payload = p.payload().to_vec();
    payload[2] = 3;
    let wrong = packet(&m1, p.ty(), p.message_id(), p.token(), &payload);
    server
        .ingest(&wrong, CLIENT, 1, no_entropy, no_authorization)
        .unwrap();
    // Cancellation before transport exposure must not discard the queued error.
    assert_eq!(server.discard_candidate(1), Ok(true));
    server.flush(&mut si, 1).unwrap();
    let saved = si.last();
    let fresh = packet(
        &m1,
        p.ty(),
        p.message_id().wrapping_add(50),
        p.token(),
        p.payload(),
    );
    assert_eq!(
        server.ingest(&fresh, CLIENT, 2, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    assert!(!server.has_candidate());
    server
        .ingest(&wrong, CLIENT, 3, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut si, 3).unwrap();
    assert_eq!(si.last(), saved);
    assert_eq!(server.next_deadline(), Some(1 + EXCHANGE_LIFETIME_MS));
    assert_eq!(
        server.ingest(
            &fresh,
            CLIENT,
            1 + EXCHANGE_LIFETIME_MS,
            entropy(4),
            no_authorization
        ),
        Ok(Status::Progress)
    );
    assert!(server.has_candidate());
    server.flush(&mut si, 1 + EXCHANGE_LIFETIME_MS).unwrap();
    assert_eq!(decode(&si.last()).unwrap().code(), Code::CHANGED);
}

#[test]
fn incoming_error_request_retires_with_exact_ack_and_preserves_active_ownership() {
    use crate::provisioning::PeerErrorCode;
    for cancel in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapRecovery::<3>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<3>::new(&b, a.peer(), CLIENT, 0).unwrap();
        let (_, old_client, old_server) = exchange(&mut client, &mut server, 0, 3);
        let active = server.active_connection_id();
        let old_owner = *old_server.principal();
        let mut ci = Io::default();
        let mut si = Io::default();
        client.start(10, entropy(5)).unwrap();
        client.flush(&mut ci, 10).unwrap();
        server
            .ingest(&ci.last(), CLIENT, 10, entropy(6), no_authorization)
            .unwrap();
        server.flush(&mut si, 10).unwrap();
        client
            .ingest(&si.last(), SERVER, 10, no_entropy, |_| true)
            .unwrap();
        client.flush(&mut ci, 10).unwrap();
        let third = ci.last();
        let p = decode(&third).unwrap();
        let (_, prefix_len) = ConnectionId::decode_prefix(p.payload()).unwrap();
        let mut payload = p.payload()[..prefix_len].to_vec();
        payload.extend_from_slice(&[1, 0x64, b's', b't', b'o', b'p']);
        let error = packet(&third, p.ty(), p.message_id(), p.token(), &payload);
        assert_eq!(
            server.ingest(&error, CLIENT, 11, no_entropy, no_authorization),
            Ok(Status::Progress)
        );
        if cancel {
            assert_eq!(server.discard_candidate(11), Ok(true));
        }
        si.fail = true;
        assert_eq!(
            server.flush(&mut si, 11),
            Err(RecoveryError::Bootstrap(PollError::Io(9)))
        );
        let saved = si.last();
        assert!(decode(&saved).unwrap().payload().is_empty());
        assert_eq!(server.peer_error().unwrap().diagnostic(), Some("stop"));
        si.fail = false;
        if cancel {
            server.flush(&mut si, 12).unwrap();
        } else {
            assert_eq!(
                server.flush(&mut si, 12),
                Err(RecoveryError::Bootstrap(PollError::PeerAbort(
                    PeerErrorCode::Unspecified
                )))
            );
        }
        assert!(!server.has_candidate());
        assert_eq!(server.active_connection_id(), active);
        assert_eq!(*old_server.principal(), old_owner);
        let mut collision = error.clone();
        *collision.last_mut().unwrap() ^= 1;
        assert_eq!(
            server.ingest(&collision, CLIENT, 13, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        server
            .ingest(&error, CLIENT, 13, no_entropy, no_authorization)
            .unwrap();
        server.flush(&mut si, 13).unwrap();
        assert_eq!(si.last(), saved);
        assert_eq!(server.peer_error().unwrap().diagnostic(), Some("stop"));
        assert_eq!(client.discard_candidate(14), Ok(true));
        let (_, fresh_client, fresh_server) = exchange(&mut client, &mut server, 15, 7);
        let (old_client, _) = old_client.into_parts();
        let (old_server, _) = old_server.into_parts();
        let (fresh_client, _) = fresh_client.into_parts();
        let (fresh_server, _) = fresh_server.into_parts();
        assert_ne!(old_client.sender_key(), fresh_client.sender_key());
        assert_ne!(old_server.sender_key(), fresh_server.sender_key());
        assert_eq!(fresh_client.sender_key(), fresh_server.recipient_key());
        server.flush(&mut si, 11 + EXCHANGE_LIFETIME_MS).unwrap();
        assert!(server.peer_error().is_none());
    }
}

#[test]
fn peer_error_recovery_preserves_live_owner_ack_cache_and_fresh_authentication() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<3>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<3>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let (_, old_client, old_server) = exchange(&mut client, &mut server, 0, 3);
    let old_principal = *old_client.principal();
    let active = client.active_connection_id();
    let mut ci = Io::default();
    client.start(10, entropy(5)).unwrap();
    client.flush(&mut ci, 10).unwrap();
    let m1 = ci.last();
    let p = decode(&m1).unwrap();
    let options = [Opt::new(OptionNumber::CONTENT_FORMAT, &[64])];
    let mut bytes = [0; DATAGRAM_CAPACITY];
    let mid = MessageId::new(900);
    let len = CoapMessage::new(Type::Confirmable, Code::INTERNAL_SERVER_ERROR, mid)
        .with_token(p.token())
        .with_options(&options)
        .with_payload(&[1, 0x63, b'b', b'a', b'd'])
        .encode(&mut bytes)
        .unwrap();
    let error = bytes[..len].to_vec();
    assert_eq!(
        client.ingest(&error, SERVER, 11, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    ci.fail = true;
    assert_eq!(
        client.flush(&mut ci, 11),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    assert!(client.has_candidate());
    ci.fail = false;
    assert_eq!(
        client.flush(&mut ci, 12),
        Err(RecoveryError::Bootstrap(PollError::PeerAbort(
            super::super::super::PeerErrorCode::Unspecified
        )))
    );
    assert!(!client.has_candidate());
    assert_eq!(client.active_connection_id(), active);
    assert_eq!(*old_client.principal(), old_principal);
    assert!(client.take_session().is_none());
    assert_eq!(client.peer_error().unwrap().diagnostic(), Some("bad"));
    let ack = ci.last();
    assert_eq!(ack, [0x60, 0, 3, 0x84]);
    assert_eq!(*ci.destinations.last().unwrap(), SERVER);
    let mut changed = error.clone();
    *changed.last_mut().unwrap() = b'x';
    assert_eq!(
        client.ingest(&changed, SERVER, 13, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    assert_eq!(
        client.ingest(&error, SERVER, 13, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    ci.short = true;
    assert_eq!(
        client.flush(&mut ci, 13),
        Err(RecoveryError::Bootstrap(PollError::ShortSend))
    );
    assert_eq!(ci.last(), ack);
    ci.short = false;
    client.flush(&mut ci, 14).unwrap();
    assert_eq!(ci.last(), ack);
    // Recovery is an explicit caller decision; a forged error never starts it.
    let (_, fresh_client, fresh_server) = exchange(&mut client, &mut server, 15, 7);
    let (old_client, _) = old_client.into_parts();
    let (old_server, _) = old_server.into_parts();
    let (fresh_client, _) = fresh_client.into_parts();
    let (fresh_server, _) = fresh_server.into_parts();
    assert_ne!(old_client.sender_key(), fresh_client.sender_key());
    assert_ne!(old_server.sender_key(), fresh_server.sender_key());
    assert_eq!(fresh_client.sender_key(), fresh_server.recipient_key());
    assert_eq!(fresh_client.recipient_key(), fresh_server.sender_key());
    client.flush(&mut ci, 11 + EXCHANGE_LIFETIME_MS).unwrap();
    assert!(client.peer_error().is_none());
}

#[test]
fn initial_admission_critical_refusal_is_bounded_and_recovers_through_handoff() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    let bad = critical_request(&m1, 900);
    assert_eq!(
        server.ingest(&bad, CLIENT, 10, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    assert!(!server.has_candidate());
    assert!(server.caches.iter().all(Option::is_none));
    assert_eq!(server.next_deadline(), Some(10));
    // No pending owner can be overwritten, and no crypto work is admitted until flush.
    assert_eq!(
        server.ingest(&m1, CLIENT, 11, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    si.fail = true;
    assert_eq!(
        server.flush(&mut si, 11),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    let saved = si.last();
    let parsed = decode(&saved).unwrap();
    assert_eq!(parsed.ty(), Type::Acknowledgement);
    assert_eq!(parsed.code(), Code::BAD_OPTION);
    assert_eq!(parsed.message_id(), MessageId::new(900));
    assert_eq!(parsed.token(), decode(&bad).unwrap().token());
    assert!(parsed.payload().is_empty());
    assert_eq!(parsed.options().count(), 0);
    assert!(saved.len() <= 12);
    si.fail = false;
    si.short = true;
    assert_eq!(
        server.flush(&mut si, 12),
        Err(RecoveryError::Bootstrap(PollError::ShortSend))
    );
    assert_eq!(si.last(), saved);
    si.short = false;
    assert_eq!(server.flush(&mut si, 13), Ok(Status::Progress));
    assert_eq!(si.last(), saved);
    assert!(si.destinations.iter().all(|endpoint| *endpoint == CLIENT));
    assert_eq!(server.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
    assert_eq!(
        server.ingest(
            &critical_request(&m1, 901),
            CLIENT,
            14,
            no_entropy,
            no_authorization
        ),
        Ok(Status::Ignored)
    );
    assert!(!server.has_candidate());
    // A distinct valid MID can use the sole candidate/cache slot while the refusal lives.
    assert_eq!(
        server.ingest(&m1, CLIENT, 15, entropy(4), no_authorization),
        Ok(Status::Progress)
    );
    server.flush(&mut si, 15).unwrap();
    client
        .ingest(&si.last(), SERVER, 15, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut ci, 15).unwrap();
    server
        .ingest(&ci.last(), CLIENT, 15, no_entropy, |_| true)
        .unwrap();
    server.flush(&mut si, 15).unwrap();
    client
        .ingest(&si.last(), SERVER, 15, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut ci, 15).unwrap();
    let client_session = client.take_session().unwrap();
    // A refused-MID mutation retains the original token/reply and blocks handoff until drained.
    let p = decode(&m1).unwrap();
    let changed = packet(
        &m1,
        Type::Confirmable,
        MessageId::new(900),
        Token::from_checked(&[9]),
        p.payload(),
    );
    assert_eq!(
        server.ingest(&changed, CLIENT, 16, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    assert!(server.take_session().is_none());
    server.flush(&mut si, 16).unwrap();
    assert_eq!(si.last(), saved);
    let server_session = server.take_session().unwrap();
    assert!(client.confirm_handoff());
    assert!(server.confirm_handoff());
    let active = server.active_connection_id();
    let (client_context, _) = client_session.into_parts();
    let (server_context, _) = server_session.into_parts();
    assert_eq!(client_context.sender_key(), server_context.recipient_key());
    assert_eq!(client_context.recipient_key(), server_context.sender_key());
    assert_eq!(client_context.common_iv(), server_context.common_iv());
    assert_eq!(
        server.ingest(&changed, CLIENT, 17, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    server.flush(&mut si, 17).unwrap();
    assert_eq!(si.last(), saved);
    assert_eq!(server.active_connection_id(), active);
    assert!(!server.has_candidate());
    assert_eq!(
        server.control.as_ref().unwrap().expires,
        10 + EXCHANGE_LIFETIME_MS
    );
}

#[test]
fn initial_admission_refusal_rejects_unrelated_and_opposite_role_traffic() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let bad = critical_request(&ci.last(), 900);
    let p = decode(&bad).unwrap();
    let non = packet(
        &bad,
        Type::NonConfirmable,
        p.message_id(),
        p.token(),
        p.payload(),
    );
    let mut opposite = bad.clone();
    opposite[1] = Code::CHANGED.as_raw();
    let mut oversized = bad.clone();
    oversized.resize(DATAGRAM_CAPACITY + 1, 0);
    for bytes in [&non[..], &opposite, &oversized, &[0x40, 2]] {
        assert_eq!(
            server.ingest(bytes, CLIENT, 0, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
    }
    assert_eq!(
        server.ingest(&bad, OTHER, 0, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    let options = [
        Opt::new(OptionNumber::URI_PATH, b"other"),
        Opt::new(OptionNumber::new(99), &[]),
    ];
    let mut out = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::con(Code::POST, p.message_id(), p.token())
        .with_options(&options)
        .encode(&mut out)
        .unwrap();
    assert_eq!(
        server.ingest(&out[..len], CLIENT, 0, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    assert!(server.control.is_none());
    // Critical option handling precedes EDHOC payload parsing.
    let empty = packet(&bad, Type::Confirmable, p.message_id(), p.token(), &[]);
    assert_eq!(
        server.ingest(&empty, CLIENT, 1, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    server.flush(&mut si, 1).unwrap();
    for bytes in [&non[..], &opposite] {
        assert_eq!(
            server.ingest(bytes, CLIENT, 2, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
    }
    assert_eq!(server.flush(&mut si, 2), Ok(Status::Idle));
    assert_eq!(si.sent.len(), 1);
    assert!(!server.has_candidate());
}

#[test]
fn initial_admission_refusal_expiry_and_clock_overflow_release_no_crypto_work() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut ci = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    let bad = critical_request(&m1, 900);
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut si = Io::default();
    server
        .ingest(&bad, CLIENT, 0, no_entropy, no_authorization)
        .unwrap();
    assert_eq!(
        server.flush(&mut si, EXCHANGE_LIFETIME_MS),
        Ok(Status::Idle)
    );
    assert!(si.sent.is_empty());
    assert!(server.control.is_none());
    assert_eq!(server.next_deadline(), None);
    assert_eq!(
        server.ingest(
            &m1,
            CLIENT,
            EXCHANGE_LIFETIME_MS,
            entropy(4),
            no_authorization
        ),
        Ok(Status::Progress)
    );
    server.flush(&mut si, EXCHANGE_LIFETIME_MS).unwrap();
    assert_eq!(decode(&si.last()).unwrap().code(), Code::CHANGED);
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, u64::MAX).unwrap();
    assert_eq!(
        server.ingest(&bad, CLIENT, u64::MAX, no_entropy, no_authorization),
        Err(RecoveryError::Bootstrap(PollError::Clock))
    );
    assert!(server.control.is_none());
    assert!(!server.has_candidate());
    assert!(server.caches.iter().all(Option::is_none));
    assert_eq!(server.next_deadline(), None);
}

#[test]
fn initial_admission_refusal_survives_candidate_retirement_without_renewal() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    let bad = critical_request(&m1, 900);
    server
        .ingest(&bad, CLIENT, 10, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut si, 10).unwrap();
    let saved = si.last();
    server
        .ingest(&m1, CLIENT, 11, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut si, 11).unwrap();
    assert!(server.discard_candidate(12).unwrap());
    assert!(!server.has_candidate());
    assert_eq!(
        server.control.as_ref().unwrap().expires,
        10 + EXCHANGE_LIFETIME_MS
    );
    assert_eq!(
        server.ingest(
            &bad,
            CLIENT,
            10 + EXCHANGE_LIFETIME_MS - 1,
            no_entropy,
            no_authorization
        ),
        Ok(Status::Progress)
    );
    server
        .flush(&mut si, 10 + EXCHANGE_LIFETIME_MS - 1)
        .unwrap();
    assert_eq!(si.last(), saved);
    server.flush(&mut si, 10 + EXCHANGE_LIFETIME_MS).unwrap();
    assert!(server.control.is_none());
    // The retired handshake still owns its later expiry; the refusal cannot evict it.
    assert_eq!(server.next_deadline(), Some(11 + EXCHANGE_LIFETIME_MS));
    assert_eq!(
        server.ingest(
            &critical_request(&m1, 901),
            CLIENT,
            10 + EXCHANGE_LIFETIME_MS,
            no_entropy,
            no_authorization
        ),
        Ok(Status::Progress)
    );
    assert!(!server.has_candidate());
    assert_eq!(server.caches.iter().flatten().count(), 1);
    server.flush(&mut si, 10 + EXCHANGE_LIFETIME_MS).unwrap();
    assert_eq!(
        decode(&si.last()).unwrap().message_id(),
        MessageId::new(901)
    );
}

#[test]
fn initial_admission_critical_mutation_cannot_cross_completed_cache_owner() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let ([m1, m2, _, _], _, _) = exchange(&mut client, &mut server, 0, 3);
    let mut si = Io::default();
    let mid = decode(&m1).unwrap().message_id().get();
    assert_eq!(
        server.ingest(
            &critical_request(&m1, mid),
            CLIENT,
            1,
            no_entropy,
            no_authorization
        ),
        Ok(Status::Ignored)
    );
    assert!(server.control.is_none());
    assert_eq!(
        server.ingest(&m1, CLIENT, 1, no_entropy, no_authorization),
        Ok(Status::Progress)
    );
    server.flush(&mut si, 1).unwrap();
    assert_eq!(si.last(), m2);
    // A fresh refusal works even when the single completion cache is occupied.
    assert_eq!(
        server.ingest(
            &critical_request(&m1, 900),
            CLIENT,
            2,
            no_entropy,
            no_authorization
        ),
        Ok(Status::Progress)
    );
    assert!(!server.has_candidate());
    server.flush(&mut si, 2).unwrap();
    assert_eq!(decode(&si.last()).unwrap().code(), Code::BAD_OPTION);
    assert_eq!(server.caches.iter().flatten().count(), 1);
}

#[test]
fn bootstrap_options_recovery_handoff_preserves_refusal_mid_and_failed_send() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    server
        .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut si, 0).unwrap();
    let m2 = si.last();
    let bad = critical_request(&m1, 900);
    server
        .ingest(&bad, CLIENT, 100, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut si, 100).unwrap();
    let saved = si.last();
    client
        .ingest(&m2, SERVER, 101, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut ci, 101).unwrap();
    server
        .ingest(&ci.last(), CLIENT, 101, no_entropy, |_| true)
        .unwrap();
    server.flush(&mut si, 101).unwrap();
    client
        .ingest(&si.last(), SERVER, 101, no_entropy, |_| true)
        .unwrap();
    assert!(server.take_session().is_some());
    assert!(server.confirm_handoff());
    assert!(client.take_session().is_some());
    assert!(client.confirm_handoff());
    let mut opposite_role = bad.clone();
    opposite_role[1] = 68;
    let count = si.sent.len();
    let cache = server.caches.iter().flatten().next().unwrap();
    assert_eq!(
        cache.classify(&opposite_role, 102),
        CompletedCacheMatch::RefusedMid
    );
    assert_eq!(
        server
            .ingest(&opposite_role, CLIENT, 102, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    server.flush(&mut si, 102).unwrap();
    assert_eq!(si.sent.len(), count);
    let p = decode(&m1).unwrap();
    let changed = packet(
        &m1,
        Type::Confirmable,
        MessageId::new(900),
        Token::from_checked(&[9]),
        p.payload(),
    );
    assert_eq!(
        server
            .ingest(&changed, CLIENT, 102, no_entropy, no_authorization)
            .unwrap(),
        Status::Progress
    );
    assert!(!server.has_candidate());
    si.fail = true;
    assert_eq!(
        server.flush(&mut si, 102),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    assert_eq!(si.last(), saved);
    si.fail = false;
    si.short = true;
    assert_eq!(
        server.flush(&mut si, 103),
        Err(RecoveryError::Bootstrap(PollError::ShortSend))
    );
    assert_eq!(si.last(), saved);
    si.short = false;
    server.flush(&mut si, 104).unwrap();
    assert_eq!(si.last(), saved);
    let own_expiry = 100 + EXCHANGE_LIFETIME_MS;
    server
        .ingest(
            &changed,
            CLIENT,
            own_expiry - 1,
            no_entropy,
            no_authorization,
        )
        .unwrap();
    server.flush(&mut si, own_expiry - 1).unwrap();
    assert_eq!(si.last(), saved);
    let count = si.sent.len();
    server.flush(&mut si, own_expiry).unwrap();
    assert_eq!(si.sent.len(), count);
    let cache = server.caches.iter().flatten().next().unwrap();
    assert_eq!(
        cache.classify(&changed, own_expiry),
        CompletedCacheMatch::NoMatch
    );
}

#[test]
fn bootstrap_options_recovery_retirement_keeps_later_refusal_expiry() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let m1 = ci.last();
    server
        .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut si, 0).unwrap();
    server
        .ingest(
            &critical_request(&m1, 900),
            CLIENT,
            100,
            no_entropy,
            no_authorization,
        )
        .unwrap();
    server.flush(&mut si, 100).unwrap();
    assert!(server.discard_candidate(EXCHANGE_LIFETIME_MS + 1).unwrap());
    assert!(!server.has_candidate());
    assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS + 100));
    let p = decode(&m1).unwrap();
    let changed = packet(
        &m1,
        Type::Confirmable,
        MessageId::new(900),
        Token::from_checked(&[9]),
        p.payload(),
    );
    let count = si.sent.len();
    assert_eq!(
        server
            .ingest(
                &changed,
                CLIENT,
                EXCHANGE_LIFETIME_MS + 99,
                no_entropy,
                no_authorization
            )
            .unwrap(),
        Status::Ignored
    );
    assert!(!server.has_candidate());
    server.flush(&mut si, EXCHANGE_LIFETIME_MS + 99).unwrap();
    assert_eq!(si.sent.len(), count); // retired owners quarantine without replies
    assert_eq!(
        server
            .ingest(
                &changed,
                CLIENT,
                EXCHANGE_LIFETIME_MS + 100,
                entropy(5),
                no_authorization
            )
            .unwrap(),
        Status::Progress
    );
    assert!(server.has_candidate());
}

#[test]
fn either_peer_alone_can_restart_with_distinct_active_local_ids_and_fresh_keys() {
    for restarted_client in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
        let (_, old_a, old_b) = exchange(&mut client, &mut server, 0, 3);
        let old_client_id = client.active_connection_id().unwrap();
        let old_server_id = server.active_connection_id().unwrap();
        let (old_a, _) = old_a.into_parts();
        let (old_b, _) = old_b.into_parts();
        if restarted_client {
            client = CoapRecovery::new(&a, b.peer(), SERVER, 100).unwrap();
        } else {
            server = CoapRecovery::new(&b, a.peer(), CLIENT, 100).unwrap();
        }
        let (_, new_a, new_b) = exchange(&mut client, &mut server, 100, 5);
        let (new_a, _) = new_a.into_parts();
        let (new_b, _) = new_b.into_parts();
        assert_ne!(old_a.sender_key(), new_a.sender_key());
        assert_ne!(old_b.sender_key(), new_b.sender_key());
        assert_eq!(new_a.sender_key(), new_b.recipient_key());
        assert_eq!(new_a.recipient_key(), new_b.sender_key());
        assert_eq!(new_a.common_iv(), new_b.common_iv());
        if restarted_client {
            assert_ne!(server.active_connection_id().unwrap(), old_server_id);
        } else {
            assert_ne!(client.active_connection_id().unwrap(), old_client_id);
        }
    }
}

#[test]
fn original_message4_cache_survives_candidate_traffic_and_failed_sends() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let (old, old_session, _) = exchange(&mut client, &mut server, 0, 3);
    let old_principal = *old_session.principal();
    let old_id = server.active_connection_id();
    let mut restarted = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 100).unwrap();
    let mut a_io = Io::default();
    restarted.start(100, entropy(5)).unwrap();
    restarted.flush(&mut a_io, 100).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 100, entropy(6), |_| true)
        .unwrap();
    assert_eq!(server.active_connection_id(), old_id);
    assert_eq!(*old_session.principal(), old_principal);
    assert_eq!(
        server
            .ingest(&old[2], CLIENT, 101, no_entropy, no_authorization)
            .unwrap(),
        Status::Progress
    );
    let mut b_io = Io {
        fail: true,
        ..Io::default()
    };
    assert_eq!(
        server.flush(&mut b_io, 101),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    assert_eq!(b_io.last(), old[3]);
    b_io.fail = false;
    server.flush(&mut b_io, 101).unwrap();
    assert_eq!(b_io.sent[1], old[3]);
    let fresh_m2 = b_io.last();
    restarted
        .ingest(&fresh_m2, SERVER, 101, no_entropy, |_| true)
        .unwrap();
    restarted.flush(&mut a_io, 101).unwrap();
    let mut corrupt = a_io.last();
    *corrupt.last_mut().unwrap() ^= 1;
    assert_eq!(
        server.ingest(&corrupt, CLIENT, 102, no_entropy, |_| true),
        Ok(Status::Progress)
    );
    assert!(matches!(
        server.flush(&mut b_io, 102),
        Err(RecoveryError::Bootstrap(PollError::Provisioning(_)))
    ));
    assert!(!server.has_candidate());
    assert_eq!(server.active_connection_id(), old_id);
    assert!(server.take_session().is_none());
    server
        .ingest(&old[2], CLIENT, 103, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut b_io, 103).unwrap();
    assert_eq!(b_io.last(), old[3]);
}

#[test]
fn changed_cached_mids_and_wrong_endpoint_cannot_admit_a_candidate() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let (old, _, _) = exchange(&mut client, &mut server, 0, 3);
    let parsed = decode(&old[0]).unwrap();
    let changed = packet(
        &old[0],
        parsed.ty(),
        parsed.message_id(),
        Token::from_checked(b"changed"),
        parsed.payload(),
    );
    assert_eq!(
        server
            .ingest(&changed, CLIENT, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert_eq!(
        server
            .ingest(&old[0], OTHER, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    let mut malformed = old[0][..4].to_vec();
    malformed[0] = 0xff;
    assert_eq!(
        server
            .ingest(&malformed, CLIENT, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert!(!server.has_candidate());
    assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
}

#[test]
fn old_ack_and_non_responses_are_owned_even_without_an_ack_to_send() {
    for ty in [Type::Acknowledgement, Type::NonConfirmable] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapRecovery::<3>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<3>::new(&b, a.peer(), CLIENT, 0).unwrap();
        let (old, _, _) = exchange_with_response_type(&mut client, &mut server, 0, 3, ty);
        let old_reply = old[1].clone();
        client.start(1, entropy(5)).unwrap();
        assert_eq!(
            client
                .ingest(&old_reply, SERVER, 1, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
        let mut changed = old_reply.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert_eq!(
            client
                .ingest(&changed, SERVER, 1, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
        assert!(client.has_candidate());
        assert_eq!(
            client.active_connection_id(),
            Some(ConnectionId::new(0).unwrap())
        );
    }
}

#[test]
fn revocation_and_wrong_pin_leave_the_confirmed_session_identity_reserved() {
    for authorized in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let wrong = identity(7, 0);
        let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
        exchange(&mut client, &mut server, 0, 3);
        let old_id = server.active_connection_id();
        let source = if authorized { &wrong } else { &a };
        let mut restarted = CoapRecovery::<2>::new(source, b.peer(), SERVER, 100).unwrap();
        let mut a_io = Io::default();
        let mut b_io = Io::default();
        restarted.start(100, entropy(5)).unwrap();
        restarted.flush(&mut a_io, 100).unwrap();
        server
            .ingest(&a_io.last(), CLIENT, 100, entropy(6), no_authorization)
            .unwrap();
        server.flush(&mut b_io, 100).unwrap();
        restarted
            .ingest(&b_io.last(), SERVER, 100, no_entropy, |_| true)
            .unwrap();
        restarted.flush(&mut a_io, 100).unwrap();
        assert_eq!(
            server.ingest(&a_io.last(), CLIENT, 100, no_entropy, |_| authorized),
            Ok(Status::Progress)
        );
        let error = server.flush(&mut b_io, 100);
        assert_eq!(
            error,
            Err(RecoveryError::Bootstrap(PollError::Provisioning(
                if authorized {
                    Error::Authentication
                } else {
                    Error::Unauthorized
                }
            )))
        );
        assert_eq!(server.active_connection_id(), old_id);
        assert!(server.take_session().is_none());
        assert!(!server.has_candidate());
        assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    }
}

#[test]
fn bounded_cache_capacity_never_evicts_old_generations_and_active_id_outlives_cache() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    assert!(matches!(
        CoapRecovery::<0>::new(&a, b.peer(), SERVER, 0),
        Err(RecoveryError::Capacity)
    ));
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let (old, _, _) = exchange(&mut client, &mut server, 0, 3);
    exchange(&mut client, &mut server, 100, 5);
    assert_eq!(client.start(101, no_entropy), Err(RecoveryError::Capacity));
    let active = client.active_connection_id().unwrap();
    let mut io = Io::default();
    server
        .ingest(&old[2], CLIENT, 101, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut io, 101).unwrap();
    assert_eq!(io.last(), old[3]);
    client.flush(&mut io, EXCHANGE_LIFETIME_MS + 100).unwrap();
    assert!(client.caches.iter().all(Option::is_none));
    assert_eq!(client.active_connection_id(), Some(active));
    client
        .start(EXCHANGE_LIFETIME_MS + 100, entropy(7))
        .unwrap();
    assert_ne!(client.candidate.as_ref().unwrap().local_id, active);
}

#[test]
fn handoff_confirmation_and_abandonment_hold_the_old_application_id() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<3>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<3>::new(&b, a.peer(), CLIENT, 0).unwrap();
    exchange(&mut client, &mut server, 0, 3);
    let old_id = client.active_connection_id();
    let mut a_io = Io::default();
    let mut b_io = Io::default();
    client.start(100, entropy(5)).unwrap();
    client.flush(&mut a_io, 100).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 100, entropy(6), |_| true)
        .unwrap();
    server.flush(&mut b_io, 100).unwrap();
    client
        .ingest(&b_io.last(), SERVER, 100, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a_io, 100).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 100, no_entropy, |_| true)
        .unwrap();
    server.flush(&mut b_io, 100).unwrap();
    client
        .ingest(&b_io.last(), SERVER, 100, no_entropy, |_| true)
        .unwrap();
    let _session = client.take_session().unwrap();
    assert!(client.awaiting_handoff());
    assert_eq!(client.active_connection_id(), old_id);
    assert_eq!(client.start(100, no_entropy), Err(RecoveryError::Busy));
    client.flush(&mut a_io, EXCHANGE_LIFETIME_MS + 100).unwrap();
    assert!(client.awaiting_handoff());
    assert_eq!(client.active_connection_id(), old_id);
    assert!(client.abandon_handoff(EXCHANGE_LIFETIME_MS + 100).unwrap());
    assert!(!client.confirm_handoff());
    assert_eq!(client.active_connection_id(), old_id);
    client
        .start(EXCHANGE_LIFETIME_MS + 100, entropy(7))
        .unwrap();
    assert_ne!(client.candidate.as_ref().unwrap().local_id, old_id.unwrap());
}

#[test]
fn timeouts_and_clock_errors_preserve_reservations_and_original_deadlines() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut controller = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut io = Io::default();
    controller.start(0, entropy(3)).unwrap();
    controller.flush(&mut io, 0).unwrap();
    let first_mid = decode(&io.last()).unwrap().message_id();
    assert_eq!(controller.next_deadline(), Some(2000));
    assert_eq!(
        controller.flush(&mut io, 62000),
        Err(RecoveryError::Bootstrap(PollError::Timeout))
    );
    assert!(!controller.has_candidate());
    assert_eq!(controller.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    assert!(controller.reserves_message_id(first_mid, SERVER, 62000));
    controller.start(62000, entropy(5)).unwrap();
    controller.flush(&mut io, 62000).unwrap();
    assert_eq!(
        controller.flush(&mut io, 61999),
        Err(RecoveryError::Bootstrap(PollError::Clock))
    );
    assert!(!controller.has_candidate());
    assert_eq!(
        controller.start(62000, no_entropy),
        Err(RecoveryError::Capacity)
    );
    assert_eq!(controller.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    controller.flush(&mut io, EXCHANGE_LIFETIME_MS).unwrap();
    controller.start(EXCHANGE_LIFETIME_MS, entropy(7)).unwrap();
    controller.discard_candidate(EXCHANGE_LIFETIME_MS).unwrap();
    assert_eq!(
        controller.start(u64::MAX, no_entropy),
        Err(RecoveryError::Bootstrap(PollError::Clock))
    );
}

#[test]
fn failed_or_short_candidate_send_retries_exact_bytes_without_renewing_entropy() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut controller = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut io = Io {
        short: true,
        ..Io::default()
    };
    controller.start(0, entropy(3)).unwrap();
    assert_eq!(
        controller.flush(&mut io, 0),
        Err(RecoveryError::Bootstrap(PollError::ShortSend))
    );
    let original = io.last();
    assert!(controller.has_candidate());
    assert!(controller.take_session().is_none());
    io.short = false;
    controller.flush(&mut io, 0).unwrap();
    assert_eq!(io.last(), original);
    controller.discard_candidate(1).unwrap();
    assert_eq!(
        controller.start(1, no_entropy),
        Err(RecoveryError::Capacity)
    );
    assert_eq!(controller.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
}

#[test]
fn incoming_capacity_and_invalid_profile_are_ignored_without_entropy_or_eviction() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<1>::new(&b, a.peer(), CLIENT, 0).unwrap();
    exchange(&mut client, &mut server, 0, 3);
    let old_id = server.active_connection_id();
    let mut fresh = CoapRecovery::<1>::new(&a, b.peer(), SERVER, 1).unwrap();
    let mut io = Io::default();
    fresh.start(1, entropy(5)).unwrap();
    fresh.flush(&mut io, 1).unwrap();
    assert_eq!(
        server
            .ingest(&io.last(), CLIENT, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert_eq!(server.active_connection_id(), old_id);
    let options = [
        Opt::new(OptionNumber::URI_PATH, b".well-known"),
        Opt::new(OptionNumber::URI_PATH, b"edhoc"),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[65]),
    ];
    let mut bytes = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::con(
        Code::POST,
        MessageId::new(900),
        Token::from_checked(b"invalid"),
    )
    .with_options(&options)
    .with_payload(b"\xf5invalid")
    .encode(&mut bytes)
    .unwrap();
    assert_eq!(
        server
            .ingest(&bytes[..len], CLIENT, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert_eq!(server.active_connection_id(), old_id);
}

#[test]
fn compact_completion_storage_drops_large_handshake_states() {
    assert!(core::mem::size_of::<Cache>() < core::mem::size_of::<CoapProvisioner<'_>>());
    std::println!(
        "Cache bytes={}, CoapProvisioner bytes={}, CoapRecovery<2> bytes={}",
        core::mem::size_of::<Cache>(),
        core::mem::size_of::<CoapProvisioner<'_>>(),
        core::mem::size_of::<CoapRecovery<'_, 2>>()
    );
}

#[test]
fn old_confirmable_message4_keeps_exact_ack_after_fresh_candidate_starts() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut a_io = Io::default();
    let mut b_io = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut a_io, 0).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 0, entropy(4), |_| true)
        .unwrap();
    server.flush(&mut b_io, 0).unwrap();
    let m2 = b_io.last();
    let old_mid = decode(&m2).unwrap().message_id();
    client.ingest(&m2, SERVER, 0, no_entropy, |_| true).unwrap();
    client.flush(&mut a_io, 0).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 0, no_entropy, |_| true)
        .unwrap();
    server.flush(&mut b_io, 0).unwrap();
    let m4 = b_io.last();
    let final_response = decode(&m4).unwrap();
    let separate_m4 = packet(
        &m4,
        Type::Confirmable,
        old_mid,
        final_response.token(),
        final_response.payload(),
    );
    client
        .ingest(&separate_m4, SERVER, 0, no_entropy, |_| true)
        .unwrap();
    assert!(client.take_session().is_none());
    a_io.fail = true;
    assert_eq!(
        client.flush(&mut a_io, 0),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    let original_ack = a_io.last();
    a_io.fail = false;
    client.flush(&mut a_io, 0).unwrap();
    assert_eq!(a_io.last(), original_ack);
    client.take_session().unwrap();
    client.confirm_handoff();
    client.start(1, entropy(5)).unwrap();
    assert_eq!(
        client
            .ingest(&m2, SERVER, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert_eq!(
        client
            .ingest(&separate_m4, SERVER, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Progress
    );
    let before = a_io.sent.len();
    client.flush(&mut a_io, 1).unwrap();
    assert_eq!(a_io.sent[before], original_ack);
    assert_eq!(decode(&original_ack).unwrap().ty(), Type::Acknowledgement);
    assert!(decode(&original_ack).unwrap().payload().is_empty());
    assert!(client.has_candidate());
}

#[test]
fn retiring_a_late_completed_candidate_preserves_its_later_completion_lifetime() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let mut a_io = Io::default();
    let mut b_io = Io::default();
    client.start(0, entropy(3)).unwrap();
    client.flush(&mut a_io, 0).unwrap();
    let mut acknowledgement = [0; 4];
    CoapMessage::empty_ack(decode(&a_io.last()).unwrap().message_id())
        .encode(&mut acknowledgement)
        .unwrap();
    client
        .ingest(&acknowledgement, SERVER, 0, no_entropy, no_authorization)
        .unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 0, entropy(4), |_| true)
        .unwrap();
    server.flush(&mut b_io, 0).unwrap();
    client
        .ingest(&b_io.last(), SERVER, 200_000, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a_io, 200_000).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 200_000, no_entropy, |_| true)
        .unwrap();
    b_io.fail = true;
    assert_eq!(
        server.flush(&mut b_io, 200_000),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    assert_eq!(
        server.flush(&mut b_io, 199_999),
        Err(RecoveryError::Bootstrap(PollError::Clock))
    );
    assert!(!server.has_candidate());
    assert_eq!(server.next_deadline(), Some(200_000 + EXCHANGE_LIFETIME_MS));
    assert_eq!(
        server.caches.iter().flatten().next().unwrap().local_id,
        ConnectionId::new(1).unwrap()
    );
}

#[test]
fn all_compact_ids_reserved_refuses_admission_even_with_a_free_cache_slot() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut controller = CoapRecovery::<24>::new(&a, b.peer(), SERVER, 0).unwrap();
    controller.active = Some(ConnectionId::new(0).unwrap());
    for raw in 1..24 {
        let operation = Operation {
            mid: MessageId::new(u16::from(raw)),
            token: Token::from_checked(&[raw]),
        };
        controller.caches[usize::from(raw)] = Some(Cache {
            control: None,
            local_id: ConnectionId::new(raw).unwrap(),
            expires: EXCHANGE_LIFETIME_MS,
            replies: false,
            role: CachedRole::Client {
                peer_error_slot: None,
                abort: None,
                operations: [operation; 4],
                acks: [None, None, None],
                pending: 0,
            },
        });
    }
    assert!(controller.caches[0].is_none());
    assert_eq!(
        controller.start(0, no_entropy),
        Err(RecoveryError::Capacity)
    );
    assert_eq!(
        controller.active_connection_id(),
        Some(ConnectionId::new(0).unwrap())
    );
}

#[test]
fn new_bootstrap_mid_sampling_is_bounded_and_cannot_reuse_old_operations() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    let (old, _, _) = exchange(&mut client, &mut server, 0, 3);
    let mut calls = 0;
    let mut repeated = entropy(3);
    assert_eq!(
        client.start(1, |bytes| {
            calls += 1;
            repeated(bytes)
        }),
        Err(RecoveryError::Bootstrap(PollError::Provisioning(
            Error::Entropy
        )))
    );
    assert_eq!(calls, 8);
    assert!(!client.has_candidate());
    let echo_mid = decode(&old[0]).unwrap().message_id().wrapping_add(2);
    assert!(client.reserves_message_id(echo_mid, SERVER, 1));
    let mut fresh = entropy(5);
    let mut attempts = 0;
    client
        .start(1, |bytes| {
            if bytes.len() == 22 {
                attempts += 1;
                if attempts == 1 {
                    return repeated(bytes);
                }
            }
            fresh(bytes)
        })
        .unwrap();
    assert_eq!(attempts, 2);
    let mut io = Io::default();
    client.flush(&mut io, 1).unwrap();
    let new_mid = decode(&io.last()).unwrap().message_id();
    assert_ne!(new_mid, decode(&old[0]).unwrap().message_id());
    assert_ne!(
        new_mid.wrapping_add(1),
        decode(&old[2]).unwrap().message_id()
    );
}

fn complete_without_handoff<const A: usize, const B: usize>(
    client: &mut CoapRecovery<'_, A>,
    server: &mut CoapRecovery<'_, B>,
    now: u64,
) -> [Vec<u8>; 4] {
    let mut a_io = Io::default();
    let mut b_io = Io::default();
    client.start(now, entropy(5)).unwrap();
    client.flush(&mut a_io, now).unwrap();
    let m1 = a_io.last();
    server
        .ingest(&m1, CLIENT, now, entropy(6), |_| true)
        .unwrap();
    server.flush(&mut b_io, now).unwrap();
    let m2 = b_io.last();
    client
        .ingest(&m2, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a_io, now).unwrap();
    let m3 = a_io.last();
    server
        .ingest(&m3, CLIENT, now, no_entropy, |_| true)
        .unwrap();
    server.flush(&mut b_io, now).unwrap();
    let m4 = b_io.last();
    client
        .ingest(&m4, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a_io, now).unwrap();
    [m1, m2, m3, m4]
}

#[test]
fn an_expired_authenticated_candidate_releases_busy_without_changing_active_id() {
    for via_ingest in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
        let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
        exchange(&mut client, &mut server, 0, 3);
        let old_client_id = client.active_connection_id();
        let old_server_id = server.active_connection_id();
        let final_messages = complete_without_handoff(&mut client, &mut server, 100);
        let deadline = 100 + EXCHANGE_LIFETIME_MS;
        assert!(client.has_candidate());
        assert!(server.has_candidate());
        assert_eq!(client.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
        assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
        let mut io = Io::default();
        if via_ingest {
            assert_eq!(
                client
                    .ingest(
                        &final_messages[3],
                        SERVER,
                        deadline,
                        no_entropy,
                        no_authorization
                    )
                    .unwrap(),
                Status::Expired
            );
            assert_eq!(
                server
                    .ingest(
                        &final_messages[2],
                        CLIENT,
                        deadline,
                        no_entropy,
                        no_authorization
                    )
                    .unwrap(),
                Status::Expired
            );
        } else {
            assert_eq!(client.flush(&mut io, deadline).unwrap(), Status::Expired);
            assert_eq!(server.flush(&mut io, deadline).unwrap(), Status::Expired);
        }
        assert!(!client.has_candidate());
        assert!(!server.has_candidate());
        assert!(!client.awaiting_handoff());
        assert!(!server.awaiting_handoff());
        assert!(client.take_session().is_none());
        assert!(server.take_session().is_none());
        assert_eq!(client.active_connection_id(), old_client_id);
        assert_eq!(server.active_connection_id(), old_server_id);
        assert_eq!(client.next_deadline(), None);
        assert_eq!(server.next_deadline(), None);
        client.start(deadline, entropy(7)).unwrap();
        assert_ne!(
            Some(client.candidate.as_ref().unwrap().local_id),
            old_client_id
        );
    }
}

#[test]
fn expiry_after_failed_message4_send_does_not_transmit_or_keep_candidate_busy() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapRecovery::<2>::new(&a, b.peer(), SERVER, 0).unwrap();
    let mut server = CoapRecovery::<2>::new(&b, a.peer(), CLIENT, 0).unwrap();
    exchange(&mut client, &mut server, 0, 3);
    let old_id = server.active_connection_id();
    let old_deadline = server.next_deadline();
    let mut a_io = Io::default();
    let mut b_io = Io::default();
    client.start(100, entropy(5)).unwrap();
    client.flush(&mut a_io, 100).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 100, entropy(6), |_| true)
        .unwrap();
    server.flush(&mut b_io, 100).unwrap();
    client
        .ingest(&b_io.last(), SERVER, 100, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut a_io, 100).unwrap();
    server
        .ingest(&a_io.last(), CLIENT, 100, no_entropy, |_| true)
        .unwrap();
    b_io.fail = true;
    assert_eq!(
        server.flush(&mut b_io, 100),
        Err(RecoveryError::Bootstrap(PollError::Io(9)))
    );
    let attempts = b_io.sent.len();
    assert!(server.has_candidate());
    assert!(server.take_session().is_none());
    assert_eq!(
        server.caches.iter().flatten().next().unwrap().expires,
        old_deadline.unwrap()
    );
    assert_eq!(
        server.flush(&mut b_io, 100 + EXCHANGE_LIFETIME_MS).unwrap(),
        Status::Expired
    );
    assert_eq!(b_io.sent.len(), attempts);
    assert!(!server.has_candidate());
    assert!(server.take_session().is_none());
    assert_eq!(server.active_connection_id(), old_id);
    assert_eq!(server.next_deadline(), None);
    server
        .start(100 + EXCHANGE_LIFETIME_MS, entropy(7))
        .unwrap();
}
