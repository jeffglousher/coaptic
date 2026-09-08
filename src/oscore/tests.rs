//! RFC 8613 Appendix C vectors and a pairwise request/response.

use super::{
    DeriveParams, Error, LIVE_REQUESTS, OscoreContext, RequestRef, SecurityContext, cbor,
    header::{self, OptionClass, PartialIv},
};
use crate::message::{
    BlockValue, Code, ContentFormat, Message, MessageId, NoResponse, Opt, OptionsBuilder, Token,
    Type, decode, encode, encode_uint,
};
use crate::storage::{DatagramIo, Endpoint};

fn hex(s: &str) -> [u8; 64] {
    let mut out = [0u8; 64];
    let n = s.len() / 2;
    assert!(n <= 64);
    for i in 0..n {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

fn hex_n(s: &str) -> ([u8; 64], usize) {
    (hex(s), s.len() / 2)
}

fn slice(buf: &[u8; 64], n: usize) -> &[u8] {
    &buf[..n]
}

const MASTER_SECRET: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];
const MASTER_SALT: [u8; 8] = [0x9e, 0x7c, 0xa9, 0x22, 0x23, 0x78, 0x63, 0x40];

fn client_c1() -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[],
        recipient_id: &[0x01],
        id_context: &[],
    })
    .unwrap()
}

fn server_c1() -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[0x01],
        recipient_id: &[],
        id_context: &[],
    })
    .unwrap()
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
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(Some((n, ep)))
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        let mut slot = [0u8; 256];
        slot[..bytes.len()].copy_from_slice(bytes);
        self.last_send = Some((dest, slot, bytes.len()));
        Ok(bytes.len())
    }
}

#[test]
fn c1_key_derivation_with_master_salt() {
    let client = client_c1();
    assert_eq!(
        client.sender_key(),
        &[
            0xf0, 0x91, 0x0e, 0xd7, 0x29, 0x5e, 0x6a, 0xd4, 0xb5, 0x4f, 0xc7, 0x93, 0x15, 0x43,
            0x02, 0xff
        ]
    );
    assert_eq!(
        client.recipient_key(),
        &[
            0xff, 0xb1, 0x4e, 0x09, 0x3c, 0x94, 0xc9, 0xca, 0xc9, 0x47, 0x16, 0x48, 0xb4, 0xf9,
            0x87, 0x10
        ]
    );
    assert_eq!(
        client.common_iv(),
        &[
            0x46, 0x22, 0xd4, 0xdd, 0x6d, 0x94, 0x41, 0x68, 0xee, 0xfb, 0x54, 0x98, 0x7c
        ]
    );

    let server = server_c1();
    assert_eq!(server.sender_key(), client.recipient_key());
    assert_eq!(server.recipient_key(), client.sender_key());
    assert_eq!(server.common_iv(), client.common_iv());
}

#[test]
fn c1_info_cbor() {
    let mut info = [0u8; 32];
    let n = cbor::encode_info(&mut info, &[], None, "Key", 16).unwrap();
    assert_eq!(&info[..n], &hex("8540f60a634b657910")[..9]);
    let n = cbor::encode_info(&mut info, &[0x01], None, "Key", 16).unwrap();
    assert_eq!(&info[..n], &hex("854101f60a634b657910")[..10]);
    let n = cbor::encode_info(&mut info, &[], None, "IV", 13).unwrap();
    assert_eq!(&info[..n], &hex("8540f60a6249560d")[..8]);
}

#[test]
fn c2_key_derivation_without_salt() {
    let client = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &[],
        sender_id: &[0x00],
        recipient_id: &[0x01],
        id_context: &[],
    })
    .unwrap();
    assert_eq!(
        client.sender_key(),
        &[
            0x32, 0x1b, 0x26, 0x94, 0x32, 0x53, 0xc7, 0xff, 0xb6, 0x00, 0x3b, 0x0b, 0x64, 0xd7,
            0x40, 0x41
        ]
    );
    assert_eq!(
        client.recipient_key(),
        &[
            0xe5, 0x7b, 0x56, 0x35, 0x81, 0x51, 0x77, 0xcd, 0x67, 0x9a, 0xb4, 0xbc, 0xec, 0x9d,
            0x7d, 0xda
        ]
    );
    assert_eq!(
        client.common_iv(),
        &[
            0xbe, 0x35, 0xae, 0x29, 0x7d, 0x2d, 0xac, 0xe9, 0x10, 0xc5, 0x2e, 0x99, 0xf9
        ]
    );
}

