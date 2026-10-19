//! One native async App client and server on real localhost UDP sockets.
use coaptic::{App, Request, Response, get, resources::SmallDevice};
use coaptic_async_host::{Clock, Udp};
use core::{
    future::{pending, poll_fn},
    pin::pin,
    task::Poll,
};
use std::{net::UdpSocket, time::Duration};

fn hello(_: Request<'_>) -> Response<'static> {
    Response::content(b"hello from async App")
}
fn socket() -> Udp<[u8; 1153]> {
    Udp::new(
        UdpSocket::bind("127.0.0.1:0").expect("bind localhost"),
        [0; 1153],
    )
    .expect("register readiness")
}
fn main() {
    async_io::block_on(async {
        let clock = Clock::default();
        let transport = socket();
        let endpoint = transport.local_endpoint().unwrap();
        let mut server = App::with_resources(SmallDevice::new())
            .randomness(|bytes| getrandom::fill(bytes).is_ok())
            .allow_unprotected()
            .route("hello", get(hello))
            .bind(transport)
            .unwrap();
        let mut client = App::with_resources(SmallDevice::new())
            .randomness(|bytes| getrandom::fill(bytes).is_ok())
            .allow_unprotected()
            .bind(socket())
            .unwrap();
        println!(
            "SmallDevice/Constrained: routes=4 calls=1 deferred=0 security_requests=4 associations=1, unprotected, blockwise=false"
        );
        println!(
            "server={} client={} bytes (includes UDP scratch; excludes host reactor)",
            core::mem::size_of_val(&server),
            core::mem::size_of_val(&client)
        );
        let mut server_timer = clock;
        let mut client_timer = clock;
        let serving = async {
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
        let requesting = async {
            let sending = client.get("hello").to(endpoint).send_async(
                clock.now_ms().unwrap(),
                || clock.now_ms(),
                &mut client_timer,
            );
            println!(
                "initial send future={} bytes",
                core::mem::size_of_val(&sending)
            );
            let call = sending.await.unwrap();
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
                    assert_eq!(response.payload(), b"hello from async App");
                    println!("{}", String::from_utf8_lossy(response.payload()));
                    break;
                }
            }
        };
        println!(
            "server task={} client task={} bytes",
            core::mem::size_of_val(&serving),
            core::mem::size_of_val(&requesting)
        );
        let mut serving = pin!(serving);
        let mut requesting = pin!(requesting);
        let mut deadline = pin!(async_io::Timer::after(Duration::from_secs(5)));
        poll_fn(|cx| {
            assert!(serving.as_mut().poll(cx).is_pending());
            if requesting.as_mut().poll(cx).is_ready() {
                return Poll::Ready(());
            }
            assert!(deadline.as_mut().poll(cx).is_pending(), "loopback deadline");
            Poll::Pending
        })
        .await;
    });
}
