//! Site and [`App::poll`] against a loopback [`DatagramIo`].

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{
    AppAssembled, DEFAULT_ROUTES, Error, INLINE_PAYLOAD, LINK_FORMAT_PER_ROUTE, LinkFormatScratch,
    Method, RESPONSE_BODY, Request, Response, Site, fetch, get, ipatch, link_format_capacity,
    patch, post, put,
};
use crate::app::App;
use crate::error::{EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Echo as EchoOpt, EncodedUint, Message, MessageId,
    MissingBlocks, ObserveTransmission, Opt, OptionNumber, OptionsBuilder, ProblemDetails,
    QBlockTransmission, Token, Transmission, Type, decode, encode, encode_uint,
};
use crate::storage::{
    BlockKey, DatagramIo, DedupEntry, DedupKey, Endpoint, ExchangeKey, MemoryLayout, MemoryProfile,
    ObserveKey, profiles,
};

const LARGE: [u8; 2000] = [b'A'; 2000];
const WIRE: usize = 1472;

fn get_temp(_req: Request<'_>) -> Response<'static> {
    Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
}

fn get_led(_: Request<'_>) -> Response<'static> {
    // Demo payload. Real LED state is firmware-owned, not an App bag.
    Response::content(b"off")
}

fn put_led(_req: Request<'_>) -> Response<'static> {
    Response::changed()
}

fn post_led(_: Request<'_>) -> Response<'static> {
    Response::changed()
}

fn post_create(_: Request<'_>) -> Response<'static> {
    Response::created()
        .location_path("location1")
        .location_path("location2")
        .location_query("first=1")
        .location_query("second=2")
}

fn post_max_opts(_: Request<'_>) -> Response<'static> {
    let echo = EchoOpt::mint(0, &[]).expect("echo");
    Response::created()
        .location_path("l0")
        .location_path("l1")
        .location_path("l2")
        .location_path("l3")
        .location_path("l4")
        .location_path("l5")
        .location_path("l6")
        .location_path("l7")
        .location_query("q0=0")
        .location_query("q1=1")
        .location_query("q2=2")
        .location_query("q3=3")
        .location_query("q4=4")
        .location_query("q5=5")
        .location_query("q6=6")
        .location_query("q7=7")
        .etag(b"etag1")
        .observe(0)
        .max_age(5)
        .content_format(ContentFormat::TEXT_PLAIN)
        .echo(echo)
}

fn get_large_with_opts(_: Request<'_>) -> Response<'static> {
    let echo = EchoOpt::mint(0, &[]).expect("echo");
    Response::content(&LARGE)
        .content_format(ContentFormat::OCTET_STREAM)
        .location_path("l0")
        .location_path("l1")
        .etag(b"etag1")
        .echo(echo)
}

fn get_separate(_: Request<'_>) -> Response<'static> {
    Response::content(b"separate-payload")
        .content_format(ContentFormat::TEXT_PLAIN)
        .separate()
}

fn put_body(req: Request<'_>) -> Response<'static> {
    if req.has_body() {
        Response::changed().payload_copy(req.body().unwrap_or(&[]))
    } else {
        Response::changed().payload_copy(req.payload())
    }
}

fn fetch_query(req: Request<'_>) -> Response<'static> {
    assert_eq!(req.method(), Some(Method::Fetch));
    Response::content_copy(req.payload()).content_format(ContentFormat::TEXT_PLAIN)
}

fn patch_doc(req: Request<'_>) -> Response<'static> {
    assert_eq!(req.method(), Some(Method::Patch));
    Response::changed().payload_copy(req.payload())
}

fn ipatch_doc(req: Request<'_>) -> Response<'static> {
    assert_eq!(req.method(), Some(Method::IPatch));
    // Distinct from PATCH so a shared handler cannot green both methods.
    Response::changed().payload_copy(b"idempotent")
}

fn put_large(req: Request<'_>) -> Response<'static> {
    if req.body() == Some(&LARGE[..]) {
        Response::changed()
    } else {
        Response::new(Code::BAD_REQUEST)
    }
}

fn get_large(_: Request<'_>) -> Response<'static> {
    Response::content(&LARGE)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"large-v1")
}

fn get_obs(_req: Request<'_>) -> Response<'static> {
    Response::content(b"obs-0")
        .content_format(ContentFormat::TEXT_PLAIN)
        .max_age(5)
        .observe(0)
}

fn obs_snapshot() -> Response<'static> {
    Response::content(b"obs-snap").content_format(ContentFormat::TEXT_PLAIN)
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

/// Captures more than one TX datagram so a separate ACK + CON, or a CON
/// retransmit, can be checked. Send is not delivered to `recv` (the caller
/// injects inbox, or leaves it empty to model loss).
#[derive(Default)]
struct RecordIo {
    inbox: Option<(Endpoint, [u8; 256], usize)>,
    sent: [Option<(Endpoint, [u8; 256], usize)>; 8],
    sent_n: usize,
}

impl DatagramIo for RecordIo {
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
        if self.sent_n >= self.sent.len() {
            return Err("full");
        }
        let mut slot = [0u8; 256];
        slot[..bytes.len()].copy_from_slice(bytes);
        self.sent[self.sent_n] = Some((dest, slot, bytes.len()));
        self.sent_n += 1;
        Ok(bytes.len())
    }
}

fn encode_req(code: Code, path: &[&str], payload: &[u8]) -> ([u8; 256], usize) {
    encode_req_mid(code, path, payload, 0x1001)
}

fn encode_req_mid(code: Code, path: &[&str], payload: &[u8], mid: u16) -> ([u8; 256], usize) {
    encode_req_echo(Type::Confirmable, code, path, payload, None, None, mid)
}

fn encode_req_with_echo(
    code: Code,
    path: &[&str],
    payload: &[u8],
    echo: &EchoOpt,
) -> ([u8; 256], usize) {
    encode_req_echo(
        Type::Confirmable,
        code,
        path,
        payload,
        None,
        Some(echo.as_slice()),
        0x1001,
    )
}

fn encode_req_with_echo_mid(
    code: Code,
    path: &[&str],
    payload: &[u8],
    echo: &EchoOpt,
    mid: u16,
) -> ([u8; 256], usize) {
    encode_req_echo(
        Type::Confirmable,
        code,
        path,
        payload,
        None,
        Some(echo.as_slice()),
        mid,
    )
}

fn encode_req_ty(
    ty: Type,
    code: Code,
    path: &[&str],
    payload: &[u8],
    no_response: Option<u32>,
) -> ([u8; 256], usize) {
    encode_req_echo(ty, code, path, payload, no_response, None, 0x1001)
}

fn encode_req_echo(
    ty: Type,
    code: Code,
    path: &[&str],
    payload: &[u8],
    no_response: Option<u32>,
    echo: Option<&[u8]>,
    mid: u16,
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
    if let Some(echo) = echo {
        opts.push(Opt::echo(echo)).expect("echo");
    }
    let msg = Message::new(ty, code, MessageId::new(mid))
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut buf = [0u8; 256];
    let n = encode(&msg, &mut buf).expect("encode");
    (buf, n)
}

fn encode_req_extra(
    code: Code,
    path: &[&str],
    extra: &[Opt<'_>],
    payload: &[u8],
) -> ([u8; 256], usize) {
    let token = Token::new(&[0xA1]).expect("token");
    let mut opts = OptionsBuilder::<8>::new();
    for segment in path {
        opts.push(Opt::uri_path(segment)).expect("path");
    }
    for opt in extra {
        opts.push(*opt).expect("extra");
    }
    let msg = Message::new(Type::Confirmable, code, MessageId::new(0x1001))
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut buf = [0u8; 256];
    let n = encode(&msg, &mut buf).expect("encode");
    (buf, n)
}

fn app_with_site(io: Loopback) -> App<profiles::Default, Loopback> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_led))
        .bind(io)
        .expect("bind")
}

struct LastReply {
    ty: Type,
    code: Code,
    payload: [u8; 64],
    payload_len: usize,
    content_format: Option<ContentFormat>,
    block2: Option<BlockValue>,
    block1: Option<BlockValue>,
    echo: Option<EchoOpt>,
}

fn last_reply<const BLOCK_WISE: bool>(
    app: &App<profiles::Default, Loopback, DEFAULT_ROUTES, BLOCK_WISE>,
) -> LastReply
where
    profiles::Default: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    let (_, bytes, n) = app.transport().last_send.expect("sent");
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
        block2: parsed.block2().and_then(Result::ok),
        block1: parsed.block1().and_then(Result::ok),
        echo: EchoOpt::from_option(parsed.echo()),
    }
}

#[test]
fn get_sensors_temp_is_content() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"21.5");
    assert_eq!(parsed.content_format, Some(ContentFormat::TEXT_PLAIN));
    assert!(parsed.block2.is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn constrained_plain_get_is_content() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_temp))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let (_, bytes, n) = app.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..n]).expect("decode");
    assert_eq!(parsed.code(), Code::CONTENT);
    assert_eq!(parsed.payload(), b"21.5");
    assert!(parsed.block1().is_none());
    assert!(parsed.block2().is_none());
}

#[test]
fn malformed_block2_is_bad_option() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::new(OptionNumber::BLOCK2, &[1, 0, 0, 0])];
    let (wire, n) = encode_req_extra(Code::GET, &["sensors", "temp"], &extra, &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement, "CON still ACK'd");
    assert_eq!(parsed.code, Code::BAD_OPTION);
    assert_ne!(parsed.code, Code::CONTENT, "must not ignore bad Block2");
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn unrecognized_critical_option_is_bad_option() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::new(OptionNumber::new(65001), &[])];
    let (wire, n) = encode_req_extra(Code::GET, &["sensors", "temp"], &extra, &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement, "CON still ACK'd");
    assert_eq!(parsed.code, Code::BAD_OPTION);
    assert_ne!(
        parsed.code,
        Code::CONTENT,
        "must not dispatch unknown critical"
    );
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn oscore_option_without_context_is_bad_option_not_outer_post() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::new(OptionNumber::OSCORE, &[0x09])];
    let (wire, n) = encode_req_extra(Code::POST, &["items"], &extra, b"ciphertext");
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["items"], post(post_create))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement, "CON still ACK'd");
    assert_eq!(parsed.code, Code::BAD_OPTION);
    assert_ne!(
        parsed.code,
        Code::CREATED,
        "OSCORE without context must not run the outer POST handler"
    );
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn oscore_option_without_context_is_bad_option_not_outer_fetch() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::new(OptionNumber::OSCORE, &[0x09])];
    let (wire, n) = encode_req_extra(Code::FETCH, &["probe"], &extra, b"ciphertext");
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["probe"], fetch(fetch_query))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement, "CON still ACK'd");
    assert_eq!(parsed.code, Code::BAD_OPTION);
    assert_ne!(
        parsed.code,
        Code::CONTENT,
        "OSCORE without context must not run the outer FETCH handler"
    );
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn proxy_uri_is_505_not_404() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_get(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"21.5")
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::proxy_uri("coap://example.com/x")];
    let (wire, n) = encode_req_extra(Code::GET, &[], &extra, &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(counting_get))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement, "CON still ACK'd");
    assert_eq!(parsed.code, Code::PROXYING_NOT_SUPPORTED);
    assert_ne!(parsed.code, Code::NOT_FOUND, "must not 4.04 a Proxy-Uri");
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    assert_eq!(HITS.load(Ordering::SeqCst), 0, "handler must not run");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn proxy_scheme_is_505_before_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_get(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::proxy_scheme("coap")];
    let (wire, n) = encode_req_extra(Code::GET, &["sensors", "temp"], &extra, &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(counting_get))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::PROXYING_NOT_SUPPORTED);
    assert_ne!(
        parsed.code,
        Code::CONTENT,
        "must not dispatch a Proxy-Scheme request"
    );
    assert_eq!(HITS.load(Ordering::SeqCst), 0, "handler must not run");
}

#[test]
fn created_response_carries_location_path_and_query() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["items"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["items"], post(post_create))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let (_, bytes, n) = app.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..n]).expect("decode created");
    assert_eq!(parsed.ty(), Type::Acknowledgement);
    assert_eq!(parsed.code(), Code::CREATED);
    let mut segs = parsed.location_path();
    assert_eq!(segs.next().map(|s| s.expect("utf8")), Some("location1"));
    assert_eq!(segs.next().map(|s| s.expect("utf8")), Some("location2"));
    assert!(segs.next().is_none());
    let mut qs = parsed.location_query();
    assert_eq!(qs.next().map(|s| s.expect("utf8")), Some("first=1"));
    assert_eq!(qs.next().map(|s| s.expect("utf8")), Some("second=2"));
    assert!(qs.next().is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn duplicate_con_get_replays_without_second_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_get(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(counting_get))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 1);
    let (_, first, first_n) = app.transport().sent[0].expect("ack");
    let parsed = decode(&first[..first_n]).expect("decode");
    assert_eq!(parsed.ty(), Type::Acknowledgement);
    assert_eq!(parsed.code(), Code::CONTENT);
    assert_eq!(parsed.payload(), b"21.5");
    assert_eq!(app.engine_mut().tx_occupied(), 0);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "handler must not re-run");
    assert_eq!(app.transport().sent_n, 2);
    let (_, replay, replay_n) = app.transport().sent[1].expect("replay");
    assert_eq!(&first[..first_n], &replay[..replay_n]);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn duplicate_con_post_does_not_reinvoke_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_post(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::changed()
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["leds", "0"], post(counting_post))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 1);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "POST must not re-run");
    assert_eq!(app.transport().sent_n, 2);
    let (_, first, first_n) = app.transport().sent[0].expect("first ack");
    let (_, replay, replay_n) = app.transport().sent[1].expect("replay");
    assert_eq!(&first[..first_n], &replay[..replay_n]);
    let parsed = decode(&replay[..replay_n]).expect("decode");
    assert_eq!(parsed.ty(), Type::Acknowledgement);
    assert_eq!(parsed.code(), Code::CHANGED);
}

#[test]
fn duplicate_con_patch_does_not_reinvoke_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_patch(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::changed().payload_copy(b"patched")
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::PATCH, &["delta"], b"p");
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["delta"], patch(counting_patch))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "PATCH must not re-run");
    let (_, first, first_n) = app.transport().sent[0].expect("first");
    let (_, replay, replay_n) = app.transport().sent[1].expect("replay");
    assert_eq!(&first[..first_n], &replay[..replay_n]);
}

#[test]
fn duplicate_con_fetch_does_not_reinvoke_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_fetch(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content_copy(b"sel").content_format(ContentFormat::TEXT_PLAIN)
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::FETCH, &["query"], b"sel");
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["query"], fetch(counting_fetch))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        1,
        "FETCH must not re-run (App policy; RFC 8132 calls FETCH idempotent)"
    );
    let (_, first, first_n) = app.transport().sent[0].expect("first");
    let (_, replay, replay_n) = app.transport().sent[1].expect("replay");
    assert_eq!(&first[..first_n], &replay[..replay_n]);
}

#[test]
fn duplicate_con_separate_replays_empty_ack() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_separate(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"separate-payload")
            .content_format(ContentFormat::TEXT_PLAIN)
            .separate()
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let req_mid = MessageId::new(0x1001);
    let (wire, n) = encode_req(Code::GET, &["separate"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["separate"], get(counting_separate))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 2, "empty ACK then CON");
    assert_eq!(app.engine_mut().tx_occupied(), 1, "pending CON");

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "handler must not re-run");
    assert_eq!(app.transport().sent_n, 3, "replay empty ACK only");
    let (_, replay, replay_n) = app.transport().sent[2].expect("replay ACK");
    let ack = decode(&replay[..replay_n]).expect("decode");
    assert!(ack.is_empty_ack());
    assert_eq!(ack.message_id(), req_mid);
    assert_eq!(app.engine_mut().tx_occupied(), 1, "body CON still pending");
}

#[test]
fn duplicate_con_get_after_exchange_lifetime_reruns_handler() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_get(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(counting_get))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);

    let live = u64::from(Transmission::EXCHANGE_LIFETIME_MS) - 1;
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(live).expect("still live");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(u64::from(Transmission::EXCHANGE_LIFETIME_MS))
        .expect("expired");
    assert_eq!(HITS.load(Ordering::SeqCst), 2);
}

#[test]
fn duplicate_con_post_empty_dedup_row_acks_without_rerun() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_post(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::changed()
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mid = MessageId::new(0x1001);
    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["leds", "0"], post(counting_post))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 1);

    // access_tx-fail / pin-evict leftover: live row, nothing to send.
    let key = DedupKey::new(mid, peer);
    assert!(app.engine_mut().remove_dedup(key));
    let empty =
        DedupEntry::new(mid, peer).with_due_ms(u64::from(Transmission::EXCHANGE_LIFETIME_MS));
    assert!(!empty.has_usable_replay());
    app.engine_mut().insert_dedup(empty).expect("empty row");

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit on empty row");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "POST must not re-run");
    assert_eq!(app.transport().sent_n, 2, "must ACK, not silence");
    let (_, ack_bytes, ack_n) = app.transport().sent[1].expect("empty ACK");
    let ack = decode(&ack_bytes[..ack_n]).expect("decode");
    assert!(ack.is_empty_ack());
    assert_eq!(ack.message_id(), mid);

    let id = app.engine().lookup_dedup(key).expect("row stays live");
    let stored = app.engine().dedup_entry(id).expect("entry");
    assert!(
        stored.has_usable_replay(),
        "empty Hit must persist a usable ACK, not stay metadata-only"
    );

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(2).expect("second retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 3);
    let (_, again, again_n) = app.transport().sent[2].expect("replay");
    let again_ack = decode(&again[..again_n]).expect("decode");
    assert!(again_ack.is_empty_ack());

    let live = u64::from(Transmission::EXCHANGE_LIFETIME_MS) - 1;
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(live).expect("still live");
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        1,
        "must not silence-then-rerun before EXCHANGE_LIFETIME"
    );
}

#[test]
fn duplicate_con_post_insert_failure_does_not_rerun() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    fn counting_post(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::changed()
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let filler = Endpoint::v4([192, 0, 2, 99], 5683);
    let mid = MessageId::new(0x1001);
    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["leds", "0"], post(counting_post))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");

    // Never-expire Engine-pair rows fill the table. App will not evict
    // `due_ms == 0`, so store_request_dedup fails after the first POST.
    for i in 0..profiles::Default::DEDUP_ENTRIES {
        let row = DedupEntry::new(MessageId::new(i as u16), filler);
        assert_eq!(row.due_ms(), 0);
        app.engine_mut().insert_dedup(row).expect("fill");
    }

    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 1);
    let key = DedupKey::new(mid, peer);
    assert!(
        app.engine().lookup_dedup(key).is_none(),
        "insert must fail while the table is full of never-expire rows"
    );

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit after insert failure");
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        1,
        "POST must not re-run when Dedup insert failed"
    );
    assert_eq!(app.transport().sent_n, 2, "must ACK, not silence");
    let (_, ack_bytes, ack_n) = app.transport().sent[1].expect("fail-closed ACK");
    let ack = decode(&ack_bytes[..ack_n]).expect("decode");
    assert!(ack.is_empty_ack(), "no cached replay: empty ACK");
    assert_eq!(ack.message_id(), mid);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(2).expect("second retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().sent_n, 3);

    let live = u64::from(Transmission::EXCHANGE_LIFETIME_MS) - 1;
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(live).expect("still live");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(u64::from(Transmission::EXCHANGE_LIFETIME_MS))
        .expect("expired");
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        2,
        "after EXCHANGE_LIFETIME a new exchange may run"
    );
}

#[test]
fn push_opt_fails_loud_when_builder_is_full() {
    let mut opts = OptionsBuilder::<1>::new();
    super::push_opt(&mut opts, Opt::uri_path("a")).expect("first");
    assert_eq!(
        super::push_opt(&mut opts, Opt::uri_path("b")),
        Err(EncodeError::OptionsFull)
    );
}

#[test]
fn created_metadata_on_wire_excludes_unsolicited_observe() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["items"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["items"], post(post_max_opts))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let (_, bytes, n) = app.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..n]).expect("decode created");
    assert_eq!(parsed.code(), Code::CREATED);
    assert_ne!(parsed.code(), Code::INTERNAL_SERVER_ERROR);
    let mut segs = parsed.location_path();
    for want in ["l0", "l1", "l2", "l3", "l4", "l5", "l6", "l7"] {
        assert_eq!(segs.next().map(|s| s.expect("utf8")), Some(want));
    }
    assert!(segs.next().is_none());
    let mut qs = parsed.location_query();
    for want in [
        "q0=0", "q1=1", "q2=2", "q3=3", "q4=4", "q5=5", "q6=6", "q7=7",
    ] {
        assert_eq!(qs.next().map(|s| s.expect("utf8")), Some(want));
    }
    assert!(qs.next().is_none());
    assert_eq!(parsed.etag().next(), Some(&b"etag1"[..]));
    assert!(parsed.observe().is_none());
    assert_eq!(
        parsed.content_format().and_then(Result::ok),
        Some(ContentFormat::TEXT_PLAIN)
    );
    assert!(parsed.echo().is_some());
    assert_eq!(parsed.max_age().and_then(Result::ok), Some(5));
}

#[test]
fn separate_response_is_empty_ack_then_con() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let token = Token::new(&[0xA1]).expect("token");
    let req_mid = MessageId::new(0x1001);
    let (wire, n) = encode_req(Code::GET, &["separate"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["separate"], get(get_separate))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(app.transport().sent_n, 2, "empty ACK then CON");

    let (_, ack_bytes, ack_n) = app.transport().sent[0].expect("empty ACK");
    let ack = decode(&ack_bytes[..ack_n]).expect("decode ACK");
    assert!(ack.is_empty_ack());
    assert_eq!(ack.message_id(), req_mid);

    let (_, con_bytes, con_n) = app.transport().sent[1].expect("CON");
    let con = decode(&con_bytes[..con_n]).expect("decode CON");
    assert_eq!(con.ty(), Type::Confirmable);
    assert_eq!(con.code(), Code::CONTENT);
    assert_eq!(con.token(), token);
    assert_eq!(con.payload(), b"separate-payload");
    assert_eq!(
        con.content_format().and_then(Result::ok),
        Some(ContentFormat::TEXT_PLAIN)
    );
    assert_ne!(con.message_id(), req_mid);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 1, "pending CON");

    let mut ack_wire = [0u8; 256];
    let ack_n = encode(&Message::empty_ack(con.message_id()), &mut ack_wire).expect("encode ACK");
    app.transport_mut().inbox = Some((peer, ack_wire, ack_n));
    app.poll(0).expect("ack poll");
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn empty_con_is_rst() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mid = MessageId::new(0x5049);
    let ping = Message::new(Type::Confirmable, Code::EMPTY, mid);
    let mut wire = [0u8; 256];
    let n = encode(&ping, &mut wire).expect("encode ping");
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let (_, bytes, n) = app.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..n]).expect("decode RST");
    assert!(parsed.is_empty_rst());
    assert_eq!(parsed.message_id(), mid);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn unknown_path_is_not_found() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["nope"], &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::NOT_FOUND);
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    let details = ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).expect("cbor");
    assert_eq!(details.response_code(), Some(Code::NOT_FOUND));
    assert_eq!(details.title_text(), Some("Not Found"));
}

#[test]
fn wrong_method_is_not_allowed() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["sensors", "temp"], &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::METHOD_NOT_ALLOWED);
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    let details = ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).expect("cbor");
    assert_eq!(details.response_code(), Some(Code::METHOD_NOT_ALLOWED));
    assert_eq!(details.title_text(), Some("Method Not Allowed"));
}

