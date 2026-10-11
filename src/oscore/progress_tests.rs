//! Finite in-memory qualification, not a sustained network availability claim.
use super::*;
use crate::{App, CallFailure, Request, Response, get, profiles};

fn content(_: Request<'_>) -> Response<'static> {
    Response::content(b"complete authenticated response")
}

fn rejected_traffic_progress(expire: bool) {
    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);
    let stranger = Endpoint::v4([192, 0, 2, 3], 5683);
    let mut server = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route("value", get(content))
        .oscore(server_c1())
        .bind(Loopback::default())
        .unwrap();
    let mut client = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .oscore(client_c1())
        .bind(Loopback::default())
        .unwrap();
    let deadline = if expire { 2560 } else { 3000 };
    let call = client
        .get("value")
        .to(server_ep)
        .deadline(deadline)
        .send(0)
        .unwrap();
    let original = client.transport().last_send.unwrap();
    server.transport_mut().inbox = Some((client_ep, original.1, original.2));
    server.poll(0).unwrap();
    let (_, valid, valid_n) = server.transport().last_send.unwrap();
    let mut corrupt = valid;
    corrupt[valid_n - 1] ^= 1;
    let request = decode(&original.1[..original.2]).unwrap();
    let mut plain = [0; 256];
    let plain_n = encode(
        &Message::new(Type::Acknowledgement, Code::CONTENT, request.message_id())
            .with_token(request.token())
            .with_payload(b"unauthenticated"),
        &mut plain,
    )
    .unwrap();
    let checkpoint = client.oscore().unwrap().replay_checkpoint();

    // A packet on every poll, including the exact RTO. Cover forged responses
    // from the expected endpoint as well as a different peer and bad framing.
    for index in 0..256 {
        let now = index * 10;
        let packet = match index % 4 {
            0 => (server_ep, corrupt, valid_n),
            1 => (server_ep, plain, plain_n),
            2 => (stranger, corrupt, valid_n),
            _ => (stranger, [0; 256], 1),
        };
        client.transport_mut().last_send = None;
        client.transport_mut().inbox = Some(packet);
        client.poll(now).unwrap();
        assert!(client.take_response(call).is_none());
        assert_eq!(client.oscore().unwrap().replay_checkpoint(), checkpoint);
        assert_eq!(client.engine_mut().rx_occupied(), 0);
        assert_eq!(client.engine_mut().tx_occupied(), 1);
        if now == u64::from(crate::message::Transmission::ACK_TIMEOUT_MS) {
            assert_eq!(client.transport().last_send, Some(original));
        } else {
            assert!(client.transport().last_send.is_none());
        }
    }
    assert_eq!(client.metrics().con_retransmit, 1);
    if expire {
        client.transport_mut().inbox = Some((server_ep, corrupt, valid_n));
        client.poll(deadline).unwrap();
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            CallFailure::DeadlineExceeded
        );
        // A delayed authentic response cannot resurrect the expired Call.
        client.transport_mut().inbox = Some((server_ep, valid, valid_n));
        client.poll(deadline + 1).unwrap();
        assert!(client.take_response(call).is_none());
    } else {
        client.transport_mut().inbox = Some((server_ep, valid, valid_n));
        client.poll(2560).unwrap();
        let response = client.take_response(call).unwrap().unwrap();
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), b"complete authenticated response");
    }
    assert_eq!(client.engine_mut().tx_occupied(), 0);
    assert_eq!(client.engine_mut().rx_occupied(), 0);

    // Exercise repeated binding reuse after refusal/expiry; the endpoint
    // must remain usable beyond one fresh request.
    for round in 0..8 {
        let now = 4000 + round;
        let fresh = client.get("value").to(server_ep).send(now).unwrap();
        let (_, bytes, n) = client.transport().last_send.unwrap();
        server.transport_mut().inbox = Some((client_ep, bytes, n));
        server.poll(now).unwrap();
        let (_, bytes, n) = server.transport().last_send.unwrap();
        client.transport_mut().inbox = Some((server_ep, bytes, n));
        client.poll(now).unwrap();
        let response = client.take_response(fresh).unwrap().unwrap();
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), b"complete authenticated response");
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        assert_eq!(client.engine_mut().rx_occupied(), 0);
    }
}

#[test]
fn protected_rejected_traffic_preserves_retry_and_valid_completion() {
    rejected_traffic_progress(false);
}

#[test]
fn protected_rejected_traffic_preserves_deadline_and_recovery() {
    rejected_traffic_progress(true);
}

#[test]
fn checked_clock_refusal_preserves_protected_retry_deadline_and_recovery() {
    use crate::platform::{CheckedClock, ClockError};
    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut client = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .oscore(client_c1())
        .bind(Loopback::default())
        .unwrap();
    let mut server = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route("value", get(content))
        .oscore(server_c1())
        .bind(Loopback::default())
        .unwrap();
    let mut samples = [
        Ok(100),
        Ok(200),
        Err("offline"),
        Ok(199),
        Ok(2100),
        Ok(3100),
        Ok(3101),
    ]
    .into_iter();
    let mut clock = CheckedClock::new(|| samples.next().unwrap());
    let call = client
        .get("value")
        .to(server_ep)
        .deadline(3100)
        .send(clock.now_ms().unwrap())
        .unwrap();
    let original = client.transport().last_send.unwrap();
    client.poll(clock.now_ms().unwrap()).unwrap();
    client.transport_mut().last_send = None;
    // Leave a packet queued: refused samples must not enter poll or consume it.
    let queued = (server_ep, [0; 256], 1);
    client.transport_mut().inbox = Some(queued);
    let replay = client.oscore().unwrap().replay_checkpoint();
    for expected in [
        ClockError::Source("offline"),
        ClockError::Regressed {
            previous_ms: 200,
            observed_ms: 199,
        },
    ] {
        let result = clock.now_ms().map(|now| client.poll(now).unwrap());
        assert_eq!(result, Err(expected));
        assert_eq!(client.transport().inbox, Some(queued));
        assert!(client.transport().last_send.is_none());
        assert_eq!(client.oscore().unwrap().replay_checkpoint(), replay);
        assert_eq!(client.engine_mut().tx_occupied(), 1);
        assert!(client.take_response(call).is_none());
    }
    client.poll(clock.now_ms().unwrap()).unwrap();
    assert_eq!(client.transport().last_send, Some(original));
    assert_eq!(client.metrics().con_retransmit, 1);
    client.poll(clock.now_ms().unwrap()).unwrap();
    assert_eq!(
        client.take_response(call).unwrap().unwrap_err(),
        CallFailure::DeadlineExceeded
    );
    assert_eq!(client.engine_mut().tx_occupied(), 0);

    let now = clock.now_ms().unwrap();
    let fresh = client.get("value").to(server_ep).send(now).unwrap();
    let (_, bytes, n) = client.transport().last_send.unwrap();
    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(now).unwrap();
    let (_, bytes, n) = server.transport().last_send.unwrap();
    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(now).unwrap();
    let response = client.take_response(fresh).unwrap().unwrap();
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"complete authenticated response");
    assert_eq!(client.engine_mut().tx_occupied(), 0);
}
