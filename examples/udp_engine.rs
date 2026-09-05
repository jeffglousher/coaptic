//! Taste of [`coaptic::DatagramIo`]: one trait bind, any transport.
//!
//! ```text
//! cargo run --example udp_engine --features std
//! ```
//!
//! `UdpSocket` is a [`DatagramIo`]. A `no_std` radio implements the same
//! trait. The core never sends (`CALLER.md`).

use std::net::UdpSocket;
use std::time::Instant;

use coaptic::{
    Code, ContentFormat, DatagramIo, Endpoint, EngineBuilder, Ids, Memory, Message, Opt,
    OptionsBuilder, Retransmit, Token, Type, decode, encode, profiles,
};

/// One monotonic domain for `progress` / pending CON / Observe / Echo.
fn now_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn main() {
    let origin = Instant::now();

    let mut engine = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(true)
        .build(Memory::<profiles::Default>::with_block_wise())
        .expect("Default + block_wise");

    let mut server = UdpSocket::bind("127.0.0.1:0").expect("server bind");
    let mut client = UdpSocket::bind("127.0.0.1:0").expect("client bind");
    let server_ep = Endpoint::from(server.local_addr().expect("server addr"));
    let timeout = std::time::Duration::from_millis(200);
    server
        .set_read_timeout(Some(timeout))
        .expect("server timeout");
    client
        .set_read_timeout(Some(timeout))
        .expect("client timeout");

    send_client_get(&mut client, server_ep);

    // Last mile: trait recv → RX slot. Not socket.recv_from + write_rx.
    let rx = engine
        .recv_from(&mut server)
        .expect("recv_from")
        .expect("datagram");

    let progress = engine.progress(now_ms(origin));

    if let Some(retransmit) = progress.retransmit() {
        match retransmit {
            Retransmit::Due(pending) => {
                engine
                    .send_tx(&mut server, pending.tx_slot())
                    .expect("retransmit");
            }
            Retransmit::GiveUp(pending) => {
                engine.release_tx(pending.tx_slot()).expect("give up TX");
            }
        }
    }

    let rx = progress.rx_ready().unwrap_or(rx);
    let dest = engine.rx_endpoint(rx).expect("RX endpoint");
    let parsed = engine.decode_rx(rx).expect("decode_rx");
    assert_eq!(parsed.code(), Code::GET);

    let cf = ContentFormat::TEXT_PLAIN.encode();
    let mut opts = OptionsBuilder::<4>::new();
    opts.push(Opt::content_format(&cf)).expect("cf room");
    let reply = Message::new(Type::Acknowledgement, Code::CONTENT, parsed.message_id())
        .with_token(parsed.token())
        .with_options(opts.as_slice())
        .with_payload(b"ok");

    let tx = engine.acquire_tx().expect("TX slot");
    engine.encode_tx(tx, &reply).expect("encode_tx");
    engine.set_tx_endpoint(tx, dest).expect("TX endpoint");
    engine.send_tx(&mut server, tx).expect("send_tx");
    engine.release_tx(tx).expect("release TX");
    engine.release_rx(rx).expect("release RX");

    let _ = progress.observe_notify();
    let _ = progress.observe_expired();
    let _ = progress.qblock_recover();

    let mut buf = [0u8; 1472];
    let n = match DatagramIo::recv(&mut client, &mut buf).expect("client recv") {
        Some((n, _)) => n,
        None => panic!("client expected a response"),
    };
    let parsed = decode(&buf[..n]).expect("client decode");
    assert_eq!(parsed.code(), Code::CONTENT);
    eprintln!(
        "udp_engine: {} via DatagramIo {:?}",
        parsed.code(),
        parsed.payload()
    );
}

fn send_client_get<T: DatagramIo>(client: &mut T, server: Endpoint)
where
    T::Error: core::fmt::Debug,
{
    // Caller entropy. The core does not call an OS RNG.
    let token = Token::mint(2, &[0xC0, 0xA1]).expect("token");
    let mut ids = Ids::new(0x1000);
    let mut opts = OptionsBuilder::<4>::new();
    opts.push(Opt::uri_path("hello")).expect("path room");
    let msg = ids.con(Code::GET, token).with_options(opts.as_slice());
    let mut wire = [0u8; 1472];
    let n = encode(&msg, &mut wire).expect("encode GET");
    client.send(server, &wire[..n]).expect("client send");
}
