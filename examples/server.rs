//! README server; explicitly plaintext on localhost.

#[path = "support/host_error.rs"]
pub mod host_error;

use coaptic::storage::UdpSocketIo;
use coaptic::{App, Code, Request, Response, get, post};
use std::{
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

fn temperature(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

fn echo(request: Request<'_>) -> Response<'static> {
    Response::try_content_copy(request.payload())
        .unwrap_or_else(|_| Response::new(Code::REQUEST_ENTITY_TOO_LARGE))
}

fn main() -> Result<(), host_error::Error> {
    let socket = UdpSocket::bind("127.0.0.1:5683")?;
    socket.set_nonblocking(true)?;
    let mut server = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .route("sensors/temp", get(temperature))
        .route("echo", post(echo))
        .allow_plaintext()
        .bind(UdpSocketIo::new(socket, [0; 1473])?)?;

    let clock = Instant::now();
    loop {
        server.poll(
            u64::try_from(clock.elapsed().as_millis()).map_err(|_| host_error::Error::Clock)?,
        )?;
        thread::sleep(Duration::from_millis(1));
    }
}
