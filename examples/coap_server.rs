//! Taste of [`coaptic::Server`]: routes and resources, one `poll` loop.
//!
//! ```text
//! cargo run --example coap_server --features std
//! ```
//!
//! Client GET `/sensors/temp` → 2.05 Content. Engine slots stay off the
//! happy path (`CALLER.md`).

use std::net::UdpSocket;
use std::time::Instant;

use coaptic::{
    Code, ContentFormat, DatagramIo, Endpoint, EngineBuilder, Ids, Memory, Opt, OptionsBuilder,
    Reply, Request, Resource, Server, Token, decode, encode, profiles,
};

/// GET /sensors/temp
struct Temp;

impl Resource for Temp {
    fn handle<'a>(_request: &'a Request<'a>) -> Reply<'a> {
        Reply::content(b"21.5").with_content_format(ContentFormat::TEXT_PLAIN)
    }
}

/// GET or PUT /leds/0
struct Led;

impl Resource for Led {
    fn handle<'a>(request: &'a Request<'a>) -> Reply<'a> {
        match request.method() {
            Some(coaptic::Method::Get) => Reply::content(b"off"),
            Some(coaptic::Method::Put) => Reply::changed(),
            _ => Reply::method_not_allowed(),
        }
    }
}

/// GET /.well-known/core — link-format is just a payload.
struct CoreLink;

impl Resource for CoreLink {
    fn handle<'a>(_request: &'a Request<'a>) -> Reply<'a> {
        Reply::content(b"</sensors/temp>;if=\"sensor\",</leds/0>")
            .with_content_format(ContentFormat::LINK_FORMAT)
    }
}

fn now_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn main() {
    let origin = Instant::now();

    let engine = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(true)
        .build(Memory::<profiles::Default>::with_block_wise())
        .expect("Default + block_wise");

    let socket = UdpSocket::bind("127.0.0.1:0").expect("server bind");
    let mut client = UdpSocket::bind("127.0.0.1:0").expect("client bind");
    let server_ep = Endpoint::from(socket.local_addr().expect("server addr"));
    let timeout = std::time::Duration::from_millis(200);
    socket
        .set_read_timeout(Some(timeout))
        .expect("server timeout");
    client
        .set_read_timeout(Some(timeout))
        .expect("client timeout");

    let mut server = Server::new(engine, socket);
    server
        .router()
        .at(&["sensors", "temp"])
        .get(Temp)
        .at(&["leds", "0"])
        .get(Led)
        .put(Led)
        .at(&[".well-known", "core"])
        .get(CoreLink);

    send_client_get(&mut client, server_ep, &["sensors", "temp"]);
    server.poll(now_ms(origin)).expect("poll");

    let mut buf = [0u8; 1472];
    let n = match DatagramIo::recv(&mut client, &mut buf).expect("client recv") {
        Some((n, _)) => n,
        None => panic!("client expected a response"),
    };
    let parsed = decode(&buf[..n]).expect("client decode");
    assert_eq!(parsed.code(), Code::CONTENT);
    assert_eq!(parsed.payload(), b"21.5");
    eprintln!(
        "coap_server: {} {:?}",
        parsed.code(),
        core::str::from_utf8(parsed.payload()).unwrap_or("?")
    );
}

fn send_client_get<T: DatagramIo>(client: &mut T, server: Endpoint, path: &[&str])
where
    T::Error: core::fmt::Debug,
{
    let token = Token::mint(2, &[0xC0, 0xA1]).expect("token");
    let mut ids = Ids::new(0x1000);
    let mut opts = OptionsBuilder::<4>::new();
    for segment in path {
        opts.push(Opt::uri_path(segment)).expect("path room");
    }
    let msg = ids.con(Code::GET, token).with_options(opts.as_slice());
    let mut wire = [0u8; 1472];
    let n = encode(&msg, &mut wire).expect("encode GET");
    client.send(server, &wire[..n]).expect("client send");
}