#[test]
fn c3_key_derivation_with_id_context() {
    let id_context = [0x37, 0xcb, 0xf3, 0x21, 0x00, 0x17, 0xa2, 0xd3];
    let client = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[],
        recipient_id: &[0x01],
        id_context: &id_context,
    })
    .unwrap();
    assert_eq!(
        client.sender_key(),
        &[
            0xaf, 0x2a, 0x13, 0x00, 0xa5, 0xe9, 0x57, 0x88, 0xb3, 0x56, 0x33, 0x6e, 0xee, 0xcd,
            0x2b, 0x92
        ]
    );
    assert_eq!(
        client.common_iv(),
        &[
            0x2c, 0xa5, 0x8f, 0xb8, 0x5f, 0xf1, 0xb8, 0x1c, 0x0b, 0x71, 0x81, 0xb8, 0x5e
        ]
    );
}

#[test]
fn c4_protected_request() {
    let mut client = client_c1();
    client.set_sender_seq(20);
    let mut host_path = OptionsBuilder::<2>::new();
    host_path.push(Opt::uri_host("localhost")).unwrap();
    host_path.push(Opt::uri_path("tv1")).unwrap();
    let plain = Message::new(Type::Confirmable, Code::GET, MessageId::new(0x5d1f))
        .with_token(Token::from_checked(&[0x00, 0x00, 0x39, 0x74]))
        .with_options(host_path.as_slice());

    let mut out = [0u8; 64];
    let n = client.protect_request(&plain, &mut out).unwrap();
    let (expected, en) =
        hex_n("44025d1f00003974396c6f63616c686f7374620914ff612f1092f1776f1c1668b3825e");
    assert_eq!(&out[..n], slice(&expected, en));
}

#[test]
fn c5_protected_request_nonzero_kid() {
    let mut client = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &[],
        sender_id: &[0x00],
        recipient_id: &[0x01],
        id_context: &[],
    })
    .unwrap();
    client.set_sender_seq(20);
    let mut host_path = OptionsBuilder::<2>::new();
    host_path.push(Opt::uri_host("localhost")).unwrap();
    host_path.push(Opt::uri_path("tv1")).unwrap();
    let plain = Message::new(Type::Confirmable, Code::GET, MessageId::new(0x71c3))
        .with_token(Token::from_checked(&[0x00, 0x00, 0xb9, 0x32]))
        .with_options(host_path.as_slice());

    let mut out = [0u8; 64];
    let n = client.protect_request(&plain, &mut out).unwrap();
    let (expected, en) =
        hex_n("440271c30000b932396c6f63616c686f737463091400ff4ed339a5a379b0b8bc731fffb0");
    assert_eq!(&out[..n], slice(&expected, en));
}

#[test]
fn c6_protected_request_kid_context() {
    let id_context = [0x37, 0xcb, 0xf3, 0x21, 0x00, 0x17, 0xa2, 0xd3];
    let mut client = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[],
        recipient_id: &[0x01],
        id_context: &id_context,
    })
    .unwrap();
    client.set_sender_seq(20);
    let mut host_path = OptionsBuilder::<2>::new();
    host_path.push(Opt::uri_host("localhost")).unwrap();
    host_path.push(Opt::uri_path("tv1")).unwrap();
    let plain = Message::new(Type::Confirmable, Code::GET, MessageId::new(0x2f8e))
        .with_token(Token::from_checked(&[0xef, 0x9b, 0xbf, 0x7a]))
        .with_options(host_path.as_slice());

    let mut out = [0u8; 80];
    let n = client.protect_request(&plain, &mut out).unwrap();
    let (expected, en) = hex_n(
        "44022f8eef9bbf7a396c6f63616c686f73746b19140837cbf3210017a2d3ff72cd7273fd331ac45cffbe55c3",
    );
    assert_eq!(&out[..n], slice(&expected, en));
}

#[test]
fn c7_protected_response() {
    let mut client = client_c1();
    client.set_sender_seq(20);
    let mut server = server_c1();

    let mut host_path = OptionsBuilder::<2>::new();
    host_path.push(Opt::uri_host("localhost")).unwrap();
    host_path.push(Opt::uri_path("tv1")).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(0x5d1f))
        .with_token(Token::from_checked(&[0x00, 0x00, 0x39, 0x74]))
        .with_options(host_path.as_slice());

    let mut wire = [0u8; 64];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected_req = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 64];
    let (_plain, request) = server
        .unprotect_request(&protected_req, &mut inner)
        .unwrap();

    let resp = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(0x5d1f))
        .with_token(Token::from_checked(&[0x00, 0x00, 0x39, 0x74]))
        .with_payload(b"Hello World!");

    let mut out = [0u8; 64];
    let n = server.protect_response(&resp, request, &mut out).unwrap();
    let (expected, en) = hex_n("64445d1f0000397490ffdbaad1e9a7e7b2a813d3c31524378303cdafae119106");
    assert_eq!(&out[..n], slice(&expected, en));

    let protected_resp = decode(&out[..n]).unwrap();
    let mut opened = [0u8; 64];
    let inner = client
        .unprotect_response(&protected_resp, request, &mut opened)
        .unwrap();
    assert_eq!(inner.code(), Code::CONTENT);
    assert_eq!(inner.payload(), b"Hello World!");
}