fn rfc8132_loopback(io: Loopback) -> App<profiles::Default, Loopback> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["query"], fetch(fetch_query))
        .route(&["delta"], patch(patch_doc))
        .route(&["idem"], ipatch(ipatch_doc))
        .bind(io)
        .expect("bind")
}

fn poll_rfc8132(
    code: Code,
    path: &[&str],
    payload: &[u8],
) -> (LastReply, App<profiles::Default, Loopback>) {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(code, path, payload);
    let mut app = rfc8132_loopback(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let reply = last_reply(&app);
    (reply, app)
}

#[test]
fn fetch_query_is_content() {
    let (parsed, mut app) = poll_rfc8132(Code::FETCH, &["query"], b"sel");
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"sel");
    assert_eq!(parsed.content_format, Some(ContentFormat::TEXT_PLAIN));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn patch_delta_is_changed() {
    let (parsed, mut app) = poll_rfc8132(Code::PATCH, &["delta"], b"delta");
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::CHANGED);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"delta");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn ipatch_delta_is_changed() {
    let (parsed, mut app) = poll_rfc8132(Code::IPATCH, &["idem"], b"delta");
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::CHANGED);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"idempotent");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn fetch_on_get_route_is_not_allowed() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::FETCH, &["sensors", "temp"], b"sel");
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::METHOD_NOT_ALLOWED);
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    let details = ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).expect("cbor");
    assert_eq!(details.response_code(), Some(Code::METHOD_NOT_ALLOWED));
    assert_eq!(details.title_text(), Some("Method Not Allowed"));
}

#[test]
fn put_led_is_changed() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::PUT, &["leds", "0"], b"1");
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CHANGED);

    let (wire, n) = encode_req_mid(Code::GET, &["leds", "0"], &[], 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"off");
}

// Deterministic routing fixture only; the policy security contract is tested
// separately with authenticated tokens. This fixture is not a MAC example.
fn test_echo_policy(check: super::EchoCheck) -> super::EchoDecision {
    if check
        .echo
        .ok()
        .flatten()
        .is_some_and(|echo| echo.is_time_fresh(check.now_ms, ECHO_FRESH_MS))
    {
        super::EchoDecision::Accept
    } else {
        super::EchoDecision::Challenge(EchoOpt::mint(check.now_ms, &[]).unwrap())
    }
}

const ECHO_FRESH_MS: u64 = 5;

fn assert_echo_401(app: &App<profiles::Default, Loopback>, now_ms: u64) -> EchoOpt {
    let parsed = last_reply(app);
    assert_eq!(parsed.ty, Type::Acknowledgement);
    assert_eq!(parsed.code, Code::UNAUTHORIZED);
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    let details = ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).expect("cbor");
    assert_eq!(details.response_code(), Some(Code::UNAUTHORIZED));
    assert_eq!(details.title_text(), Some("Unauthorized"));
    let echo = parsed.echo.expect("RFC 9175 Echo on 4.01");
    assert!(echo.is_time_fresh(now_ms, ECHO_FRESH_MS));
    assert_eq!(echo.issued_at(), Some(now_ms));
    echo
}

#[test]
fn echo_freshness_missing_is_401_problem() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::PUT, &["leds", "0"], b"1");
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.echo_policy(test_echo_policy);
    app.poll(10).expect("poll");
    let challenge = assert_echo_401(&app, 10);

    let (wire, n) = encode_req_with_echo_mid(Code::PUT, &["leds", "0"], b"1", &challenge, 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(11).expect("retry");
    assert_eq!(last_reply(&app).code, Code::CHANGED);
}

#[test]
fn echo_freshness_stale_is_401_problem() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let stale = EchoOpt::mint(9, &[]).expect("mint");
    let (wire, n) = encode_req_with_echo(Code::PUT, &["leds", "0"], b"1", &stale);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.echo_policy(test_echo_policy);
    app.poll(15).expect("poll");
    assert_echo_401(&app, 15);
}

#[test]
fn echo_freshness_fresh_runs_handler() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let fresh = EchoOpt::mint(9, &[]).expect("mint");
    let (wire, n) = encode_req_with_echo(Code::PUT, &["leds", "0"], b"1", &fresh);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.echo_policy(test_echo_policy);
    app.poll(10).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CHANGED);
    assert!(parsed.echo.is_none());
}

#[test]
fn echo_freshness_builder_missing_is_401_problem() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::PUT, &["leds", "0"], b"1");
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .echo_policy(test_echo_policy)
        .block_wise::<false>()
        .route(&["leds", "0"], get(get_led).put(put_led))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(10).expect("poll");
    assert_echo_401(&app, 10);
}

#[test]
fn get_led_is_demo_payload() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
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
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
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
        Some(u32::from(crate::message::NoResponse::SUPPRESS_2)),
    );
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    assert!(app.transport().last_send.is_none());
}

#[test]
fn no_response_con_sends_empty_ack() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let req_mid = MessageId::new(0x1001);
    let (wire, n) = encode_req_ty(
        Type::Confirmable,
        Code::GET,
        &["sensors", "temp"],
        &[],
        Some(u32::from(crate::message::NoResponse::SUPPRESS_2)),
    );
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_temp))
        .bind(RecordIo {
            inbox: Some((peer, wire, n)),
            ..RecordIo::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(app.transport().sent_n, 1, "empty ACK only");
    let (_, bytes, n) = app.transport().sent[0].expect("ack");
    let ack = decode(&bytes[..n]).expect("decode");
    assert!(ack.is_empty_ack());
    assert_eq!(ack.message_id(), req_mid);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn well_known_core_lists_registered_paths() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &[".well-known", "core"], &[]);
    // Catalog must be generated from these `.route` registrations (not a stored string).
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["a"], get(get_temp))
        .route(&["b"], get(get_led))
        .well_known_core()
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(parsed.content_format, Some(ContentFormat::LINK_FORMAT));
    let body = core::str::from_utf8(&parsed.payload[..parsed.payload_len]).expect("utf8");
    assert!(body.contains("</a>"), "{body}");
    assert!(body.contains("</b>"), "{body}");
    assert!(!body.contains(".well-known"), "{body}");
    assert_eq!(body, "</a>,</b>");
}

/// Sixteen short registered paths. Catalog is 143 bytes — past
/// [`INLINE_PAYLOAD`] (128), so a 128-byte write would silently drop links.
const TEN_PLUS_ROUTES: [&str; 16] = [
    "ep/00", "ep/01", "ep/02", "ep/03", "ep/04", "ep/05", "ep/06", "ep/07", "ep/08", "ep/09",
    "ep/10", "ep/11", "ep/12", "ep/13", "ep/14", "ep/15",
];

const TEN_PLUS_CATALOG: &str = concat!(
    "</ep/00>,</ep/01>,</ep/02>,</ep/03>,</ep/04>,</ep/05>,</ep/06>,</ep/07>,",
    "</ep/08>,</ep/09>,</ep/10>,</ep/11>,</ep/12>,</ep/13>,</ep/14>,</ep/15>"
);

fn well_known_request() -> ([u8; 256], usize) {
    encode_req(Code::GET, &[".well-known", "core"], &[])
}

fn dispatch_well_known<'a, const N: usize>(site: &Site<N>, scratch: &'a mut [u8]) -> Response<'a> {
    let (wire, n) = well_known_request();
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    site.dispatch(req, scratch)
}

fn assert_full_catalog(body: &str) {
    assert!(
        TEN_PLUS_CATALOG.len() > INLINE_PAYLOAD,
        "catalog must exceed INLINE_PAYLOAD so a 128-byte write would truncate"
    );
    assert_eq!(body, TEN_PLUS_CATALOG);
    assert_eq!(
        body.bytes().filter(|&b| b == b',').count(),
        TEN_PLUS_ROUTES.len() - 1
    );
    for path in TEN_PLUS_ROUTES {
        assert_eq!(body.matches(path).count(), 1, "{path} in {body}");
    }
}

#[test]
fn well_known_core_lists_all_registered_routes_past_inline() {
    const _: () = assert!(link_format_capacity::<16>() == 16 * LINK_FORMAT_PER_ROUTE);
    const _: () = assert!(link_format_capacity::<16>() > INLINE_PAYLOAD);

    let mut site = Site::<16>::new();
    for path in TEN_PLUS_ROUTES {
        site.route(path, get(get_temp));
    }
    site.well_known_core();
    assert_eq!(site.len(), 16);
    assert_eq!(site.capacity(), 16);

    let mut scratch = LinkFormatScratch::<16>::new();
    let response = dispatch_well_known(&site, scratch.as_mut());
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.format(), Some(ContentFormat::LINK_FORMAT));
    let body = core::str::from_utf8(response.payload()).expect("utf8");
    assert_full_catalog(body);
}

#[test]
fn well_known_core_poll_lists_every_upfront_route() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = well_known_request();
    let mut builder = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .routes::<16>();
    for path in TEN_PLUS_ROUTES {
        builder = builder.route(path, get(get_temp));
    }
    let mut app = builder
        .well_known_core()
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    assert_eq!(app.site().len(), 16);
    assert_eq!(app.site().capacity(), 16);
    app.poll(0).expect("poll");

    let (_, bytes, sent) = app.transport().last_send.expect("sent");
    let parsed = decode(&bytes[..sent]).expect("decode reply");
    assert_eq!(parsed.code(), Code::CONTENT);
    assert_eq!(
        parsed.content_format().and_then(Result::ok),
        Some(ContentFormat::LINK_FORMAT)
    );
    let body = core::str::from_utf8(parsed.payload()).expect("utf8");
    assert_full_catalog(body);
}

#[test]
fn well_known_core_overflow_is_internal_error() {
    const TOO_LONG: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
    const _: () = assert!(TOO_LONG.len() + 3 > INLINE_PAYLOAD);

    let mut site = Site::<1>::new();
    site.route(TOO_LONG, get(get_temp)).well_known_core();
    let mut scratch = LinkFormatScratch::<1>::new();
    let response = dispatch_well_known(&site, scratch.as_mut());
    assert_eq!(response.code(), Code::INTERNAL_SERVER_ERROR);
    assert!(response.payload().is_empty());
    assert_ne!(response.format(), Some(ContentFormat::LINK_FORMAT));
}

/// A path longer than [`LINK_FORMAT_PER_ROUTE`] still fits on N=1 because
/// capacity floors at [`INLINE_PAYLOAD`].
#[test]
fn well_known_core_small_n_keeps_inline_floor() {
    const PATH: &str = "sensors/temperature/celsius";
    const _: () = assert!(PATH.len() + 3 > LINK_FORMAT_PER_ROUTE);
    const _: () = assert!(PATH.len() + 3 <= INLINE_PAYLOAD);
    const _: () = assert!(link_format_capacity::<1>() == INLINE_PAYLOAD);

    let mut site = Site::<1>::new();
    site.route(PATH, get(get_temp)).well_known_core();
    let mut scratch = LinkFormatScratch::<1>::new();
    let response = dispatch_well_known(&site, scratch.as_mut());
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.format(), Some(ContentFormat::LINK_FORMAT));
    let body = core::str::from_utf8(response.payload()).expect("utf8");
    assert_eq!(body, "</sensors/temperature/celsius>");
}

/// IANA experimental 65000–65535. Even ⇒ elective (LSB clear); unrecognized
/// critical options (e.g. 65001) are 4.02 before the handler, so this
/// number is safe to carry through decode and App dispatch.
#[derive(Clone, Copy)]
struct Experimental(OptionNumber);

impl Experimental {
    const fn new() -> Self {
        Self(OptionNumber::new(65000))
    }

    const fn number(self) -> OptionNumber {
        self.0
    }
}

fn put_std_and_custom(req: Request<'_>) -> Response<'static> {
    let experimental = Experimental::new();
    match (req.content_format(), req.get_option(experimental.number())) {
        (Some(Ok(ContentFormat::JSON)), Some(opt)) => Response::changed().payload_copy(opt.value()),
        _ => Response::new(Code::BAD_REQUEST),
    }
}

#[test]
fn request_carries_standard_and_custom_option() {
    let experimental = Experimental::new();
    assert!(!experimental.number().is_critical());

    let cf = ContentFormat::JSON.encode();
    let extra = [
        Opt::content_format(&cf),
        Opt::new(experimental.number(), b"vendor-x"),
    ];
    let (wire, n) = encode_req_extra(Code::PUT, &["probe"], &extra, &[]);

    let parsed = decode(&wire[..n]).expect("decode");
    assert_eq!(
        parsed.content_format().and_then(Result::ok),
        Some(ContentFormat::JSON)
    );
    assert_eq!(
        parsed.get_option(experimental.number()).map(Opt::value),
        Some(&b"vendor-x"[..])
    );
    parsed.check_rfc7252_options().expect("elective custom");

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["probe"], put(put_std_and_custom))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let reply = last_reply(&app);
    assert_eq!(reply.code, Code::CHANGED);
    assert_eq!(&reply.payload[..reply.payload_len], b"vendor-x");
}

#[test]
fn site_dispatch_table() {
    let mut site = Site::<4>::new();
    site.route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_led));

    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).code(), Code::CONTENT);

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).code(), Code::CONTENT);
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).payload(), b"off");
}

#[test]
fn route_path_splits_static_uri() {
    let mut site = Site::<4>::new();
    site.route("sensors/temp", get(get_temp));
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).code(), Code::CONTENT);
}

#[test]
fn route_slash_path_ignores_leading_slash() {
    let mut site = Site::<4>::new();
    site.route("/leds/0", get(get_led));
    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).code(), Code::CONTENT);
}

#[test]
#[should_panic(expected = "empty segment")]
fn route_rejects_empty_segments() {
    let mut site = Site::<4>::new();
    site.route("sensors//temp", get(get_temp));
}

#[test]
fn replacing_route_changes_handler() {
    let mut site = Site::<2>::new();
    site.route(&["leds", "0"], get(get_led));
    site.route(&["leds", "0"], post(post_led));

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(
        site.dispatch(req, &mut [0u8; 1]).code(),
        Code::METHOD_NOT_ALLOWED
    );

    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req, &mut [0u8; 1]).code(), Code::CHANGED);
}

#[test]
fn response_builders() {
    assert_eq!(Response::changed().code(), Code::CHANGED);
    assert_eq!(Response::not_found().code(), Code::NOT_FOUND);
    assert_eq!(
        Response::method_not_allowed().code(),
        Code::METHOD_NOT_ALLOWED
    );
    assert_eq!(Response::content(b"x").payload(), b"x");
    let cf = Response::content(b"x")
        .content_format(ContentFormat::LINK_FORMAT)
        .max_age(60)
        .observe(3)
        .etag(b"ab");
    assert_eq!(cf.format(), Some(ContentFormat::LINK_FORMAT));
    assert_eq!(cf.max_age_secs(), Some(60));
    assert_eq!(cf.observe_seq(), Some(3));
    assert_eq!(cf.etag_bytes(), Some(&b"ab"[..]));
    let created = Response::created()
        .location_path("location1")
        .location_path("location2")
        .location_query("first=1")
        .location_query("second=2");
    assert_eq!(created.code(), Code::CREATED);
    assert_eq!(created.location_paths(), &["location1", "location2"]);
    assert_eq!(created.location_queries(), &["first=1", "second=2"]);
    assert!(Response::content(b"later").separate().is_separate());
    assert!(!Response::content(b"now").is_separate());
    let echo = EchoOpt::mint(9, &[]).expect("mint");
    assert_eq!(
        Response::unauthorized().echo(echo).echo_option(),
        Some(echo)
    );

    let problem = Response::problem(Code::BAD_REQUEST)
        .title("Bad Request")
        .detail("Uri-Path is not UTF-8");
    assert_eq!(problem.code(), Code::BAD_REQUEST);
    assert_eq!(problem.format(), Some(ContentFormat::PROBLEM_DETAILS));
    let details = problem.problem_details().expect("problem");
    assert_eq!(details.response_code(), Some(Code::BAD_REQUEST));
    assert_eq!(details.title_text(), Some("Bad Request"));
    assert_eq!(details.detail_text(), Some("Uri-Path is not UTF-8"));
    assert!(Response::not_found().problem_details().is_none());
}

#[test]
fn request_view_has_path_token_and_no_body() {
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(req.path(), &["sensors", "temp"][..]);
    assert_eq!(req.token(), Token::new(&[0xA1]).expect("token"));
    assert!(!req.has_body());
    assert!(req.body().is_none());
    assert!(req.payload().is_empty());
    assert!(req.content_format().is_none());
    assert!(req.uri_query().next().is_none());
}

struct BlockReq<'a> {
    code: Code,
    path: &'a [&'a str],
    payload: &'a [u8],
    num: u32,
    more: bool,
    size: u16,
    mid: u16,
    size1: Option<u32>,
    q_block: bool,
}

fn encode_req_block1(
    code: Code,
    path: &[&str],
    payload: &[u8],
    num: u32,
    more: bool,
    size: u16,
    mid: u16,
) -> ([u8; 256], usize) {
    encode_block_req(BlockReq {
        code,
        path,
        payload,
        num,
        more,
        size,
        mid,
        size1: None,
        q_block: false,
    })
}

fn encode_block_req(req: BlockReq<'_>) -> ([u8; 256], usize) {
    let token = Token::new(&[0xA1]).expect("token");
    let mut opts = OptionsBuilder::<8>::new();
    for segment in req.path {
        opts.push(Opt::uri_path(segment)).expect("path");
    }
    let size1_enc = req.size1.map(encode_uint);
    if let Some(ref encoded) = size1_enc {
        opts.push(Opt::size1(encoded)).expect("size1");
    }
    let block = BlockValue::from_size(req.num, req.more, req.size).expect("block");
    let encoded = block.encode();
    if req.q_block {
        opts.push(Opt::q_block1(&encoded)).expect("q-block1");
        opts.push(Opt::request_tag(b"upload-1"))
            .expect("request-tag");
    } else {
        opts.push(Opt::block1(&encoded)).expect("block1");
    }
    let msg = Message::new(Type::Confirmable, req.code, MessageId::new(req.mid))
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(req.payload);
    let mut buf = [0u8; 256];
    let n = encode(&msg, &mut buf).expect("encode");
    (buf, n)
}

const LED_PATH: &[&str] = &["leds", "0"];

fn q_block1(payload: &[u8], num: u32, more: bool, mid: u16, size1: u32) -> BlockReq<'_> {
    BlockReq {
        code: Code::PUT,
        path: LED_PATH,
        payload,
        num,
        more,
        size: 16,
        mid,
        size1: Some(size1),
        q_block: true,
    }
}

#[test]
fn block1_incomplete_is_continue() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &payload, 0, true, 16, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let reply = last_reply(&app);
    assert_eq!(reply.code, Code::CONTINUE);
    let block1 = reply.block1.expect("RFC 7959 echoes Block1 on 2.31");
    assert_eq!(block1.num(), 0);
    assert!(block1.more());
}

#[test]
fn block1_acked_num_retransmit_is_continue() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &payload, 0, true, 16, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("first NUM0");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    // Lossy retransmit of the already-acked NUM (same CON MID + payload).
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &payload, 0, true, 16, 0x1001);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("retransmit NUM0");
    let reply = last_reply(&app);
    assert_eq!(
        reply.code,
        Code::CONTINUE,
        "retransmit of acked NUM must replay 2.31, not 4.08"
    );
    let block1 = reply.block1.expect("RFC 7959 echoes Block1 on 2.31");
    assert_eq!(block1.num(), 0);
    assert!(block1.more());

    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &payload, 2, true, 16, 0x1003);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(2).expect("gap NUM2");
    assert_eq!(
        last_reply(&app).code,
        Code::REQUEST_ENTITY_INCOMPLETE,
        "true Gap stays 4.08"
    );
}

#[test]
fn block1_complete_exposes_body() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let first = [b'A'; 16];
    let second = *b"REST";
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &first, 0, true, 16, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &second, 1, false, 16, 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CHANGED);
    assert_eq!(
        &parsed.payload[..parsed.payload_len],
        b"AAAAAAAAAAAAAAAAREST"
    );
}

#[test]
fn qblock1_incomplete_is_continue() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_block_req(q_block1(&payload, 0, true, 0x1001, 32));
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);
}

#[test]
fn qblock1_complete_exposes_body() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let first = [b'A'; 16];
    let second = *b"REST";
    let (wire, n) = encode_block_req(q_block1(&first, 0, true, 0x1001, 20));
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    let (wire, n) = encode_block_req(q_block1(&second, 1, false, 0x1002, 20));
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CHANGED);
    assert_eq!(
        &parsed.payload[..parsed.payload_len],
        b"AAAAAAAAAAAAAAAAREST"
    );
}

#[test]
fn qblock1_holes_are_request_entity_incomplete() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let first = [0x11u8; 16];
    let last = [0x33u8; 8];
    let (wire, n) = encode_block_req(q_block1(&first, 0, true, 0x1001, 40));
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    let (wire, n) = encode_block_req(q_block1(&last, 2, false, 0x1002, 40));
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    app.transport_mut().inbox = None;
    app.transport_mut().last_send = None;
    app.poll(2).expect("armed");
    assert!(app.transport().last_send.is_none());
    app.poll(1 + u64::from(QBlockTransmission::NON_RECEIVE_TIMEOUT_MS))
        .expect("recover");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::NonConfirmable);
    assert_eq!(parsed.code, Code::REQUEST_ENTITY_INCOMPLETE);
    assert_eq!(parsed.content_format, Some(ContentFormat::MISSING_BLOCKS));
    let mut nums = [0u32; 4];
    let n =
        MissingBlocks::decode(&parsed.payload[..parsed.payload_len], &mut nums).expect("cbor-seq");
    assert_eq!(&nums[..n], &[1]);
    assert!(ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).is_err());
}

#[test]
fn qblock1_apply_error_is_request_entity_incomplete() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_block_req(q_block1(&payload, 0, true, 0x1001, 32));
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(last_reply(&app).code, Code::CONTINUE);

    let (wire, n) = encode_block_req(q_block1(&payload, 0, true, 0x1002, 32));
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("duplicate");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::REQUEST_ENTITY_INCOMPLETE);
    assert_eq!(parsed.content_format, Some(ContentFormat::PROBLEM_DETAILS));
    let details = ProblemDetails::decode(&parsed.payload[..parsed.payload_len]).expect("cbor");
    assert_eq!(
        details.response_code(),
        Some(Code::REQUEST_ENTITY_INCOMPLETE)
    );
    assert_eq!(details.title_text(), Some("Request Entity Incomplete"));
}

#[test]
fn qblock2_recover_sent_from_poll() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let token = Token::new(&[0xA1]).expect("token");
    let key = BlockKey::new(token, peer);
    let body: [u8; 40] = core::array::from_fn(|i| (i + 7) as u8);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(Loopback::default())
        .expect("bind");
    let engine = app.engine_mut();
    engine
        .apply_q_block2(
            key,
            BlockValue::from_size(0, true, 16).expect("0"),
            &body[..16],
            Some(40),
        )
        .expect("0");
    engine
        .apply_q_block2(
            key,
            BlockValue::from_size(2, false, 16).expect("2"),
            &body[32..],
            Some(40),
        )
        .expect("2");
    app.poll(0).expect("arm");
    assert!(app.transport().last_send.is_none());
    app.poll(u64::from(QBlockTransmission::NON_RECEIVE_TIMEOUT_MS))
        .expect("poll recover");
    let (_, bytes, n) = app.transport().last_send.expect("sent recover");
    let parsed = decode(&bytes[..n]).expect("decode recover");
    assert_eq!(parsed.ty(), Type::NonConfirmable);
    assert_eq!(parsed.code(), Code::GET);
    assert_eq!(parsed.token(), token);
    let mut nums = [0u32; 2];
    let mut count = 0usize;
    for item in parsed.q_block2() {
        let v = item.expect("val");
        assert!(!v.more());
        nums[count] = v.num();
        count += 1;
    }
    assert_eq!(&nums[..count], &[1]);
    assert!(parsed.observe().is_none());
}

