//! RFC 8613 Appendix C vectors and a pairwise request/response.

use super::{
    DeriveParams, Error, LIVE_REQUESTS, OscoreContext, RequestRef, SecurityContext, cbor,
    header::PartialIv,
};
use crate::message::{Code, Message, MessageId, Opt, OptionsBuilder, Token, Type, decode, encode};
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