#[test]
fn c8_protected_response_with_piv() {
    let mut server = server_c1();
    let request = super::RequestRef::from_kid(&[], PartialIv::from_seq(20)).unwrap();
    let resp = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(0x5d1f))
        .with_token(Token::from_checked(&[0x00, 0x00, 0x39, 0x74]))
        .with_payload(b"Hello World!");
    let mut out = [0u8; 64];
    let n = server
        .protect_response_with_piv(&resp, request, &mut out)
        .unwrap();
    // Appendix C.8
    let (expected, en) =
        hex_n("64445d1f00003974920100ff4d4c13669384b67354b2b6175ff4b8658c666a6cf88e");
    assert_eq!(&out[..n], slice(&expected, en));
}

#[test]
fn replay_rejects_duplicate_piv() {
    let mut client = client_c1();
    let mut server = server_c1();
    let mut path = OptionsBuilder::<1>::new();
    path.push(Opt::uri_path("tv1")).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 64];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 64];
    server.unprotect_request(&protected, &mut inner).unwrap();
    let mut inner2 = [0u8; 64];
    assert_eq!(
        server.unprotect_request(&protected, &mut inner2),
        Err(Error::Replay)
    );
}

#[test]
fn trait_object_not_needed_but_impl_works() {
    fn protect<C: OscoreContext>(ctx: &mut C, msg: &Message<'_>, out: &mut [u8]) -> usize {
        ctx.protect_request(msg, out).unwrap()
    }
    let mut client = client_c1();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]));
    let mut out = [0u8; 64];
    assert!(protect(&mut client, &req, &mut out) > 4);
}

#[test]
fn app_protected_get_round_trip() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"Hello World!")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected request");
    let req = decode(&bytes[..n]).unwrap();
    assert_eq!(req.code(), Code::POST);
    assert!(req.oscore().is_some());

    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(0).unwrap();
    let (_, bytes, n) = server.transport().last_send.expect("protected response");
    let resp = decode(&bytes[..n]).unwrap();
    assert_eq!(resp.code(), Code::CHANGED);
    assert!(resp.oscore().is_some());

    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(0).unwrap();
    let response = client.take_response(call).expect("unprotected response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"Hello World!");
}

#[test]
fn app_five_sequential_oscore_gets() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"ok")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    for i in 0..=LIVE_REQUESTS {
        let call = client
            .get("tv1")
            .to(server_ep)
            .send(u64::from(i as u32))
            .unwrap();
        let (_, bytes, n) = client.transport().last_send.expect("protected request");
        server.transport_mut().inbox = Some((client_ep, bytes, n));
        server.transport_mut().last_send = None;
        server.poll(u64::from(i as u32)).unwrap();
        let (_, bytes, n) = server
            .transport()
            .last_send
            .unwrap_or_else(|| panic!("response {i} dropped (live table?)"));
        client.transport_mut().inbox = Some((server_ep, bytes, n));
        client.poll(u64::from(i as u32)).unwrap();
        let response = client
            .take_response(call)
            .unwrap_or_else(|| panic!("call {i} incomplete"));
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), b"ok");
    }
    assert_eq!(
        client.oscore().expect("ctx").sender_seq(),
        (LIVE_REQUESTS + 1) as u64
    );
}

#[test]
fn app_plain_response_does_not_complete_oscore_call() {
    use crate::{App, profiles};

    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected request");
    let req = decode(&bytes[..n]).unwrap();
    assert!(req.oscore().is_some());

    let plain = Message::new(Type::Acknowledgement, Code::CONTENT, req.message_id())
        .with_token(req.token())
        .with_payload(b"pwned");
    let mut wire = [0u8; 256];
    let pn = encode(&plain, &mut wire).unwrap();
    client.transport_mut().inbox = Some((server_ep, wire, pn));
    client.poll(0).unwrap();
    assert!(
        client.take_response(call).is_none(),
        "unprotected 2.xx must not complete an OSCORE Call"
    );
}

#[test]
fn app_bad_ciphertext_is_silent_drop() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"Hello World!")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let _ = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, mut bytes, n) = client.transport().last_send.expect("protected request");
    bytes[n - 1] ^= 0xff;
    let parsed = decode(&bytes[..n]).unwrap();
    assert!(parsed.oscore().is_some());
    assert!(parsed.code().is_request());

    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.transport_mut().last_send = None;
    server.poll(0).unwrap();
    assert!(
        server.transport().last_send.is_none(),
        "AEAD failure must not emit an unprotected 4.00"
    );
}