#[test]
fn block_wise_bind_uses_body_pools() {
    let app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(Loopback::default())
        .expect("bind");
    assert!(app.engine().has_body_pools());
}

struct WideLoopback {
    inbox: Option<(Endpoint, [u8; WIRE], usize)>,
    sends: [[u8; WIRE]; 4],
    send_lens: [usize; 4],
    send_n: usize,
}

impl Default for WideLoopback {
    fn default() -> Self {
        Self {
            inbox: None,
            sends: [[0; WIRE]; 4],
            send_lens: [0; 4],
            send_n: 0,
        }
    }
}

impl DatagramIo for WideLoopback {
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

    fn send(&mut self, _dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if bytes.len() > WIRE {
            return Err("too long");
        }
        if self.send_n >= self.sends.len() {
            return Err("send log full");
        }
        let i = self.send_n;
        self.sends[i][..bytes.len()].copy_from_slice(bytes);
        self.send_lens[i] = bytes.len();
        self.send_n += 1;
        Ok(bytes.len())
    }
}

fn encode_wide(code: Code, path: &[&str], extra: &[Opt<'_>], mid: u16) -> ([u8; WIRE], usize) {
    encode_wide_token(code, path, extra, mid, Token::new(&[0xA1]).expect("token"))
}

/// Observe register wire with a one-byte Token (NSTART fan-out tests).
fn encode_nstart_wire(
    code: Code,
    path: &[&str],
    extra: &[Opt<'_>],
    mid: u16,
    token: u8,
) -> ([u8; WIRE], usize) {
    encode_wide_token(code, path, extra, mid, Token::new(&[token]).expect("token"))
}

fn encode_wide_token(
    code: Code,
    path: &[&str],
    extra: &[Opt<'_>],
    mid: u16,
    token: Token,
) -> ([u8; WIRE], usize) {
    let mut opts = OptionsBuilder::<8>::new();
    for segment in path {
        opts.push(Opt::uri_path(segment)).expect("path");
    }
    for opt in extra {
        opts.push(*opt).expect("extra");
    }
    let msg = Message::new(Type::Confirmable, code, MessageId::new(mid))
        .with_token(token)
        .with_options(opts.as_slice());
    let mut buf = [0u8; WIRE];
    let n = encode(&msg, &mut buf).expect("encode");
    (buf, n)
}

fn last_wide<const BLOCK_WISE: bool>(
    app: &App<profiles::Default, WideLoopback, DEFAULT_ROUTES, BLOCK_WISE>,
) -> crate::message::ParsedMessage<'_>
where
    profiles::Default: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    let n = app.transport().send_n;
    assert!(n > 0, "expected a send");
    decode(&app.transport().sends[n - 1][..app.transport().send_lens[n - 1]]).expect("decode")
}

#[test]
fn duplicate_con_post_oversize_ack_replays_from_tx_pin() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    static BODY: [u8; DedupEntry::REPLAY_MAX + 16] = [b'P'; DedupEntry::REPLAY_MAX + 16];
    fn counting_post(_: Request<'_>) -> Response<'static> {
        HITS.fetch_add(1, Ordering::SeqCst);
        Response::changed().payload_copy_full(&BODY)
    }
    HITS.store(0, Ordering::SeqCst);

    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::POST, &["bulk"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["bulk"], post(counting_post))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("first");
    assert_eq!(HITS.load(Ordering::SeqCst), 1);
    assert_eq!(app.transport().send_n, 1);
    assert_eq!(app.engine_mut().tx_occupied(), 1, "oversize ACK pins TX");
    let first_n = app.transport().send_lens[0];
    let mut first = [0u8; WIRE];
    first[..first_n].copy_from_slice(&app.transport().sends[0][..first_n]);
    let parsed = decode(&first[..first_n]).expect("decode");
    assert_eq!(parsed.ty(), Type::Acknowledgement);
    assert_eq!(parsed.code(), Code::CHANGED);
    assert!(first_n > DedupEntry::REPLAY_MAX);

    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("retransmit");
    assert_eq!(HITS.load(Ordering::SeqCst), 1, "POST must not re-run");
    assert_eq!(app.transport().send_n, 2);
    assert_eq!(app.transport().send_lens[1], first_n);
    assert_eq!(
        &app.transport().sends[1][..first_n],
        &first[..first_n],
        "replay must match pinned ACK bytes"
    );
    assert_eq!(app.engine_mut().tx_occupied(), 1);
}

#[test]
fn large_get_ships_block2_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["large"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(get_large))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    {
        let first = last_wide(&app);
        assert_eq!(first.code(), Code::CONTENT);
        assert_eq!(
            first.content_format().and_then(Result::ok),
            Some(ContentFormat::OCTET_STREAM)
        );
        let block = first.block2().expect("Block2").expect("val");
        assert_eq!(block.num(), 0);
        assert!(block.more());
        assert_eq!(block.szx(), BlockValue::SZX_MAX);
        assert_eq!(first.payload(), &LARGE[..1024]);
    }

    let next = BlockValue::from_size(1, false, 1024)
        .expect("num 1")
        .encode();
    let extra = [Opt::block2(&next)];
    let (wire, n) = encode_wide(Code::GET, &["large"], &extra, 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("poll");
    let second = last_wide(&app);
    let block = second.block2().expect("Block2").expect("val");
    assert_eq!(block.num(), 1);
    assert!(!block.more());
    assert_eq!(second.payload(), &LARGE[1024..]);
    assert_eq!(second.etag().next(), Some(&b"large-v1"[..]));
}

#[test]
fn large_get_location_etag_echo_and_block2_all_on_wire() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["loud"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["loud"], get(get_large_with_opts))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    let first = last_wide(&app);
    assert_eq!(first.code(), Code::CONTENT);
    assert_ne!(first.code(), Code::INTERNAL_SERVER_ERROR);
    let block = first.block2().expect("Block2").expect("val");
    assert_eq!(block.num(), 0);
    assert!(block.more());
    let mut segs = first.location_path();
    assert_eq!(segs.next().map(|s| s.expect("utf8")), Some("l0"));
    assert_eq!(segs.next().map(|s| s.expect("utf8")), Some("l1"));
    assert!(segs.next().is_none());
    assert_eq!(first.etag().next(), Some(&b"etag1"[..]));
    assert!(first.echo().is_some());
}

#[test]
fn large_get_without_body_pools_fails_clearly() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["large"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["large"], get(get_large))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    assert_eq!(
        app.poll(0),
        Err(Error::Message(SlotMessageError::Encode(
            EncodeError::BufferTooSmall
        )))
    );
}

#[test]
fn large_get_q_block2_issues_a_window() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let q = BlockValue::from_size(0, false, 1024).expect("q").encode();
    let extra = [Opt::q_block2(&q)];
    let (wire, n) = encode_wide(Code::GET, &["large"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(get_large))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(app.transport().send_n, 2);

    let first = decode(&app.transport().sends[0][..app.transport().send_lens[0]]).expect("first");
    assert_eq!(first.ty(), Type::Acknowledgement);
    let q0 = first.q_block2().next().expect("Q-Block2").expect("val");
    assert_eq!(q0.num(), 0);
    assert!(q0.more());
    assert_eq!(first.payload(), &LARGE[..1024]);
    assert_eq!(first.size2().and_then(Result::ok), Some(2000));
    assert_eq!(first.etag().next(), Some(&b"large-v1"[..]));

    let second = decode(&app.transport().sends[1][..app.transport().send_lens[1]]).expect("second");
    assert_eq!(second.ty(), Type::NonConfirmable);
    let q1 = second.q_block2().next().expect("Q-Block2").expect("val");
    assert_eq!(q1.num(), 1);
    assert!(!q1.more());
    assert_eq!(second.payload(), &LARGE[1024..]);
    assert_eq!(second.etag().next(), Some(&b"large-v1"[..]));
}

fn observe_registered<const BLOCK_WISE: bool>(
    app: &App<profiles::Default, WideLoopback, DEFAULT_ROUTES, BLOCK_WISE>,
    peer: Endpoint,
) -> bool
where
    profiles::Default: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    observe_live(app, peer, Token::new(&[0xA1]).expect("token"))
}

fn observe_live<const BLOCK_WISE: bool>(
    app: &App<profiles::Default, WideLoopback, DEFAULT_ROUTES, BLOCK_WISE>,
    peer: Endpoint,
    token: Token,
) -> bool
where
    profiles::Default: MemoryLayout<BLOCK_WISE> + AppAssembled<BLOCK_WISE>,
{
    app.engine()
        .lookup_observe(ObserveKey::new(token, peer))
        .is_some()
}

#[test]
fn observe_insert_miss_strips_observe_option() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback::default())
        .expect("bind");

    let n = profiles::Default::OBSERVE_ENTRIES;
    for i in 0..n {
        let token = Token::new(&[i as u8 + 1]).expect("token");
        let (wire, len) = encode_wide_token(
            Code::GET,
            &["sensors", "temp"],
            &extra,
            0x1001 + i as u16,
            token,
        );
        app.transport_mut().inbox = Some((peer, wire, len));
        app.transport_mut().send_n = 0;
        app.poll(0).expect("register");
        let reply = last_wide(&app);
        assert_eq!(reply.code(), Code::CONTENT);
        assert_eq!(reply.observe().and_then(Result::ok), Some(0));
        assert!(observe_live(&app, peer, token));
    }

    let overflow = Token::new(&[0xFF]).expect("token");
    let (wire, len) = encode_wide_token(Code::GET, &["sensors", "temp"], &extra, 0x10FF, overflow);
    app.transport_mut().inbox = Some((peer, wire, len));
    app.transport_mut().send_n = 0;
    app.poll(0).expect("overflow register");
    let reply = last_wide(&app);
    assert_eq!(reply.code(), Code::CONTENT, "still a representation");
    assert!(
        reply.observe().is_none(),
        "insert miss must not claim registration"
    );
    assert!(!observe_live(&app, peer, overflow));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
}

#[test]
fn observe_register_notify_deregister() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    {
        let first = last_wide(&app);
        assert_eq!(first.code(), Code::CONTENT);
        assert_eq!(first.token(), Token::new(&[0xA1]).expect("token"));
        assert_eq!(first.observe().and_then(Result::ok), Some(0));
        assert_eq!(first.payload(), b"obs-0");
    }
    assert!(observe_registered(&app, peer));

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-1").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(sent, 1);
    {
        let note = last_wide(&app);
        assert_eq!(note.ty(), Type::NonConfirmable);
        assert_eq!(note.code(), Code::CONTENT);
        assert_eq!(note.token(), Token::new(&[0xA1]).expect("token"));
        assert_eq!(note.observe().and_then(Result::ok), Some(1));
        assert_eq!(note.payload(), b"obs-1");
    }

    let extra = [Opt::observe_deregister()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(20).expect("poll");
    assert!(!observe_registered(&app, peer));

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(30, &["sensors", "temp"], Response::content(b"obs-2"))
        .expect("notify after deregister");
    assert_eq!(sent, 0);
}

#[test]
fn notify_nstart_one_per_endpoint() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_nstart_wire(Code::GET, &["sensors", "temp"], &extra, 0x1001, 0xA1);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("first register");
    let (wire, n) = encode_nstart_wire(Code::GET, &["sensors", "temp"], &extra, 0x1002, 0xA2);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).expect("second register");

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-n").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(
        sent, 1,
        "NSTART=1: one notify per endpoint even with two rows"
    );
    assert_eq!(app.transport().send_n, 1);
    assert_eq!(app.metrics().observe_notify, 1);
    assert_eq!(app.metrics().nstart_reject, 1);
}

#[test]
fn notify_fans_out_to_distinct_endpoints() {
    let peer_a = Endpoint::v4([192, 0, 2, 1], 5683);
    let peer_b = Endpoint::v4([192, 0, 2, 3], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_nstart_wire(Code::GET, &["sensors", "temp"], &extra, 0x1001, 0xA1);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer_a, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("register a");
    let (wire, n) = encode_nstart_wire(Code::GET, &["sensors", "temp"], &extra, 0x1002, 0xB1);
    app.transport_mut().inbox = Some((peer_b, wire, n));
    app.poll(1).expect("register b");

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-n").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(sent, 2);
    assert_eq!(app.transport().send_n, 2);
}

fn inject_empty_rst(
    app: &mut App<profiles::Default, WideLoopback>,
    peer: Endpoint,
    mid: MessageId,
) {
    let mut wire = [0u8; WIRE];
    let n = encode(&Message::empty_rst(mid), &mut wire).expect("rst");
    app.transport_mut().inbox = Some((peer, wire, n));
}

#[test]
fn observe_non_notify_rst_drops_interest() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-1").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(sent, 1);
    let note = last_wide(&app);
    assert_eq!(note.ty(), Type::NonConfirmable);
    let mid = note.message_id();
    assert!(!note.token().is_empty(), "notify Token is not empty");

    inject_empty_rst(&mut app, peer, mid);
    app.poll(11).expect("rst");
    assert!(
        !observe_registered(&app, peer),
        "RFC 7641 RST of NON notify must drop the observer"
    );

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(12, &["sensors", "temp"], Response::content(b"obs-2"))
        .expect("notify after rst");
    assert_eq!(sent, 0);
}

#[test]
fn observe_con_notify_rst_drops_interest() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-non").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("first non");
    assert_eq!(sent, 1);
    assert_eq!(last_wide(&app).ty(), Type::NonConfirmable);

    let now = 10 + ObserveTransmission::CONFIRM_INTERVAL_MS;
    app.transport_mut().send_n = 0;
    let sent = app
        .notify(
            now,
            &["sensors", "temp"],
            Response::content(b"obs-con").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("con notify");
    assert_eq!(sent, 1);
    let note = last_wide(&app);
    assert_eq!(note.ty(), Type::Confirmable);
    let mid = note.message_id();

    inject_empty_rst(&mut app, peer, mid);
    app.poll(now).expect("rst");
    assert!(
        !observe_registered(&app, peer),
        "RFC 7641 RST of CON notify must drop the observer"
    );
}

#[test]
fn empty_con_ping_rst_does_not_drop_observe() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    inject_empty_rst(&mut app, peer, MessageId::new(0x5049));
    app.poll(1).expect("unrelated rst");
    assert!(
        observe_registered(&app, peer),
        "empty RST with a foreign MID must not cancel Observe"
    );
}

#[test]
fn observe_max_age_expiry_preserves_interest() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    app.poll(5_000).expect("stale representation");
    assert!(observe_registered(&app, peer));
    assert_eq!(
        app.notify(
            5_001,
            &["sensors", "temp"],
            Response::content(b"updated").content_format(ContentFormat::TEXT_PLAIN)
        )
        .unwrap(),
        1
    );
}

#[test]
fn observe_source_sends_on_signal_poll() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs).observe(obs_snapshot))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    assert_eq!(app.signal(&["sensors", "temp"]), 1);
    app.transport_mut().send_n = 0;
    app.poll(10).expect("poll notify");
    let note = last_wide(&app);
    assert_eq!(note.token(), Token::new(&[0xA1]).expect("token"));
    assert_eq!(note.observe().and_then(Result::ok), Some(1));
    assert_eq!(note.payload(), b"obs-snap");
}

#[test]
fn observe_signal_survives_tx_saturated_poll() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs).observe(obs_snapshot))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("register");
    assert!(observe_registered(&app, peer));

    let key = ObserveKey::new(Token::new(&[0xA1]).expect("token"), peer);
    let id = app.engine().lookup_observe(key).expect("row");
    assert_eq!(app.engine().observe_interest(id).expect("row").seq(), 0);

    let mut held = [None; profiles::Default::TX_DATAGRAM_SLOTS];
    {
        let engine = app.engine_mut();
        for slot in &mut held {
            *slot = engine.acquire_tx();
        }
        assert!(engine.acquire_tx().is_none(), "TX full");
    }

    assert_eq!(app.signal(&["sensors", "temp"]), 1);
    app.transport_mut().send_n = 0;
    let err = app.poll(10).expect_err("TX full");
    assert_eq!(err, Error::Saturated);
    let row = app.engine().observe_interest(id).expect("still registered");
    assert!(row.is_pending(), "due must survive send miss");
    assert_eq!(row.seq(), 0, "seq must not gap");
    assert_eq!(app.transport().send_n, 0);

    for tx in held.into_iter().flatten() {
        app.engine_mut().release_tx(tx).expect("free TX");
    }
    app.poll(10).expect("retry notify");
    let note = last_wide(&app);
    assert_eq!(note.observe().and_then(Result::ok), Some(1));
    assert_eq!(note.payload(), b"obs-snap");
    let row = app.engine().observe_interest(id).expect("row");
    assert!(!row.is_pending());
    assert_eq!(row.seq(), 1);
}

/// Send writes into recv so one App is both client and server.
#[derive(Default)]
struct Echo {
    pending: Option<(Endpoint, [u8; 256], usize)>,
}

impl DatagramIo for Echo {
    type Error = &'static str;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((ep, bytes, n)) = self.pending.take() else {
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
        self.pending = Some((dest, slot, bytes.len()));
        Ok(bytes.len())
    }
}

fn echo_app() -> App<profiles::Default, Echo> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_body))
        .bind(Echo::default())
        .expect("bind")
}

#[test]
fn client_get_round_trip_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert!(app.take_response(call).is_none());
    app.poll(0).expect("server handle");
    assert!(app.take_response(call).is_none());
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
    assert!(!response.payload_truncated());
    assert_eq!(response.payload_src_len(), 4);
    assert!(response.body().is_none());
    assert_eq!(response.format(), Some(ContentFormat::TEXT_PLAIN));
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(response.peer(), Some(peer));
    assert_eq!(response.ty(), Some(Type::Acknowledgement));
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_take_response_truncates_non_block_payload_at_inline() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let (mid, token) = {
        let (_, bytes, n) = app.transport().sent[0].expect("first");
        let first = decode(&bytes[..n]).expect("decode first");
        (first.message_id(), first.token())
    };
    let payload = [b'x'; 200];
    inject_piggyback_ack(&mut app, peer, mid, token, &payload);
    app.poll(0).expect("match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload().len(), INLINE_PAYLOAD);
    assert_eq!(response.payload(), &payload[..INLINE_PAYLOAD]);
    assert!(response.payload_truncated());
    assert_eq!(response.payload_src_len(), 200);
    assert!(
        response.body().is_none(),
        "datagram App has no Block2 assembled hold"
    );
}

#[test]
fn client_take_response_inline_payload_exact_is_not_truncated() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let (mid, token) = {
        let (_, bytes, n) = app.transport().sent[0].expect("first");
        let first = decode(&bytes[..n]).expect("decode first");
        (first.message_id(), first.token())
    };
    let payload = [b'y'; INLINE_PAYLOAD];
    inject_piggyback_ack(&mut app, peer, mid, token, &payload);
    app.poll(0).expect("match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.payload(), &payload);
    assert!(!response.payload_truncated());
    assert_eq!(response.payload_src_len(), INLINE_PAYLOAD);
}

#[test]
fn client_put_round_trip_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app
        .put(&["leds", "0"])
        .to(peer)
        .payload(b"on")
        .content_format(ContentFormat::TEXT_PLAIN)
        .send(0)
        .expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CHANGED);
    assert_eq!(response.payload(), b"on");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

fn echo_rfc8132_app() -> App<profiles::Default, Echo> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["query"], fetch(fetch_query))
        .route(&["delta"], patch(patch_doc))
        .route(&["idem"], ipatch(ipatch_doc))
        .bind(Echo::default())
        .expect("bind")
}

fn client_rfc8132_round_trip(
    send: impl FnOnce(&mut App<profiles::Default, Echo>, Endpoint) -> crate::Call,
    expect_code: Code,
    expect_payload: &[u8],
) {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_rfc8132_app();
    let call = send(&mut app, peer);
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), expect_code);
    assert_eq!(response.payload(), expect_payload);
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(response.peer(), Some(peer));
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_fetch_round_trip_without_slot_id() {
    client_rfc8132_round_trip(
        |app, peer| {
            app.fetch(&["query"])
                .to(peer)
                .payload(b"sel")
                .content_format(ContentFormat::TEXT_PLAIN)
                .send(0)
                .expect("send")
        },
        Code::CONTENT,
        b"sel",
    );
}

#[test]
fn client_patch_round_trip_without_slot_id() {
    client_rfc8132_round_trip(
        |app, peer| {
            app.patch(&["delta"])
                .to(peer)
                .payload(b"delta")
                .send(0)
                .expect("send")
        },
        Code::CHANGED,
        b"delta",
    );
}

#[test]
fn client_ipatch_round_trip_without_slot_id() {
    client_rfc8132_round_trip(
        |app, peer| {
            app.ipatch(&["idem"])
                .to(peer)
                .payload(b"delta")
                .send(0)
                .expect("send")
        },
        Code::CHANGED,
        b"idempotent",
    );
}

#[test]
fn client_non_get_matches() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .non()
        .send(0)
        .expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
    assert_eq!(response.ty(), Some(Type::NonConfirmable));
}

#[test]
fn client_get_sends_query_accept_etag_if_match_and_block2() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(RecordIo::default())
        .expect("bind");
    let block = BlockValue::from_size(0, false, 64).expect("szx");
    let _call = app
        .get("query")
        .to(peer)
        .query("first=1")
        .query("second=2")
        .accept(ContentFormat::TEXT_PLAIN)
        .etag(b"etag1")
        .if_match(b"etag1")
        .if_none_match()
        .block2(block)
        .send(0)
        .expect("send");
    assert_eq!(app.transport().sent_n, 1);
    let (_, bytes, n) = app.transport().sent[0].expect("tx");
    let parsed = decode(&bytes[..n]).expect("decode");
    assert_eq!(parsed.code(), Code::GET);
    let mut queries = parsed.uri_query();
    assert_eq!(queries.next().and_then(Result::ok), Some("first=1"));
    assert_eq!(queries.next().and_then(Result::ok), Some("second=2"));
    assert!(queries.next().is_none());
    assert_eq!(
        parsed.accept().and_then(Result::ok),
        Some(ContentFormat::TEXT_PLAIN)
    );
    assert_eq!(parsed.etag().next(), Some(&b"etag1"[..]));
    assert_eq!(parsed.if_match().next(), Some(&b"etag1"[..]));
    assert!(parsed.if_none_match());
    let b2 = parsed.block2().and_then(Result::ok).expect("block2");
    assert_eq!(b2.num(), 0);
    assert!(!b2.more());
    assert_eq!(b2.size(), 64);
}

#[test]
fn client_full_path_query_and_extras_all_on_wire() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(RecordIo::default())
        .expect("bind");
    let block = BlockValue::from_size(0, false, 64).expect("szx");
    let path = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7"];
    let _call = app
        .get(&path)
        .to(peer)
        .query("a=0")
        .query("a=1")
        .query("a=2")
        .query("a=3")
        .query("a=4")
        .query("a=5")
        .query("a=6")
        .query("a=7")
        .accept(ContentFormat::TEXT_PLAIN)
        .etag(b"etag1")
        .if_match(b"etag1")
        .if_none_match()
        .block2(block)
        .send(0)
        .expect("send");
    assert_eq!(app.transport().sent_n, 1);
    let (_, bytes, n) = app.transport().sent[0].expect("tx");
    let parsed = decode(&bytes[..n]).expect("decode");
    assert_eq!(parsed.code(), Code::GET);
    let mut segs = parsed.uri_path();
    for want in path {
        assert_eq!(segs.next().and_then(Result::ok), Some(want));
    }
    assert!(segs.next().is_none());
    let mut queries = parsed.uri_query();
    for i in 0..8 {
        let want = match i {
            0 => "a=0",
            1 => "a=1",
            2 => "a=2",
            3 => "a=3",
            4 => "a=4",
            5 => "a=5",
            6 => "a=6",
            _ => "a=7",
        };
        assert_eq!(queries.next().and_then(Result::ok), Some(want));
    }
    assert!(queries.next().is_none());
    assert_eq!(
        parsed.accept().and_then(Result::ok),
        Some(ContentFormat::TEXT_PLAIN)
    );
    assert_eq!(parsed.etag().next(), Some(&b"etag1"[..]));
    assert_eq!(parsed.if_match().next(), Some(&b"etag1"[..]));
    assert!(parsed.if_none_match());
    assert!(parsed.block2().and_then(Result::ok).is_some());
}

