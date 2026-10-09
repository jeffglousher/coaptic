//! Two Apps exchange a complete protected response over localhost UDP.
//!
//! Run: `cargo run --example oscore_pair --features std`.
//! A fresh OS-generated secret provisions both peers inside this process.
//! This demonstrates the API, not enrollment or durable device storage. Never
//! reuse these contexts after a restart: retained credentials require durable
//! sender reservations and replay checkpoints as described in `SECURITY.md`.

use std::{
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

use coaptic::{
    App, Code, Endpoint, Request, Response, get,
    oscore::{DeriveParams, SecurityContext},
};

const BODY: [u8; 200] = [b'x'; 200];

fn representation(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY)
}

fn context(
    secret: &[u8],
    sender: &[u8],
    recipient: &[u8],
) -> Result<SecurityContext, coaptic::oscore::Error> {
    SecurityContext::derive(DeriveParams {
        master_secret: secret,
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(|error| std::io::Error::other(error.to_string()))?;
    let server_socket = UdpSocket::bind("127.0.0.1:0")?;
    server_socket.set_nonblocking(true)?;
    let peer = Endpoint::from(server_socket.local_addr()?);
    let client_socket = UdpSocket::bind("127.0.0.1:0")?;
    client_socket.set_nonblocking(true)?;

    let mut server = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .oscore(context(&secret, &[2], &[1])?)
        .route("telemetry", get(representation))
        .bind(server_socket)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .oscore(context(&secret, &[1], &[2])?)
        .full_responses()
        .bind(client_socket)?;
    secret.fill(0);

    let clock = Instant::now();
    let call = client.get("telemetry").to(peer).send(0)?;
    let mut output = [0; BODY.len()];
    loop {
        let now = u64::try_from(clock.elapsed().as_millis())?;
        server.poll(now)?;
        client.poll(now)?;
        if let Some(reply) = client.take_response_into(call, &mut output)? {
            let reply = reply?;
            if reply.code() != Code::CONTENT || reply.payload() != BODY {
                return Err("unexpected protected response".into());
            }
            println!("OSCORE: received all {} bytes", reply.payload().len());
            return Ok(());
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err("protected exchange timed out".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