#[test]
fn app_oscore_con_retransmit_replays_protected_ack() {
    use crate::{App, Request, Response, get, profiles};
    use core::sync::atomic::{AtomicU32, Ordering};

    static HITS: AtomicU32 = AtomicU32::new(0);
    fn hello(_req: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"Hello World!")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let _ = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected request");

    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(0).unwrap();
    let first = server.transport().last_send.expect("first ACK");

    server.transport_mut().last_send = None;
    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(1).unwrap();
    let replay = server.transport().last_send.expect("Dedup replay");
    assert_eq!(first, replay);
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
}

#[test]
fn live_request_table_is_four_and_saturates() {
    let mut ctx = client_c1();
    let request = RequestRef::from_kid(&[], PartialIv::from_seq(1)).unwrap();
    for i in 0..LIVE_REQUESTS {
        let token = Token::from_checked(&[i as u8 + 1]);
        ctx.remember(token, request).unwrap();
    }
    assert_eq!(
        ctx.remember(Token::from_checked(&[LIVE_REQUESTS as u8 + 1]), request),
        Err(Error::Saturated)
    );
}

#[test]
fn observe_register_is_outer_fetch_and_dual_class() {
    let mut client = client_c1();
    let mut path = OptionsBuilder::<2>::new();
    path.push(Opt::uri_path("obs")).unwrap();
    path.push(Opt::observe_register()).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 64];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::FETCH);
    assert_eq!(outer.observe().and_then(Result::ok), Some(0));
    assert!(outer.oscore().is_some());

    let mut server = server_c1();
    let mut inner = [0u8; 64];
    let (plain, _) = server.unprotect_request(&outer, &mut inner).unwrap();
    assert_eq!(plain.code(), Code::GET);
    assert_eq!(plain.observe().and_then(Result::ok), Some(0));
}

#[test]
fn observe_notification_inner_empty_outer_seq_and_piv() {
    let mut client = client_c1();
    let mut server = server_c1();
    let mut path = OptionsBuilder::<2>::new();
    path.push(Opt::uri_path("obs")).unwrap();
    path.push(Opt::observe_register()).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 128];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected_req = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 128];
    let (_plain, request) = server
        .unprotect_request(&protected_req, &mut inner)
        .unwrap();

    let seq = crate::message::encode_uint(3);
    let mut opts = OptionsBuilder::<1>::new();
    opts.push(Opt::observe(&seq)).unwrap();
    let resp = Message::new(Type::NonConfirmable, Code::CONTENT, MessageId::new(2))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice())
        .with_payload(b"obs-1");
    let n = server
        .protect_response_with_piv(&resp, request, &mut wire)
        .unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::CONTENT);
    assert_eq!(outer.observe().and_then(Result::ok), Some(3));
    assert_eq!(
        outer.max_age().and_then(Result::ok),
        Some(0),
        "Observe notification: Outer Max-Age 0 (RFC 8613 §4.1.3.1)"
    );
    let header = super::OscoreHeader::parse(outer.oscore().unwrap()).unwrap();
    assert!(header.piv.is_some(), "notification MUST carry a Partial IV");

    let opened = client
        .unprotect_response(&outer, request, &mut inner)
        .unwrap();
    assert_eq!(opened.code(), Code::CONTENT);
    assert_eq!(opened.payload(), b"obs-1");
    assert_eq!(opened.observe().and_then(Result::ok), Some(3));
}

#[test]
fn app_oscore_observe_register_notify() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"obs-0").observe(0)
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("obs", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("obs").observe().to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected register");
    let req = decode(&bytes[..n]).unwrap();
    assert_eq!(req.code(), Code::FETCH);
    assert!(req.oscore().is_some());
    assert_eq!(req.observe().and_then(Result::ok), Some(0));

    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(0).unwrap();
    let (_, bytes, n) = server
        .transport()
        .last_send
        .expect("protected register ACK");
    let resp = decode(&bytes[..n]).unwrap();
    assert_eq!(resp.code(), Code::CONTENT);
    assert!(resp.oscore().is_some());
    assert_eq!(
        resp.max_age().and_then(Result::ok),
        Some(0),
        "Observe register ACK: Outer Max-Age 0"
    );

    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(0).unwrap();
    let initial = client.take_response(call).expect("register");
    assert_eq!(initial.code(), Code::CONTENT);
    assert_eq!(initial.payload(), b"obs-0");
    assert!(initial.observe_seq().is_some());

    server.transport_mut().last_send = None;
    let sent = server
        .notify(10, &["obs"], Response::content(b"obs-1"))
        .expect("notify");
    assert_eq!(sent, 1);
    let (_, bytes, n) = server.transport().last_send.expect("protected notify");
    let note = decode(&bytes[..n]).unwrap();
    assert!(note.oscore().is_some());
    assert_eq!(note.code(), Code::CONTENT);
    assert!(note.observe().is_some());

    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(10).unwrap();
    let got = client.take_response(call).expect("protected notification");
    assert_eq!(got.payload(), b"obs-1");
    assert!(got.observe_seq().is_some());
}

