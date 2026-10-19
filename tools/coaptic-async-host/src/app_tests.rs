use super::{Clock, Udp};
use coaptic::storage::{MemoryProfile, profiles};
use coaptic::{App, Code, Request, Response, get, resources::SmallDevice};
use core::{
    future::{Future, pending, poll_fn},
    pin::pin,
    task::Poll,
};
use std::{net::UdpSocket, time::Duration};

const RX_BYTES: usize = profiles::Constrained::RX_DATAGRAM_BYTES;
const BODY: [u8; 2000] = [b'x'; 2000];

fn socket() -> Udp<[u8; RX_BYTES + 1]> {
    Udp::new(UdpSocket::bind("127.0.0.1:0").unwrap(), [0; RX_BYTES + 1]).unwrap()
}

fn content(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY).etag(b"v1")
}
fn upload(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.body().unwrap_or(request.payload()), BODY);
    Response::changed()
}
fn observe(_: Request<'_>) -> Response<'static> {
    Response::content(b"initial").observe(0)
}

// Poll each independent App once per wake. OS socket/timer readiness registers
// the waker; this orchestration neither self-wakes nor spins while idle.
async fn serve_until<T>(server: impl Future<Output = ()>, client: impl Future<Output = T>) -> T {
    let mut server = pin!(server);
    let mut client = pin!(client);
    let mut limit = pin!(async_io::Timer::after(Duration::from_secs(5)));
    poll_fn(|cx| {
        assert!(server.as_mut().poll(cx).is_pending(), "server ended");
        if let Poll::Ready(value) = client.as_mut().poll(cx) {
            return Poll::Ready(value);
        }
        assert!(
            limit.as_mut().poll(cx).is_pending(),
            "App loopback deadline"
        );
        Poll::Pending
    })
    .await
}

macro_rules! progress {
    ($app:expr, $clock:expr, $timer:expr) => {{
        let now = $clock.now_ms().unwrap();
        $app.progress_async(now, &mut || $clock.now_ms(), $timer, now + 1000)
            .await
            .unwrap();
    }};
}
macro_rules! wait {
    ($app:expr, $clock:expr, $timer:expr) => {{
        $app.wait_async($clock.now_ms().unwrap(), $timer, pending())
            .await
            .unwrap();
    }};
}

#[test]
fn native_app_classic_and_q_blockwise_over_real_udp() {
    for q in [false, true] {
        async_io::block_on(async {
            let clock = Clock::default();
            let transport = socket();
            let endpoint = transport.local_endpoint().unwrap();
            let mut server = App::with_resources(SmallDevice::new().block_wise::<true>())
                .deterministic_for_tests()
                .allow_unprotected()
                .route("body", get(content).post(upload))
                .bind(transport)
                .unwrap();
            let mut client = App::with_resources(SmallDevice::new().block_wise::<true>())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(socket())
                .unwrap();
            let mut server_timer = clock;
            let mut client_timer = clock;
            serve_until(
                async {
                    loop {
                        wait!(server, clock, &mut server_timer);
                        progress!(server, clock, &mut server_timer);
                    }
                },
                async {
                    let request = client.get("body").to(endpoint);
                    let request = if q { request.q_block2() } else { request };
                    let call = request
                        .send_async(
                            clock.now_ms().unwrap(),
                            || clock.now_ms(),
                            &mut client_timer,
                        )
                        .await
                        .unwrap();
                    loop {
                        wait!(client, clock, &mut client_timer);
                        progress!(client, clock, &mut client_timer);
                        if let Some(response) = client.take_response(call) {
                            let response = response.unwrap();
                            assert_eq!(response.body().unwrap_or(response.payload()), BODY);
                            break;
                        }
                    }
                    let request = client.post("body").to(endpoint).payload(&BODY);
                    let request = if q {
                        request
                            .request_tag(coaptic::storage::BodyTag::new(b"q").unwrap())
                            .q_block1()
                    } else {
                        request
                    };
                    let call = request
                        .send_async(
                            clock.now_ms().unwrap(),
                            || clock.now_ms(),
                            &mut client_timer,
                        )
                        .await
                        .unwrap();
                    loop {
                        wait!(client, clock, &mut client_timer);
                        progress!(client, clock, &mut client_timer);
                        if let Some(response) = client.take_response(call) {
                            assert_eq!(response.unwrap().code(), Code::CHANGED);
                            break;
                        }
                    }
                },
            )
            .await;
        });
    }
}

#[test]
fn native_app_observe_deferred_completion_and_local_wake() {
    async_io::block_on(serve_until(pending(), async {
        let clock = Clock::default();
        let transport = socket();
        let endpoint = transport.local_endpoint().unwrap();
        let mut server = App::with_resources(SmallDevice::new().deferred::<1>())
            .deterministic_for_tests()
            .allow_unprotected()
            .route(
                "sensor",
                get(observe).observe(|| Response::content(b"next")),
            )
            .bind(transport)
            .unwrap();
        let mut client = App::with_resources(SmallDevice::new())
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(socket())
            .unwrap();
        let mut server_timer = clock;
        let mut client_timer = clock;
        let call = client
            .get("sensor")
            .to(endpoint)
            .observe()
            .send_async(
                clock.now_ms().unwrap(),
                || clock.now_ms(),
                &mut client_timer,
            )
            .await
            .unwrap();
        wait!(server, clock, &mut server_timer);
        let mut handle = None;
        let now = clock.now_ms().unwrap();
        server
            .progress_with_async(
                now,
                &mut || clock.now_ms(),
                &mut server_timer,
                now + 1000,
                |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                },
            )
            .await
            .unwrap();
        let handle = handle.expect("deferred ownership");
        // Local asynchronous work wins while a socket is also readable, without
        // consuming ingress; the next wait/progress pass still receives it.
        assert_eq!(
            client
                .wait_async(clock.now_ms().unwrap(), &mut client_timer, async {})
                .await
                .unwrap(),
            coaptic::app::AppWake::Local
        );
        wait!(client, clock, &mut client_timer);
        progress!(client, clock, &mut client_timer);
        assert!(client.take_response(call).is_none());
        async_io::Timer::after(Duration::from_millis(2)).await;
        let now = clock.now_ms().unwrap();
        server
            .complete_async(
                handle,
                Response::content(b"initial").observe(0),
                now,
                &mut || clock.now_ms(),
                &mut server_timer,
                now + 1000,
            )
            .await
            .unwrap();
        wait!(client, clock, &mut client_timer);
        progress!(client, clock, &mut client_timer);
        assert_eq!(
            client.take_response(call).unwrap().unwrap().payload(),
            b"initial"
        );
        // Retire the separate response's CON before sending another notification.
        wait!(server, clock, &mut server_timer);
        progress!(server, clock, &mut server_timer);
        let now = clock.now_ms().unwrap();
        assert_eq!(
            server
                .notify_confirmable_async(
                    now,
                    &["sensor"],
                    Response::content(b"next"),
                    &mut || clock.now_ms(),
                    &mut server_timer,
                    now + 1000
                )
                .await
                .unwrap(),
            1
        );
        wait!(client, clock, &mut client_timer);
        progress!(client, clock, &mut client_timer);
        assert_eq!(
            client.take_response(call).unwrap().unwrap().payload(),
            b"next"
        );
    }));
}

