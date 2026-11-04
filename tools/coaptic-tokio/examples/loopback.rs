//! One unprotected loopback exchange, then shutdown. No daemon or busy polling.
use coaptic::{App, Request, Response, get, resources::SmallDevice};
use coaptic_tokio::{Clock, Udp};
use core::{
    future::{Future, pending, poll_fn},
    pin::pin,
    task::Poll,
};
use std::{fmt::Debug, io, time::Duration};

fn error(value: impl Debug) -> io::Error {
    io::Error::other(format!("{value:?}"))
}

fn hello(_: Request<'_>) -> Response<'static> {
    Response::content(b"hello from Coaptic")
}

async fn exchange() -> io::Result<()> {
    let clock = Clock::default();
    // Socket creation and the runtime belong to this application, not the adapter.
    let server_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    let client_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    let endpoint = server_socket.local_addr()?.into();
    let mut server = App::with_resources(SmallDevice::new())
        // Loopback demonstration only; choose real randomness and protection
        // appropriate to the deployment when adapting this example.
        .deterministic_for_tests()
        .allow_unprotected()
        .route("hello", get(hello))
        .bind(Udp::new(server_socket, [0; 1153])?)
        .map_err(error)?;
    let mut client = App::with_resources(SmallDevice::new())
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Udp::new(client_socket, [0; 1153])?)
        .map_err(error)?;
    let mut server_timer = clock;
    let mut client_timer = clock;
    let serve = async {
        loop {
            server
                .wait_async(clock.now_ms()?, &mut server_timer, pending())
                .await
                .map_err(error)?;
            let now = clock.now_ms()?;
            if let Err(failure) = server
                .progress_async(now, &mut || clock.now_ms(), &mut server_timer, now + 1000)
                .await
            {
                return Err(error(failure));
            }
        }
    };
    let request = async {
        let call = client
            .get("hello")
            .to(endpoint)
            .send_async(clock.now_ms()?, || clock.now_ms(), &mut client_timer)
            .await
            .map_err(error)?;
        loop {
            client
                .wait_async(clock.now_ms()?, &mut client_timer, pending())
                .await
                .map_err(error)?;
            let now = clock.now_ms()?;
            client
                .progress_async(now, &mut || clock.now_ms(), &mut client_timer, now + 1000)
                .await
                .map_err(error)?;
            if let Some(response) = client.take_response(call) {
                let response = response.map_err(error)?;
                if response.payload() != b"hello from Coaptic" {
                    return Err(io::Error::other("unexpected response"));
                }
                println!("{}", String::from_utf8_lossy(response.payload()));
                return Ok(());
            }
        }
    };
    let mut serve = pin!(serve);
    let mut request = pin!(request);
    poll_fn(|cx| {
        if let Poll::Ready(result) = serve.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        request.as_mut().poll(cx)
    })
    .await
    // Dropping the server future cancels its pending receive; Apps and sockets
    // are dropped before the caller drops the runtime.
}

fn run() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), exchange())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "loopback deadline"))?
    })
}

fn main() -> io::Result<()> {
    // This debug example constructs two bounded Apps in one future. Make its
    // caller-owned stack explicit instead of depending on Windows main's 1 MiB
    // default. This is example/runtime storage, not an adapter allocation.
    std::thread::Builder::new()
        .name("coaptic-loopback".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| io::Error::other("loopback thread panicked"))?
}
