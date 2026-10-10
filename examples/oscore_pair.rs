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

#[path = "support/host_error.rs"]
pub mod host_error;

use coaptic::storage::UdpSocketIo;
use coaptic::{
    App, Code, Endpoint,
    oscore::{DeriveParams, SecurityContext},
};

#[path = "support/fixed_service.rs"]
mod fixed_service;

use fixed_service::TELEMETRY;

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

fn main() -> Result<(), host_error::Error> {
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(|_| host_error::Error::Entropy)?;
    let server_socket = UdpSocket::bind("127.0.0.1:0")?;
    server_socket.set_nonblocking(true)?;
    let peer = Endpoint::from(server_socket.local_addr()?);
    let client_socket = UdpSocket::bind("127.0.0.1:0")?;
    client_socket.set_nonblocking(true)?;

    let mut server = fixed_service::server(
        UdpSocketIo::new(server_socket, [0; 1473])?,
        context(&secret, &[2], &[1])?,
        |bytes| getrandom::fill(bytes).is_ok(),
    )?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .oscore(context(&secret, &[1], &[2])?)
        .full_responses()
        .bind(UdpSocketIo::new(client_socket, [0; 1473])?)?;
    secret.fill(0);

    let clock = Instant::now();
    let call = client.get("telemetry").to(peer).send(0)?;
    let mut output = [0; TELEMETRY.len()];
    loop {
        let now =
            u64::try_from(clock.elapsed().as_millis()).map_err(|_| host_error::Error::Clock)?;
        server.poll(now)?;
        client.poll(now)?;
        if let Some(reply) = client.take_response_into(call, &mut output)? {
            let reply = reply?;
            if reply.code() != Code::CONTENT || reply.payload() != TELEMETRY {
                return Err(host_error::Error::UnexpectedReply);
            }
            println!("OSCORE: received all {} bytes", reply.payload().len());
            return Ok(());
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err(host_error::Error::Timeout);
        }
        thread::sleep(Duration::from_millis(1));
    }
}