fn get_tagged(_: Request<'_>) -> Response<'static> {
    Response::valid().etag(b"etag1")
}

#[test]
fn client_take_response_copies_etag() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["validate"], get(get_tagged))
        .bind(Echo::default())
        .expect("bind");
    let call = app.get("validate").to(peer).send(0).expect("send");
    app.poll(0).expect("server");
    app.poll(0).expect("client");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::VALID);
    assert_eq!(response.etag_bytes(), Some(&b"etag1"[..]));
}

#[test]
fn client_get_unknown_path_is_problem_details() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app.get(&["nope"]).to(peer).send(0).expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::NOT_FOUND);
    let details = response.problem_details().expect("cbor");
    assert_eq!(details.response_code(), Some(Code::NOT_FOUND));
    assert_eq!(details.title_text(), Some("Not Found"));
}

#[test]
fn client_path_too_long_is_error() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let err = app
        .get(&["a", "b", "c", "d", "e", "f", "g", "h", "too-many"])
        .to(peer)
        .send(0)
        .expect_err("path");
    assert_eq!(err, Error::Path);
}

/// Send queues into recv so one App is both client and server (Block2 / Q-Block2).
#[derive(Default)]
struct Pipe {
    slots: [Option<(Endpoint, [u8; WIRE], usize)>; 8],
    head: usize,
    len: usize,
}

impl DatagramIo for Pipe {
    type Error = &'static str;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        if self.len == 0 {
            return Ok(None);
        }
        let i = self.head;
        let Some((ep, bytes, n)) = self.slots[i].take() else {
            return Ok(None);
        };
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
        if n > buf.len() {
            return Err("short buf");
        }
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(Some((n, ep)))
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if bytes.len() > WIRE {
            return Err("too long");
        }
        if self.len >= self.slots.len() {
            return Err("pipe full");
        }
        let i = (self.head + self.len) % self.slots.len();
        let mut slot = [0u8; WIRE];
        slot[..bytes.len()].copy_from_slice(bytes);
        self.slots[i] = Some((dest, slot, bytes.len()));
        self.len += 1;
        Ok(bytes.len())
    }
}

fn pipe_app() -> App<profiles::Default, Pipe, DEFAULT_ROUTES, true> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(get_large))
        .route(&["upload"], put(put_large))
        .bind(Pipe::default())
        .expect("bind")
}

struct Taken {
    code: Code,
    format: Option<ContentFormat>,
    token: Option<Token>,
    peer: Option<Endpoint>,
    has_body: bool,
    body: [u8; RESPONSE_BODY],
    body_len: usize,
}

fn poll_until_response(
    app: &mut App<profiles::Default, Pipe, DEFAULT_ROUTES, true>,
    call: crate::Call,
) -> Taken {
    for t in 0u64..16 {
        app.poll(t).expect("poll");
        if let Some(response) = app.take_response(call) {
            let response = response.expect("remote response");
            let mut body = [0u8; RESPONSE_BODY];
            let (has_body, body_len) = match response.body() {
                Some(src) => {
                    body[..src.len()].copy_from_slice(src);
                    (true, src.len())
                }
                None => (false, 0),
            };
            return Taken {
                code: response.code(),
                format: response.format(),
                token: response.token(),
                peer: response.peer(),
                has_body,
                body,
                body_len,
            };
        }
    }
    panic!("client block-wise exchange did not complete");
}

#[test]
fn client_get_block2_assembles_body_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let call = app.get(&["large"]).to(peer).send(0).expect("send");
    let response = poll_until_response(&mut app, call);
    assert_eq!(response.code, Code::CONTENT);
    assert_eq!(response.format, Some(ContentFormat::OCTET_STREAM));
    assert!(response.has_body);
    assert_eq!(&response.body[..response.body_len], &LARGE[..]);
    assert_eq!(response.token, Some(call.token()));
    assert_eq!(response.peer, Some(peer));
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_put_block1_assembles_body_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let call = app
        .put(&["upload"])
        .to(peer)
        .payload(&LARGE)
        .content_format(ContentFormat::OCTET_STREAM)
        .send(0)
        .expect("send");
    let response = poll_until_response(&mut app, call);
    assert_eq!(response.code, Code::CHANGED);
    assert_eq!(response.token, Some(call.token()));
    assert_eq!(response.peer, Some(peer));
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_put_q_block1_assembles_body_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let call = app
        .put(&["upload"])
        .to(peer)
        .payload(&LARGE)
        .request_tag(crate::storage::BodyTag::new(b"upload-1").unwrap())
        .q_block1()
        .send(0)
        .expect("send");
    let response = poll_until_response(&mut app, call);
    assert_eq!(response.code, Code::CHANGED);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_get_q_block2_assembles_body_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let call = app
        .get(&["large"])
        .to(peer)
        .q_block2()
        .send(0)
        .expect("send");
    let response = poll_until_response(&mut app, call);
    assert_eq!(response.code, Code::CONTENT);
    assert!(response.has_body);
    assert_eq!(&response.body[..response.body_len], &LARGE[..]);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_get_slash_path_round_trip() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app.get("sensors/temp").to(peer).send(0).expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
}

#[test]
fn client_empty_path_segment_is_error() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let err = app
        .get("sensors//temp")
        .to(peer)
        .send(0)
        .expect_err("empty");
    assert_eq!(err, Error::Path);
    let err = app
        .get(&["sensors", "", "temp"])
        .to(peer)
        .send(0)
        .expect_err("empty slice");
    assert_eq!(err, Error::Path);
}

fn echo_obs_app() -> App<profiles::Default, Echo> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(Echo::default())
        .expect("bind")
}

fn client_observe_live(app: &App<profiles::Default, Echo>, call: crate::Call) -> bool {
    let key = ObserveKey::new_client(call.token(), call.peer());
    app.engine().lookup_observe(key).is_some()
}

#[test]
fn client_observe_register_notify_deregister() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_obs_app();
    let call = app
        .get("sensors/temp")
        .observe()
        .to(peer)
        .send(0)
        .expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let initial = app
        .take_response(call)
        .expect("initial")
        .expect("remote response");
    assert_eq!(initial.code(), Code::CONTENT);
    assert_eq!(initial.payload(), b"obs-0");
    assert_eq!(initial.observe_seq(), Some(0));
    assert_eq!(initial.token(), Some(call.token()));
    assert!(client_observe_live(&app, call));

    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-1").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(sent, 1);
    app.poll(10).expect("client notify");
    let note = app
        .take_response(call)
        .expect("notification")
        .expect("remote response");
    assert_eq!(note.payload(), b"obs-1");
    assert_eq!(note.observe_seq(), Some(1));
    assert_eq!(note.token(), Some(call.token()));
    assert_eq!(note.peer(), Some(peer));

    let stop = app
        .get("sensors/temp")
        .deregister()
        .to(peer)
        .send(20)
        .expect("deregister");
    assert_eq!(stop, call);
    assert!(!client_observe_live(&app, call));
    app.poll(20).expect("server deregister");
    app.poll(20).expect("client deregister reply");
    let _ = app.take_response(call);

    let sent = app
        .notify(30, &["sensors", "temp"], Response::content(b"obs-2"))
        .expect("notify after stop");
    assert_eq!(sent, 0);
    app.poll(30).expect("no notify");
    assert!(app.take_response(call).is_none());
    assert!(!client_observe_live(&app, call));
}

fn record_client() -> App<profiles::Default, RecordIo> {
    App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(RecordIo::default())
        .expect("bind")
}

fn inject_piggyback_ack(
    app: &mut App<profiles::Default, RecordIo>,
    peer: Endpoint,
    mid: MessageId,
    token: Token,
    payload: &[u8],
) {
    let cf = ContentFormat::TEXT_PLAIN.encode();
    let mut opts = OptionsBuilder::<4>::new();
    opts.push(Opt::content_format(&cf)).expect("cf");
    let ack = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut buf = [0u8; 256];
    let n = encode(&ack, &mut buf).expect("ack");
    app.transport_mut().inbox = Some((peer, buf, n));
}

fn inject_separate_con(
    app: &mut App<profiles::Default, RecordIo>,
    peer: Endpoint,
    mid: MessageId,
    token: Token,
    payload: &[u8],
) {
    let cf = ContentFormat::TEXT_PLAIN.encode();
    let mut opts = OptionsBuilder::<4>::new();
    opts.push(Opt::content_format(&cf)).expect("cf");
    let con = Message::new(Type::Confirmable, Code::CONTENT, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut buf = [0u8; 256];
    let n = encode(&con, &mut buf).expect("separate");
    app.transport_mut().inbox = Some((peer, buf, n));
}

#[test]
fn client_separate_response_stops_con_retransmit() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let (req_mid, token) = {
        let (_, bytes, n) = app.transport().sent[0].expect("first");
        let first = decode(&bytes[..n]).expect("decode first");
        (first.message_id(), first.token())
    };
    assert_eq!(token, call.token());
    assert_eq!(app.engine_mut().tx_occupied(), 1);

    let sep_mid = MessageId::new(req_mid.get().wrapping_add(1));
    assert_ne!(sep_mid, req_mid);
    inject_separate_con(&mut app, peer, sep_mid, token, b"later");
    app.poll(0).expect("match separate");
    let response = app
        .take_response(call)
        .expect("matched")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"later");
    assert_eq!(
        app.engine_mut().tx_occupied(),
        0,
        "request pending CON must be released"
    );

    let after_match = app.transport().sent_n;
    let timeout = u64::from(Transmission::ACK_TIMEOUT_MS);
    app.poll(timeout).expect("past request RTO");
    assert_eq!(
        app.transport().sent_n,
        after_match,
        "separate CON must stop request retransmit"
    );
}

#[test]
fn client_con_retransmit_after_dropped_first_send() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert_eq!(app.transport().sent_n, 1);
    let mut first_wire = [0u8; 256];
    let (first_ty, first_code, first_token, mid, first_n) = {
        let (_, bytes, n) = app.transport().sent[0].expect("first");
        first_wire[..n].copy_from_slice(&bytes[..n]);
        let first = decode(&bytes[..n]).expect("decode first");
        (
            first.ty(),
            first.code(),
            first.token(),
            first.message_id(),
            n,
        )
    };
    assert_eq!(first_ty, Type::Confirmable);
    assert_eq!(first_code, Code::GET);
    assert_eq!(first_token, call.token());
    assert_eq!(app.engine_mut().tx_occupied(), 1);
    assert!(app.take_response(call).is_none());

    let timeout = u64::from(Transmission::ACK_TIMEOUT_MS);
    app.poll(timeout - 1).expect("before RTO");
    assert_eq!(app.transport().sent_n, 1);

    app.poll(timeout).expect("retransmit");
    assert_eq!(app.transport().sent_n, 2);
    let mut retry_wire = [0u8; 256];
    let (retry_ty, retry_code, retry_mid, retry_token, retry_n) = {
        let (_, bytes, n) = app.transport().sent[1].expect("retry");
        retry_wire[..n].copy_from_slice(&bytes[..n]);
        let retry = decode(&bytes[..n]).expect("decode retry");
        (
            retry.ty(),
            retry.code(),
            retry.message_id(),
            retry.token(),
            n,
        )
    };
    assert_eq!(
        &first_wire[..first_n],
        &retry_wire[..retry_n],
        "Due resends the same CON bytes"
    );
    assert_eq!(retry_ty, Type::Confirmable);
    assert_eq!(retry_code, Code::GET);
    assert_eq!(retry_mid, mid);
    assert_eq!(retry_token, call.token());
    assert_eq!(retry_n, first_n);

    inject_piggyback_ack(&mut app, peer, mid, call.token(), b"21.5");
    app.poll(timeout).expect("client match");
    let response = app
        .take_response(call)
        .expect("matched after loss")
        .expect("remote response");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_con_give_up_releases_after_max_retransmit() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert_eq!(app.transport().sent_n, 1);

    let mut now = u64::from(Transmission::ACK_TIMEOUT_MS);
    let mut timeout = Transmission::ACK_TIMEOUT_MS;
    for _ in 0..Transmission::MAX_RETRANSMIT {
        app.poll(now).expect("due");
        timeout = timeout.saturating_mul(2);
        now = now.saturating_add(u64::from(timeout));
    }
    assert_eq!(
        app.transport().sent_n,
        1 + usize::from(Transmission::MAX_RETRANSMIT)
    );
    assert_eq!(app.engine_mut().tx_occupied(), 1);
    assert!(app.take_response(call).is_none());

    app.poll(now).expect("give up");
    assert_eq!(
        app.transport().sent_n,
        1 + usize::from(Transmission::MAX_RETRANSMIT),
        "GiveUp does not send"
    );
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::TimedOut
    );
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn poll_progresses_when_rx_saturated() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(RecordIo::default())
        .expect("bind");
    {
        let engine = app.engine_mut();
        for _ in 0..profiles::Constrained::RX_DATAGRAM_SLOTS {
            let id = engine.acquire_rx().expect("rx");
            engine
                .storage_mut()
                .rx_datagram_mut()
                .pin(id)
                .expect("pin stuck RX");
        }
        assert_eq!(
            engine.rx_occupied(),
            profiles::Constrained::RX_DATAGRAM_SLOTS
        );
    }

    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert_eq!(app.transport().sent_n, 1);

    let timeout = u64::from(Transmission::ACK_TIMEOUT_MS);
    let err = app.poll(timeout).expect_err("RX pool full");
    assert_eq!(err, Error::Saturated);
    assert_eq!(
        app.transport().sent_n,
        2,
        "RTO Due must still send when recv_from is Saturated"
    );
    assert!(app.take_response(call).is_none());
}

#[test]
fn client_second_con_nstart_does_not_leak_tx() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let first = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("first CON");
    assert_eq!(app.transport().sent_n, 1);
    assert_eq!(app.engine_mut().tx_occupied(), 1);
    let first_mid = {
        let (_, bytes, n) = app.transport().sent[0].expect("first wire");
        decode(&bytes[..n]).expect("decode first").message_id()
    };
    assert!(app.engine().lookup_pending_con(first_mid, peer).is_some());

    let err = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect_err("NSTART");
    assert_eq!(err, Error::Saturated);
    assert_eq!(
        app.transport().sent_n,
        1,
        "second CON must not go on the wire without RTO"
    );
    assert_eq!(app.engine_mut().tx_occupied(), 1, "no orphan TX");
    assert!(app.engine().lookup_pending_con(first_mid, peer).is_some());
    assert!(app.take_response(first).is_none());
}

fn client_exchange_live(app: &App<profiles::Default, RecordIo>, call: crate::Call) -> bool {
    let key = ExchangeKey::new(call.token(), call.peer());
    app.engine().lookup_exchange(key).is_some()
}

fn inject_empty(app: &mut App<profiles::Default, RecordIo>, peer: Endpoint, msg: Message<'_>) {
    let mut buf = [0u8; 256];
    let n = encode(&msg, &mut buf).expect("empty");
    app.transport_mut().inbox = Some((peer, buf, n));
}

#[test]
fn client_empty_rst_forgets_outstanding_call() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert!(client_exchange_live(&app, call));
    assert_eq!(app.engine_mut().tx_occupied(), 1);
    let mid = {
        let (_, bytes, n) = app.transport().sent[0].expect("wire");
        decode(&bytes[..n]).expect("decode").message_id()
    };

    inject_empty(&mut app, peer, Message::empty_rst(mid));
    app.poll(0).expect("rst");
    let response = app.take_response(call).expect("rst completes Call");
    assert_eq!(response.unwrap_err(), crate::CallFailure::Reset);
    assert!(!client_exchange_live(&app, call));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
}

#[test]
fn client_empty_ack_then_silence_expires_exchange() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let mid = {
        let (_, bytes, n) = app.transport().sent[0].expect("wire");
        decode(&bytes[..n]).expect("decode").message_id()
    };

    inject_empty(&mut app, peer, Message::empty_ack(mid));
    app.poll(0).expect("ack");
    assert!(
        app.take_response(call).is_none(),
        "empty ACK is not a response"
    );
    assert!(
        client_exchange_live(&app, call),
        "exchange waits for the separate body"
    );
    assert_eq!(app.engine_mut().tx_occupied(), 0);

    let life = u64::from(Transmission::EXCHANGE_LIFETIME_MS);
    app.poll(life - 1).expect("before lifetime");
    assert!(client_exchange_live(&app, call));
    assert!(app.take_response(call).is_none());

    app.poll(life).expect("expire");
    let response = app.take_response(call).expect("lifetime snapshot");
    assert_eq!(response.unwrap_err(), crate::CallFailure::TimedOut);
    assert!(!client_exchange_live(&app, call));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_non_loss_expires_exchange() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .non()
        .to(peer)
        .send(0)
        .expect("send");
    assert!(client_exchange_live(&app, call));
    assert_eq!(app.engine_mut().tx_occupied(), 0, "NON releases TX");

    let life = u64::from(Transmission::NON_LIFETIME_MS);
    app.poll(life - 1).expect("before NON lifetime");
    assert!(client_exchange_live(&app, call));
    assert!(app.take_response(call).is_none());

    app.poll(life).expect("expire NON");
    let response = app.take_response(call).expect("NON lifetime snapshot");
    assert_eq!(response.unwrap_err(), crate::CallFailure::TimedOut);
    assert!(!client_exchange_live(&app, call));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn client_fifth_untaken_send_is_saturated() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let mut calls = [None; 4];
    for (i, slot) in calls.iter_mut().enumerate() {
        let call = app
            .get(&["sensors", "temp"])
            .to(peer)
            .send(0)
            .expect("send");
        let (mid, token) = {
            let (_, bytes, n) = app.transport().sent[i].expect("wire");
            let parsed = decode(&bytes[..n]).expect("decode");
            (parsed.message_id(), parsed.token())
        };
        assert_eq!(token, call.token());
        inject_piggyback_ack(&mut app, peer, mid, token, b"21.5");
        app.poll(0).expect("complete");
        *slot = Some(call);
    }
    let err = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect_err("inbox full");
    assert_eq!(err, Error::Saturated);
    assert_eq!(
        app.transport().sent_n,
        4,
        "fifth send must not go on the wire"
    );
    for call in calls {
        let response = app
            .take_response(call.expect("call"))
            .expect("untaken reply kept")
            .expect("remote response");
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), b"21.5");
    }
    app.get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send after take");
}

#[test]
fn metrics_server_get_moves_rx_tx_progress() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    assert_eq!(app.metrics(), crate::Metrics::ZERO);
    app.poll(0).expect("poll");
    let snap = app.metrics();
    assert_eq!(snap.rx_accepted, 1);
    assert_eq!(snap.rx_error, 0);
    assert_eq!(snap.tx_ok, 1);
    assert_eq!(snap.tx_fail, 0);
    assert_eq!(snap.progress, 1);
    last_reply(&app);
}

#[test]
fn metrics_malformed_datagram_is_rx_error() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut wire = [0u8; 256];
    wire[0] = 0x40;
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, 1)),
        last_send: None,
    });
    app.poll(0).expect("drop");
    let snap = app.metrics();
    assert_eq!(snap.rx_accepted, 1);
    assert_eq!(snap.rx_error, 1);
    assert_eq!(snap.tx_ok, 0);
}

#[test]
fn metrics_observe_register_notify_deregister() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("register");
    assert_eq!(app.metrics().observe_register, 1);
    assert_eq!(app.metrics().rx_accepted, 1);
    assert_eq!(app.metrics().tx_ok, 1);

    app.reset_metrics();
    let sent = app
        .notify(
            10,
            &["sensors", "temp"],
            Response::content(b"obs-1").content_format(ContentFormat::TEXT_PLAIN),
        )
        .expect("notify");
    assert_eq!(sent, 1);
    assert_eq!(app.metrics().observe_notify, 1);
    assert_eq!(app.metrics().tx_ok, 1);

    let extra = [Opt::observe_deregister()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1002);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(20).expect("deregister");
    assert_eq!(app.metrics().observe_cancel, 1);
}

#[test]
fn metrics_client_retransmit_and_give_up() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    assert_eq!(app.metrics().tx_ok, 1);

    let timeout = u64::from(Transmission::ACK_TIMEOUT_MS);
    app.poll(timeout - 1).expect("before RTO");
    assert_eq!(app.metrics().con_retransmit, 0);

    app.poll(timeout).expect("retransmit");
    assert_eq!(app.metrics().con_retransmit, 1);
    assert_eq!(app.metrics().tx_ok, 2);
    assert_eq!(app.metrics().give_up, 0);

    let mut now = timeout;
    let mut wait = Transmission::ACK_TIMEOUT_MS.saturating_mul(2);
    for _ in 1..Transmission::MAX_RETRANSMIT {
        now = now.saturating_add(u64::from(wait));
        app.poll(now).expect("due");
        wait = wait.saturating_mul(2);
    }
    assert_eq!(
        app.metrics().con_retransmit,
        u32::from(Transmission::MAX_RETRANSMIT)
    );
    now = now.saturating_add(u64::from(wait));
    app.poll(now).expect("give up");
    assert_eq!(app.metrics().give_up, 1);
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::TimedOut
    );
}

#[test]
fn metrics_nstart_reject() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let _first = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("first CON");
    let err = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect_err("NSTART");
    assert_eq!(err, Error::Saturated);
    assert_eq!(app.metrics().nstart_reject, 1);
    assert_eq!(app.metrics().tx_ok, 1);
}

#[test]
fn metrics_empty_rst_path() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let mid = {
        let (_, bytes, n) = app.transport().sent[0].expect("wire");
        decode(&bytes[..n]).expect("decode").message_id()
    };
    inject_empty(&mut app, peer, Message::empty_rst(mid));
    app.poll(0).expect("rst");
    assert_eq!(app.metrics().empty_rst, 1);
    assert_eq!(app.metrics().rx_accepted, 1);
    assert!(app.take_response(call).is_some());
}

#[test]
fn metrics_block1_assemble() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &payload, 0, true, 16, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["leds", "0"], put(put_body))
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            last_send: None,
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert_eq!(app.metrics().block1_assemble, 1);
    assert_eq!(last_reply(&app).code, Code::CONTINUE);
}

#[test]
fn metrics_rx_saturated_and_reset() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(RecordIo::default())
        .expect("bind");
    {
        let engine = app.engine_mut();
        for _ in 0..profiles::Constrained::RX_DATAGRAM_SLOTS {
            let id = engine.acquire_rx().expect("rx");
            engine
                .storage_mut()
                .rx_datagram_mut()
                .pin(id)
                .expect("pin stuck RX");
        }
    }
    app.reset_metrics();
    let _call = app
        .get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send");
    let timeout = u64::from(Transmission::ACK_TIMEOUT_MS);
    let err = app.poll(timeout).expect_err("RX pool full");
    assert_eq!(err, Error::Saturated);
    assert_eq!(app.metrics().saturated, 1);
    assert_eq!(app.metrics().con_retransmit, 1);
    app.reset_metrics();
    assert_eq!(app.metrics(), crate::Metrics::ZERO);
}