#[test]
fn app_plain_notify_does_not_complete_oscore_observe() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"obs-0").observe(0)
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("obs", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("obs").observe().to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("register");
    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(0).unwrap();
    let (_, bytes, n) = server.transport().last_send.expect("ACK");
    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(0).unwrap();
    let _ = client.take_response(call).expect("initial");

    let seq = crate::message::encode_uint(1);
    let opts = [Opt::observe(&seq)];
    let plain = Message::new(Type::NonConfirmable, Code::CONTENT, MessageId::new(99))
        .with_token(call.token())
        .with_options(&opts)
        .with_payload(b"pwned");
    let mut wire = [0u8; 256];
    let pn = encode(&plain, &mut wire).unwrap();
    client.transport_mut().inbox = Some((server_ep, wire, pn));
    client.poll(10).unwrap();
    assert!(
        client.take_response(call).is_none(),
        "plain Observe notify must not complete an OSCORE Call"
    );
}

#[test]
fn derive_rejects_identical_ids() {
    assert_eq!(
        SecurityContext::derive(DeriveParams {
            master_secret: &MASTER_SECRET,
            master_salt: &[],
            sender_id: &[1],
            recipient_id: &[1],
            id_context: &[],
        })
        .unwrap_err(),
        Error::IdCollision
    );
}

/// RFC 8613 Figure 5 + encode_as_outer decisions.
#[test]
fn figure5_option_class_matrix() {
    // Class U only.
    for n in [3, 7, 9, 35, 39] {
        assert_eq!(header::classify(n), OptionClass::Outer, "U-only {n}");
        assert!(!header::classify(n).in_plaintext());
        assert!(header::encode_as_outer(n));
    }
    // Hop-Limit (RFC 8768): not in Figure 5; proxy processing → Outer.
    assert_eq!(header::classify(16), OptionClass::Outer);
    assert!(header::encode_as_outer(16));

    // Dual (E+U).
    for n in [6, 14, 23, 27, 28, 60, 258] {
        assert_eq!(header::classify(n), OptionClass::Dual, "Dual {n}");
        assert!(header::classify(n).in_plaintext());
    }
    assert!(header::encode_as_outer(6), "Observe is both fields");
    assert!(
        !header::encode_as_outer(14),
        "application Max-Age stays Inner (§4.1.3.1)"
    );
    assert!(
        !header::encode_as_outer(23) && !header::encode_as_outer(27),
        "Block stays Inner-only on encode"
    );
    assert!(!header::encode_as_outer(28) && !header::encode_as_outer(60));
    assert!(
        !header::encode_as_outer(258),
        "No-Response MUST be Inner; Outer SHOULD NOT (§4.1.3.6)"
    );

    // Class E only (Figure 5), including ETag — not Dual.
    for n in [1, 4, 5, 8, 11, 12, 15, 17, 20] {
        assert_eq!(header::classify(n), OptionClass::Inner, "E-only {n}");
        assert!(header::classify(n).in_plaintext());
        assert!(!header::encode_as_outer(n));
    }
    // Unknown / later options: Class E (§4.1).
    for n in [19, 31, 99, 252, 292] {
        assert_eq!(header::classify(n), OptionClass::Inner, "unknown {n}");
        assert!(!header::encode_as_outer(n));
    }
}

fn inject_outer_opts(
    protected: &crate::message::ParsedMessage<'_>,
    extra: &[Opt<'_>],
    out: &mut [u8],
) -> usize {
    let mut opts = OptionsBuilder::<8>::new();
    for opt in protected.options() {
        opts.push(opt).unwrap();
    }
    for opt in extra {
        opts.push(*opt).unwrap();
    }
    let msg = Message::new(protected.ty(), protected.code(), protected.message_id())
        .with_token(protected.token())
        .with_options(opts.as_slice())
        .with_payload(protected.payload());
    msg.encode(out).unwrap()
}

#[test]
fn protect_keeps_max_age_etag_inner_not_outer() {
    let mut client = client_c1();
    let mut server = server_c1();
    let mut path = OptionsBuilder::<1>::new();
    path.push(Opt::uri_path("tv1")).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 128];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected_req = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 128];
    let (_plain, request) = server
        .unprotect_request(&protected_req, &mut inner)
        .unwrap();

    let age = encode_uint(120);
    let mut opts = OptionsBuilder::<2>::new();
    opts.push(Opt::etag(b"v1")).unwrap();
    opts.push(Opt::max_age(&age)).unwrap();
    let resp = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice())
        .with_payload(b"ok");
    let n = server.protect_response(&resp, request, &mut wire).unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::CHANGED);
    assert!(outer.oscore().is_some());
    assert!(
        outer.etag().next().is_none(),
        "ETag is Class E — must not appear Outer"
    );
    assert!(
        outer.max_age().is_none(),
        "successful non-Observe response must not copy application Max-Age to Outer"
    );

    let opened = client
        .unprotect_response(&outer, request, &mut inner)
        .unwrap();
    assert_eq!(opened.code(), Code::CONTENT);
    assert_eq!(opened.etag().next(), Some(&b"v1"[..]));
    assert_eq!(opened.max_age().and_then(Result::ok), Some(120));
}

