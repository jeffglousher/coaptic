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
        sender.send(address, &[0x42; 64]).await.unwrap();
        // Windows may report WSAEMSGSIZE instead of a copied oversized prefix.
        assert!(receiver.recv(&mut bytes).await.is_err());
        assert_eq!(bytes, [0xa5; 32]);
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
