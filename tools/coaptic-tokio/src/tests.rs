use super::*;
use coaptic::{App, Request, Response, get, platform::AsyncTimer, resources::SmallDevice};
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
fn one_tokio_sleep_moves_and_fires_without_a_second_insert() {
    run(async {
        let clock = Clock::default();
        let mut timer = clock.timer();
        let later = clock.now_ms().unwrap() + 1000;
        {
            let mut wait = pin!(timer.wait_until(later));
            poll_fn(|cx| {
                assert!(wait.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(timer.inserts(), 1);
        assert_eq!(timer.armed(), Some(later));
        timer.schedule(Some(later));
        assert_eq!(timer.inserts(), 1);
        timer.wait_until(0).await;
        assert_eq!(timer.inserts(), 1);
        let started = std::time::Instant::now();
        let deadline = clock.now_ms().unwrap() + 25;
        timer.wait_until(deadline).await;
        let elapsed = started.elapsed();
        assert_eq!(timer.inserts(), 1);
        assert!(
            elapsed >= Duration::from_millis(15) && elapsed < Duration::from_millis(200),
            "deadline moved onto the existing sleep, elapsed {elapsed:?}"
        );
        assert!(clock.now_ms().unwrap() >= deadline);
    });
}

#[test]
fn outstanding_exchange_keeps_one_sleep_when_a_packet_arrives() {
    run(async {
        let clock = Clock::default();
        let mut timer = clock.timer();
        let transport = socket::<1153>();
        let local = transport.local_endpoint().unwrap();
        let mut app = App::with_resources(SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(transport)
            .unwrap();
        let peer = socket::<64>();
        let now = clock.now_ms().unwrap();
        app.get("x")
            .to(peer.local_endpoint().unwrap())
            .send_async(now, || clock.now_ms(), &mut timer)
            .await
            .unwrap();
        let now = clock.now_ms().unwrap();
        {
            let mut wait = pin!(app.wait_async(now, &mut timer, pending()));
            poll_fn(|cx| {
                assert!(wait.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            let mut sender = socket::<64>();
            sender.send(local, &[0x40, 0x01, 0x00, 0x01]).await.unwrap();
            assert_eq!(wait.await.unwrap(), coaptic::app::AppWake::Ingress);
        }
        assert_eq!(timer.inserts(), 1);
        let armed = timer.armed();
        assert!(armed.is_some());
        let now = clock.now_ms().unwrap();
        {
            let mut again = pin!(app.wait_async(now, &mut timer, pending()));
            let _ = poll_fn(|cx| Poll::Ready(again.as_mut().poll(cx))).await;
        }
        assert_eq!(timer.inserts(), 1);
        assert_eq!(timer.armed(), armed);
    });
}

const BODY: [u8; 2000] = [0x35; 2000];
fn content(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY).etag(b"v1")
}

#[test]
fn complete_app_blockwise_exchange_uses_callers_runtime() {
    run(async {
        let clock = Clock::default();
        let transport = socket::<1153>();
        let endpoint = transport.local_endpoint().unwrap();
        let mut server = App::with_resources(SmallDevice::new().block_wise::<true>())
            .deterministic_for_tests()
            .allow_unprotected()
            .route("body", get(content))
            .bind(transport)
            .unwrap();
        let mut client = App::with_resources(SmallDevice::new().block_wise::<true>())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(socket::<1153>())
            .unwrap();
        let mut server_timer = clock.timer();
        let mut client_timer = clock.timer();
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
            let call = client
                .get("body")
                .to(endpoint)
                .send_async(
                    clock.now_ms().unwrap(),
                    || clock.now_ms(),
                    &mut client_timer,
                )
                .await
                .unwrap();
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
                    assert_eq!(response.body().unwrap_or(response.payload()), BODY);
                    break;
                }
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
