use super::*;
use crate::provisioning::PollError;

type Cloud<'a> = CloudAdmission<'a, 2, 2, 2, 4>;

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        source_datagrams: 64,
        global_crypto: 16,
        source_crypto: 8,
        ..AdmissionLimits::default()
    }
}

fn rebody(bytes: &[u8], body: &[u8], mid: Option<u16>) -> Vec<u8> {
    let p = decode(bytes).unwrap();
    let options: Vec<_> = p.options().collect();
    let mut out = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::con(
        Code::POST,
        mid.map_or(p.message_id(), MessageId::new),
        p.token(),
    )
    .with_options(&options)
    .with_payload(body)
    .encode(&mut out)
    .unwrap();
    out[..len].to_vec()
}

fn awaiting_third<'a>(
    cloud: &mut Cloud<'a>,
    registry: &Registry,
    client: &'a Identity,
    endpoint: Endpoint,
    now: u64,
) -> (CoapProvisioner<'a>, [Vec<u8>; 3]) {
    let mut client =
        CoapProvisioner::start(client, cloud.identity.peer(), SERVER, now, entropy(3)).unwrap();
    let mut ci = Io::default();
    let mut si = Io::default();
    client.flush(&mut ci, now).unwrap();
    cloud
        .ingest(ci.last(), endpoint, now, registry, no_entropy)
        .unwrap();
    cloud.flush(&mut si, now, registry).unwrap();
    client
        .ingest(si.last(), SERVER, now, no_entropy, |_| {
            panic!("Echo cannot authorize")
        })
        .unwrap();
    client.flush(&mut ci, now).unwrap();
    let first = ci.last().to_vec();
    cloud
        .ingest(&first, endpoint, now, registry, entropy(4))
        .unwrap();
    cloud.flush(&mut si, now, registry).unwrap();
    let second = si.last().to_vec();
    client
        .ingest(&second, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut ci, now).unwrap();
    (client, [first, second, ci.last().to_vec()])
}

fn abort_body(template: &[u8], code: &[u8], mid: u16) -> Vec<u8> {
    let p = decode(template).unwrap();
    let (_, prefix) = ConnectionId::decode_prefix(p.payload()).unwrap();
    let mut body = p.payload()[..prefix].to_vec();
    body.extend_from_slice(code);
    rebody(template, &body, Some(mid))
}

