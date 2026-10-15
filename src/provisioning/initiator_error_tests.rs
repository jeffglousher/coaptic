use super::*;

#[path = "final_error_tests.rs"]
mod final_errors;

fn awaiting_second<'a>(
    a: &'a Identity,
    b: &'a Identity,
    peer: PinnedPeer,
    echo: bool,
) -> (CoapProvisioner<'a>, CoapProvisioner<'a>, Vec<u8>) {
    let mut client = CoapProvisioner::start(a, peer, SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(b, a.peer(), CLIENT, 0);
    let mut ci = Io::default();
    let mut si = Io::default();
    client.flush(&mut ci, 0).unwrap();
    if echo {
        client
            .ingest(
                &echo_response(&ci.last(), b"reachability"),
                SERVER,
                0,
                no_entropy,
                no_authorization,
            )
            .unwrap();
        client.flush(&mut ci, 0).unwrap();
    }
    server
        .ingest(&ci.last(), CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut si, 0).unwrap();
    (client, server, si.last())
}

fn response_payload(bytes: &[u8], ty: Type, mid: MessageId, payload: &[u8]) -> Vec<u8> {
    let p = decode(bytes).unwrap();
    let options: Vec<_> = p.options().collect();
    recode(bytes, ty, mid, p.token(), &options, payload)
}

fn receipt(request: &[u8], ty: Type, mid: MessageId, critical: bool) -> Vec<u8> {
    let p = decode(request).unwrap();
    let options = [Opt::new(OptionNumber::new(99), &[])];
    let mut bytes = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::new(ty, Code::CHANGED, mid)
        .with_token(p.token())
        .with_options(if critical { &options } else { &[] })
        .encode(&mut bytes)
        .unwrap();
    bytes[..len].to_vec()
}

fn assert_error_request(bytes: &[u8], mid: u16, peer_id: u8, code: PeerErrorCode) {
    let p = decode(bytes).unwrap();
    assert_eq!(p.ty(), Type::Confirmable);
    assert_eq!(p.code(), Code::POST);
    assert_eq!(p.message_id(), MessageId::new(mid));
    assert_eq!(p.payload()[0], peer_id);
    assert_eq!(PeerError::parse(&p.payload()[1..]).unwrap().code(), code);
    assert!(p.options().any(|option| option.number() == OptionNumber::CONTENT_FORMAT && option.value() == [65]));
}

fn flush_aborted(client: &mut CoapProvisioner<'_>, io: &mut Io, now: u64) {
    assert!(matches!(
        client.flush(io, now),
        Err(PollError::Provisioning(_))
    ));
}

#[test]
fn second_message_failures_route_errors_including_equal_connection_ids() {
    for case in 0..4 {
        for ty in [
            Type::Acknowledgement,
            Type::Confirmable,
            Type::NonConfirmable,
        ] {
            let a = identity(1, 0);
            let b = identity(2, 1);
            let peer = if case == 0 {
                identity(2, 9).peer()
            } else {
                b.peer()
            };
            let (mut client, mut server, good) = awaiting_second(&a, &b, peer, false);
            let mut payload = decode(&good).unwrap().payload().to_vec();
            if case == 2 {
                *payload.last_mut().unwrap() ^= 1;
            } else if case == 3 {
                let combined_len = crate::provisioning::lakers::CBORDecoder::new(&payload)
                    .bytes()
                    .unwrap()
                    .len();
                let start =
                    payload.len() - combined_len + crate::provisioning::lakers::P256_ELEM_LEN;
                payload[start] ^= 1; // C_R=1 becomes C_I=0 before MAC verification.
            }
            let mid = if ty == Type::Acknowledgement {
                MessageId::new(0)
            } else {
                MessageId::new(900)
            };
            let bad = response_payload(&good, ty, mid, &payload);
            let policy = |_: &Principal| {
                assert_eq!(case, 1);
                false
            };
            assert_eq!(
                client.ingest(&bad, OTHER, 10, no_entropy, no_authorization),
                Ok(Status::Ignored)
            );
            assert_eq!(
                client.ingest(&bad, SERVER, 10, no_entropy, policy),
                Ok(Status::Progress)
            );
            let mut ci = Io::default();
            let error = match case {
                1 => Error::Unauthorized,
                3 => Error::Profile,
                _ => Error::Authentication,
            };
            assert_eq!(
                client.flush(&mut ci, 10),
                Err(PollError::Provisioning(error))
            );
            let sent = ci.last();
            let code = if case == 0 {
                PeerErrorCode::UnknownCredential
            } else {
                PeerErrorCode::Unspecified
            };
            assert_error_request(&sent, 1, if case == 3 { 0 } else { 1 }, code);
            if ty == Type::Confirmable {
                assert_eq!(ci.attempts.len(), 2);
                assert!(decode(&ci.attempts[0].1).unwrap().is_empty_ack());
                assert_eq!(decode(&ci.attempts[0].1).unwrap().message_id(), mid);
            } else {
                assert_eq!(ci.attempts.len(), 1);
            }
            assert!(client.take_session().is_none());
            let mut collision = bad.clone();
            *collision.last_mut().unwrap() ^= 1;
            assert_eq!(
                client.ingest(&collision, SERVER, 11, no_entropy, no_authorization),
                Ok(Status::Ignored)
            );
            if case != 3 {
                server
                    .ingest(&sent, CLIENT, 11, no_entropy, no_authorization)
                    .unwrap();
                let mut si = Io::default();
                assert_eq!(server.flush(&mut si, 11), Err(PollError::PeerAbort(code)));
                let reply = si.last();
                assert!(decode(&reply).unwrap().payload().is_empty());
                assert_eq!(
                    client.ingest(&reply, SERVER, 11, no_entropy, no_authorization),
                    Ok(Status::Progress)
                );
                let sends = ci.attempts.len();
                flush_aborted(&mut client, &mut ci, 2010);
                assert_eq!(ci.attempts.len(), sends);
                assert!(server.take_session().is_none());
            }
        }
    }
}

#[test]
fn error_request_retries_exact_bytes_and_does_not_extend_the_wait() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let (mut client, _, second) = awaiting_second(&a, &b, identity(2, 9).peer(), false);
    client
        .ingest(&second, SERVER, 10, no_entropy, no_authorization)
        .unwrap();
    let mut ci = Io {
        fail: true,
        ..Io::default()
    };
    assert_eq!(client.flush(&mut ci, 10), Err(PollError::Io(9)));
    let saved = ci.last();
    let early = receipt(&saved, Type::Acknowledgement, MessageId::new(1), false);
    assert_eq!(
        client.ingest(&early, SERVER, 11, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    ci.fail = false;
    ci.short = true;
    assert_eq!(client.flush(&mut ci, 11), Err(PollError::ShortSend));
    ci.short = false;
    assert_eq!(
        client.flush(&mut ci, 12),
        Err(PollError::Provisioning(Error::Authentication))
    );
    assert_eq!(ci.last(), saved);
    for now in [2012, 6012, 14012, 30012] {
        flush_aborted(&mut client, &mut ci, now);
        assert_eq!(ci.last(), saved);
    }
    assert_eq!(ci.attempts.len(), 7); // two failed sends, initial send, four retries
    assert_eq!(client.next_deadline(), Some(62012));
    flush_aborted(&mut client, &mut ci, 62012);
    assert_eq!(client.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
    assert_eq!(
        client.flush(&mut ci, 10 + EXCHANGE_LIFETIME_MS),
        Ok(Status::Expired)
    );
    assert_eq!(ci.attempts.len(), 7);
    assert!(client.take_session().is_none());
}

#[test]
fn error_request_receipts_stop_retries_and_separate_responses_are_exactly_acknowledged() {
    for critical in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let (mut client, _, second) = awaiting_second(&a, &b, identity(2, 9).peer(), false);
        client
            .ingest(&second, SERVER, 10, no_entropy, no_authorization)
            .unwrap();
        let mut ci = Io::default();
        assert_eq!(
            client.flush(&mut ci, 10),
            Err(PollError::Provisioning(Error::Authentication))
        );
        let sent = ci.last();
        let empty = [0x60, 0, 0, 1];
        assert_eq!(
            client.ingest(&empty, SERVER, 11, no_entropy, no_authorization),
            Ok(Status::Progress)
        );
        flush_aborted(&mut client, &mut ci, 2010);
        assert_eq!(ci.attempts.len(), 1);
        let separate = receipt(&sent, Type::Confirmable, MessageId::new(901), critical);
        assert_eq!(
            client.ingest(&separate, OTHER, 2011, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        client
            .ingest(&separate, SERVER, 2011, no_entropy, no_authorization)
            .unwrap();
        ci.fail = true;
        assert_eq!(client.flush(&mut ci, 2011), Err(PollError::Io(9)));
        let ack = ci.last();
        assert_eq!(decode(&ack).unwrap().is_empty_rst(), critical);
        assert_eq!(decode(&ack).unwrap().is_empty_ack(), !critical);
        ci.fail = false;
        flush_aborted(&mut client, &mut ci, 2012);
        assert_eq!(ci.last(), ack);
        let mut collision = separate.clone();
        collision[1] = Code::BAD_REQUEST.as_raw();
        assert_eq!(
            client.ingest(&collision, SERVER, 2013, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        client
            .ingest(&separate, SERVER, 2013, no_entropy, no_authorization)
            .unwrap();
        flush_aborted(&mut client, &mut ci, 2013);
        assert_eq!(ci.last(), ack);
        assert!(client.take_session().is_none());
    }
}

#[test]
fn fourth_message_failure_uses_a_new_mid_after_echo_and_never_exposes_a_session() {
    for oversized in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let (mut client, mut server, second) = awaiting_second(&a, &b, b.peer(), true);
        client
            .ingest(&second, SERVER, 1, no_entropy, |_| true)
            .unwrap();
        let mut ci = Io::default();
        let mut si = Io::default();
        client.flush(&mut ci, 1).unwrap();
        assert_eq!(decode(&ci.last()).unwrap().message_id(), MessageId::new(1));
        server
            .ingest(&ci.last(), CLIENT, 1, no_entropy, |_| true)
            .unwrap();
        server.flush(&mut si, 1).unwrap();
        let good = si.last();
        let mut payload = if oversized {
            std::vec![0; crate::provisioning::lakers::MAX_MESSAGE_SIZE_LEN + 1]
        } else {
            decode(&good).unwrap().payload().to_vec()
        };
        *payload.last_mut().unwrap() ^= 1;
        let bad = response_payload(&good, Type::Confirmable, MessageId::new(902), &payload);
        client
            .ingest(&bad, SERVER, 2, no_entropy, no_authorization)
            .unwrap();
        assert!(matches!(
            client.flush(&mut ci, 2),
            Err(PollError::Provisioning(_))
        ));
        assert_error_request(&ci.last(), 3, 1, PeerErrorCode::Unspecified);
        assert!(client.take_session().is_none());
        server
            .ingest(&ci.last(), CLIENT, 3, no_entropy, no_authorization)
            .unwrap();
        assert_eq!(
            server.flush(&mut si, 3),
            Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
        );
        assert!(server.take_session().is_none());
        let ack = si.last();
        client
            .ingest(&ack, SERVER, 3, no_entropy, no_authorization)
            .unwrap();
        let sends = ci.attempts.len();
        flush_aborted(&mut client, &mut ci, 2002);
        assert_eq!(ci.attempts.len(), sends);
    }
}

#[cfg(feature = "std")]
#[test]
fn initiator_error_udp_loopback_completes_abort_then_fresh_authentication() {
    use crate::storage::UdpSocketIo;
    use std::{net::UdpSocket, time::Duration};
    let socket = || {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
    };
    let ca = socket();
    let sa = socket();
    let ce = Endpoint::from(ca.local_addr().unwrap());
    let se = Endpoint::from(sa.local_addr().unwrap());
    let mut ci = UdpSocketIo::new(ca, [0; DATAGRAM_CAPACITY + 1]).unwrap();
    let mut si = UdpSocketIo::new(sa, [0; DATAGRAM_CAPACITY + 1]).unwrap();
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, identity(2, 9).peer(), se, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), ce, 0);
    client.flush(&mut ci, 0).unwrap();
    server
        .poll(&mut si, 1, entropy(4), no_authorization)
        .unwrap();
    assert!(matches!(
        client.poll(&mut ci, 2, no_entropy, no_authorization),
        Err(PollError::Provisioning(Error::Authentication))
    ));
    assert!(matches!(
        server.poll(&mut si, 3, no_entropy, no_authorization),
        Err(PollError::PeerAbort(PeerErrorCode::UnknownCredential))
    ));
    assert!(matches!(
        client.poll(&mut ci, 4, no_entropy, no_authorization),
        Err(PollError::Provisioning(Error::Authentication))
    ));
    assert!(client.take_session().is_none());
    assert!(server.take_session().is_none());
    let mut client = CoapProvisioner::start(&a, b.peer(), se, 10, fresh_operations(5, 20)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), ce, 10);
    client.flush(&mut ci, 10).unwrap();
    server
        .poll(&mut si, 11, entropy(6), no_authorization)
        .unwrap();
    client.poll(&mut ci, 12, no_entropy, |_| true).unwrap();
    server.poll(&mut si, 13, no_entropy, |_| true).unwrap();
    client.poll(&mut ci, 14, no_entropy, |_| true).unwrap();
    let client = client.take_session().unwrap();
    let server = server.take_session().unwrap();
    assert_eq!(client.principal(), &b.peer().principal());
    let (client, _) = client.into_parts();
    let (server, _) = server.into_parts();
    assert_eq!(client.sender_key(), server.recipient_key());
    assert_eq!(client.recipient_key(), server.sender_key());
}