#[cfg(feature = "oscore")]
#[test]
fn options_full_500_under_oscore_is_protected() {
    use crate::oscore::{DeriveParams, SecurityContext};
    use crate::storage::{EngineBuilder, Memory};

    const MASTER_SECRET: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    const MASTER_SALT: [u8; 8] = [0x9e, 0x7c, 0xa9, 0x22, 0x23, 0x78, 0x63, 0x40];
    let mut client = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[],
        recipient_id: &[0x01],
        id_context: &[],
    })
    .unwrap();
    let mut server = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &MASTER_SALT,
        sender_id: &[0x01],
        recipient_id: &[],
        id_context: &[],
    })
    .unwrap();

    let token = Token::from_checked(&[1]);
    let req = Message::new(Type::Confirmable, Code::GET, MessageId::new(1)).with_token(token);
    let mut wire = [0u8; 256];
    let n = client.protect_request(&req, &mut wire).unwrap();
    let protected = decode(&wire[..n]).unwrap();
    let mut inner = [0u8; 256];
    let (_plain, request) = server.unprotect_request(&protected, &mut inner).unwrap();

    let mut engine = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .expect("engine");
    let tx = engine.acquire_tx().expect("tx");
    let ctx = Some(server);
    super::encode_options_full_500::<_, &'static str>(
        &mut engine,
        tx,
        Type::Acknowledgement,
        MessageId::new(1),
        token,
        &ctx,
        Some(request),
    )
    .expect("protect 5.00");

    let access = engine.access_tx(tx).expect("access");
    let parsed = decode(access.as_bytes()).expect("decode");
    assert!(
        parsed.oscore().is_some(),
        "OptionsFull 5.00 under OSCORE must be protected"
    );
    assert_ne!(
        parsed.code(),
        Code::INTERNAL_SERVER_ERROR,
        "outer code must not leak unprotected 5.00"
    );
}

#[cfg(feature = "oscore")]
#[test]
fn options_full_500_oscore_without_request_is_not_plaintext() {
    use crate::oscore::{DeriveParams, Error as OscoreError, SecurityContext};
    use crate::storage::{EngineBuilder, Memory};

    const MASTER_SECRET: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    let server = SecurityContext::derive(DeriveParams {
        master_secret: &MASTER_SECRET,
        master_salt: &[],
        sender_id: &[0x01],
        recipient_id: &[],
        id_context: &[],
    })
    .unwrap();

    let mut engine = EngineBuilder::new()
        .profile::<profiles::Default>()
        .block_wise(false)
        .build(Memory::<profiles::Default>::new())
        .expect("engine");
    let tx = engine.acquire_tx().expect("tx");
    let ctx = Some(server);
    let err = super::encode_options_full_500::<_, &'static str>(
        &mut engine,
        tx,
        Type::Acknowledgement,
        MessageId::new(1),
        Token::from_checked(&[1]),
        &ctx,
        None,
    )
    .unwrap_err();
    assert_eq!(err, Error::Oscore(OscoreError::Context));
    let access = engine.access_tx(tx).expect("empty tx");
    assert!(
        access.as_bytes().is_empty(),
        "must not emit unprotected 5.00 when RequestRef is missing"
    );
}

#[test]
fn response_unknown_option_policy_precedes_completion_and_ack() {
    for ty in [
        Type::Confirmable,
        Type::NonConfirmable,
        Type::Acknowledgement,
    ] {
        for critical in [true, false] {
            let peer = Endpoint::v4([192, 0, 2, 2], 5683);
            let mut app = App::profile::<profiles::Default>()
                .deterministic_for_tests()
                .block_wise::<false>()
                .bind(RecordIo::default())
                .unwrap();
            let call = app.get("value").to(peer).send(0).unwrap();
            let (_, req, rn) = app.transport().sent[0].unwrap();
            let request_mid = decode(&req[..rn]).unwrap().message_id();
            let mid = if ty == Type::Acknowledgement {
                request_mid
            } else {
                MessageId::new(400)
            };
            let extras = [Opt::new(
                OptionNumber::new(if critical { 65001 } else { 65000 }),
                &[],
            )];
            let msg = Message::new(ty, Code::CONTENT, mid)
                .with_token(call.token())
                .with_options(&extras)
                .with_payload(b"answer");
            let mut wire = [0u8; 256];
            let n = encode(&msg, &mut wire).unwrap();
            app.transport_mut().inbox = Some((peer, wire, n));
            app.poll(1).unwrap();
            assert_eq!(app.take_response(call).is_some(), !critical);
            assert_eq!(app.engine_mut().rx_occupied(), 0);
            if ty == Type::Confirmable {
                let (_, reply, rn) = app.transport().sent[1].unwrap();
                let response = decode(&reply[..rn]).unwrap();
                assert_eq!(
                    response.ty(),
                    if critical {
                        Type::Reset
                    } else {
                        Type::Acknowledgement
                    }
                );
                assert_eq!(response.message_id(), mid);
                assert!(response.is_empty());
            } else {
                assert_eq!(app.transport().sent_n, 1, "no ACK or RST to ACK/NON");
            }
            if critical && ty == Type::Acknowledgement {
                assert_eq!(app.engine_mut().tx_occupied(), 0, "matching ACK stops RTO");
                app.poll(u64::from(Transmission::ACK_TIMEOUT_MS) * 2)
                    .unwrap();
                assert_eq!(
                    app.transport().sent_n,
                    1,
                    "rejected ACK body is not a lost ACK"
                );
                assert!(app.take_response(call).is_none());
            }
        }
    }
}

#[test]
fn block2_requests_with_new_tokens_select_requested_ranges_and_release_slots() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(get_large))
        .bind(WideLoopback::default())
        .expect("bind");
    // More independent requests than the TX body pool capacity, including
    // backwards seeks and a smaller negotiated size. Tokens need not match.
    for (i, num) in [2, 0, 1, 2, 3, 0].into_iter().enumerate() {
        let block = BlockValue::from_size(num, false, 512)
            .expect("block")
            .encode();
        let token = Token::from_checked(&[i as u8]);
        let (wire, n) = encode_wide_token(
            Code::GET,
            &["large"],
            &[Opt::block2(&block)],
            0x7000 + i as u16,
            token,
        );
        app.transport_mut().inbox = Some((peer, wire, n));
        app.transport_mut().send_n = 0;
        app.poll(i as u64).expect("requested block");
        let response = last_wide(&app);
        let got = response.block2().unwrap().unwrap();
        assert_eq!(got.num(), num);
        assert_eq!(got.size(), 512);
        assert_eq!(response.token(), token);
        let offset = num as usize * 512;
        let end = (offset + 512).min(LARGE.len());
        assert_eq!(response.payload(), &LARGE[offset..end]);
        assert_eq!(got.more(), end < LARGE.len());
        assert!(
            app.engine_mut()
                .lookup_tx_body(BlockKey::new(token, peer))
                .is_none()
        );
    }
}

#[test]
fn block2_outside_representation_refuses_without_retaining_body() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let block = BlockValue::from_size(100, false, 512).unwrap().encode();
    let (wire, n) = encode_wide(Code::GET, &["large"], &[Opt::block2(&block)], 0x7100);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(get_large))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .unwrap();
    app.poll(0).expect("bad range is a protocol rejection");
    assert_eq!(last_wide(&app).code(), Code::BAD_REQUEST);
    assert!(
        app.engine_mut()
            .lookup_tx_body(BlockKey::new(Token::from_checked(&[0xA1]), peer))
            .is_none()
    );
}

#[test]
fn block2_continuations_preserve_ordered_queries_and_accept() {
    #[derive(Default)]
    struct CheckedPipe {
        pipe: Pipe,
        requests: usize,
    }
    impl DatagramIo for CheckedPipe {
        type Error = &'static str;
        fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            self.pipe.recv(buf)
        }
        fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            let parsed = decode(bytes).unwrap();
            if parsed.code() == Code::GET {
                let mut queries = parsed.uri_query();
                assert_eq!(queries.next(), Some(Ok("rt=Type1")));
                assert_eq!(queries.next(), Some(Ok("")));
                assert_eq!(queries.next(), Some(Ok("if=If1")));
                assert_eq!(queries.next(), None);
                assert_eq!(parsed.accept(), Some(Ok(ContentFormat::OCTET_STREAM)));
                assert_eq!(parsed.echo(), Some(&b"challenge"[..]));
                assert_eq!(
                    parsed.no_response(),
                    Some(Ok(crate::message::NoResponse::DEFAULT))
                );
                assert_eq!(parsed.request_tag().next(), Some(&b"response"[..]));
                self.requests += 1;
            }
            self.pipe.send(dest, bytes)
        }
    }
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route("large", get(get_large))
        .bind(CheckedPipe::default())
        .unwrap();
    let call = app
        .get("large")
        .to(Endpoint::v4([192, 0, 2, 2], 5683))
        .query("rt=Type1")
        .query("")
        .query("if=If1")
        .accept(ContentFormat::OCTET_STREAM)
        .echo(EchoOpt::new(b"challenge").unwrap())
        .no_response(crate::message::NoResponse::DEFAULT)
        .request_tag(crate::storage::BodyTag::new(b"response").unwrap())
        .block2(BlockValue::from_size(0, false, 64).unwrap())
        .send(0)
        .unwrap();
    let mut complete = false;
    for now in 0..200 {
        app.poll(now).unwrap();
        if let Some(response) = app.take_response(call) {
            let response = response.expect("remote response");
            assert_eq!(response.body(), Some(&LARGE[..]));
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert!(app.transport().requests > 1);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn query_retention_bounds_refuse_before_sending_or_occupying_slots() {
    let bytes = [b'a'; 257];
    let text = core::str::from_utf8(&bytes).unwrap();
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for (first, second, accepted) in [(255, 1, true), (255, 2, false), (256, 0, false)] {
        let mut app = pipe_app();
        let result = app
            .get("large")
            .to(peer)
            .query(&text[..first])
            .query(&text[..second])
            .send(0);
        if accepted {
            assert!(result.is_ok());
            assert_eq!(app.transport().len, 1);
        } else {
            assert_eq!(
                result,
                Err(Error::Message(SlotMessageError::Encode(
                    EncodeError::OptionValueTooLong
                )))
            );
            assert_eq!(app.transport().len, 0);
            assert_eq!(app.engine_mut().tx_occupied(), 0);
            assert!(
                app.get("large").to(peer).send(1).is_ok(),
                "refusal must not consume a live call"
            );
        }
    }
}

#[test]
fn excess_query_count_is_refused_instead_of_silently_dropped() {
    let mut app = pipe_app();
    let mut outgoing = app.get("large").to(Endpoint::v4([192, 0, 2, 2], 5683));
    for _ in 0..9 {
        outgoing = outgoing.query("rt=x");
    }
    assert_eq!(
        outgoing.send(0),
        Err(Error::Message(SlotMessageError::Encode(
            EncodeError::OptionValueTooLong
        )))
    );
    assert_eq!(app.transport().len, 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn block1_and_qblock1_preserve_query_and_accept_on_every_upload_block() {
    struct CheckedUpload {
        pipe: Pipe,
        requests: usize,
        assembled: [u8; 2000],
        filled: usize,
    }
    impl Default for CheckedUpload {
        fn default() -> Self {
            Self {
                pipe: Pipe::default(),
                requests: 0,
                assembled: [0; 2000],
                filled: 0,
            }
        }
    }
    impl DatagramIo for CheckedUpload {
        type Error = &'static str;
        fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            self.pipe.recv(buf)
        }
        fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            let parsed = decode(bytes).unwrap();
            if parsed.code() == Code::PUT {
                let mut queries = parsed.uri_query();
                assert_eq!(queries.next(), Some(Ok("target=one")));
                assert_eq!(queries.next(), Some(Ok("")));
                assert_eq!(queries.next(), Some(Ok("version=2")));
                assert_eq!(queries.next(), None);
                assert_eq!(parsed.accept(), Some(Ok(ContentFormat::OCTET_STREAM)));
                assert_eq!(parsed.echo(), Some(&b"challenge"[..]));
                assert_eq!(
                    parsed.content_format(),
                    Some(Ok(ContentFormat::OCTET_STREAM))
                );
                if parsed.q_block1().is_some() {
                    assert_eq!(parsed.request_tag().next(), Some(&b"upload-1"[..]));
                }
                let block = parsed
                    .block1()
                    .or_else(|| parsed.q_block1())
                    .unwrap()
                    .unwrap();
                let offset = block.num() as usize * usize::from(block.size());
                assert_eq!(offset, self.filled);
                self.assembled[offset..offset + parsed.payload().len()]
                    .copy_from_slice(parsed.payload());
                self.filled += parsed.payload().len();
                self.requests += 1;
            }
            self.pipe.send(dest, bytes)
        }
    }
    for qblock in [false, true] {
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route(
                "upload",
                put(|req: Request<'_>| {
                    assert_eq!(req.body().unwrap(), &LARGE[..]);
                    Response::changed()
                }),
            )
            .bind(CheckedUpload::default())
            .unwrap();
        let mut request = app
            .put("upload")
            .to(Endpoint::v4([192, 0, 2, 2], 5683))
            .query("target=one")
            .query("")
            .query("version=2")
            .accept(ContentFormat::OCTET_STREAM)
            .echo(EchoOpt::new(b"challenge").unwrap())
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&LARGE);
        if qblock {
            request = request
                .request_tag(crate::storage::BodyTag::new(b"upload-1").unwrap())
                .q_block1();
        }
        let call = request.send(0).unwrap();
        let mut complete = false;
        for now in 0..100 {
            app.poll(now).unwrap();
            if let Some(response) = app.take_response(call) {
                let response = response.expect("remote response");
                assert_eq!(response.code(), Code::CHANGED);
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert!(app.transport().requests > 1);
        assert_eq!(
            &app.transport().assembled[..app.transport().filled],
            &LARGE[..]
        );
        assert_eq!(app.engine_mut().rx_occupied(), 0);
        assert_eq!(app.engine_mut().tx_occupied(), 0);
    }
}

#[test]
fn upload_query_overflow_refuses_without_io_or_body_allocation() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let long = [b'x'; 257];
    let text = core::str::from_utf8(&long).unwrap();
    for qblock in [false, true] {
        for count_overflow in [false, true] {
            let mut app = pipe_app();
            let mut outgoing = app.put("upload").to(peer).payload(&LARGE);
            if qblock {
                outgoing = outgoing
                    .request_tag(crate::storage::BodyTag::new(b"upload-1").unwrap())
                    .q_block1();
            }
            if count_overflow {
                for _ in 0..9 {
                    outgoing = outgoing.query("x=1");
                }
            } else {
                outgoing = outgoing.query(&text[..255]).query(&text[..2]);
            }
            assert_eq!(
                outgoing.send(0),
                Err(Error::Message(SlotMessageError::Encode(
                    EncodeError::OptionValueTooLong
                )))
            );
            assert_eq!(app.transport().len, 0);
            assert_eq!(app.engine_mut().tx_occupied(), 0);
            // A rejected upload must not consume the bounded outgoing body pool
            // or live-call admission: all normal block-wise work still completes.
            let call = app.put("upload").to(peer).payload(&LARGE).send(1).unwrap();
            assert_eq!(poll_until_response(&mut app, call).code, Code::CHANGED);
            assert_eq!(app.engine_mut().rx_occupied(), 0);
            assert_eq!(app.engine_mut().tx_occupied(), 0);
        }
    }
}

#[test]
fn fragmented_conditional_upload_is_refused_instead_of_becoming_unconditional() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for qblock in [false, true] {
        for condition in 0..3 {
            let mut app = pipe_app();
            let mut request = app.put("upload").to(peer).payload(&LARGE);
            if qblock {
                request = request
                    .request_tag(crate::storage::BodyTag::new(b"upload-1").unwrap())
                    .q_block1();
            }
            request = match condition {
                0 => request.if_match(b"version"),
                1 => request.if_none_match(),
                _ => request.etag(b"version"),
            };
            assert_eq!(request.send(0), Err(Error::ConditionalUploadUnsupported));
            assert_eq!(
                app.transport().len,
                0,
                "no unconditional fragment may escape"
            );
            assert_eq!(app.engine_mut().tx_occupied(), 0);
            let call = app.put("upload").to(peer).payload(&LARGE).send(1).unwrap();
            assert_eq!(poll_until_response(&mut app, call).code, Code::CHANGED);
        }
    }
}

#[test]
fn single_datagram_conditional_upload_preserves_conditions() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for condition in 0..3 {
        let mut app = pipe_app();
        let request = app.put("upload").to(peer).payload(b"small");
        let request = match condition {
            0 => request.if_match(b"version"),
            1 => request.if_none_match(),
            _ => request.etag(b"version"),
        };
        request.send(0).unwrap();
        let (_, bytes, n) = app.transport().slots[0].as_ref().unwrap();
        let message = decode(&bytes[..*n]).unwrap();
        assert_eq!(message.payload(), b"small");
        assert!(message.block1().is_none());
        let option = match condition {
            0 => OptionNumber::IF_MATCH,
            1 => OptionNumber::IF_NONE_MATCH,
            _ => OptionNumber::ETAG,
        };
        assert_eq!(
            message.get_option(option).map(Opt::value),
            Some(if condition == 1 {
                &b""[..]
            } else {
                &b"version"[..]
            })
        );
    }
}

#[test]
fn qblock_upload_requires_tag_and_refusal_preserves_capacity() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    assert_eq!(
        app.put("upload")
            .to(peer)
            .payload(&LARGE)
            .q_block1()
            .send(0),
        Err(Error::RequestTagRequired)
    );
    assert_eq!(app.transport().len, 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    let call = app
        .put("upload")
        .to(peer)
        .payload(&LARGE)
        .request_tag(crate::storage::BodyTag::EMPTY)
        .q_block1()
        .send(1)
        .unwrap();
    assert_eq!(poll_until_response(&mut app, call).code, Code::CHANGED);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn active_request_tag_cannot_be_recycled_at_the_same_peer() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let other = Endpoint::v4([192, 0, 2, 3], 5683);
    let tag = crate::storage::BodyTag::new(b"unique").unwrap();
    let mut app = pipe_app();
    let first = app.get("large").to(peer).request_tag(tag).send(0).unwrap();
    assert_eq!(
        app.get("large").to(peer).request_tag(tag).send(0),
        Err(Error::RequestTagInUse)
    );
    assert_eq!(app.transport().len, 1);
    assert_eq!(poll_until_response(&mut app, first).code, Code::CONTENT);
    app.get("large").to(peer).request_tag(tag).send(1).unwrap();
    app.get("large").to(other).request_tag(tag).send(1).unwrap();
}

#[test]
fn malformed_or_missing_qblock_request_tag_is_refused_before_dispatch() {
    let block = BlockValue::from_size(0, false, 16).unwrap().encode();
    for tags in [
        &[][..],
        &[&b"123456789"[..]][..],
        &[&b"a"[..], &b"b"[..]][..],
    ] {
        let mut opts = OptionsBuilder::<8>::new();
        opts.push(Opt::q_block1(&block)).unwrap();
        for tag in tags {
            opts.push(Opt::request_tag(tag)).unwrap();
        }
        let (wire, n) = encode_wide(Code::PUT, &["upload"], opts.as_slice(), 0x7711);
        let peer = Endpoint::v4([192, 0, 2, 2], 5683);
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route(
                "upload",
                put(|_: Request<'_>| -> Response<'static> {
                    panic!("invalid tag reached handler")
                }),
            )
            .bind(WideLoopback {
                inbox: Some((peer, wire, n)),
                ..WideLoopback::default()
            })
            .unwrap();
        app.poll(0).unwrap();
        assert_eq!(last_wide(&app).code(), Code::BAD_OPTION);
        assert_eq!(app.engine_mut().rx_occupied(), 0);
    }
}

#[test]
fn disabled_block_assembly_refuses_before_handler_dispatch() {
    let (wire, n) = encode_req_block1(Code::PUT, &["upload"], &[0x11; 16], 0, true, 16, 0x7111);
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(
            "upload",
            put(|_: Request<'_>| -> Response<'static> {
                panic!("fragment reached non-block handler")
            }),
        )
        .bind(Loopback {
            inbox: Some((peer, wire, n)),
            ..Loopback::default()
        })
        .unwrap();
    app.poll(0).unwrap();
    assert_eq!(last_reply(&app).code, Code::BAD_OPTION);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
}

#[test]
fn empty_query_values_count_toward_the_option_bound() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let mut outgoing = app.get("value").to(peer);
    for _ in 0..9 {
        outgoing = outgoing.query("");
    }
    assert_eq!(
        outgoing.send(0),
        Err(Error::Message(SlotMessageError::Encode(
            EncodeError::OptionValueTooLong
        )))
    );
    assert_eq!(app.transport().len, 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert!(app.get("value").to(peer).query("").send(1).is_ok());
}

#[test]
fn local_failures_are_not_remote_gateway_timeout_responses() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app.get("value").to(peer).send(0).unwrap();
    let (_, request, len) = app.transport().sent[0].unwrap();
    let mid = decode(&request[..len]).unwrap().message_id();
    let message =
        Message::new(Type::Acknowledgement, Code::GATEWAY_TIMEOUT, mid).with_token(call.token());
    let mut wire = [0; 256];
    let n = encode(&message, &mut wire).unwrap();
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).unwrap();
    assert!(
        !app.cancel(call),
        "completed response must not be replaced by cancellation"
    );
    assert_eq!(
        app.take_response(call).unwrap().unwrap().code(),
        Code::GATEWAY_TIMEOUT
    );
    assert!(!app.cancel(call));
}

#[test]
fn untaken_cancelled_calls_remain_bounded_and_release_after_take() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let mut calls = [None; 4];
    for saved in &mut calls {
        let call = app.get("value").to(peer).send(0).unwrap();
        assert!(app.cancel(call));
        assert!(!app.cancel(call));
        assert_eq!(app.engine_mut().tx_occupied(), 0);
        *saved = Some(call);
    }
    assert_eq!(app.get("value").to(peer).send(1), Err(Error::Saturated));
    assert_eq!(app.transport().sent_n, 4);
    for call in calls.into_iter().flatten() {
        assert_eq!(
            app.take_response(call).unwrap().unwrap_err(),
            crate::CallFailure::Cancelled
        );
        assert!(app.take_response(call).is_none());
    }
    assert!(app.get("value").to(peer).send(2).is_ok());
}

#[test]
fn deadline_refusal_boundary_and_reused_tx_ownership() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    for deadline in [0, 10] {
        assert_eq!(
            app.get("value").to(peer).deadline(deadline).send(10),
            Err(Error::DeadlineElapsed)
        );
    }
    assert_eq!(app.transport().sent_n, 0);
    let first = app
        .get("value")
        .non()
        .to(peer)
        .deadline(20)
        .send(10)
        .unwrap();
    let second = app.get("other").to(peer).send(11).unwrap();
    app.poll(19).unwrap();
    assert!(app.take_response(first).is_none());
    app.poll(20).unwrap();
    assert_eq!(
        app.take_response(first).unwrap().unwrap_err(),
        crate::CallFailure::DeadlineExceeded
    );
    assert_eq!(
        app.engine_mut().tx_occupied(),
        1,
        "expired NON must not free another call's reused TX"
    );
    assert!(client_exchange_live(&app, second));
    assert!(app.cancel(second));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert_eq!(
        app.transport().sent_n,
        2,
        "cancellation emits no extra message"
    );
}

