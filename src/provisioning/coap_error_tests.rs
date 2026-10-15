use super::*;

fn awaiting_third<'a>(
    a: &'a Identity,
    b: &'a Identity,
    peer: PinnedPeer,
    local_id: ConnectionId,
) -> (CoapProvisioner<'a>, CoapProvisioner<'a>, Vec<u8>) {
    let mut client = CoapProvisioner::start(a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen_with_id(b, peer, CLIENT, local_id, 0);
    let mut ci = Io::default();
    let mut si = Io::default();
    client.flush(&mut ci, 0).unwrap();
    server
        .ingest(&ci.last(), CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut si, 0).unwrap();
    client
        .ingest(&si.last(), SERVER, 0, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut ci, 0).unwrap();
    (client, server, ci.last())
}

fn third_payload(third: &[u8], id: ConnectionId, body: &[u8]) -> Vec<u8> {
    let p = decode(third).unwrap();
    let mut payload = id.lakers().as_cbor().to_vec();
    payload.extend_from_slice(body);
    let options: Vec<_> = p.options().collect();
    recode(third, p.ty(), p.message_id(), p.token(), &options, &payload)
}

#[test]
fn initial_failure_sends_fixed_error_including_equal_id_and_entropy_failure() {
    for case in 0..4 {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
        let local_id = if case == 0 {
            ConnectionId::INITIATOR
        } else {
            ConnectionId::RESPONDER
        };
        let mut server = CoapProvisioner::listen_with_id(&b, a.peer(), CLIENT, local_id, 0);
        let mut ci = Io::default();
        let mut si = Io::default();
        client.flush(&mut ci, 0).unwrap();
        let original = ci.last();
        let p = decode(&original).unwrap();
        let options: Vec<_> = p.options().collect();
        let mut payload = p.payload().to_vec();
        if case == 1 {
            payload[1] = 0;
        }
        if case == 2 {
            payload.truncate(2);
        }
        let message = recode(
            &original,
            p.ty(),
            p.message_id(),
            p.token(),
            &options,
            &payload,
        );
        assert_eq!(
            server.ingest(
                &message,
                CLIENT,
                10,
                |_| {
                    assert_eq!(case, 3, "invalid profile must not request entropy");
                    false
                },
                no_authorization
            ),
            Ok(Status::Progress)
        );
        si.fail = true;
        assert_eq!(server.flush(&mut si, 10), Err(PollError::Io(9)));
        let saved = si.last();
        let response = decode(&saved).unwrap();
        assert_eq!(
            response.code(),
            if case == 3 {
                Code::INTERNAL_SERVER_ERROR
            } else {
                Code::BAD_REQUEST
            }
        );
        assert_eq!(
            PeerError::parse(response.payload()).unwrap().diagnostic(),
            Some("EDHOC failed")
        );
        assert_eq!(response.token(), p.token());
        assert_eq!(response.message_id(), p.message_id());
        si.fail = false;
        let error = if case == 3 {
            Error::Entropy
        } else {
            Error::Profile
        };
        assert_eq!(
            server.flush(&mut si, 11),
            Err(PollError::Provisioning(error))
        );
        assert_eq!(si.last(), saved);
        assert!(server.take_session().is_none());
        client
            .ingest(&saved, SERVER, 11, no_entropy, no_authorization)
            .unwrap();
        assert_eq!(
            client.flush(&mut ci, 11),
            Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
        );
        server
            .ingest(&message, CLIENT, 12, no_entropy, no_authorization)
            .unwrap();
        assert_eq!(
            server.flush(&mut si, 12),
            Err(PollError::Provisioning(error))
        );
        assert_eq!(si.last(), saved);
        assert_eq!(server.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
    }
}

#[test]
fn final_failure_distinguishes_unknown_reference_from_bad_mac_and_denied_policy() {
    for case in 0..4 {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let peer = match case {
            0 => identity(1, 9).peer(),
            1 => identity(7, 0).peer(),
            _ => a.peer(),
        };
        let (mut client, mut server, third) = awaiting_third(&a, &b, peer, ConnectionId::RESPONDER);
        let message = if case == 3 {
            third_payload(&third, ConnectionId::RESPONDER, &[0x41, 0])
        } else {
            third.clone()
        };
        let mut si = Io::default();
        assert_eq!(
            server.ingest(&message, CLIENT, 10, no_entropy, |_| {
                assert_eq!(case, 2, "unverified input must not invoke policy");
                false
            }),
            Ok(Status::Progress)
        );
        let error = match case {
            2 => Error::Unauthorized,
            3 => Error::Parsing,
            _ => Error::Authentication,
        };
        si.short = true;
        assert_eq!(server.flush(&mut si, 10), Err(PollError::ShortSend));
        let saved = si.last();
        let parsed = decode(&saved).unwrap();
        let peer_error = PeerError::parse(parsed.payload()).unwrap();
        let code = if case == 0 {
            PeerErrorCode::UnknownCredential
        } else {
            PeerErrorCode::Unspecified
        };
        assert_eq!(peer_error.code(), code);
        assert_eq!(parsed.code(), Code::BAD_REQUEST);
        si.short = false;
        assert_eq!(
            server.flush(&mut si, 11),
            Err(PollError::Provisioning(error))
        );
        assert_eq!(si.last(), saved);
        let mut ci = Io::default();
        client
            .ingest(&saved, SERVER, 11, no_entropy, no_authorization)
            .unwrap();
        assert_eq!(client.flush(&mut ci, 11), Err(PollError::PeerAbort(code)));
        assert!(client.take_session().is_none());
        assert!(server.take_session().is_none());
    }
}

#[test]
fn peer_error_request_ack_is_retained_without_an_edhoc_error_loop() {
    for local_id in [
        ConnectionId::RESPONDER,
        ConnectionId::from_slice(b"server!").unwrap(),
    ] {
        for (body, code) in [
            (&[0, 0x81][..], PeerErrorCode::ReservedSuccess),
            (
                &[1, 0x64, b's', b't', b'o', b'p'],
                PeerErrorCode::Unspecified,
            ),
            (&[3, 0xf5], PeerErrorCode::UnknownCredential),
        ] {
            let a = identity(1, 0);
            let b = identity(2, 1);
            let (_, mut server, third) = awaiting_third(&a, &b, a.peer(), local_id);
            let message = third_payload(&third, local_id, body);
            let p = decode(&message).unwrap();
            let mut si = Io::default();
            assert_eq!(
                server.ingest(&message, CLIENT, 10, no_entropy, no_authorization),
                Ok(Status::Progress)
            );
            assert_eq!(server.peer_error().unwrap().code(), code);
            assert!(server.take_session().is_none());
            si.fail = true;
            assert_eq!(server.flush(&mut si, 10), Err(PollError::Io(9)));
            let saved = si.last();
            let response = decode(&saved).unwrap();
            assert_eq!(response.ty(), Type::Acknowledgement);
            assert_eq!(response.code(), Code::CHANGED);
            assert_eq!(response.token(), p.token());
            assert_eq!(response.message_id(), p.message_id());
            assert!(response.payload().is_empty());
            assert_eq!(response.options().count(), 0);
            si.fail = false;
            si.short = true;
            assert_eq!(server.flush(&mut si, 11), Err(PollError::ShortSend));
            si.short = false;
            assert_eq!(server.flush(&mut si, 12), Err(PollError::PeerAbort(code)));
            assert_eq!(si.last(), saved);
            let mut changed = message.clone();
            *changed.last_mut().unwrap() ^= 1;
            assert_eq!(
                server.ingest(&changed, CLIENT, 13, no_entropy, no_authorization),
                Ok(Status::Ignored)
            );
            server
                .ingest(&message, CLIENT, 13, no_entropy, no_authorization)
                .unwrap();
            assert_eq!(server.flush(&mut si, 13), Err(PollError::PeerAbort(code)));
            assert_eq!(si.last(), saved);
            assert_eq!(server.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
            assert_eq!(
                server.flush(&mut si, 10 + EXCHANGE_LIFETIME_MS),
                Ok(Status::Expired)
            );
            assert!(server.peer_error().is_none());
        }
    }
}

#[test]
fn wrong_route_malformed_or_response_only_errors_preserve_message3_progress() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let (_, mut server, third) = awaiting_third(&a, &b, a.peer(), ConnectionId::RESPONDER);
    for body in [&[1][..], &[1, 0x41, 0], &[2, 2], &[3, 0xf4], &[23, 0xf6]] {
        let message = third_payload(&third, ConnectionId::RESPONDER, body);
        assert_eq!(
            server.ingest(&message, CLIENT, 1, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
    }
    let message = third_payload(&third, ConnectionId::INITIATOR, &[1, 0x60]);
    assert_eq!(
        server.ingest(&message, CLIENT, 1, no_entropy, no_authorization),
        Ok(Status::Ignored)
    );
    assert_eq!(
        server.ingest(&third, CLIENT, 2, no_entropy, |_| true),
        Ok(Status::Progress)
    );
    let mut si = Io::default();
    assert_eq!(server.flush(&mut si, 2), Ok(Status::Complete));
    assert!(server.take_session().is_some());
}

#[cfg(feature = "std")]
#[test]
fn udp_loopback_delivers_internal_failure_and_then_fresh_authenticated_recovery() {
    use crate::storage::UdpSocketIo;
    use std::{net::UdpSocket, time::Duration};
    let socket = || {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        UdpSocketIo::new(socket, [0; DATAGRAM_CAPACITY + 1]).unwrap()
    };
    let mut ci = socket();
    let mut si = socket();
    let client_endpoint = ci.socket().local_addr().unwrap().into();
    let server_endpoint = si.socket().local_addr().unwrap().into();
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), server_endpoint, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), client_endpoint, 0);
    client.flush(&mut ci, 0).unwrap();
    assert!(matches!(
        server.poll(&mut si, 1, |_| false, no_authorization),
        Err(PollError::Provisioning(Error::Entropy))
    ));
    assert!(matches!(
        client.poll(&mut ci, 1, no_entropy, no_authorization),
        Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
    ));
    assert_eq!(
        client.peer_error().unwrap().diagnostic(),
        Some("EDHOC failed")
    );
    let mut client =
        CoapProvisioner::start(&a, b.peer(), server_endpoint, 2, fresh_operations(5, 50)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), client_endpoint, 2);
    client.flush(&mut ci, 2).unwrap();
    server
        .poll(&mut si, 2, entropy(6), no_authorization)
        .unwrap();
    client.poll(&mut ci, 2, no_entropy, |_| true).unwrap();
    server.poll(&mut si, 2, no_entropy, |_| true).unwrap();
    client.poll(&mut ci, 2, no_entropy, |_| true).unwrap();
    let (client_context, _) = client.take_session().unwrap().into_parts();
    let (server_context, _) = server.take_session().unwrap().into_parts();
    assert_eq!(client_context.sender_key(), server_context.recipient_key());
    assert_eq!(client_context.recipient_key(), server_context.sender_key());
}