#[test]
fn protect_keeps_no_response_inner_not_outer() {
    let mut client = client_c1();
    let nr = encode_uint(u32::from(NoResponse::SUPPRESS_ALL));
    let mut opts = OptionsBuilder::<2>::new();
    opts.push(Opt::uri_path("tv1")).unwrap();
    opts.push(Opt::no_response(&nr)).unwrap();
    let req = Message::new(Type::NonConfirmable, Code::POST, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice());
    let mut wire = [0u8; 128];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::POST);
    assert!(
        outer.no_response().is_none(),
        "No-Response MUST be Inner; Outer SHOULD NOT (§4.1.3.6)"
    );

    let mut server = server_c1();
    let mut inner = [0u8; 128];
    let (plain, _) = server.unprotect_request(&outer, &mut inner).unwrap();
    assert_eq!(
        plain
            .no_response()
            .and_then(Result::ok)
            .map(NoResponse::get),
        Some(NoResponse::SUPPRESS_ALL)
    );
}

#[test]
fn protect_observe_response_outer_max_age_zero_keeps_inner() {
    let mut client = client_c1();
    let mut server = server_c1();
    let mut path = OptionsBuilder::<2>::new();
    path.push(Opt::uri_path("obs")).unwrap();
    path.push(Opt::observe_register()).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 128];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected_req = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 128];
    let (_plain, request) = server
        .unprotect_request(&protected_req, &mut inner)
        .unwrap();

    let seq = encode_uint(1);
    let age = encode_uint(90);
    let mut opts = OptionsBuilder::<2>::new();
    opts.push(Opt::observe(&seq)).unwrap();
    opts.push(Opt::max_age(&age)).unwrap();
    let resp = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice())
        .with_payload(b"obs");
    let n = server.protect_response(&resp, request, &mut wire).unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::CONTENT);
    assert_eq!(outer.max_age().and_then(Result::ok), Some(0));

    let opened = client
        .unprotect_response(&outer, request, &mut inner)
        .unwrap();
    assert_eq!(
        opened.max_age().and_then(Result::ok),
        Some(90),
        "Inner Max-Age is the application value; Outer 0 is discarded (§8.4)"
    );
}

#[test]
fn stitch_discards_injected_outer_etag_max_age_no_response() {
    let mut client = client_c1();
    let mut server = server_c1();
    let mut path = OptionsBuilder::<1>::new();
    path.push(Opt::uri_path("tv1")).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 192];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected_req = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 192];
    let (_plain, request) = server
        .unprotect_request(&protected_req, &mut inner)
        .unwrap();

    let age = encode_uint(45);
    let mut opts = OptionsBuilder::<2>::new();
    opts.push(Opt::etag(b"real")).unwrap();
    opts.push(Opt::max_age(&age)).unwrap();
    let resp = Message::new(Type::Acknowledgement, Code::CONTENT, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice())
        .with_payload(b"ok");
    let n = server.protect_response(&resp, request, &mut wire).unwrap();
    let protected = decode(&wire[..n]).unwrap();

    let fake_age = encode_uint(3600);
    let fake_nr = encode_uint(u32::from(NoResponse::SUPPRESS_ALL));
    let injected = [
        Opt::etag(b"pwned"),
        Opt::max_age(&fake_age),
        Opt::no_response(&fake_nr),
    ];
    let mut tampered = [0u8; 192];
    let tn = inject_outer_opts(&protected, &injected, &mut tampered);
    let tampered_msg = decode(&tampered[..tn]).unwrap();
    assert_eq!(tampered_msg.etag().next(), Some(&b"pwned"[..]));
    assert_eq!(tampered_msg.max_age().and_then(Result::ok), Some(3600));
    assert!(tampered_msg.no_response().is_some());

    let opened = client
        .unprotect_response(&tampered_msg, request, &mut inner)
        .unwrap();
    assert_eq!(
        opened.etag().next(),
        Some(&b"real"[..]),
        "Outer ETag must not replace Inner ETag (§8.4)"
    );
    assert_eq!(
        opened.max_age().and_then(Result::ok),
        Some(45),
        "Outer Max-Age must not replace Inner Max-Age (§8.4)"
    );
    assert!(
        opened.no_response().is_none(),
        "Outer No-Response is ignored (§4.1.3.6)"
    );
}