#[test]
fn registry_failures_send_the_correct_exact_error_without_repeating_lookup_or_crypto() {
    for case in 0..5 {
        let server = identity(254, 254);
        let client_id = identity(1, 1);
        let other = identity(2, 1);
        let mut registry = match case {
            0 => Registry::new(&server, &[]),
            1 => Registry::new(&server, &[&client_id, &other]),
            2 => Registry::new(&server, &[&other]),
            _ => Registry::new(&server, &[&client_id]),
        };
        let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
        let (mut client, wires) = awaiting_third(&mut cloud, &registry, &client_id, FIRST, 0);
        if case == 3 {
            registry.entries[0]
                .revoke(|_, _, _| Ok::<_, ()>(()))
                .unwrap();
        }
        let third = if case == 4 {
            abort_body(&wires[2], &[0x41, 0], 1)
        } else {
            wires[2].clone()
        };
        assert_eq!(
            cloud.ingest(&third, THIRD, 10, &registry, no_entropy),
            Ok(Status::Ignored)
        );
        assert_eq!(
            cloud.ingest(&third, FIRST, 10, &registry, no_entropy),
            Ok(Status::Progress)
        );
        let local_id = cloud.pending.iter().flatten().next().unwrap().local_id;
        let lookups = registry.lookups.get();
        let crypto = cloud.crypto;
        let mut io = Io {
            fail: true,
            ..Io::default()
        };
        assert_eq!(cloud.flush(&mut io, 10, &registry), Err(CloudError::Io(9)));
        let saved = io.last().to_vec();
        let code = if case < 2 {
            PeerErrorCode::UnknownCredential
        } else {
            PeerErrorCode::Unspecified
        };
        check_response(&third, &saved, Code::BAD_REQUEST);
        assert_eq!(
            PeerError::parse(decode(&saved).unwrap().payload())
                .unwrap()
                .code(),
            code
        );
        io.fail = false;
        io.short = true;
        assert_eq!(
            cloud.flush(&mut io, 11, &registry),
            Err(CloudError::ShortSend)
        );
        io.short = false;
        let error = match case {
            3 => Error::Unauthorized,
            4 => Error::Parsing,
            _ => Error::Authentication,
        };
        assert_eq!(
            cloud.flush(&mut io, 12, &registry),
            Err(CloudError::Provisioning(error))
        );
        assert_eq!(io.last(), saved);
        client
            .ingest(&saved, SERVER, 12, no_entropy, |_| {
                panic!("a failure cannot authorize")
            })
            .unwrap();
        assert_eq!(
            client.flush(&mut Io::default(), 12),
            Err(PollError::PeerAbort(code))
        );
        assert!(client.take_session().is_none());
        assert!(cloud.take_session(12, &registry).unwrap().is_none());
        let mut collision = third.clone();
        *collision.last_mut().unwrap() ^= 1;
        assert_eq!(
            cloud.ingest(&collision, FIRST, 13, &registry, no_entropy),
            Ok(Status::Ignored)
        );
        cloud
            .ingest(&third, FIRST, 13, &registry, no_entropy)
            .unwrap();
        assert_eq!(
            cloud.flush(&mut io, 13, &registry),
            Err(CloudError::Provisioning(error))
        );
        assert_eq!(io.last(), saved);
        assert_eq!(registry.lookups.get(), lookups);
        assert_eq!(cloud.crypto, crypto);
        assert!(cloud.reserves_connection_id(local_id));
        assert_eq!(cloud.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
    }
}

#[test]
fn initial_registry_failures_require_echo_and_cache_suite_profile_key_and_entropy_errors() {
    for case in 0..4 {
        let server = identity(254, 254);
        let client = identity(1, 1);
        let registry = Registry::new(&server, &[&client]);
        let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
        let original = m1(&client, &server, 700, None);
        let mut payload = decode(&original).unwrap().payload().to_vec();
        match case {
            0 => payload[2] = 4,
            1 => payload[1] = 0,
            2 => payload[5..37].fill(0xff),
            _ => {}
        }
        let first = request(&payload, 700, b"first", None);
        cloud
            .ingest(&first, FIRST, 0, &registry, no_entropy)
            .unwrap();
        assert!(cloud.pending.iter().all(Option::is_none));
        assert_eq!(cloud.crypto, 0);
        let mut io = Io::default();
        cloud.flush(&mut io, 0, &registry).unwrap();
        let echo = decode(io.last()).unwrap().echo().unwrap().to_vec();
        let retry = request(&payload, 701, b"retry", Some(&echo));
        cloud
            .ingest(&retry, FIRST, 1, &registry, |_| {
                assert_eq!(case, 3);
                false
            })
            .unwrap();
        let error = match case {
            2 => Error::InvalidKey,
            3 => Error::Entropy,
            _ => Error::Profile,
        };
        assert_eq!(
            cloud.flush(&mut io, 1, &registry),
            Err(CloudError::Provisioning(error))
        );
        let saved = io.last().to_vec();
        check_response(
            &retry,
            &saved,
            if case == 3 {
                Code::INTERNAL_SERVER_ERROR
            } else {
                Code::BAD_REQUEST
            },
        );
        assert_eq!(
            PeerError::parse(decode(&saved).unwrap().payload())
                .unwrap()
                .code(),
            if case == 0 {
                PeerErrorCode::WrongCipherSuite
            } else {
                PeerErrorCode::Unspecified
            }
        );
        if case == 0 {
            assert_eq!(decode(&saved).unwrap().payload(), [2, 0x82, 2, 3]);
        }
        assert_eq!(registry.lookups.get(), 0);
        cloud
            .ingest(&retry, FIRST, 2, &registry, no_entropy)
            .unwrap();
        assert_eq!(
            cloud.flush(&mut io, 2, &registry),
            Err(CloudError::Provisioning(error))
        );
        assert_eq!(io.last(), saved);
        assert!(cloud.take_session(2, &registry).unwrap().is_none());
    }
}

#[test]
fn peer_abort_before_message_three_is_acknowledged_without_lookup_or_crypto() {
    for (body, code) in [
        (&[0, 0xf5][..], PeerErrorCode::ReservedSuccess),
        (
            &[1, 0x64, b's', b't', b'o', b'p'][..],
            PeerErrorCode::Unspecified,
        ),
        (&[3, 0xf5][..], PeerErrorCode::UnknownCredential),
    ] {
        let server = identity(254, 254);
        let client_id = identity(1, 1);
        let registry = Registry::new(&server, &[&client_id]);
        let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
        let (_, wires) = awaiting_third(&mut cloud, &registry, &client_id, FIRST, 0);
        let (_, prefix) =
            ConnectionId::decode_prefix(decode(&wires[2]).unwrap().payload()).unwrap();
        let id = cloud.pending[0].as_ref().unwrap().local_id;
        for invalid in [&[2, 2][..], &[1], &[3, 0xf4]] {
            let invalid = abort_body(&wires[2], invalid, 901);
            assert_eq!(
                cloud.ingest(&invalid, FIRST, 10, &registry, no_entropy),
                Ok(Status::Ignored)
            );
        }
        let abort = abort_body(&wires[2], body, 901);
        let crypto = cloud.crypto;
        let lookups = registry.lookups.get();
        cloud
            .ingest(&abort, FIRST, 10, &registry, no_entropy)
            .unwrap();
        let mut io = Io {
            fail: true,
            ..Io::default()
        };
        assert_eq!(cloud.flush(&mut io, 10, &registry), Err(CloudError::Io(9)));
        let saved = io.last().to_vec();
        let p = decode(&saved).unwrap();
        assert_eq!(p.code(), Code::CHANGED);
        assert!(p.options().next().is_none());
        assert!(p.payload().is_empty());
        io.fail = false;
        assert_eq!(
            cloud.flush(&mut io, 11, &registry),
            Err(CloudError::PeerAbort {
                endpoint: FIRST,
                local_id: id,
                code
            })
        );
        assert_eq!(io.last(), saved);
        assert_eq!(cloud.peer_error(id, FIRST).unwrap().code(), code);
        assert_eq!(
            cloud.peer_error(id, FIRST).unwrap().diagnostic(),
            (code == PeerErrorCode::Unspecified).then_some("stop")
        );
        assert!(cloud.peer_error(id, SECOND).is_none());
        assert_eq!(cloud.crypto, crypto);
        assert_eq!(registry.lookups.get(), lookups);
        assert!(cloud.take_session(11, &registry).unwrap().is_none());
        assert_eq!(
            cloud.ingest(&wires[2], FIRST, 12, &registry, no_entropy),
            Ok(Status::Ignored)
        );
        assert!(prefix > 1, "exercise the seven-byte cloud connection ID");
    }
}

#[test]
fn final_cloud_error_receipt_survives_handoff_and_preserves_original_deadlines() {
    for handoff in [false, true] {
        let server = identity(254, 254);
        let client_id = identity(1, 1);
        let registry = Registry::new(&server, &[&client_id]);
        let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
        let (mut client, wires) = awaiting_third(&mut cloud, &registry, &client_id, FIRST, 0);
        cloud
            .ingest(&wires[2], FIRST, 0, &registry, no_entropy)
            .unwrap();
        let id = cloud.pending[0].as_ref().unwrap().local_id;
        let mut io = Io::default();
        cloud.flush(&mut io, 0, &registry).unwrap();
        let fourth = io.last().to_vec();
        let installed = if handoff {
            Some(cloud.take_session(0, &registry).unwrap().unwrap())
        } else {
            None
        };
        let mut bad = fourth.clone();
        *bad.last_mut().unwrap() ^= 1;
        client
            .ingest(&bad, SERVER, 10, no_entropy, |_| {
                panic!("bad M4 cannot authorize")
            })
            .unwrap();
        let mut ci = Io::default();
        assert_eq!(
            client.flush(&mut ci, 10),
            Err(PollError::Provisioning(Error::Authentication))
        );
        let abort = ci.last().to_vec();
        let mid = decode(&abort).unwrap().message_id();
        let crypto = cloud.crypto;
        cloud
            .ingest(&abort, FIRST, 10, &registry, no_entropy)
            .unwrap();
        assert!(cloud.take_session(10, &registry).unwrap().is_none());
        io.fail = true;
        assert_eq!(cloud.flush(&mut io, 10, &registry), Err(CloudError::Io(9)));
        let saved = io.last().to_vec();
        io.fail = false;
        io.short = true;
        assert_eq!(
            cloud.flush(&mut io, 11, &registry),
            Err(CloudError::ShortSend)
        );
        io.short = false;
        assert_eq!(
            cloud.flush(&mut io, 12, &registry),
            Err(CloudError::PeerAbort {
                endpoint: FIRST,
                local_id: id,
                code: PeerErrorCode::Unspecified
            })
        );
        assert_eq!(io.last(), saved);
        assert_eq!(
            cloud.peer_error(id, FIRST).unwrap().diagnostic(),
            Some("EDHOC failed")
        );
        assert_eq!(cloud.crypto, crypto);
        assert_eq!(
            cloud.ingest(
                &rebody(&abort, decode(&abort).unwrap().payload(), Some(902)),
                FIRST,
                13,
                &registry,
                no_entropy
            ),
            Ok(Status::Ignored)
        );
        cloud
            .ingest(&abort, FIRST, 13, &registry, no_entropy)
            .unwrap();
        cloud.flush(&mut io, 13, &registry).unwrap();
        assert_eq!(io.last(), saved);
        for (request, response) in [(&wires[0], &wires[1]), (&wires[2], &fourth)] {
            cloud
                .ingest(request, FIRST, 14, &registry, no_entropy)
                .unwrap();
            cloud.flush(&mut io, 14, &registry).unwrap();
            assert_eq!(io.last(), response);
        }
        assert!(!cloud.reserves_message_id(
            decode(&wires[2]).unwrap().message_id(),
            FIRST,
            EXCHANGE_LIFETIME_MS
        ));
        assert!(cloud.reserves_message_id(mid, FIRST, EXCHANGE_LIFETIME_MS));
        assert_eq!(cloud.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
        if let Some(installed) = installed {
            let (session, association) = installed.into_parts();
            assert_eq!(session.principal(), &client_id.peer().principal());
            assert!(
                cloud
                    .active
                    .iter()
                    .flatten()
                    .any(|entry| *entry == association)
            );
        } else {
            assert!(cloud.active.iter().all(Option::is_none));
        }
    }
}

#[test]
fn failure_notification_keeps_round_robin_progress_for_a_valid_peer() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let second = identity(2, 2);
    let registry = Registry::new(&server, &[&second]);
    let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
    let (_, a) = awaiting_third(&mut cloud, &registry, &first, FIRST, 0);
    let (mut valid, b) = awaiting_third(&mut cloud, &registry, &second, SECOND, 0);
    cloud
        .ingest(&a[2], FIRST, 10, &registry, no_entropy)
        .unwrap();
    cloud
        .ingest(&b[2], SECOND, 10, &registry, no_entropy)
        .unwrap();
    let mut io = Io::default();
    assert_eq!(
        cloud.flush(&mut io, 10, &registry),
        Err(CloudError::Provisioning(Error::Authentication))
    );
    assert_eq!(io.sent.last().unwrap().0, FIRST);
    assert_eq!(cloud.flush(&mut io, 10, &registry), Ok(Status::Progress));
    assert_eq!(io.sent.last().unwrap().0, SECOND);
    valid
        .ingest(io.last(), SERVER, 10, no_entropy, |_| true)
        .unwrap();
    let admitted = cloud.take_session(10, &registry).unwrap().unwrap();
    let (local, _) = admitted.into_parts().0.into_parts();
    let (peer, _) = valid.take_session().unwrap().into_parts();
    assert_eq!(local.sender_key(), peer.recipient_key());
}

#[test]
fn final_errors_cannot_take_another_exchange_mid_at_the_same_endpoint() {
    let server = identity(254, 254);
    let client_id = identity(1, 1);
    let registry = Registry::new(&server, &[&client_id]);
    let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
    let old = finish(&mut cloud, &registry, &client_id, FIRST, 0);
    let old_id = old.server.association().local_id;
    let (initiator, first) = Initiator::start(&client_id, server.peer(), entropy(5)).unwrap();
    let mut body = Vec::from([0xf5]);
    body.extend_from_slice(first.as_bytes());
    let echo = cloud.echo(FIRST, 1);
    cloud
        .ingest(
            &request(&body, 100, b"new", Some(&echo)),
            FIRST,
            1,
            &registry,
            entropy(6),
        )
        .unwrap();
    let id = cloud.pending[0].as_ref().unwrap().local_id;
    let mut io = Io::default();
    cloud.flush(&mut io, 1, &registry).unwrap();
    let (_, third) = initiator
        .receive_message_2(
            &Message::from_slice(decode(io.last()).unwrap().payload()).unwrap(),
            |_| true,
        )
        .unwrap();
    let mut body = id.lakers().as_cbor().to_vec();
    body.extend_from_slice(third.as_bytes());
    let third = request(&body, 101, b"third", None);
    cloud
        .ingest(&third, FIRST, 2, &registry, no_entropy)
        .unwrap();
    cloud.flush(&mut io, 2, &registry).unwrap();
    let collision = abort_body(&third, &[1, 0x60], 2); // Old retained message 1 owns MID 2.
    assert_eq!(
        cloud.ingest(&collision, FIRST, 3, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.take_session(3, &registry).unwrap().is_some());
    let collision = abort_body(&old.m3, &[1, 0x60], 101); // New retained message 3 owns MID 101.
    assert_eq!(
        cloud.ingest(&collision, FIRST, 3, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.peer_error(id, FIRST).is_none());
    assert!(cloud.peer_error(old_id, FIRST).is_none());
    let allowed = abort_body(&old.m3, &[1, 0x60], 102);
    cloud
        .ingest(&allowed, FIRST, 3, &registry, no_entropy)
        .unwrap();
    assert_eq!(
        cloud.flush(&mut io, 3, &registry),
        Err(CloudError::PeerAbort {
            endpoint: FIRST,
            local_id: old_id,
            code: PeerErrorCode::Unspecified
        })
    );
}

#[test]
fn revoked_registry_policy_stops_a_queued_late_error_receipt_without_releasing_ids() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let mut registry = Registry::new(&server, &[&first]);
    let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
    let result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let id = result.server.association().local_id;
    let late = abort_body(&result.m3, &[1, 0x60], 903);
    cloud
        .ingest(&late, FIRST, 10, &registry, no_entropy)
        .unwrap();
    registry.entries[0]
        .revoke(|_, _, _| Ok::<_, ()>(()))
        .unwrap();
    let mut io = Io::default();
    assert_eq!(cloud.flush(&mut io, 11, &registry), Ok(Status::Idle));
    assert!(io.sent.is_empty());
    assert_eq!(
        cloud.ingest(&late, FIRST, 12, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.reserves_connection_id(id));
    assert!(cloud.reserves_message_id(MessageId::new(903), FIRST, 12));
    assert_eq!(cloud.next_deadline(), Some(10 + EXCHANGE_LIFETIME_MS));
}

#[cfg(feature = "std")]
#[test]
fn registry_udp_loopback_delivers_unknown_credential_then_fresh_authorized_recovery() {
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
    let server = identity(254, 254);
    let client_id = identity(1, 1);
    let mut registry = Registry::new(&server, &[]);
    let mut cloud = Cloud::new(&server, 0, limits(), entropy(9)).unwrap();
    let mut client =
        CoapProvisioner::start(&client_id, server.peer(), server_endpoint, 0, entropy(3)).unwrap();
    client.flush(&mut ci, 0).unwrap();
    let mut received = [0; DATAGRAM_CAPACITY];
    for (host_time, client_time) in [(1, 2), (3, 4)] {
        let (len, from) = si.recv(&mut received).unwrap().unwrap();
        assert_eq!(from, client_endpoint);
        cloud
            .ingest(&received[..len], from, host_time, &registry, entropy(4))
            .unwrap();
        cloud.flush(&mut si, host_time, &registry).unwrap();
        client
            .poll(&mut ci, client_time, no_entropy, |_| true)
            .unwrap();
    }
    let (len, from) = si.recv(&mut received).unwrap().unwrap();
    cloud
        .ingest(&received[..len], from, 5, &registry, no_entropy)
        .unwrap();
    assert!(matches!(
        cloud.flush(&mut si, 5, &registry),
        Err(CloudError::Provisioning(Error::Authentication))
    ));
    assert!(matches!(
        client.poll(&mut ci, 6, no_entropy, |_| panic!(
            "errors cannot authorize"
        )),
        Err(PollError::PeerAbort(PeerErrorCode::UnknownCredential))
    ));
    assert!(client.take_session().is_none());
    assert!(cloud.take_session(6, &registry).unwrap().is_none());
    registry = Registry::new(&server, &[&client_id]);
    let now = 6 + EXCHANGE_LIFETIME_MS;
    let mut client =
        CoapProvisioner::start(&client_id, server.peer(), server_endpoint, now, entropy(5))
            .unwrap();
    client.flush(&mut ci, now).unwrap();
    for step in 1..=3 {
        let (len, from) = si.recv(&mut received).unwrap().unwrap();
        cloud
            .ingest(&received[..len], from, now + step, &registry, entropy(6))
            .unwrap();
        cloud.flush(&mut si, now + step, &registry).unwrap();
        client
            .poll(&mut ci, now + step, no_entropy, |_| true)
            .unwrap();
    }
    let admitted = cloud.take_session(now + 3, &registry).unwrap().unwrap();
    assert_eq!(admitted.association().endpoint(), client_endpoint);
    let (server, _) = admitted.into_parts().0.into_parts();
    let (client, _) = client.take_session().unwrap().into_parts();
    assert_eq!(client.sender_key(), server.recipient_key());
    assert_eq!(client.recipient_key(), server.sender_key());
}