#[test]
fn cancellation_reclaims_tagged_upload_and_download_bodies() {
    use crate::storage::BodyTag;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    let tag = BodyTag::new(b"upload").unwrap();
    let call = app
        .put("upload")
        .to(peer)
        .request_tag(tag)
        .payload(&LARGE)
        .send(0)
        .unwrap();
    let tx_key = BlockKey::new(call.token(), peer).with_identity(tag);
    assert!(app.engine_mut().lookup_tx_body(tx_key).is_some());
    let rx_key = BlockKey::new(call.token(), peer).with_identity(BodyTag::new(b"etag").unwrap());
    let block = BlockValue::from_size(0, true, 16).unwrap();
    app.engine_mut()
        .apply_block2(rx_key, block, &[1; 16], Some(32))
        .unwrap();
    assert!(app.cancel(call));
    assert!(app.engine_mut().lookup_rx_body(rx_key).is_none());
    assert!(app.engine_mut().lookup_tx_body(tx_key).is_none());
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::Cancelled
    );
    assert!(
        app.put("upload")
            .to(peer)
            .request_tag(tag)
            .payload(&LARGE)
            .send(1)
            .is_ok()
    );
}

#[test]
fn deadline_terminates_observe_after_the_initial_response() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_obs_app();
    let call = app
        .get("sensors/temp")
        .observe()
        .to(peer)
        .deadline(20)
        .send(0)
        .unwrap();
    app.poll(0).unwrap();
    app.poll(0).unwrap();
    assert!(app.take_response(call).unwrap().is_ok());
    assert!(client_observe_live(&app, call));
    app.poll(19).unwrap();
    assert!(app.take_response(call).is_none());
    app.poll(20).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::DeadlineExceeded
    );
    assert!(!client_observe_live(&app, call));
}

#[test]
fn deadline_progresses_when_receive_fails() {
    struct BadReceive;
    impl DatagramIo for BadReceive {
        type Error = &'static str;
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            Err("receive failure")
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            Ok(bytes.len())
        }
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(BadReceive)
        .unwrap();
    let call = app.get("value").to(peer).deadline(1).send(0).unwrap();
    assert!(app.poll(1).is_err());
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::DeadlineExceeded
    );
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
}

#[test]
fn deadline_wins_at_exact_boundary_but_preserves_an_earlier_response() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for received_at in [9, 10] {
        let mut app = record_client();
        let call = app.get("value").to(peer).deadline(10).send(0).unwrap();
        let (_, request, len) = app.transport().sent[0].unwrap();
        let mid = decode(&request[..len]).unwrap().message_id();
        let message = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
            .with_token(call.token())
            .with_payload(b"value");
        let mut wire = [0; 256];
        let n = encode(&message, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(received_at).unwrap();
        app.poll(11).unwrap();
        let result = app.take_response(call).unwrap();
        if received_at == 9 {
            assert_eq!(result.unwrap().payload(), b"value");
        } else {
            assert_eq!(result.unwrap_err(), crate::CallFailure::DeadlineExceeded);
        }
        assert_eq!(app.engine_mut().rx_occupied(), 0);
        assert_eq!(app.engine_mut().tx_occupied(), 0);
    }
}

#[test]
fn missing_initial_block_completes_with_typed_failure_and_reclaims_state() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    let call = app.get("large").to(peer).send(0).unwrap();
    let request = decode(&app.transport().sends[0][..app.transport().send_lens[0]]).unwrap();
    let block = BlockValue::from_size(1, true, 16).unwrap().encode();
    let options = [Opt::block2(&block)];
    let message = Message::new(Type::Acknowledgement, Code::CONTENT, request.message_id())
        .with_token(call.token())
        .with_options(&options)
        .with_payload(&[1; 16]);
    let mut wire = [0; WIRE];
    let n = encode(&message, &mut wire).unwrap();
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::BlockTransfer(crate::error::BlockTransferError::Gap)
    );
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    assert!(app.get("value").to(peer).send(2).is_ok());
}

#[test]
fn qblock1_bad_size_never_dispatches_and_valid_retry_completes_once() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn handler(req: Request<'_>) -> Response<'static> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        put_body(req)
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    for bad_hint in [None, Some(0), Some(19), Some(21), Some(u32::MAX)] {
        CALLS.store(0, Ordering::SeqCst);
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route(LED_PATH, put(handler))
            .bind(Loopback {
                inbox: None,
                last_send: None,
            })
            .unwrap();
        for phase in 0..4 {
            let payload: &[u8] = if phase < 2 {
                b"AAAAAAAAAAAAAAAA"
            } else {
                b"REST"
            };
            let mut req = q_block1(
                payload,
                if phase < 2 { 0 } else { 1 },
                phase < 2,
                0x1100 + phase,
                20,
            );
            req.size1 = match phase {
                0 => None,
                2 => bad_hint,
                _ => Some(20),
            };
            let (wire, n) = encode_block_req(req);
            app.transport_mut().inbox = Some((peer, wire, n));
            app.transport_mut().last_send = None;
            app.poll(u64::from(phase)).unwrap();
            let reply = last_reply(&app);
            match phase {
                0 | 2 => assert_eq!(reply.code, Code::REQUEST_ENTITY_INCOMPLETE),
                1 => assert_eq!(reply.code, Code::CONTINUE),
                _ => {
                    assert_eq!(reply.code, Code::CHANGED);
                    assert_eq!(&reply.payload[..reply.payload_len], b"AAAAAAAAAAAAAAAAREST");
                }
            }
            assert_eq!(CALLS.load(Ordering::SeqCst), usize::from(phase == 3));
        }
    }
}

