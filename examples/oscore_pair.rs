//! Two Apps exchange a complete protected response over localhost UDP.
//!
//! Run: `cargo run --example oscore_pair --features std`.
//! Heap-backed: `cargo run --example oscore_pair --features std,alloc -- --alloc`.
//! Both modes use explicit capacities and the same protected service handlers.
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

use coaptic::storage::{MemoryLayout, UdpSocketIo};
use coaptic::{
    App, Code, Endpoint,
    app::{AppAssembled, AppStorage},
    oscore::{DeriveParams, SecurityContext},
    platform::CheckedClock,
    resources,
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

#[derive(Debug, PartialEq)]
enum Mode {
    Fixed,
    #[cfg(feature = "alloc")]
    Alloc,
}

fn mode(
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> Result<Mode, host_error::Error> {
    let mut args = args.into_iter();
    match (args.next(), args.next()) {
        (None, None) => Ok(Mode::Fixed),
        (Some(arg), None) if arg.as_ref() == "--alloc" => {
            #[cfg(feature = "alloc")]
            {
                Ok(Mode::Alloc)
            }
            #[cfg(not(feature = "alloc"))]
            {
                Err(host_error::Error::Arguments(
                    "--alloc requires building with --features std,alloc",
                ))
            }
        }
        _ => Err(host_error::Error::Arguments("usage: oscore_pair [--alloc]")),
    }
}

fn main() -> Result<(), host_error::Error> {
    let mode = mode(std::env::args_os().skip(1))?;
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(|_| host_error::Error::Entropy)?;
    let server_socket = UdpSocket::bind("127.0.0.1:0")?;
    server_socket.set_nonblocking(true)?;
    let peer = Endpoint::from(server_socket.local_addr()?);
    let client_socket = UdpSocket::bind("127.0.0.1:0")?;
    client_socket.set_nonblocking(true)?;

    let server_context = context(&secret, &[2], &[1])?;
    let client_context = context(&secret, &[1], &[2])?;
    secret.fill(0);

    match mode {
        Mode::Fixed => {
            let server = fixed_service::server(
                UdpSocketIo::new(server_socket, [0; 1153])?,
                server_context,
                |bytes| getrandom::fill(bytes).is_ok(),
            )?;
            let client = App::with_resources(resources::SmallDevice::new())
                .randomness(|bytes| getrandom::fill(bytes).is_ok())
                .oscore(client_context)
                .bind(UdpSocketIo::new(client_socket, [0; 1153])?)?;
            println!("Storage: fixed constrained profile (2 RX/TX slots, 1152 bytes each)");
            exchange(server, client, peer)
        }
        #[cfg(feature = "alloc")]
        Mode::Alloc => {
            // Allocated once at bind; no pool growth during polling. Larger storage
            // does not increase App's route, peer or security-context limits.
            let heap_limit = 512 * 1024;
            let server = App::with_resources(resources::Service::new().allocated(heap_limit))
                .randomness(|bytes| getrandom::fill(bytes).is_ok())
                .oscore(server_context)
                .route("telemetry", coaptic::get(fixed_service::telemetry))
                .route("echo", coaptic::post(fixed_service::echo))
                .bind(UdpSocketIo::new(server_socket, [0; 1473])?)?;
            let client = App::with_resources(resources::Service::new().allocated(heap_limit))
                .randomness(|bytes| getrandom::fill(bytes).is_ok())
                .oscore(client_context)
                .bind(UdpSocketIo::new(client_socket, [0; 1473])?)?;
            println!(
                "Storage: service heap pools (32 RX/TX slots, 1472 bytes each; 512 KiB heap request limit)"
            );
            exchange(server, client, peer)
        }
    }
}

// One request/poll/response path for both storage backends.
fn exchange<
    P,
    S,
    const SCRATCH: usize,
    const N: usize,
    const B: bool,
    const D: usize,
    const C: usize,
>(
    mut server: App<P, UdpSocketIo<[u8; SCRATCH]>, N, B, S, D, C>,
    mut client: App<P, UdpSocketIo<[u8; SCRATCH]>, N, B, S, D, C>,
    peer: Endpoint,
) -> Result<(), host_error::Error>
where
    P: MemoryLayout<B> + AppAssembled<B>,
    S: AppStorage<Token = P::Token>,
{
    let epoch = Instant::now();
    let mut clock = CheckedClock::new(|| u64::try_from(epoch.elapsed().as_millis()));
    let call = client.get("telemetry").to(peer).send(clock.now_ms()?)?;
    let mut output = [0; TELEMETRY.len()];
    loop {
        let now = clock.now_ms()?;
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
        if epoch.elapsed() >= Duration::from_secs(5) {
            return Err(host_error::Error::Timeout);
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_fixed_storage() {
        assert_eq!(mode([] as [&str; 0]).unwrap(), Mode::Fixed);
    }

    #[test]
    fn refuses_unknown_or_repeated_arguments() {
        for args in [&["--plaintext"][..], &["--alloc", "--alloc"][..]] {
            assert!(matches!(mode(args), Err(host_error::Error::Arguments(_))));
        }
    }

    #[test]
    fn allocation_requires_explicit_feature_and_argument() {
        #[cfg(feature = "alloc")]
        assert_eq!(mode(["--alloc"]).unwrap(), Mode::Alloc);
        #[cfg(not(feature = "alloc"))]
        assert!(matches!(
            mode(["--alloc"]),
            Err(host_error::Error::Arguments(
                "--alloc requires building with --features std,alloc"
            ))
        ));
    }
}
