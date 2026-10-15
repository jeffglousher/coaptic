use super::*;

#[test]
fn final_error_owns_one_receipt_after_handoff_and_preserves_original_replies() {
    for handoff in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let (mut client, mut server, second) = awaiting_second(&a, &b, b.peer(), false);
        let mut ci = Io::default();
        let mut si = Io::default();
        client
            .ingest(&second, SERVER, 1, no_entropy, |_| true)
            .unwrap();
        client.flush(&mut ci, 1).unwrap();
        let third = ci.last();
        server
            .ingest(&third, CLIENT, 1, no_entropy, |_| true)
            .unwrap();
        server.flush(&mut si, 1).unwrap();
        let fourth = si.last();
        let first = match &server.role {
            Role::Server(server) => server.exchanges[0]
                .as_ref()
                .unwrap()
                .request
                .as_slice()
                .to_vec(),
            _ => unreachable!(),
        };
        let installed = if handoff {
            Some(server.take_session().unwrap())
        } else {
            None
        };
        let mut payload = decode(&fourth).unwrap().payload().to_vec();
        *payload.last_mut().unwrap() ^= 1;
        client
            .ingest(
                &response_payload(&fourth, Type::Acknowledgement, MessageId::new(1), &payload),
                SERVER,
                2,
                no_entropy,
                no_authorization,
            )
            .unwrap();
        flush_aborted(&mut client, &mut ci, 2);
        let final_error = ci.last();
        let p = decode(&final_error).unwrap();
        let options: Vec<_> = p.options().collect();
        for payload in [&[1, 2, 2][..], &[1, 3, 0xf5], &[7, 1, 0x60], &[1, 1]] {
            let invalid = recode(
                &final_error,
                p.ty(),
                p.message_id(),
                p.token(),
                &options,
                payload,
            );
            assert_eq!(
                server.ingest(&invalid, CLIENT, 3, no_entropy, no_authorization),
                Ok(Status::Ignored)
            );
        }
        assert_eq!(
            server.ingest(&final_error, OTHER, 9, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        server
            .ingest(&final_error, CLIENT, 10, no_entropy, no_authorization)
            .unwrap();
        assert!(server.take_session().is_none());
        assert_eq!(
            server.peer_error().unwrap().diagnostic(),
            Some("EDHOC failed")
        );
        si.fail = true;
        assert_eq!(server.flush(&mut si, 10), Err(PollError::Io(9)));
        let saved = si.last();
        let reply = decode(&saved).unwrap();
        assert_eq!(reply.code(), Code::CHANGED);
        assert_eq!(reply.message_id(), p.message_id());
        assert_eq!(reply.token(), p.token());
        assert!(reply.options().next().is_none());
        assert!(reply.payload().is_empty());
        si.fail = false;
        si.short = true;
        assert_eq!(server.flush(&mut si, 11), Err(PollError::ShortSend));
        si.short = false;
        assert_eq!(
            server.flush(&mut si, 12),
            Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
        );
        assert_eq!(si.last(), saved);
        assert_eq!(server.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
        let fresh_mid = recode(
            &final_error,
            p.ty(),
            p.message_id().wrapping_add(1),
            p.token(),
            &options,
            p.payload(),
        );
        assert_eq!(
            server.ingest(&fresh_mid, CLIENT, 13, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        let mut collision = final_error.clone();
        *collision.last_mut().unwrap() ^= 1;
        assert_eq!(
            server.ingest(&collision, CLIENT, 13, no_entropy, no_authorization),
            Ok(Status::Ignored)
        );
        for (request, response) in [(&first, &second), (&third, &fourth), (&final_error, &saved)] {
            server
                .ingest(request, CLIENT, 14, no_entropy, no_authorization)
                .unwrap();
            assert_eq!(
                server.flush(&mut si, 14),
                Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
            );
            assert_eq!(&si.last(), response);
        }
        assert_eq!(
            server.ingest(
                &third,
                CLIENT,
                1 + EXCHANGE_LIFETIME_MS,
                no_entropy,
                no_authorization
            ),
            Ok(Status::Ignored)
        );
        server
            .ingest(
                &final_error,
                CLIENT,
                1 + EXCHANGE_LIFETIME_MS,
                no_entropy,
                no_authorization,
            )
            .unwrap();
        assert_eq!(
            server.flush(&mut si, 1 + EXCHANGE_LIFETIME_MS),
            Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
        );
        assert_eq!(si.last(), saved);
        assert_eq!(
            server.flush(&mut si, 10 + EXCHANGE_LIFETIME_MS),
            Ok(Status::Expired)
        );
        assert!(server.peer_error().is_none());
        if let Some(installed) = installed {
            assert_eq!(installed.principal(), &a.peer().principal());
            assert_eq!(installed.into_parts().0.recipient_id(), &[1]);
        }
    }
}

#[cfg(feature = "std")]
#[test]
fn final_error_udp_loopback_finishes_the_message_four_failure_after_server_handoff() {
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
    let ce = ci.socket().local_addr().unwrap().into();
    let se = si.socket().local_addr().unwrap().into();
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), se, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), ce, 0);
    client.flush(&mut ci, 0).unwrap();
    server
        .poll(&mut si, 1, entropy(4), no_authorization)
        .unwrap();
    client.poll(&mut ci, 2, no_entropy, |_| true).unwrap();
    let mut third = [0; DATAGRAM_CAPACITY];
    let (len, from) = si.recv(&mut third).unwrap().unwrap();
    server
        .ingest(&third[..len], from, 3, no_entropy, |_| true)
        .unwrap();
    let mut saved = Io::default();
    server.flush(&mut saved, 3).unwrap();
    let installed = server.take_session().unwrap();
    let fourth = saved.last();
    let p = decode(&fourth).unwrap();
    let mut payload = p.payload().to_vec();
    *payload.last_mut().unwrap() ^= 1;
    let invalid = response_payload(&fourth, p.ty(), p.message_id(), &payload);
    si.send(ce, &invalid).unwrap();
    assert!(matches!(
        client.poll(&mut ci, 4, no_entropy, no_authorization),
        Err(PollError::Provisioning(Error::Authentication))
    ));
    assert!(matches!(
        server.poll(&mut si, 5, no_entropy, no_authorization),
        Err(PollError::PeerAbort(PeerErrorCode::Unspecified))
    ));
    assert!(matches!(
        client.poll(&mut ci, 6, no_entropy, no_authorization),
        Err(PollError::Provisioning(Error::Authentication))
    ));
    assert!(client.take_session().is_none());
    assert_eq!(installed.principal(), &a.peer().principal());
    assert_eq!(client.next_deadline(), Some(4 + EXCHANGE_LIFETIME_MS));
}