#[test]
fn protect_keeps_block2_inner_not_outer() {
    let mut client = client_c1();
    let block = BlockValue::from_size(0, false, 1024).expect("szx").encode();
    let mut opts = OptionsBuilder::<2>::new();
    opts.push(Opt::uri_path("large")).unwrap();
    opts.push(Opt::block2(&block)).unwrap();
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(opts.as_slice());
    let mut wire = [0u8; 128];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let outer = decode(&wire[..n]).unwrap();
    assert_eq!(outer.code(), Code::POST);
    assert!(outer.oscore().is_some());
    assert!(
        outer.block2().is_none(),
        "Inner Block-wise must not copy Block2 to Outer"
    );

    let mut server = server_c1();
    let mut inner = [0u8; 128];
    let (plain, _) = server.unprotect_request(&outer, &mut inner).unwrap();
    assert_eq!(plain.code(), Code::GET);
    let got = plain.block2().expect("Inner Block2").expect("val");
    assert_eq!(got.num(), 0);
    assert!(!got.more());
}

const LARGE: [u8; 2000] = [b'B'; 2000];
const WIRE: usize = 1472;

#[derive(Default)]
struct WideLoopback {
    inbox: Option<(Endpoint, [u8; WIRE], usize)>,
    last_send: Option<(Endpoint, [u8; WIRE], usize)>,
}

impl DatagramIo for WideLoopback {
    type Error = &'static str;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((ep, bytes, n)) = self.inbox.take() else {
            return Ok(None);
        };
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(Some((n, ep)))
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if bytes.len() > WIRE {
            return Err("too long");
        }
        let mut slot = [0u8; WIRE];
        slot[..bytes.len()].copy_from_slice(bytes);
        self.last_send = Some((dest, slot, bytes.len()));
        Ok(bytes.len())
    }
}

fn pump_oscore_block(
    client: &mut crate::App<crate::storage::profiles::Default, WideLoopback, 8, true>,
    server: &mut crate::App<crate::storage::profiles::Default, WideLoopback, 8, true>,
    client_ep: Endpoint,
    server_ep: Endpoint,
    now: u64,
) {
    if let Some((_, bytes, n)) = client.transport_mut().last_send.take() {
        server.transport_mut().inbox = Some((client_ep, bytes, n));
        server.poll(now).unwrap();
    }
    if let Some((_, bytes, n)) = server.transport_mut().last_send.take() {
        client.transport_mut().inbox = Some((server_ep, bytes, n));
        client.poll(now).unwrap();
    }
}

