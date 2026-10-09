//! README server; explicitly plaintext on localhost.

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("127.0.0.1:5683")?;
    socket.set_nonblocking(true)?;
    let mut server = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .route("sensors/temp", get(temperature))
        .route("echo", post(echo))
        .allow_plaintext()
        .bind(socket)?;

    let clock = Instant::now();
    loop {
        server.poll(clock.elapsed().as_millis() as u64)?;
        thread::sleep(Duration::from_millis(1));
    }
}
