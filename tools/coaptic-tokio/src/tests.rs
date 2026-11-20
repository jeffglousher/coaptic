use super::*;
use coaptic::{
    App, Code, ContentFormat, Error, Request, Response,
    app::AsyncCallError,
    get,
    oscore::{BoundedSecurityContext, DeriveParams},
    platform::AsyncTimer,
    resources::SmallDevice,
};
use core::{
    future::{Future, pending, poll_fn},
    pin::pin,
    task::Poll,
};

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(5), test)
                .await
                .unwrap()
        });
}

fn socket<const N: usize>() -> Udp<[u8; N]> {
    Udp::from_std(std::net::UdpSocket::bind("127.0.0.1:0").unwrap(), [0; N]).unwrap()
}

#[test]
fn adoption_preserves_options_pending_packet_and_returns_both_owners() {
    run(async {
        let raw = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        raw.set_ttl(37).unwrap();
        let address = raw.local_addr().unwrap();
        let mut sender = socket::<65>();
        sender.send(address.into(), b"queued").await.unwrap();
        let mut scratch = [0; 65];
        let pointer = scratch.as_ptr();
        let registered = UdpSocket::from_std({
            raw.set_nonblocking(true).unwrap();
            raw
        })
        .unwrap();
        let mut receiver = Udp::new(registered, &mut scratch[..]).unwrap();
        assert_eq!(receiver.socket().ttl().unwrap(), 37);
        assert_eq!(receiver.local_endpoint().unwrap(), address.into());
        let mut bytes = [0; 64];
        let (n, peer) = receiver.recv(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..n], b"queued");
        assert_eq!(peer, sender.local_endpoint().unwrap());
        let (socket, scratch) = receiver.into_parts();
        assert_eq!(scratch.as_ptr(), pointer);
        assert_eq!(socket.local_addr().unwrap(), address);
        assert_eq!(socket.ttl().unwrap(), 37);
        socket
            .send_to(b"returned", std::net::SocketAddr::from(peer))
            .await
            .unwrap();
        let (n, _) = sender.recv(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..n], b"returned");
    });
}