#[test]
fn app_oscore_block2_get_assembles() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_: Request<'_>) -> Response<'static> {
        Response::content(&LARGE).content_format(ContentFormat::OCTET_STREAM)
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .route("large", get(hello))
        .bind(WideLoopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("large").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected GET");
    let req = decode(&bytes[..n]).unwrap();
    assert!(req.oscore().is_some());
    assert!(req.block2().is_none(), "Block2 is Inner");

    for t in 0u64..8 {
        pump_oscore_block(&mut client, &mut server, client_ep, server_ep, t);
        if let Some(got) = client.take_response(call) {
            assert_eq!(got.code(), Code::CONTENT);
            assert_eq!(got.body().expect("assembled"), &LARGE[..]);
            assert!(
                server.metrics().block2_assemble >= 1 || client.metrics().block2_assemble >= 1,
                "Block2 path must not stay cold"
            );
            return;
        }
    }
    panic!("OSCORE Block2 GET did not complete");
}

#[test]
fn app_oscore_block1_put_assembles() {
    use crate::{App, Request, Response, profiles, put};

    fn accept(req: Request<'_>) -> Response<'static> {
        if req.body() == Some(&LARGE[..]) {
            Response::changed()
        } else {
            Response::new(Code::BAD_REQUEST)
        }
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .route("upload", put(accept))
        .bind(WideLoopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client
        .put("upload")
        .payload(&LARGE)
        .content_format(ContentFormat::OCTET_STREAM)
        .to(server_ep)
        .send(0)
        .unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected Block1");
    let req = decode(&bytes[..n]).unwrap();
    assert!(req.oscore().is_some());
    assert!(req.block1().is_none(), "Block1 is Inner");
    assert_eq!(req.code(), Code::POST);

    for t in 0u64..8 {
        pump_oscore_block(&mut client, &mut server, client_ep, server_ep, t);
        if let Some(got) = client.take_response(call) {
            assert_eq!(got.code(), Code::CHANGED);
            assert!(
                server.metrics().block1_assemble >= 1,
                "Block1 path must not stay cold"
            );
            return;
        }
    }
    panic!("OSCORE Block1 PUT did not complete");
}

#[test]
fn app_plain_block2_does_not_complete_oscore_call() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_: Request<'_>) -> Response<'static> {
        Response::content(&LARGE).content_format(ContentFormat::OCTET_STREAM)
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .route("large", get(hello))
        .bind(WideLoopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("large").to(server_ep).send(0).unwrap();
    pump_oscore_block(&mut client, &mut server, client_ep, server_ep, 0);
    assert!(
        client.take_response(call).is_none(),
        "first Block2 is not the complete body"
    );

    let token = call.token();
    let mid = client
        .transport()
        .last_send
        .map(|(_, bytes, n)| decode(&bytes[..n]).unwrap().message_id())
        .unwrap_or(MessageId::new(99));
    let last = BlockValue::from_size(1, false, 1024)
        .expect("num 1")
        .encode();
    let opts = [Opt::block2(&last)];
    let plain = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
        .with_token(token)
        .with_options(&opts)
        .with_payload(&LARGE[1024..]);
    let mut wire = [0u8; WIRE];
    let pn = encode(&plain, &mut wire).unwrap();
    client.transport_mut().inbox = Some((server_ep, wire, pn));
    client.poll(1).unwrap();
    assert!(
        client.take_response(call).is_none(),
        "unprotected Block2 must not complete an OSCORE Call"
    );

    // Real protected remainder still completes.
    for t in 2u64..10 {
        pump_oscore_block(&mut client, &mut server, client_ep, server_ep, t);
        if let Some(got) = client.take_response(call) {
            assert_eq!(got.body().expect("assembled"), &LARGE[..]);
            return;
        }
    }
    panic!("protected Block2 remainder did not complete after plain inject");
}

#[test]
fn app_oscore_max_age_etag_stay_inner() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"Hello World!").max_age(120).etag(b"v1")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);
    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected request");
    server.transport_mut().inbox = Some((client_ep, bytes, n));
    server.poll(0).unwrap();
    let (_, bytes, n) = server.transport().last_send.expect("protected response");
    let resp = decode(&bytes[..n]).unwrap();
    assert_eq!(resp.code(), Code::CHANGED);
    assert!(resp.oscore().is_some());
    assert!(resp.etag().next().is_none(), "App ETag must stay Inner");
    assert!(
        resp.max_age().is_none(),
        "App Max-Age must stay Inner on a non-Observe success"
    );

    client.transport_mut().inbox = Some((server_ep, bytes, n));
    client.poll(0).unwrap();
    let response = client.take_response(call).expect("unprotected response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"Hello World!");
    assert_eq!(response.etag_bytes(), Some(&b"v1"[..]));
}

#[test]
fn app_plain_etag_max_age_does_not_complete_oscore_call() {
    use crate::{App, profiles};

    let server_ep = Endpoint::v4([192, 0, 2, 2], 5683);

    let mut client = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .bind(Loopback::default())
        .unwrap();
    client.set_oscore(client_c1());

    let call = client.get("tv1").to(server_ep).send(0).unwrap();
    let (_, bytes, n) = client.transport().last_send.expect("protected request");
    let req = decode(&bytes[..n]).unwrap();

    let age = encode_uint(60);
    let opts = [Opt::etag(b"pwned"), Opt::max_age(&age)];
    let plain = Message::new(Type::Acknowledgement, Code::CONTENT, req.message_id())
        .with_token(req.token())
        .with_options(&opts)
        .with_payload(b"pwned");
    let mut wire = [0u8; 256];
    let pn = encode(&plain, &mut wire).unwrap();
    client.transport_mut().inbox = Some((server_ep, wire, pn));
    client.poll(0).unwrap();
    assert!(
        client.take_response(call).is_none(),
        "unprotected ETag/Max-Age 2.xx must not complete an OSCORE Call"
    );
}

#[test]
fn app_unprotected_request_is_401_max_age_zero() {
    use crate::{App, Request, Response, get, profiles};

    fn hello(_req: Request<'_>) -> Response<'static> {
        Response::content(b"nope")
    }

    let client_ep = Endpoint::v4([192, 0, 2, 1], 5683);

    let mut server = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route("tv1", get(hello))
        .bind(Loopback::default())
        .unwrap();
    server.set_oscore(server_c1());

    let mut path = OptionsBuilder::<1>::new();
    path.push(Opt::uri_path("tv1")).unwrap();
    let plain = Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
        .with_token(Token::from_checked(&[1]))
        .with_options(path.as_slice());
    let mut wire = [0u8; 256];
    let pn = encode(&plain, &mut wire).unwrap();
    server.transport_mut().inbox = Some((client_ep, wire, pn));
    server.poll(0).unwrap();
    let (_, bytes, n) = server.transport().last_send.expect("unprotected 4.01");
    let resp = decode(&bytes[..n]).unwrap();
    assert_eq!(resp.code(), Code::UNAUTHORIZED);
    assert!(
        resp.oscore().is_none(),
        "OSCORE processing 4.01 is unprotected"
    );
    assert_eq!(
        resp.max_age().and_then(Result::ok),
        Some(0),
        "unprotected OSCORE 4.01 uses Max-Age 0 (§8.2 / §4.1.3.1)"
    );
}
