//! Router and [`Server::poll`] against a loopback [`DatagramIo`].

use super::{Reply, Request, Resource, Server};
use crate::message::{
    Code, ContentFormat, EncodedUint, Message, MessageId, Opt, OptionsBuilder, Token, Type, decode,
    encode,
};
use crate::storage::{DatagramIo, Endpoint, EngineBuilder, Memory, profiles};

struct Temp;

impl Resource for Temp {
    fn handle<'a>(_request: &'a Request<'a>) -> Reply<'a> {
        Reply::content(b"21.5").with_content_format(ContentFormat::TEXT_PLAIN)
    }
}

struct Led;

impl Resource for Led {
    fn handle<'a>(request: &'a Request<'a>) -> Reply<'a> {
        match request.method() {
            Some(super::Method::Get) => Reply::content(b"off"),
            Some(super::Method::Put) => Reply::changed(),
            _ => Reply::method_not_allowed(),
        }
    }
}

#[derive(Default)]
struct Loopback {
    inbox: Option<(Endpoint, [u8; 256], usize)>,
    last_send: Option<(Endpoint, [u8; 256], usize)>,
}

impl DatagramIo for Loopback {
    type Error = &'static str;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((ep, bytes, n)) = self.inbox.take() else {
            return Ok(None);
        };
        if n > buf.len() {
            return Err("short buf");
        }
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(Some((n, ep)))
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if bytes.len() > 256 {
            return Err("too long");
        }
        let mut slot = [0u8; 256];
        slot[..bytes.len()].copy_from_slice(bytes);
        self.last_send = Some((dest, slot, bytes.len()));
        Ok(bytes.len())
    }
}

fn engine() -> crate::Engine<Memory<profiles::Default>> {
    EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .expect("build")
}

fn encode_req(code: Code, path: &[&str], payload: &[u8]) -> ([u8; 256], usize) {
    encode_req_ty(Type::Confirmable, code, path, payload, None)
}

fn encode_req_ty(
    ty: Type,
    code: Code,
    path: &[&str],
    payload: &[u8],
    no_response: Option<u32>,
) -> ([u8; 256], usize) {
    let token = Token::new(&[0xA1]).expect("token");
    let mut opts = OptionsBuilder::<8>::new();
    for segment in path {
        opts.push(Opt::uri_path(segment)).expect("path");
    }
    let nr = no_response.map(EncodedUint::new);
    if let Some(ref encoded) = nr {
        opts.push(Opt::no_response(encoded)).expect("nr");
    }
    let msg = Message::new(ty, code, MessageId::new(0x1001))
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut buf = [0u8; 256];
    let n = encode(&msg, &mut buf).expect("encode");
    (buf, n)
}

fn server_with_routes(io: Loopback) -> Server<Memory<profiles::Default>, Loopback> {
    let mut server = Server::new(engine(), io);
    server
        .router()
        .at(&["sensors", "temp"])
        .get(Temp)
        .at(&["leds", "0"])
        .get(Led)
        .put(Led);
    server
}

struct LastReply {
    ty: Type,
    code: Code,
    payload: [u8; 64],
    payload_len: usize,
    content_format: Option<ContentFormat>,
}

fn last_reply(server: &Server<Memory<profiles::Default>, Loopback>) -> LastReply {
    let (_, bytes, n) = server.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..n]).expect("decode reply");
    let mut payload = [0u8; 64];
    let payload_len = parsed.payload().len().min(64);
    payload[..payload_len].copy_from_slice(&parsed.payload()[..payload_len]);
    LastReply {
        ty: parsed.ty(),
        code: parsed.code(),
        payload,
        payload_len,
        content_format: parsed.content_format().and_then(Result::ok),
    }
}

#[test]
fn get_sensors_temp_is_content() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    let parsed = last_reply(&server);
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"21.5");
    assert_eq!(parsed.content_format, Some(ContentFormat::TEXT_PLAIN));
    assert_eq!(server.engine_mut().rx_occupied(), 0);
    assert_eq!(server.engine_mut().tx_occupied(), 0);
}

#[test]
fn unknown_path_is_not_found() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["nope"], &[]);
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    assert_eq!(last_reply(&server).code, Code::NOT_FOUND);
}

#[test]
fn wrong_method_is_not_allowed() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["sensors", "temp"], &[]);
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    assert_eq!(last_reply(&server).code, Code::METHOD_NOT_ALLOWED);
}

#[test]
fn put_led_is_changed() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::PUT, &["leds", "0"], b"on");
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    assert_eq!(last_reply(&server).code, Code::CHANGED);
}

#[test]
fn get_led_is_content() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    let parsed = last_reply(&server);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"off");
}

#[test]
fn non_request_gets_non_response() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req_ty(
        Type::NonConfirmable,
        Code::GET,
        &["sensors", "temp"],
        &[],
        None,
    );
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    let parsed = last_reply(&server);
    assert_eq!(parsed.ty, Type::NonConfirmable);
    assert_eq!(parsed.code, Code::CONTENT);
}

#[test]
fn no_response_suppresses_success() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req_ty(
        Type::NonConfirmable,
        Code::GET,
        &["sensors", "temp"],
        &[],
        Some(u32::from(crate::NoResponse::SUPPRESS_2)),
    );
    let mut server = server_with_routes(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    server.poll(0).expect("poll");
    assert!(server.transport().last_send.is_none());
}

#[test]
fn router_dispatch_table() {
    let mut router = super::Router::<4>::new();
    router
        .at(&["sensors", "temp"])
        .get(Temp)
        .at(&["leds", "0"])
        .put(Led);

    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(router.dispatch(&req).code(), Code::CONTENT);

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(router.dispatch(&req).code(), Code::METHOD_NOT_ALLOWED);
}

fn slot0() -> crate::SlotId {
    crate::storage::SlotId::from_index(0)
}

#[test]
fn reply_builders() {
    assert_eq!(Reply::changed().code(), Code::CHANGED);
    assert_eq!(Reply::not_found().code(), Code::NOT_FOUND);
    assert_eq!(Reply::method_not_allowed().code(), Code::METHOD_NOT_ALLOWED);
    assert_eq!(Reply::content(b"x").payload(), b"x");
    let cf = Reply::content(b"x")
        .with_content_format(ContentFormat::LINK_FORMAT)
        .with_max_age(60)
        .with_observe(3)
        .with_etag(b"ab");
    assert_eq!(cf.content_format(), Some(ContentFormat::LINK_FORMAT));
    assert_eq!(cf.max_age(), Some(60));
    assert_eq!(cf.observe(), Some(3));
    assert_eq!(cf.etag(), Some(&b"ab"[..]));
}