#[test]
fn adoption_without_runtime_and_empty_scratch_refuse() {
    let raw = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    assert!(Udp::from_std(raw, [0; 1]).is_err());
    run(async {
        let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        assert_eq!(
            Udp::new(raw, []).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    });
}

#[test]
fn pending_receive_cancels_without_consuming_next_packet() {
    run(async {
        let mut receiver = socket::<65>();
        let mut sender = socket::<65>();
        let mut bytes = [0xa5; 64];
        {
            let mut receive = pin!(receiver.recv(&mut bytes));
            poll_fn(|cx| {
                assert!(receive.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(bytes, [0xa5; 64]);
        sender
            .send(receiver.local_endpoint().unwrap(), b"after cancellation")
            .await
            .unwrap();
        let (n, _) = receiver.recv(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..n], b"after cancellation");
    });
}

#[test]
fn oversized_packet_refuses_then_exact_capacity_and_empty_packet_recover() {
    run(async {
        let mut receiver = socket::<33>();
        let mut sender = socket::<33>();
        let address = receiver.local_endpoint().unwrap();
        let mut bytes = [0xa5; 32];
        for size in [33, 64] {
            sender.send(address, &[0x42; 64][..size]).await.unwrap();
            assert_eq!(
                receiver.recv(&mut bytes).await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(bytes, [0xa5; 32]);
        }
        sender.send(address, &[0x43; 32]).await.unwrap();
        assert_eq!(receiver.recv(&mut bytes).await.unwrap().0, 32);
        assert_eq!(bytes, [0x43; 32]);
        sender.send(address, b"").await.unwrap();
        assert_eq!(receiver.recv(&mut []).await.unwrap().0, 0);
    });
}

#[test]
fn insufficient_scratch_refuses_before_receiving() {
    run(async {
        let mut receiver = socket::<8>();
        let mut sender = socket::<8>();
        sender
            .send(receiver.local_endpoint().unwrap(), b"valid")
            .await
            .unwrap();
        let mut too_large = [0; 8];
        assert_eq!(
            receiver.recv(&mut too_large).await.unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut enough = [0; 7];
        assert_eq!(receiver.recv(&mut enough).await.unwrap().0, 5);
        assert_eq!(&enough[..5], b"valid");
    });
}

#[test]
fn timer_uses_shared_epoch_and_cancellation_leaves_it_reusable() {
    run(async {
        let clock = Clock::default();
        let mut timer = clock;
        {
            let mut wait = pin!(timer.wait_until(clock.now_ms().unwrap() + 1000));
            poll_fn(|cx| {
                assert!(wait.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        timer.wait_until(0).await;
        let deadline = clock.now_ms().unwrap() + 5;
        timer.wait_until(deadline).await;
        assert!(clock.now_ms().unwrap() >= deadline);
    });
}

const BODY: [u8; 2000] = [0x35; 2000];
fn content(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY).etag(b"v1")
}

fn uploaded(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.body().unwrap_or(request.payload()), BODY);
    Response::content(b"stored")
}

fn context<const BINDINGS: usize>(
    sender: &[u8],
    recipient: &[u8],
) -> BoundedSecurityContext<BINDINGS> {
    BoundedSecurityContext::derive(DeriveParams {
        master_secret: &[0x58; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}

#[test]
fn complete_app_blockwise_exchange_uses_callers_runtime() {
    blockwise_exchange::<2, 1, 4>(false);
}

#[test]
fn protected_blockwise_exchange_refuses_full_binding_and_reuses_it() {
    // Two Calls and four datagrams per direction leave packet and response
    // credit available when the single request binding is occupied.
    blockwise_exchange::<4, 2, 1>(true);
}

fn blockwise_exchange<const DATAGRAMS: usize, const CALLS: usize, const BINDINGS: usize>(
    secured: bool,
) {
    run(async {
        let clock = Clock::default();
        let transport = socket::<1153>();
        let endpoint = transport.local_endpoint().unwrap();
        let server = App::with_resources(SmallDevice::new().block_wise::<true>())
            .deterministic_for_tests()
            .route("body", get(content).post(uploaded));
        let mut server = if secured {
            server.oscore(context(&[2], &[1]))
        } else {
            server.allow_unprotected()
        }
        .bind(transport)
        .unwrap();
        let resources = SmallDevice::new()
            .block_wise::<true>()
            .rx_datagram::<DATAGRAMS, 1152>()
            .tx_datagram::<DATAGRAMS, 1152>()
            .client_calls::<CALLS>()
            .security_requests::<BINDINGS>();
        let capacities = resources.pool_capacities();
        let client = App::with_resources(resources).deterministic_for_tests();
        let mut client = if secured {
            client.oscore(context(&[1], &[2]))
        } else {
            client.allow_unprotected()
        }
        .bind(socket::<1153>())
        .unwrap();
        let mut server_timer = clock;
        let mut client_timer = clock;
        let server_work = async {
            loop {
                server
                    .wait_async(clock.now_ms().unwrap(), &mut server_timer, pending())
                    .await
                    .unwrap();
                let now = clock.now_ms().unwrap();
                server
                    .progress_async(now, &mut || clock.now_ms(), &mut server_timer, now + 1000)
                    .await
                    .unwrap();
            }
        };
        let client_work = async {
            let held = client.engine_mut().acquire_tx_body().unwrap();
            let refused = client
                .post("body")
                .to(endpoint)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&BODY)
                .send_async(
                    clock.now_ms().unwrap(),
                    || clock.now_ms(),
                    &mut client_timer,
                )
                .await;
            assert!(matches!(
                refused,
                Err(AsyncCallError::Request(Error::Block(
                    coaptic::error::BlockTransferError::Saturated
                )))
            ));
            assert_eq!(client.metrics().tx_ok, 0);
            assert_eq!(client.resource_usage().client_requests.occupied, 0);
            assert_eq!(client.resource_usage().storage.tx_body.unwrap().occupied, 1);
            client.engine_mut().release_tx_body(held).unwrap();

            for upload in [true, false] {
                let outgoing = if upload {
                    client
                        .post("body")
                        .content_format(ContentFormat::OCTET_STREAM)
                        .payload(&BODY)
                } else {
                    client.get("body")
                };
                let call = outgoing
                    .to(endpoint)
                    .send_async(
                        clock.now_ms().unwrap(),
                        || clock.now_ms(),
                        &mut client_timer,
                    )
                    .await
                    .unwrap();
                let binding = client
                    .oscore()
                    .and_then(|oscore| oscore.lookup(call.token()));
                assert_eq!(binding.is_some(), secured);
                let sent = client.metrics().tx_ok;
                let refused = client
                    .get("body")
                    .non()
                    .to(endpoint)
                    .send_async(
                        clock.now_ms().unwrap(),
                        || clock.now_ms(),
                        &mut client_timer,
                    )
                    .await;
                if secured {
                    assert!(
                        matches!(
                            refused,
                            Err(AsyncCallError::Request(Error::Oscore(
                                coaptic::oscore::Error::Saturated
                            )))
                        ),
                        "{refused:?}"
                    );
                    assert_eq!(client.oscore().unwrap().lookup(call.token()), binding);
                } else {
                    assert!(matches!(
                        refused,
                        Err(AsyncCallError::Request(Error::Saturated))
                    ));
                }
                assert_eq!(client.metrics().tx_ok, sent);
                loop {
                    client
                        .wait_async(clock.now_ms().unwrap(), &mut client_timer, pending())
                        .await
                        .unwrap();
                    let now = clock.now_ms().unwrap();
                    client
                        .progress_async(now, &mut || clock.now_ms(), &mut client_timer, now + 1000)
                        .await
                        .unwrap();
                    if let Some(response) = client.take_response(call) {
                        let response = response.unwrap();
                        assert_eq!(response.code(), Code::CONTENT);
                        let expected: &[u8] = if upload { b"stored" } else { &BODY };
                        assert_eq!(response.body().unwrap_or(response.payload()), expected);
                        break;
                    }
                }
                let usage = client.resource_usage();
                assert_eq!(usage.client_requests.occupied, 0);
                assert_eq!(usage.storage.rx_body.unwrap().occupied, 0);
                assert_eq!(usage.storage.tx_body.unwrap().occupied, 0);
                assert!(
                    client
                        .oscore()
                        .and_then(|oscore| oscore.lookup(call.token()))
                        .is_none()
                );
                assert_eq!(client.engine().capacities(), capacities);
            }
        };
        let mut server_work = pin!(server_work);
        let mut client_work = pin!(client_work);
        // Each wake polls both independent Apps once; socket/timer readiness
        // wakes this task. No task is spawned and idle work does not spin.
        poll_fn(|cx| {
            assert!(server_work.as_mut().poll(cx).is_pending());
            client_work.as_mut().poll(cx)
        })
        .await;
    });
}

#[test]
fn lost_mutating_reply_replays_without_reapplying_the_effect() {
    // Drive the App's protocol clock explicitly. UDP and Tokio readiness
    // remain real, and no retry depends on sleeping for a wall-clock RTO.
    struct TestTimer(u64);
    impl AsyncTimer for TestTimer {
        async fn wait_until(&mut self, deadline_ms: u64) {
            if deadline_ms > self.0 {
                pending().await
            }
        }
    }

    for protected in [false, true] {
        run(async {
            let mut relay = socket::<1153>();
            let relay_endpoint = relay.local_endpoint().unwrap();
            let server = App::with_resources(SmallDevice::new()).deterministic_for_tests();
            let client = App::with_resources(SmallDevice::new()).deterministic_for_tests();
            let mut server = if protected {
                server.oscore(context(&[2], &[1]))
            } else {
                server.allow_unprotected()
            }
            .bind(socket::<1153>())
            .unwrap();
            let mut client = if protected {
                client.oscore(context(&[1], &[2]))
            } else {
                client.allow_unprotected()
            }
            .bind(socket::<1153>())
            .unwrap();
            let server_endpoint = server.transport().local_endpoint().unwrap();
            let client_endpoint = client.transport().local_endpoint().unwrap();
            let mut effects = 0;
            let mut now = 1;

            for (operation, lose_first_reply) in [true, false].into_iter().enumerate() {
                let call = client
                    .post("effect")
                    .to(relay_endpoint)
                    .payload(b"apply")
                    .deadline(now + 10_000)
                    .send_async(now, || Ok::<_, io::Error>(now), &mut TestTimer(now))
                    .await
                    .unwrap();
                let binding = client
                    .oscore()
                    .and_then(|context| context.lookup(call.token()));
                assert_eq!(binding.is_some(), protected);
                let sender_seq = protected.then_some(operation as u64 + 1);
                assert_eq!(
                    client.oscore().map(|context| context.sender_seq()),
                    sender_seq
                );

                let mut request = [0; 1152];
                let (size, peer) = relay.recv(&mut request).await.unwrap();
                assert_eq!(peer, client_endpoint);
                let mut response = [0; 1152];
                let mut response_len = 0;
                for attempt in 0..=usize::from(lose_first_reply) {
                    if attempt == 1 {
                        // The first reply never leaves the relay for the client.
                        let retry_at = client.next_timeout().unwrap();
                        assert!(retry_at > now && retry_at < now + 5000);
                        now = retry_at;
                        client
                            .progress_async(
                                now,
                                &mut || Ok::<_, io::Error>(now),
                                &mut TestTimer(now),
                                now + 1000,
                            )
                            .await
                            .unwrap();
                        let mut retry = [0; 1152];
                        let (len, from) = relay.recv(&mut retry).await.unwrap();
                        assert_eq!(from, client_endpoint);
                        assert_eq!(&retry[..len], &request[..size]);
                        assert_eq!(
                            client.oscore().map(|context| context.sender_seq()),
                            sender_seq
                        );
                        assert_eq!(
                            client
                                .oscore()
                                .and_then(|context| context.lookup(call.token())),
                            binding
                        );
                    }

                    let replay_before = server.oscore().map(|context| context.replay_checkpoint());
                    relay.send(server_endpoint, &request[..size]).await.unwrap();
                    now += 1;
                    server
                        .wait_async(now, &mut TestTimer(now), pending())
                        .await
                        .unwrap();
                    server
                        .progress_with_async(
                            now,
                            &mut || Ok::<_, io::Error>(now),
                            &mut TestTimer(now),
                            now + 1000,
                            |request, _| {
                                assert_eq!(request.code(), Code::POST);
                                assert_eq!(request.path(), ["effect"]);
                                assert_eq!(request.payload(), b"apply");
                                effects += 1;
                                Response::content(b"applied")
                                    .content_format(ContentFormat::OCTET_STREAM)
                                    .etag(b"effect")
                            },
                        )
                        .await
                        .unwrap();
                    let mut wire = [0; 1152];
                    let (len, from) = relay.recv(&mut wire).await.unwrap();
                    assert_eq!(from, server_endpoint);
                    assert_eq!(effects, operation + 1);
                    if attempt == 1 {
                        assert_eq!(&wire[..len], &response[..response_len]);
                        assert_eq!(
                            server.oscore().map(|context| context.replay_checkpoint()),
                            replay_before
                        );
                    }
                    response[..len].copy_from_slice(&wire[..len]);
                    response_len = len;
                }

                relay
                    .send(client_endpoint, &response[..response_len])
                    .await
                    .unwrap();
                now += 1;
                client
                    .wait_async(now, &mut TestTimer(now), pending())
                    .await
                    .unwrap();
                client
                    .progress_async(
                        now,
                        &mut || Ok::<_, io::Error>(now),
                        &mut TestTimer(now),
                        now + 1000,
                    )
                    .await
                    .unwrap();
                let reply = client.take_response(call).unwrap().unwrap();
                assert_eq!(reply.code(), Code::CONTENT);
                assert_eq!(reply.payload(), b"applied");
                assert_eq!(reply.etag_bytes(), Some(&b"effect"[..]));
                assert_eq!(reply.format(), Some(ContentFormat::OCTET_STREAM));
                let usage = client.resource_usage();
                assert_eq!(usage.client_requests.occupied, 0);
                assert_eq!(usage.storage.tx_datagram.occupied, 0);
                assert!(
                    client
                        .oscore()
                        .and_then(|context| context.lookup(call.token()))
                        .is_none()
                );
                now += 1;
            }
            assert_eq!(effects, 2);
        });
    }
}