#[test]
fn qblock2_without_etag_is_refused_before_sending_or_retaining_body() {
    use crate::error::BlockTransferError;
    use crate::storage::SlotId;
    fn no_tag(_: Request<'_>) -> Response<'static> {
        Response::content(&LARGE)
    }
    fn small_no_tag(_: Request<'_>) -> Response<'static> {
        Response::content(b"small")
    }
    for handler in [no_tag as fn(Request<'_>) -> Response<'static>, small_no_tag] {
        let peer = Endpoint::v4([192, 0, 2, 1], 5683);
        let q = BlockValue::from_size(0, false, 1024).unwrap().encode();
        let (wire, n) = encode_wide(Code::GET, &["large"], &[Opt::q_block2(&q)], 0x1200);
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route(&["large"], get(handler))
            .bind(WideLoopback {
                inbox: Some((peer, wire, n)),
                ..WideLoopback::default()
            })
            .unwrap();
        assert_eq!(
            app.poll(0),
            Err(Error::Block(BlockTransferError::MissingIdentity))
        );
        assert_eq!(app.transport().send_n, 0);
        assert_eq!(app.engine.tx_occupied(), 0);
        for index in 0..app.engine.capacities().tx_body_slots.unwrap() {
            assert!(
                app.engine
                    .tx_body_transfer(SlotId::from_index(index))
                    .is_none()
            );
        }
    }
}

#[test]
fn qblock2_continuation_refuses_changed_identity_or_body_then_recovers() {
    use crate::error::BlockTransferError;
    use core::sync::atomic::{AtomicUsize, Ordering};
    static MODE: AtomicUsize = AtomicUsize::new(0);
    static BODY: [u8; 168] = [b'A'; 168];
    static OTHER: [u8; 168] = [b'B'; 168];
    fn handler(_: Request<'_>) -> Response<'static> {
        match MODE.load(Ordering::SeqCst) {
            0 => Response::content(&BODY).etag(b"changed"),
            1 => Response::content(&OTHER).etag(b"body"),
            _ => Response::content(&BODY).etag(b"body"),
        }
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(&["large"], get(handler))
        .bind(WideLoopback::default())
        .unwrap();
    let key = BlockKey::new(Token::new(&[0xA1]).unwrap(), peer)
        .with_identity(crate::storage::BodyTag::new(b"body").unwrap());
    let id = app.engine.start_q_block2(key, &BODY, 0).unwrap();
    for _ in 0..crate::storage::BlockTransfer::MAX_PAYLOADS {
        app.engine.next_q_block2(id).unwrap();
    }
    let before = app.engine.tx_body_transfer(id).unwrap();
    for mode in 0..3 {
        MODE.store(mode, Ordering::SeqCst);
        let q = BlockValue::from_size(10, true, 16).unwrap().encode();
        let (wire, n) = encode_wide(
            Code::GET,
            &["large"],
            &[Opt::q_block2(&q)],
            0x1300 + mode as u16,
        );
        app.transport_mut().inbox = Some((peer, wire, n));
        if mode < 2 {
            assert_eq!(
                app.poll(mode as u64),
                Err(Error::Block(BlockTransferError::IdentityMismatch))
            );
            assert_eq!(app.engine.tx_body_transfer(id), Some(before));
            assert_eq!(app.engine.tx_body_payload(id), Some(BODY.as_slice()));
            assert_eq!(app.transport().send_n, 0);
        } else {
            app.poll(2).unwrap();
            assert_eq!(app.transport().send_n, 1);
            let reply = decode(&app.transport().sends[0][..app.transport().send_lens[0]]).unwrap();
            assert_eq!(reply.etag().next(), Some(&b"body"[..]));
            assert_eq!(reply.payload(), &BODY[160..]);
            assert_eq!(reply.size2(), Some(Ok(168)));
            assert!(app.engine.tx_body_transfer(id).is_none());
        }
    }
}

#[test]
fn response_option_bounds_are_typed_sticky_and_preserve_exact_values() {
    use crate::ResponseError;
    static LONG: [u8; 256] = [b'x'; 256];
    let long = core::str::from_utf8(&LONG).unwrap();
    for n in [0, 9] {
        assert_eq!(
            Response::content(b"ok").etag(&[1; 9][..n]).validate(),
            Err(ResponseError::EtagLength)
        );
    }
    for n in [1, 8] {
        let response = Response::content(b"ok").etag(&[1; 8][..n]);
        assert_eq!(response.validate(), Ok(()));
        assert_eq!(response.etag_bytes(), Some(&[1; 8][..n]));
    }
    let invalid = Response::content(b"ok")
        .etag(&[1; 9])
        .etag(b"good")
        .location_path(".");
    assert_eq!(invalid.validate(), Err(ResponseError::EtagLength));
    for segment in [".", ".."] {
        assert_eq!(
            Response::created().location_path(segment).validate(),
            Err(ResponseError::LocationDotSegment)
        );
    }
    assert_eq!(
        Response::created().location_path(long).validate(),
        Err(ResponseError::LocationPathBounds)
    );
    assert_eq!(
        Response::created().location_query(long).validate(),
        Err(ResponseError::LocationQueryBounds)
    );
    let mut response = Response::created()
        .location_path(&long[..255])
        .location_query(&long[..255]);
    for _ in 1..super::LOCATION_MAX {
        response = response.location_path("").location_query("");
    }
    assert_eq!(response.validate(), Ok(()));
    assert_eq!(response.location_paths().len(), 8);
    assert_eq!(response.location_queries().len(), 8);
    assert_eq!(response.location_paths()[1..], [""; 7]);
    assert_eq!(response.location_queries()[1..], [""; 7]);
    assert_eq!(
        response.location_path("").validate(),
        Err(ResponseError::LocationPathBounds)
    );
    assert_eq!(
        response.location_query("").validate(),
        Err(ResponseError::LocationQueryBounds)
    );
}

#[test]
fn location_wire_preserves_empty_segments_and_maximum_counts_and_lengths() {
    static LONG: [u8; 255] = [b'x'; 255];
    fn handler(_: Request<'_>) -> Response<'static> {
        let long = core::str::from_utf8(&LONG).unwrap();
        let mut response = Response::created()
            .etag(b"12345678")
            .location_path(long)
            .location_query(long);
        for _ in 1..super::LOCATION_MAX {
            response = response.location_path("").location_query("");
        }
        response
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["test"], &[], 0x1400);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["test"], get(handler))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .unwrap();
    app.poll(0).unwrap();
    let reply = last_wide(&app);
    assert_eq!(reply.etag().next(), Some(&b"12345678"[..]));
    for (i, value) in reply.location_path().enumerate() {
        assert_eq!(
            value.unwrap().as_bytes(),
            if i == 0 { &LONG[..] } else { &[] }
        );
    }
    for (i, value) in reply.location_query().enumerate() {
        assert_eq!(
            value.unwrap().as_bytes(),
            if i == 0 { &LONG[..] } else { &[] }
        );
    }
    assert_eq!(reply.location_path().count(), 8);
    assert_eq!(reply.location_query().count(), 8);
}

#[test]
fn invalid_response_refuses_separate_ack_and_observe_then_recovers() {
    use crate::ResponseError;
    use core::sync::atomic::{AtomicBool, Ordering};
    static VALID: AtomicBool = AtomicBool::new(false);
    fn handler(_: Request<'_>) -> Response<'static> {
        if VALID.load(Ordering::SeqCst) {
            Response::content(b"ok").etag(b"good").observe(0)
        } else {
            Response::content(b"bad").etag(b"too-long-tag").separate()
        }
    }
    VALID.store(false, Ordering::SeqCst);
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let token = Token::new(&[0xA1]).unwrap();
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["test"], get(handler))
        .bind(WideLoopback::default())
        .unwrap();
    // Repeat beyond RX/TX capacity: refusal must release every request.
    for mid in 0..12 {
        let (wire, n) = encode_wide(
            Code::GET,
            &["test"],
            &[Opt::observe_register()],
            0x1500 + mid,
        );
        app.transport_mut().inbox = Some((peer, wire, n));
        assert_eq!(
            app.poll(u64::from(mid)),
            Err(Error::Response(ResponseError::EtagLength))
        );
        assert_eq!(app.transport().send_n, 0);
        assert_eq!(app.engine.tx_occupied(), 0);
        assert!(!observe_live(&app, peer, token));
    }
    VALID.store(true, Ordering::SeqCst);
    let (wire, n) = encode_wide(Code::GET, &["test"], &[Opt::observe_register()], 0x1600);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(12).unwrap();
    assert!(observe_live(&app, peer, token));
    let id = app
        .engine
        .lookup_observe(ObserveKey::new(token, peer))
        .unwrap();
    let before = app.engine.observe_interest(id).unwrap();
    assert_eq!(
        app.notify(13, &["test"], Response::content(b"bad").location_path("..")),
        Err(Error::Response(ResponseError::LocationDotSegment))
    );
    assert_eq!(app.engine.observe_interest(id), Some(before));
    assert_eq!(app.transport().send_n, 1);
    assert_eq!(
        app.notify(14, &["test"], Response::content(b"good").etag(b"new")),
        Ok(1)
    );
}

#[test]
fn invalid_handler_response_releases_assembled_upload_body() {
    use crate::ResponseError;
    use crate::storage::SlotId;
    fn handler(req: Request<'_>) -> Response<'static> {
        assert_eq!(req.body(), Some(&b"body"[..]));
        Response::changed().location_query(core::str::from_utf8(&LONG).unwrap())
    }
    static LONG: [u8; 256] = [b'x'; 256];
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route(LED_PATH, put(handler))
        .bind(Loopback {
            inbox: None,
            last_send: None,
        })
        .unwrap();
    for mid in 0..12 {
        let (wire, n) = encode_req_block1(Code::PUT, LED_PATH, b"body", 0, false, 16, 0x1700 + mid);
        app.transport_mut().inbox = Some((peer, wire, n));
        assert_eq!(
            app.poll(u64::from(mid)),
            Err(Error::Response(ResponseError::LocationQueryBounds))
        );
        assert!(app.transport().last_send.is_none());
        for index in 0..app.engine.capacities().rx_body_slots.unwrap() {
            assert!(
                app.engine
                    .rx_body_transfer(SlotId::from_index(index))
                    .is_none()
            );
        }
    }
}

#[test]
fn client_snapshot_retains_location_max_age_echo_and_unknown_options() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(WideLoopback::default())
        .unwrap();
    let call = app.post("create").to(peer).send(0).unwrap();
    let mid = decode(&app.transport().sends[0][..app.transport().send_lens[0]])
        .unwrap()
        .message_id();
    let max_age = encode_uint(0);
    let options = [
        Opt::etag(b"version"),
        Opt::location_path(""),
        Opt::location_path("a/b"),
        Opt::location_path(""),
        Opt::max_age(&max_age),
        Opt::location_query(""),
        Opt::location_query("x=é&y"),
        Opt::echo(b"challenge"),
        Opt::new(OptionNumber::new(2048), b"opaque"),
    ];
    let msg = Message::new(Type::Acknowledgement, Code::CREATED, mid)
        .with_token(call.token())
        .with_options(&options)
        .with_payload(b"created");
    let mut wire = [0; WIRE];
    let n = encode(&msg, &mut wire).unwrap();
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).unwrap();
    let response = app.take_response(call).unwrap().unwrap();
    assert_eq!(response.location_paths(), &["", "a/b", ""]);
    assert_eq!(response.location_queries(), &["", "x=é&y"]);
    assert_eq!(response.max_age_secs(), Some(0));
    assert_eq!(
        response.echo_option(),
        Some(EchoOpt::new(b"challenge").unwrap())
    );
    assert_eq!(response.etag_bytes(), Some(&b"version"[..]));
    assert_eq!(response.payload(), b"created");
    assert!(response.received_options().eq(options));
    assert_eq!(response.validate(), Ok(()));
}

#[test]
fn client_metadata_byte_count_and_location_bounds_refuse_without_partial_reply() {
    for mode in 0..8 {
        let peer = Endpoint::v4([192, 0, 2, 2], 5683);
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .bind(WideLoopback::default())
            .unwrap();
        let call = app.get("value").to(peer).send(0).unwrap();
        let request = decode(&app.transport().sends[0][..app.transport().send_lens[0]]).unwrap();
        let mut options = OptionsBuilder::<25>::new();
        let bytes = [0x5a; 512];
        match mode {
            0 | 1 => {
                // Exact retained header bound, followed by one byte over it.
                // Header + actual Token + option delta/length extension bytes.
                let overhead = 4 + call.token().len() + 5;
                let len = super::RESPONSE_OPTION_BYTES - overhead + mode;
                options
                    .push(Opt::new(OptionNumber::new(2048), &bytes[..len]))
                    .unwrap();
            }
            2 | 3 => {
                for index in 0..(super::RESPONSE_OPTION_COUNT + mode - 2) {
                    options
                        .push(Opt::new(OptionNumber::new(2048 + 2 * index as u16), &[]))
                        .unwrap();
                }
            }
            _ => {
                for _ in 0..(super::LOCATION_MAX + mode % 2) {
                    options
                        .push(if mode < 6 {
                            Opt::location_path("")
                        } else {
                            Opt::location_query("")
                        })
                        .unwrap();
                }
            }
        }
        let msg = Message::new(Type::Acknowledgement, Code::CONTENT, request.message_id())
            .with_token(call.token())
            .with_options(options.as_slice());
        let mut wire = [0; WIRE];
        let n = encode(&msg, &mut wire).unwrap();
        if mode <= 1 {
            assert_eq!(n, super::RESPONSE_OPTION_BYTES + mode);
        }
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(1).unwrap();
        let result = app.take_response(call).unwrap();
        if mode % 2 == 0 {
            let response = result.unwrap();
            assert!(
                response
                    .received_options()
                    .eq(options.as_slice().iter().copied())
            );
        } else {
            assert_eq!(
                result.unwrap_err(),
                crate::CallFailure::ResponseMetadataBounds
            );
        }
        assert_eq!(app.engine_mut().rx_occupied(), 0);
        assert_eq!(app.engine_mut().tx_occupied(), 0);
        assert!(app.get("next").to(peer).send(2).is_ok());
    }
}

#[test]
fn block2_retains_first_fragment_metadata_and_never_exposes_partial_reply() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    let call = app.get("value").to(peer).observe().send(0).unwrap();
    for num in 0..2 {
        let index = app.transport().send_n - 1;
        let mid = decode(&app.transport().sends[index][..app.transport().send_lens[index]])
            .unwrap()
            .message_id();
        let block = BlockValue::from_size(num, num == 0, 16).unwrap().encode();
        let mut options = OptionsBuilder::<8>::new();
        options.push(Opt::etag(b"body")).unwrap();
        if num == 0 {
            options.push(Opt::observe_register()).unwrap();
            options.push(Opt::location_path("first")).unwrap();
            options
                .push(Opt::new(OptionNumber::CONTENT_FORMAT, &[0]))
                .unwrap();
            options.push(Opt::new(OptionNumber::MAX_AGE, &[5])).unwrap();
        }
        options.push(Opt::block2(&block)).unwrap();
        let msg = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
            .with_token(call.token())
            .with_options(options.as_slice())
            .with_payload(if num == 0 {
                b"abcdefghijklmnop"
            } else {
                b"qrstuvwx"
            });
        let mut wire = [0; WIRE];
        let n = encode(&msg, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(u64::from(num) + 1).unwrap();
        if num == 0 {
            assert!(app.take_response(call).is_none());
        }
    }
    let response = app.take_response(call).unwrap().unwrap();
    assert_eq!(response.body(), Some(&b"abcdefghijklmnopqrstuvwx"[..]));
    assert_eq!(response.location_paths(), &["first"]);
    assert_eq!(response.max_age_secs(), Some(5));
    assert_eq!(response.received_at_ms(), Some(1));
    assert_eq!(response.observe_seq(), Some(0));
    assert_eq!(response.format(), Some(ContentFormat::TEXT_PLAIN));
    assert!(
        app.engine
            .lookup_observe(ObserveKey::new_client(call.token(), peer))
            .is_some()
    );
    assert!(app.cancel(call));
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::Cancelled
    );
}

#[test]
fn four_untaken_response_metadata_snapshots_remain_distinct_and_bounded() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let names = ["one", "two", "three", "four"];
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(WideLoopback::default())
        .unwrap();
    let mut calls = [None; 4];
    for (index, name) in names.iter().enumerate() {
        let call = app.get("value").to(peer).non().send(0).unwrap();
        calls[index] = Some(call);
        let options = [Opt::location_path(name)];
        let msg = Message::new(
            Type::NonConfirmable,
            Code::CONTENT,
            MessageId::new(100 + index as u16),
        )
        .with_token(call.token())
        .with_options(&options);
        let mut wire = [0; WIRE];
        let n = encode(&msg, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(1).unwrap();
    }
    assert_eq!(app.get("overflow").to(peer).send(2), Err(Error::Saturated));
    for index in [2, 0, 3, 1] {
        let response = app.take_response(calls[index].unwrap()).unwrap().unwrap();
        assert_eq!(response.location_paths(), &[names[index]]);
        assert_eq!(response.received_options().count(), 1);
    }
    app.transport_mut().send_n = 0;
    assert!(app.get("next").to(peer).send(3).is_ok());
}

#[test]
fn block2_metadata_overflow_reclaims_partial_body_and_call() {
    use crate::storage::SlotId;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    let call = app.get("value").to(peer).send(0).unwrap();
    for num in 0..2 {
        let index = app.transport().send_n - 1;
        let mid = decode(&app.transport().sends[index][..app.transport().send_lens[index]])
            .unwrap()
            .message_id();
        let block = BlockValue::from_size(num, num == 0, 16).unwrap().encode();
        let mut options = OptionsBuilder::<3>::new();
        options.push(Opt::etag(b"body")).unwrap();
        options.push(Opt::block2(&block)).unwrap();
        let huge = [1; 600];
        if num == 1 {
            options
                .push(Opt::new(OptionNumber::new(2048), &huge))
                .unwrap();
        }
        let msg = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
            .with_token(call.token())
            .with_options(options.as_slice())
            .with_payload(if num == 0 {
                b"abcdefghijklmnop"
            } else {
                b"qrstuvwx"
            });
        let mut wire = [0; WIRE];
        let n = encode(&msg, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(u64::from(num) + 1).unwrap();
        if num == 0 {
            assert!(app.take_response(call).is_none());
        }
    }
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::ResponseMetadataBounds
    );
    for index in 0..app.engine.capacities().rx_body_slots.unwrap() {
        assert!(
            app.engine
                .rx_body_transfer(SlotId::from_index(index))
                .is_none()
        );
    }
    assert_eq!(app.engine.tx_occupied(), 0);
    assert!(app.get("next").to(peer).send(3).is_ok());
}

#[test]
fn malformed_elective_response_values_remain_raw_without_creating_observe() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(WideLoopback::default())
        .unwrap();
    let call = app.get("value").to(peer).observe().send(0).unwrap();
    let mid = decode(&app.transport().sends[0][..app.transport().send_lens[0]])
        .unwrap()
        .message_id();
    let options = [
        Opt::etag(&[]),
        Opt::new(OptionNumber::OBSERVE, &[1; 4]),
        Opt::new(OptionNumber::LOCATION_PATH, &[0xff]),
        Opt::location_path(".."),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[1; 3]),
        Opt::new(OptionNumber::MAX_AGE, &[1; 5]),
        Opt::echo(&[]),
    ];
    let msg = Message::new(Type::Acknowledgement, Code::CONTENT, mid)
        .with_token(call.token())
        .with_options(&options)
        .with_payload(b"ok");
    let mut wire = [0; WIRE];
    let n = encode(&msg, &mut wire).unwrap();
    app.transport_mut().inbox = Some((peer, wire, n));
    app.poll(1).unwrap();
    let response = app.take_response(call).unwrap().unwrap();
    assert_eq!(response.etag_bytes(), None);
    assert_eq!(response.format(), None);
    assert_eq!(response.observe_seq(), None);
    assert_eq!(response.max_age_secs(), None);
    assert_eq!(response.echo_option(), None);
    assert!(response.location_paths().is_empty());
    assert!(response.received_options().eq(options));
    assert!(!observe_live(&app, peer, call.token()));
}

#[test]
fn explicit_echo_retry_uses_received_challenge_and_preserves_request() {
    fn handler(req: Request<'_>) -> Response<'static> {
        assert_eq!(req.payload(), b"set-value");
        assert_eq!(req.uri_query().next(), Some(Ok("version=1")));
        if req.echo() == Some(&b"opaque-challenge"[..]) {
            Response::changed()
        } else {
            Response::new(Code::UNAUTHORIZED).echo(EchoOpt::new(b"opaque-challenge").unwrap())
        }
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .route("value", put(handler))
        .bind(Pipe::default())
        .unwrap();
    let first = app
        .put("value")
        .to(peer)
        .query("version=1")
        .payload(b"set-value")
        .send(0)
        .unwrap();
    app.poll(0).unwrap();
    app.poll(1).unwrap();
    let challenge = {
        let response = app.take_response(first).unwrap().unwrap();
        assert_eq!(response.code(), Code::UNAUTHORIZED);
        assert_eq!(response.peer(), Some(peer));
        response.echo_option().unwrap()
    };
    assert_eq!(app.transport().len, 0, "retry is explicit");
    let retry = app
        .put("value")
        .to(peer)
        .query("version=1")
        .payload(b"set-value")
        .echo(challenge)
        .send(2)
        .unwrap();
    assert_ne!(retry.token(), first.token());
    app.poll(2).unwrap();
    app.poll(3).unwrap();
    assert_eq!(
        app.take_response(retry).unwrap().unwrap().code(),
        Code::CHANGED
    );
}

#[test]
fn client_no_response_preserves_bitmap_and_never_reports_silence_as_success() {
    use crate::message::NoResponse;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for non in [false, true] {
        for (mask, found, expected) in [
            (0, true, Some(Code::CHANGED)),
            (2, true, None),
            (2, false, Some(Code::NOT_FOUND)),
            (26, false, None),
        ] {
            let mut app = App::profile::<profiles::Default>()
                .deterministic_for_tests()
                .block_wise::<true>()
                .route("value", put(|_: Request<'_>| Response::changed()))
                .bind(Pipe::default())
                .unwrap();
            let mut request = app
                .put(if found { "value" } else { "missing" })
                .to(peer)
                .payload(b"update")
                .no_response(NoResponse::new(mask))
                .deadline(20);
            if non {
                request = request.non();
            }
            let call = request.send(0).unwrap();
            let (_, wire, n) = app.transport().slots[app.transport().head].unwrap();
            let parsed = decode(&wire[..n]).unwrap();
            assert_eq!(parsed.no_response(), Some(Ok(NoResponse::new(mask))));
            if mask == 0 {
                assert_eq!(
                    parsed
                        .get_option(OptionNumber::NO_RESPONSE)
                        .unwrap()
                        .value(),
                    &[]
                );
            }
            app.poll(0).unwrap();
            app.poll(1).unwrap();
            if let Some(code) = expected {
                assert_eq!(app.take_response(call).unwrap().unwrap().code(), code);
            } else {
                assert!(app.take_response(call).is_none());
                app.poll(20).unwrap();
                assert_eq!(
                    app.take_response(call).unwrap().unwrap_err(),
                    crate::CallFailure::DeadlineExceeded
                );
            }
            assert_eq!(app.engine.tx_occupied(), 0);
        }
    }
}

#[test]
fn no_response_unsupported_upload_and_observe_refuse_before_io() {
    use crate::message::NoResponse;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    assert_eq!(
        app.put("upload")
            .to(peer)
            .payload(&LARGE)
            .no_response(NoResponse::DEFAULT)
            .send(0),
        Err(Error::NoResponseUploadUnsupported)
    );
    assert_eq!(
        app.put("upload")
            .to(peer)
            .payload(b"small")
            .q_block1()
            .no_response(NoResponse::new(2))
            .send(0),
        Err(Error::NoResponseUploadUnsupported)
    );
    assert_eq!(
        app.get("value")
            .to(peer)
            .observe()
            .no_response(NoResponse::new(2))
            .send(0),
        Err(Error::NoResponseObserveUnsupported)
    );
    assert_eq!(app.transport().len, 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    assert_eq!(
        app.get("value")
            .to(peer)
            .deregister()
            .no_response(NoResponse::new(NoResponse::SUPPRESS_ALL))
            .send(1),
        Err(Error::ObserveCancellationMismatch)
    );
    assert_eq!(app.transport().len, 0);
}

#[test]
fn failed_upload_send_retires_partial_window_and_preserves_other_call() {
    use crate::storage::{BodyTag, SlotId};
    struct FaultIo {
        sends: usize,
        fail_at: usize,
        short: bool,
    }
    impl DatagramIo for FaultIo {
        type Error = &'static str;
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            self.sends += 1;
            if self.sends == self.fail_at {
                if self.short {
                    Ok(bytes.len() - 1)
                } else {
                    Err("injected upload failure")
                }
            } else {
                Ok(bytes.len())
            }
        }
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let other = Endpoint::v4([192, 0, 2, 3], 5683);
    for qblock in [false, true] {
        for confirmable in [false, true] {
            for short in [false, true] {
                for fail_at in 1..=if qblock { 3 } else { 1 } {
                    let mut app = App::profile::<profiles::Default>()
                        .deterministic_for_tests()
                        .block_wise::<true>()
                        .bind(FaultIo {
                            sends: 0,
                            fail_at: usize::MAX,
                            short,
                        })
                        .unwrap();
                    let other_call = app.get("other").to(other).send(0).unwrap();
                    for iteration in 0..12 {
                        app.transport_mut().sends = 0;
                        app.transport_mut().fail_at = fail_at;
                        let request = app.put("value").to(peer).payload(&[7; 3000]);
                        let request = if qblock {
                            request
                                .q_block1()
                                .request_tag(BodyTag::new(b"upload").unwrap())
                        } else {
                            request
                        };
                        let request = if confirmable { request } else { request.non() };
                        assert!(request.send(iteration).is_err());
                        assert_eq!(app.transport().sends, fail_at);
                        assert_eq!(
                            app.engine.tx_occupied(),
                            1,
                            "only the unrelated CON may remain"
                        );
                        for index in 0..app.engine.capacities().tx_body_slots.unwrap() {
                            assert!(
                                app.engine
                                    .tx_body_transfer(SlotId::from_index(index))
                                    .is_none()
                            );
                        }
                        assert!(app.take_response(other_call).is_none());
                    }
                    app.transport_mut().fail_at = usize::MAX;
                    let call = app.get("value").to(peer).send(12).unwrap();
                    assert!(app.cancel(call));
                    assert!(app.cancel(other_call));
                    assert_eq!(app.engine.tx_occupied(), 0);
                }
            }
        }
    }
}

#[test]
fn client_observe_serial_order_wrap_and_time_boundary() {
    use crate::message::encode_uint;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let cases = [
        (10, 9, 1, false),
        (10, 10, 1, false),
        (10, 11, 1, true),
        (0xfffffe, 1, 1, true),
        (1, 0xfffffe, 1, false),
        (10, 0x80000a, 1, false),
        (0x80000a, 10, 1, false),
        (10, 9, 128_000, false),
        (10, 9, 128_001, true),
        (10, 10, 128_001, true),
    ];
    for (previous, next, time, accepted) in cases {
        for ty in [Type::Confirmable, Type::NonConfirmable] {
            for take_initial in [false, true] {
                let mut app = record_client();
                let call = app.get("value").observe().to(peer).non().send(0).unwrap();
                for (index, sequence, now, payload) in
                    [(0, previous, 0, b"old"), (1, next, time, b"new")]
                {
                    let seq = encode_uint(sequence);
                    let age = encode_uint(1000);
                    let opts = [Opt::observe(&seq), Opt::max_age(&age)];
                    let message = Message::new(ty, Code::CONTENT, MessageId::new(100 + index))
                        .with_token(call.token())
                        .with_options(&opts)
                        .with_payload(payload);
                    let mut wire = [0; 256];
                    let n = encode(&message, &mut wire).unwrap();
                    app.transport_mut().inbox = Some((peer, wire, n));
                    app.transport_mut().sent_n = 0;
                    app.poll(now).unwrap();
                    assert_eq!(app.transport().sent_n, usize::from(ty == Type::Confirmable));
                    if ty == Type::Confirmable {
                        let (_, bytes, n) = app.transport().sent[0].unwrap();
                        let ack = decode(&bytes[..n]).unwrap();
                        assert!(ack.is_empty());
                        assert_eq!(ack.ty(), Type::Acknowledgement);
                        assert_eq!(ack.message_id(), message.message_id());
                    }
                    if index == 0 && take_initial {
                        assert_eq!(app.take_response(call).unwrap().unwrap().payload(), b"old");
                    }
                }
                let response = app.take_response(call);
                if accepted {
                    let response = response.unwrap().unwrap();
                    assert_eq!(response.payload(), b"new");
                    assert_eq!(response.observe_seq(), Some(next));
                    assert_eq!(response.received_at_ms(), Some(time));
                } else if take_initial {
                    assert!(response.is_none());
                } else {
                    assert_eq!(response.unwrap().unwrap().payload(), b"old");
                }
                let interest = app
                    .engine
                    .lookup_observe(ObserveKey::new_client(call.token(), peer))
                    .unwrap();
                let lifetime = app
                    .engine
                    .observe_interest(interest)
                    .unwrap()
                    .lifetime()
                    .unwrap();
                assert_eq!(
                    lifetime.due_ms(),
                    1_000_000 + if accepted { time } else { 0 }
                );
                assert_eq!(app.engine.tx_occupied(), 0);
            }
        }
    }
}

#[test]
fn stale_observe_block_zero_cannot_replace_incomplete_representation() {
    use crate::message::encode_uint;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(RecordIo::default())
        .unwrap();
    let call = app.get("value").observe().to(peer).non().send(0).unwrap();
    for (step, sequence, number, tag, payload) in [
        (0, Some(10), 0, b"a", &b"abcdefghijklmnop"[..]),
        (1, Some(9), 0, b"b", &b"XXXXXXXXXXXXXXXX"[..]),
        (2, None, 1, b"a", &b"qrstuvwx"[..]),
    ] {
        let seq = sequence.map(encode_uint);
        let age = encode_uint(1000);
        let block = BlockValue::from_size(number, number == 0, 16)
            .unwrap()
            .encode();
        let mut opts = OptionsBuilder::<4>::new();
        opts.push(Opt::etag(tag)).unwrap();
        if let Some(seq) = &seq {
            opts.push(Opt::observe(seq)).unwrap();
        }
        opts.push(Opt::max_age(&age)).unwrap();
        opts.push(Opt::block2(&block)).unwrap();
        let message = Message::new(
            Type::NonConfirmable,
            Code::CONTENT,
            MessageId::new(100 + step),
        )
        .with_token(call.token())
        .with_options(opts.as_slice())
        .with_payload(payload);
        let mut wire = [0; 256];
        let n = encode(&message, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        let sends = app.transport().sent_n;
        app.poll(u64::from(step)).unwrap();
        if step < 2 {
            assert!(app.take_response(call).is_none());
        }
        if step == 1 {
            assert_eq!(
                app.transport().sent_n,
                sends,
                "stale notification must not launch a replacement download"
            );
        }
    }
    let response = app.take_response(call).unwrap().unwrap();
    assert_eq!(response.body(), Some(&b"abcdefghijklmnopqrstuvwx"[..]));
    assert_eq!(response.observe_seq(), Some(10));
    assert_eq!(response.received_at_ms(), Some(0));
    assert_eq!(app.engine.tx_occupied(), 0);
}

#[test]
fn client_observe_survives_stale_data_then_ends_on_final_response() {
    use crate::message::encode_uint;
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    for iteration in 0..12 {
        app.transport_mut().sent_n = 0;
        let call = app.get("value").observe().to(peer).non().send(0).unwrap();
        for (sequence, now) in [(0, 0), (1, 2)] {
            let seq = encode_uint(sequence);
            let zero = encode_uint(0);
            let opts = [Opt::observe(&seq), Opt::max_age(&zero)];
            let message = Message::new(
                Type::NonConfirmable,
                Code::CONTENT,
                MessageId::new(100 + sequence as u16),
            )
            .with_token(call.token())
            .with_options(&opts)
            .with_payload(b"value");
            let mut wire = [0; 256];
            let n = encode(&message, &mut wire).unwrap();
            app.transport_mut().inbox = Some((peer, wire, n));
            app.poll(now).unwrap();
            assert_eq!(
                app.take_response(call).unwrap().unwrap().observe_seq(),
                Some(sequence)
            );
            app.poll(now + 1).unwrap();
            assert!(
                app.engine
                    .lookup_observe(ObserveKey::new_client(call.token(), peer))
                    .is_some()
            );
            assert!(app.take_response(call).is_none());
        }
        let code = if iteration % 2 == 0 {
            Code::NOT_FOUND
        } else {
            Code::CONTENT
        };
        let final_message = Message::new(Type::Confirmable, code, MessageId::new(200))
            .with_token(call.token())
            .with_payload(b"final");
        inject_empty(&mut app, peer, final_message);
        app.poll(4).unwrap();
        assert_eq!(app.take_response(call).unwrap().unwrap().code(), code);
        assert!(
            app.engine
                .lookup_observe(ObserveKey::new_client(call.token(), peer))
                .is_none()
        );
        app.transport_mut().sent_n = 0;
        inject_empty(&mut app, peer, final_message);
        app.poll(5).unwrap();
        let (_, wire, n) = app.transport().sent[0].unwrap();
        let reset = decode(&wire[..n]).unwrap();
        assert!(reset.is_empty_rst());
        assert_eq!(reset.message_id(), final_message.message_id());
        assert!(app.take_response(call).is_none());
        assert_eq!(app.engine.tx_occupied(), 0);
    }
}

#[test]
fn unsolicited_observe_option_does_not_create_subscription() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    for _ in 0..12 {
        app.transport_mut().sent_n = 0;
        let call = app.get("value").to(peer).non().send(0).unwrap();
        let opts = [Opt::observe_register()];
        let response = Message::new(Type::NonConfirmable, Code::CONTENT, MessageId::new(100))
            .with_token(call.token())
            .with_options(&opts);
        inject_empty(&mut app, peer, response);
        app.poll(0).unwrap();
        assert_eq!(
            app.take_response(call).unwrap().unwrap().code(),
            Code::CONTENT
        );
        assert!(
            app.engine
                .lookup_observe(ObserveKey::new_client(call.token(), peer))
                .is_none()
        );
    }
}

#[test]
fn observe_max_age_zero_keeps_congestion_hold_and_con_retries() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    for acknowledge in [false, true] {
        let (wire, n) = encode_wide(
            Code::GET,
            &["sensors", "temp"],
            &[Opt::observe_register()],
            0x1001,
        );
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .route(&["sensors", "temp"], get(get_obs))
            .bind(WideLoopback {
                inbox: Some((peer, wire, n)),
                ..WideLoopback::default()
            })
            .unwrap();
        app.poll(0).unwrap();
        assert_eq!(
            app.notify(
                0,
                &["sensors", "temp"],
                Response::content(b"non")
                    .content_format(ContentFormat::TEXT_PLAIN)
                    .max_age(0)
            )
            .unwrap(),
            1
        );
        app.poll(1).unwrap();
        assert!(observe_registered(&app, peer));
        assert_eq!(
            app.notify(
                1,
                &["sensors", "temp"],
                Response::content(b"held").content_format(ContentFormat::TEXT_PLAIN)
            )
            .unwrap(),
            0
        );
        let now = ObserveTransmission::CONFIRM_INTERVAL_MS;
        app.transport_mut().send_n = 0;
        assert_eq!(
            app.notify(
                now,
                &["sensors", "temp"],
                Response::content(b"con")
                    .content_format(ContentFormat::TEXT_PLAIN)
                    .max_age(0)
            )
            .unwrap(),
            1
        );
        let mid = last_wide(&app).message_id();
        assert_eq!(last_wide(&app).ty(), Type::Confirmable);
        app.poll(now).unwrap();
        assert!(observe_registered(&app, peer));
        assert_eq!(app.engine.tx_occupied(), 1);
        if acknowledge {
            let mut wire = [0; WIRE];
            let n = encode(&Message::empty_ack(mid), &mut wire).unwrap();
            app.transport_mut().inbox = Some((peer, wire, n));
            app.poll(now + 1).unwrap();
            assert_eq!(app.engine.tx_occupied(), 0);
            app.transport_mut().send_n = 0;
            app.poll(now + 62_000).unwrap();
            assert!(observe_registered(&app, peer));
            assert_eq!(app.transport().send_n, 0);
        } else {
            for delta in [2_000, 6_000, 14_000, 30_000, 62_000] {
                app.transport_mut().send_n = 0;
                app.poll(now + delta).unwrap();
                assert_eq!(observe_registered(&app, peer), delta < 62_000);
                assert_eq!(app.transport().send_n, usize::from(delta < 62_000));
            }
            assert_eq!(app.engine.tx_occupied(), 0);
        }
    }
}

#[test]
fn observe_signal_at_max_age_boundary_is_not_discarded() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(
        Code::GET,
        &["sensors", "temp"],
        &[Opt::observe_register()],
        0x1001,
    );
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs).observe(obs_snapshot))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .unwrap();
    app.poll(0).unwrap();
    app.transport_mut().send_n = 0;
    assert_eq!(app.signal(&["sensors", "temp"]), 1);
    app.poll(5_000).unwrap();
    assert_eq!(app.transport().send_n, 1);
    assert!(observe_registered(&app, peer));
    assert_eq!(last_wide(&app).observe().and_then(Result::ok), Some(1));
}

#[test]
fn observe_notification_format_and_terminal_response_contract() {
    fn plain(_: Request<'_>) -> Response<'static> {
        Response::content(b"initial")
            .content_format(ContentFormat::TEXT_PLAIN)
            .observe(0)
    }
    fn absent(_: Request<'_>) -> Response<'static> {
        Response::content(b"initial").observe(0)
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    for initial_format in [None, Some(ContentFormat::TEXT_PLAIN)] {
        for next_format in [
            None,
            Some(ContentFormat::TEXT_PLAIN),
            Some(ContentFormat::OCTET_STREAM),
        ] {
            for code in [Code::CONTENT, Code::NOT_FOUND, Code::SERVICE_UNAVAILABLE] {
                let handler = if initial_format.is_some() {
                    plain
                } else {
                    absent
                };
                let (wire, n) =
                    encode_wide(Code::GET, &["obs"], &[Opt::observe_register()], 0x1001);
                let mut app = App::profile::<profiles::Default>()
                    .deterministic_for_tests()
                    .block_wise::<false>()
                    .route("obs", get(handler))
                    .bind(WideLoopback {
                        inbox: Some((peer, wire, n)),
                        ..WideLoopback::default()
                    })
                    .unwrap();
                app.poll(0).unwrap();
                let token = last_wide(&app).token();
                assert_eq!(last_wide(&app).observe().and_then(Result::ok), Some(0));
                let mut response = Response::new(code).observe(99).payload_copy(b"next");
                if let Some(format) = next_format {
                    response = response.content_format(format);
                }
                assert_eq!(app.notify(1, &["obs"], response).unwrap(), 1);
                let note = last_wide(&app);
                let mismatch = code.is_success() && initial_format != next_format;
                assert_eq!(
                    note.code(),
                    if mismatch { Code::NOT_ACCEPTABLE } else { code }
                );
                assert_eq!(note.token(), token);
                if code.is_success() && !mismatch {
                    assert!(note.observe().is_some());
                    assert_eq!(note.content_format().and_then(Result::ok), initial_format);
                    assert_eq!(note.payload(), b"next");
                    assert!(
                        app.engine
                            .lookup_observe(ObserveKey::new(token, peer))
                            .is_some()
                    );
                } else {
                    assert!(note.observe().is_none());
                    assert!(
                        app.engine
                            .lookup_observe(ObserveKey::new(token, peer))
                            .is_none()
                    );
                    assert_eq!(app.notify(3_001, &["obs"], response).unwrap(), 0);
                }
            }
        }
    }
}

#[test]
fn observe_deregistration_strips_handler_observe_and_releases_relation() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback::default())
        .unwrap();
    for (index, option) in [Opt::observe_register(), Opt::observe_deregister()]
        .into_iter()
        .enumerate()
    {
        let (wire, n) = encode_wide(
            Code::GET,
            &["sensors", "temp"],
            &[option],
            0x1001 + index as u16,
        );
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(index as u64).unwrap();
        assert_eq!(last_wide(&app).observe().is_some(), index == 0);
        assert_eq!(observe_registered(&app, peer), index == 0);
    }
}

#[cfg(feature = "oscore")]
fn fixture_echo_mac(
    peer: Endpoint,
    protected: bool,
    issued: u64,
) -> hkdf::hmac::Hmac<sha2::Sha256> {
    use hkdf::hmac::{KeyInit, Mac};
    extern crate std;
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    let key = KEY.get_or_init(|| {
        let mut key = [0; 32];
        getrandom::fill(&mut key).expect("test entropy");
        key
    });
    let mut mac = hkdf::hmac::Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    mac.update(b"coaptic-echo-fixture-v1");
    mac.update(&issued.to_be_bytes());
    match peer {
        Endpoint::V4(address, port) => {
            mac.update(&[4]);
            mac.update(&address);
            mac.update(&port.to_be_bytes());
        }
        Endpoint::V6(address, port, scope) => {
            mac.update(&[6]);
            mac.update(&address);
            mac.update(&port.to_be_bytes());
            mac.update(&scope.to_be_bytes());
        }
    }
    mac.update(&[u8::from(protected)]);
    mac
}

#[cfg(feature = "oscore")]
fn fixture_issued_echo(peer: Endpoint, protected: bool, now: u64) -> EchoOpt {
    use hkdf::hmac::Mac;
    let tag = fixture_echo_mac(peer, protected, now)
        .finalize()
        .into_bytes();
    EchoOpt::mint(now, &tag).unwrap()
}

#[cfg(feature = "oscore")]
fn fixture_authenticated_echo_policy(check: super::EchoCheck) -> super::EchoDecision {
    use hkdf::hmac::Mac;
    let verified = check.echo.ok().flatten().is_some_and(|echo| {
        echo.as_slice().len() == 40
            && echo.is_time_fresh(check.now_ms, ECHO_FRESH_MS)
            && fixture_echo_mac(
                check.peer,
                check.oscore_protected,
                echo.issued_at().unwrap(),
            )
            .verify_slice(&echo.as_slice()[8..])
            .is_ok()
    });
    if verified {
        super::EchoDecision::Accept
    } else {
        super::EchoDecision::Challenge(fixture_issued_echo(
            check.peer,
            check.oscore_protected,
            check.now_ms,
        ))
    }
}

#[cfg(feature = "oscore")]
#[test]
fn explicit_echo_policy_checks_issuance_peer_scope_class_and_expiry_before_handler() {
    static EFFECTS: AtomicUsize = AtomicUsize::new(0);
    fn effect(_: Request<'_>) -> Response<'static> {
        EFFECTS.fetch_add(1, Ordering::SeqCst);
        Response::changed()
    }
    let peer = Endpoint::v6_scoped([1; 16], 5683, 4);
    let issued = fixture_issued_echo(peer, false, 9);
    let mut altered = [0; 40];
    altered.copy_from_slice(issued.as_slice());
    altered[39] ^= 1;
    for (echo, sender, now, accepted) in [
        (issued, peer, 9, true),
        (issued, peer, 13, true),
        (issued, peer, 14, false),
        (fixture_issued_echo(peer, false, 11), peer, 10, false),
        (EchoOpt::mint(9, &[]).unwrap(), peer, 10, false),
        (EchoOpt::new(&altered).unwrap(), peer, 10, false),
        (issued, Endpoint::v6_scoped([1; 16], 5683, 5), 10, false),
        (issued, Endpoint::v6_scoped([1; 16], 5684, 4), 10, false),
        (issued, Endpoint::v6_scoped([2; 16], 5683, 4), 10, false),
        (fixture_issued_echo(peer, true, 9), peer, 10, false),
    ] {
        let before = EFFECTS.load(Ordering::SeqCst);
        let (wire, n) = encode_req_with_echo(Code::PUT, &["effect"], b"one", &echo);
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .echo_policy(fixture_authenticated_echo_policy)
            .route("effect", put(effect))
            .bind(Loopback {
                inbox: Some((sender, wire, n)),
                last_send: None,
            })
            .unwrap();
        app.poll(now).unwrap();
        assert_eq!(
            last_reply(&app).code,
            if accepted {
                Code::CHANGED
            } else {
                Code::UNAUTHORIZED
            }
        );
        assert_eq!(
            EFFECTS.load(Ordering::SeqCst) - before,
            usize::from(accepted)
        );
        assert_eq!(app.engine.tx_occupied(), 0);
    }
}

#[test]
fn echo_policy_issuance_failure_is_closed_and_reclaims_rx() {
    fn deny(check: super::EchoCheck) -> super::EchoDecision {
        assert_eq!(check.echo, Ok(None));
        assert!(!check.oscore_protected);
        super::EchoDecision::Reject
    }
    fn unreachable(_: Request<'_>) -> Response<'static> {
        panic!("rejected request reached handler")
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .echo_policy(deny)
        .block_wise::<false>()
        .route("effect", put(unreachable))
        .bind(Loopback::default())
        .unwrap();
    for mid in 1..=12 {
        let (wire, n) = encode_req_mid(Code::PUT, &["effect"], b"one", mid);
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(u64::from(mid)).unwrap();
        let response = last_reply(&app);
        assert_eq!(response.code, Code::UNAUTHORIZED);
        assert!(response.echo.is_none());
        assert_eq!(app.engine.tx_occupied(), 0);
    }
}

#[test]
fn echo_policy_receives_malformed_values_without_body_or_handler_effects() {
    fn policy(check: super::EchoCheck) -> super::EchoDecision {
        assert_eq!(check.echo, Err(crate::error::ValueError::EchoLength));
        super::EchoDecision::Reject
    }
    fn unreachable(_: Request<'_>) -> Response<'static> {
        panic!("invalid Echo reached handler")
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .echo_policy(policy)
        .route("effect", put(unreachable))
        .bind(RecordIo::default())
        .unwrap();
    for value in [&[][..], &[0; 41][..]] {
        let block = BlockValue::from_size(0, true, 16).unwrap().encode();
        let opts = [
            Opt::uri_path("effect"),
            Opt::block1(&block),
            Opt::new(OptionNumber::ECHO, value),
        ];
        let message = Message::new(Type::NonConfirmable, Code::PUT, MessageId::new(100))
            .with_token(Token::from_checked(&[1]))
            .with_options(&opts)
            .with_payload(b"abcdefghijklmnop");
        let mut wire = [0; 256];
        let n = encode(&message, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(0).unwrap();
        let (_, bytes, n) = app.transport().sent[app.transport().sent_n - 1].unwrap();
        assert_eq!(decode(&bytes[..n]).unwrap().code(), Code::UNAUTHORIZED);
        for index in 0..app.engine.capacities().rx_body_slots.unwrap() {
            assert!(
                app.engine
                    .rx_body_transfer(crate::storage::SlotId::from_index(index))
                    .is_none()
            );
        }
    }
}

#[test]
fn app_randomness_is_explicit_and_failure_never_substitutes_counters() {
    assert!(matches!(
        App::profile::<profiles::Default>()
            .block_wise::<false>()
            .bind(RecordIo::default()),
        Err(crate::BuildError::RandomnessRequired)
    ));
    assert!(matches!(
        App::profile::<profiles::Default>()
            .randomness(|_| false)
            .block_wise::<false>()
            .bind(RecordIo::default()),
        Err(crate::BuildError::RandomnessUnavailable)
    ));
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for source in [
        (|bytes: &mut [u8]| {
            bytes.fill(0);
            bytes.len() != 8
        }) as super::RandomSource,
        (|bytes: &mut [u8]| {
            bytes.fill(0);
            bytes.len() != 4
        }) as super::RandomSource,
    ] {
        let mut app = App::profile::<profiles::Default>()
            .randomness(source)
            .block_wise::<false>()
            .bind(RecordIo::default())
            .unwrap();
        for _ in 0..12 {
            assert_eq!(
                app.get("value").to(peer).send(0),
                Err(Error::Identity(super::IdentityError::RandomnessUnavailable))
            );
            assert_eq!(app.transport().sent_n, 0);
            assert_eq!(app.engine.tx_occupied(), 0);
        }
    }
}

#[test]
fn injected_entropy_drives_wire_identity_and_exact_retransmission_boundary() {
    fn fill(bytes: &mut [u8]) -> bool {
        match bytes.len() {
            2 => bytes.copy_from_slice(&0x1234u16.to_be_bytes()),
            4 => bytes.copy_from_slice(&1000u32.to_be_bytes()),
            8 => bytes.copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]),
            _ => return false,
        }
        true
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .randomness(fill)
        .block_wise::<false>()
        .bind(RecordIo::default())
        .unwrap();
    let call = app.get("value").to(peer).send(100).unwrap();
    let (_, first, n) = app.transport().sent[0].unwrap();
    assert_eq!(call.token().as_bytes(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(decode(&first[..n]).unwrap().message_id().get(), 0x1234);
    // Repeated active Tokens are rejected after bounded retries, before I/O.
    assert_eq!(
        app.get("other").to(peer).send(100),
        Err(Error::Identity(super::IdentityError::TokenExhausted))
    );
    assert_eq!(app.transport().sent_n, 1);
    app.poll(3099).unwrap();
    assert_eq!(app.transport().sent_n, 1);
    app.poll(3100).unwrap();
    assert_eq!(app.transport().sent_n, 2);
    let (_, retransmitted, rn) = app.transport().sent[1].unwrap();
    assert_eq!(&retransmitted[..rn], &first[..n]);
    app.poll(9099).unwrap();
    assert_eq!(app.transport().sent_n, 2);
    app.poll(9100).unwrap();
    assert_eq!(app.transport().sent_n, 3);
    assert!(app.cancel(call));
    assert_eq!(app.engine.tx_occupied(), 0);
}

#[test]
fn entropy_failure_during_fragment_start_reclaims_preallocated_tx_and_body() {
    static DRAWS: AtomicUsize = AtomicUsize::new(0);
    fn fill(bytes: &mut [u8]) -> bool {
        bytes.fill(0);
        bytes.len() != 4 || DRAWS.fetch_add(1, Ordering::SeqCst) == 0
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .randomness(fill)
        .block_wise::<true>()
        .bind(RecordIo::default())
        .unwrap();
    for _ in 0..12 {
        DRAWS.store(0, Ordering::SeqCst);
        assert_eq!(
            app.put("value").to(peer).payload(&[7; 3000]).send(0),
            Err(Error::Identity(super::IdentityError::RandomnessUnavailable))
        );
        assert_eq!(app.transport().sent_n, 0);
        assert_eq!(app.engine.tx_occupied(), 0);
        for i in 0..app.engine.capacities().tx_body_slots.unwrap() {
            assert!(
                app.engine
                    .tx_body_transfer(crate::storage::SlotId::from_index(i))
                    .is_none()
            );
        }
    }
}

#[test]
fn response_non_and_qblock_window_use_shared_local_mid_space() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for ty in [Type::Confirmable, Type::NonConfirmable] {
        let mut app = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route("large", get(get_large))
            .bind(WideLoopback::default())
            .unwrap();
        let q = BlockValue::from_size(0, false, 1024).unwrap().encode();
        let opts = [Opt::uri_path("large"), Opt::q_block2(&q)];
        let request = Message::new(ty, Code::GET, MessageId::new(500))
            .with_token(Token::from_checked(&[44]))
            .with_options(&opts);
        let mut wire = [0; WIRE];
        let n = encode(&request, &mut wire).unwrap();
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(0).unwrap();
        assert_eq!(app.transport().send_n, 2);
        let first = decode(&app.transport().sends[0][..app.transport().send_lens[0]]).unwrap();
        let second = decode(&app.transport().sends[1][..app.transport().send_lens[1]]).unwrap();
        assert_eq!(
            first.message_id().get(),
            if ty == Type::Confirmable { 500 } else { 1 }
        );
        assert_eq!(
            second.message_id().get(),
            if ty == Type::Confirmable { 1 } else { 2 }
        );
        app.get("other").to(peer).non().send(1).unwrap();
        assert_eq!(
            last_wide(&app).message_id().get(),
            if ty == Type::Confirmable { 2 } else { 3 }
        );
    }
}

#[test]
fn mid_reuse_wait_also_requires_pending_transmissions_to_finish() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let call = app.get("first").to(peer).send(0).unwrap();
    for _ in 1..65536 {
        app.ids.next_for::<&'static str, _>(&app.engine, 0).unwrap();
    }
    assert_eq!(
        app.get("next").to(peer).non().send(247_000),
        Err(Error::Identity(super::IdentityError::MessageIdExhausted))
    );
    assert_eq!(app.transport().sent_n, 1);
    assert!(app.cancel(call));
    app.take_response(call).unwrap().unwrap_err();
    let next = app.get("next").to(peer).non().send(247_000).unwrap();
    let (_, bytes, n) = app.transport().sent[1].unwrap();
    assert_eq!(decode(&bytes[..n]).unwrap().message_id().get(), 1);
    assert!(app.cancel(next));
}

#[test]
fn observe_cancellation_matches_every_retained_option_and_fetch_payload() {
    // RFC 7641 3.6: all options except ETags repeat the registration.
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    for change in 0..12 {
        let mut app = record_client();
        let original = app
            .get("sensors/temp")
            .non()
            .observe()
            .to(peer)
            .send(0)
            .unwrap();
        let mut stop = app.get("sensors/temp").non().deregister().to(peer);
        stop = match change {
            0 => stop.query("x=1"),
            1 => stop.query(""),
            2 => stop.accept(ContentFormat::TEXT_PLAIN),
            3 => stop.content_format(ContentFormat::TEXT_PLAIN),
            4 => stop.if_match(b"m"),
            5 => stop.if_none_match(),
            6 => stop.echo(crate::message::Echo::new(b"issued").unwrap()),
            7 => stop.no_response(crate::message::NoResponse::new(0)),
            8 => stop.request_tag(crate::storage::BodyTag::EMPTY),
            9 => stop.block2(BlockValue::new(0, false, 0).unwrap()),
            10 => stop.q_block2(),
            _ => stop.payload(b"different"),
        };
        assert_eq!(
            stop.send(1),
            Err(Error::ObserveCancellationMismatch),
            "change {change}"
        );
        assert_eq!(app.transport().sent_n, 1);
        // Refusal must leave the original selectable. A different ETag is legal.
        let stop = app
            .get("sensors/temp")
            .deregister_call(original)
            .etag(b"new")
            .to(peer)
            .send(2)
            .unwrap();
        assert_eq!(stop, original);
        let (_, bytes, n) = app.transport().sent[1].unwrap();
        let wire = decode(&bytes[..n]).unwrap();
        assert_eq!(wire.token(), original.token());
        assert_eq!(wire.observe(), Some(Ok(1)));
        assert_eq!(wire.etag().next(), Some(b"new".as_slice()));
    }
    let mut app = record_client();
    let first = app
        .fetch("sensors/temp")
        .non()
        .observe()
        .payload(b"a")
        .to(peer)
        .send(0)
        .unwrap();
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister()
            .payload(b"a")
            .to(peer)
            .send(1),
        Err(Error::ObserveCancellationMismatch)
    );
    assert_eq!(
        app.fetch("sensors/temp")
            .non()
            .deregister()
            .payload(b"b")
            .to(peer)
            .send(1),
        Err(Error::ObserveCancellationMismatch)
    );
    assert_eq!(
        app.fetch("sensors/temp")
            .non()
            .deregister()
            .payload(b"a")
            .to(peer)
            .send(1)
            .unwrap(),
        first
    );
}

#[test]
fn observe_cancellation_selects_exact_queries_and_requires_call_for_ambiguity() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = record_client();
    let a = app
        .get("sensors/temp")
        .non()
        .observe()
        .query("a")
        .query("")
        .to(peer)
        .send(0)
        .unwrap();
    let b = app
        .get("sensors/temp")
        .non()
        .observe()
        .query("")
        .query("a")
        .to(peer)
        .send(0)
        .unwrap();
    let duplicate = app
        .get("sensors/temp")
        .non()
        .observe()
        .query("a")
        .query("")
        .to(peer)
        .send(0)
        .unwrap();
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister()
            .query("a")
            .query("")
            .to(peer)
            .send(1),
        Err(Error::ObserveCancellationAmbiguous)
    );
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister_call(b)
            .query("a")
            .query("")
            .to(peer)
            .send(1),
        Err(Error::ObserveCancellationMismatch)
    );
    assert_eq!(app.transport().sent_n, 3);
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister_call(duplicate)
            .query("a")
            .query("")
            .to(peer)
            .send(1)
            .unwrap(),
        duplicate
    );
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister()
            .query("")
            .query("a")
            .to(peer)
            .send(1)
            .unwrap(),
        b
    );
    assert_eq!(
        app.get("sensors/temp")
            .non()
            .deregister()
            .query("a")
            .query("")
            .to(peer)
            .send(1)
            .unwrap(),
        a
    );
    assert_eq!(
        app.get("missing").non().deregister().to(peer).send(1),
        Err(Error::ObserveCancellationMismatch)
    );
}

#[test]
fn observe_cancellation_identity_bound_refuses_before_io_without_consuming_capacity() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .bind(WideLoopback::default())
        .unwrap();
    // Four header bytes plus Uri-Path encoding plus payload marker = seven.
    let at_bound = [b'x'; 505];
    let over_bound = [b'x'; 506];
    for _ in 0..12 {
        assert_eq!(
            app.fetch("x")
                .observe()
                .payload(&over_bound)
                .to(peer)
                .send(0),
            Err(Error::ObserveRequestTooLarge)
        );
    }
    assert_eq!(app.transport().send_n, 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    let call = app
        .fetch("x")
        .observe()
        .payload(&at_bound)
        .to(peer)
        .send(0)
        .unwrap();
    assert_eq!(app.transport().send_n, 1);
    assert!(app.cancel(call));
}

#[test]
fn observe_cancellation_replaces_unread_notification_and_old_pending_request() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_obs_app();
    let call = app.get("sensors/temp").observe().to(peer).send(0).unwrap();
    app.poll(0).unwrap();
    app.poll(0).unwrap();
    // Leave the initial notification unread. It must not masquerade as the
    // result of the new cancellation request sharing the same Token.
    assert_eq!(
        app.get("sensors/temp")
            .deregister_call(call)
            .to(peer)
            .send(1)
            .unwrap(),
        call
    );
    assert!(app.take_response(call).is_none());
    app.poll(1).unwrap();
    app.poll(1).unwrap();
    assert!(
        app.take_response(call)
            .unwrap()
            .unwrap()
            .observe_seq()
            .is_none()
    );
    assert_eq!(app.engine_mut().tx_occupied(), 0);

    let mut app = record_client();
    let call = app.get("x").observe().to(peer).send(0).unwrap();
    assert_eq!(app.engine_mut().tx_occupied(), 1);
    assert_eq!(
        app.get("x").deregister_call(call).to(peer).send(1).unwrap(),
        call
    );
    assert_eq!(
        app.engine_mut().tx_occupied(),
        1,
        "old registration CON retired"
    );
    app.poll(2000).unwrap();
    assert_eq!(
        app.transport().sent_n,
        2,
        "old registration must not retransmit"
    );
    app.poll(2001).unwrap();
    let (_, bytes, n) = app.transport().sent[2].unwrap();
    assert_eq!(decode(&bytes[..n]).unwrap().observe(), Some(Ok(1)));
}

#[test]
fn failed_observe_cancellation_retires_local_state_and_reports_uncertainty() {
    struct CancelIo {
        inner: RecordIo,
        fail: bool,
    }
    impl DatagramIo for CancelIo {
        type Error = &'static str;
        fn recv(&mut self, bytes: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            self.inner.recv(bytes)
        }
        fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            if self.fail {
                Err("injected cancellation failure")
            } else {
                self.inner.send(peer, bytes)
            }
        }
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .bind(CancelIo {
            inner: RecordIo::default(),
            fail: false,
        })
        .unwrap();
    for _ in 0..12 {
        app.transport_mut().inner.sent_n = 0;
        let call = app.get("x").observe().to(peer).send(0).unwrap();
        app.transport_mut().fail = true;
        assert!(matches!(
            app.get("x").deregister_call(call).to(peer).send(1),
            Err(Error::Io(_))
        ));
        assert_eq!(app.engine_mut().tx_occupied(), 0);
        assert_eq!(
            app.get("x").deregister_call(call).to(peer).send(2),
            Err(Error::ObserveCancellationMismatch)
        );
        assert_eq!(
            app.take_response(call).unwrap().unwrap_err(),
            crate::CallFailure::CancellationFailed
        );
        assert!(app.take_response(call).is_none());
        app.transport_mut().fail = false;
    }
}

#[test]
fn observe_cancellation_cannot_select_an_unread_terminal_response() {
    fn decline(_: Request<'_>) -> Response<'static> {
        Response::not_found()
    }
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route("x", get(decline))
        .bind(Echo::default())
        .unwrap();
    let call = app.get("x").observe().to(peer).send(0).unwrap();
    app.poll(0).unwrap();
    app.poll(0).unwrap();
    assert_eq!(
        app.get("x").deregister_call(call).to(peer).send(1),
        Err(Error::ObserveCancellationMismatch)
    );
    assert_eq!(
        app.take_response(call).unwrap().unwrap().code(),
        Code::NOT_FOUND
    );
}

#[test]
fn observe_client_and_server_roles_do_not_overwrite_or_cancel_each_other() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_obs_app();
    let call = app.get("sensors/temp").observe().to(peer).send(0).unwrap();
    app.poll(0).unwrap();
    app.poll(0).unwrap();
    let server_key = ObserveKey::new(call.token(), peer);
    let client_key = ObserveKey::new_client(call.token(), peer);
    let server = app.engine.lookup_observe(server_key).unwrap();
    let client = app.engine.lookup_observe(client_key).unwrap();
    assert_ne!(server, client);
    assert!(
        app.engine
            .observe_interest(client)
            .unwrap()
            .resource()
            .is_none()
    );
    assert_eq!(
        app.notify(
            1,
            &["sensors", "temp"],
            Response::content(b"one notification").content_format(ContentFormat::TEXT_PLAIN)
        )
        .unwrap(),
        1
    );
    app.poll(1).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap().payload(),
        b"one notification"
    );
    assert!(app.cancel(call));
    assert!(app.engine.lookup_observe(client_key).is_none());
    assert!(
        app.engine.lookup_observe(server_key).is_some(),
        "local cancellation must not remove the independent observer"
    );
    assert_eq!(
        app.take_response(call).unwrap().unwrap_err(),
        crate::CallFailure::Cancelled
    );
    // The peer's existing server relation terminates when its CON is reset.
    app.notify(
        86_400_002,
        &["sensors", "temp"],
        Response::content(b"late").content_format(ContentFormat::TEXT_PLAIN),
    )
    .unwrap();
    app.poll(86_400_002).unwrap();
    app.poll(86_400_002).unwrap();
    assert!(app.engine.lookup_observe(server_key).is_none());
}

#[test]
fn observe_route_identity_keeps_literal_slashes_and_segments_distinct() {
    fn slash() -> Response<'static> {
        Response::content(b"literal").content_format(ContentFormat::TEXT_PLAIN)
    }
    fn segments() -> Response<'static> {
        Response::content(b"segments").content_format(ContentFormat::TEXT_PLAIN)
    }
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let mut app = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route(&["a/b"], get(get_obs).observe(slash))
        .route(&["a", "b"], get(get_obs).observe(segments))
        .bind(WideLoopback::default())
        .unwrap();
    let paths: [&[&str]; 2] = [&["a/b"], &["a", "b"]];
    let tokens = [Token::new(&[1]).unwrap(), Token::new(&[2]).unwrap()];
    for (i, path) in paths.iter().enumerate() {
        let (wire, n) = encode_wide_token(
            Code::GET,
            path,
            &[Opt::observe_register()],
            100 + i as u16,
            tokens[i],
        );
        app.transport_mut().inbox = Some((peer, wire, n));
        app.poll(i as u64).unwrap();
    }
    assert_ne!(
        app.site.observe_resource(paths[0]),
        app.site.observe_resource(paths[1])
    );
    assert!(app.site.observe_resource(&["missing"]).is_none());
    for (i, path) in paths.iter().enumerate() {
        app.transport_mut().send_n = 0;
        assert_eq!(app.signal(path), 1);
        app.poll(10_000 * (i as u64 + 1)).unwrap();
        let note = last_wide(&app);
        assert_eq!(note.token(), tokens[i]);
        assert_eq!(
            note.payload(),
            if i == 0 {
                b"literal".as_slice()
            } else {
                b"segments".as_slice()
            }
        );
        assert_eq!(app.transport().send_n, 1);
    }
    assert_eq!(app.signal(&["missing"]), 0);
    assert_eq!(
        app.notify(30_000, &["missing"], Response::content(b"no"))
            .unwrap(),
        0
    );
}