#[test]
fn native_protected_app_blockwise_observe_and_checkpoint_over_real_udp() {
    use coaptic::storage::{AsyncDatagramIo, DatagramTransport};
    use coaptic::{
        Endpoint,
        oscore::{DeriveParams, SecurityContext},
    };
    use core::cell::Cell;
    struct Audited<'a> {
        udp: Udp<[u8; RX_BYTES + 1]>,
        protected: &'a Cell<usize>,
    }
    impl DatagramTransport for Audited<'_> {
        type Error = std::io::Error;
    }
    impl AsyncDatagramIo for Audited<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
            self.udp.recv(bytes).await
        }
        async fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            let message = coaptic::message::decode(bytes).unwrap();
            if message.code() != Code::EMPTY {
                assert!(
                    message.oscore().is_some(),
                    "application datagram must be protected"
                );
                self.protected.set(self.protected.get() + 1);
            }
            self.udp.send(endpoint, bytes).await
        }
    }
    let context = |sender, recipient| {
        SecurityContext::derive(DeriveParams {
            master_secret: &[7; 16],
            master_salt: &[],
            sender_id: sender,
            recipient_id: recipient,
            id_context: &[],
        })
        .unwrap()
    };
    async_io::block_on(async {
        let clock = Clock::default();
        let protected = Cell::new(0);
        let checkpoints = Cell::new(0);
        let transport = socket();
        let endpoint = transport.local_endpoint().unwrap();
        let mut server = App::with_resources(SmallDevice::new().block_wise::<true>())
            .deterministic_for_tests()
            .oscore(context(&[2], &[1]))
            .route("body", get(content))
            .route(
                "sensor",
                get(observe).observe(|| Response::content(b"next")),
            )
            .bind(Audited {
                udp: transport,
                protected: &protected,
            })
            .unwrap();
        let mut client = App::with_resources(SmallDevice::new().block_wise::<true>())
            .deterministic_for_tests()
            .oscore(context(&[1], &[2]))
            .bind(Audited {
                udp: socket(),
                protected: &protected,
            })
            .unwrap();
        let mut server_timer = clock;
        let mut client_timer = clock;
        serve_until(
            async {
                loop {
                    wait!(server, clock, &mut server_timer);
                    let now = clock.now_ms().unwrap();
                    server
                        .progress_with_oscore_checkpoint_async(
                            now,
                            &mut || clock.now_ms(),
                            &mut server_timer,
                            now + 1000,
                            |_| {
                                checkpoints.set(checkpoints.get() + 1);
                                true
                            },
                        )
                        .await
                        .unwrap();
                }
            },
            async {
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
                    wait!(client, clock, &mut client_timer);
                    progress!(client, clock, &mut client_timer);
                    if let Some(response) = client.take_response(call) {
                        let response = response.unwrap();
                        assert_eq!(response.body().unwrap(), BODY);
                        break;
                    }
                }
                let call = client
                    .get("sensor")
                    .to(endpoint)
                    .observe()
                    .send_async(
                        clock.now_ms().unwrap(),
                        || clock.now_ms(),
                        &mut client_timer,
                    )
                    .await
                    .unwrap();
                loop {
                    wait!(client, clock, &mut client_timer);
                    progress!(client, clock, &mut client_timer);
                    if let Some(response) = client.take_response(call) {
                        let response = response.unwrap();
                        assert_eq!(response.payload(), b"initial");
                        assert!(response.observe_seq().is_some());
                        break;
                    }
                }
            },
        )
        .await;
        assert!(protected.get() >= 6);
        assert!(checkpoints.get() >= 3);
    });
}
