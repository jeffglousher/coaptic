//! Site and [`App::poll`] against a loopback [`DatagramIo`].

use super::{
    AppAssembled, DEFAULT_ROUTES, Error, INLINE_PAYLOAD, LINK_FORMAT_PER_ROUTE, Method,
    RESPONSE_BODY, Request, Response, Site, fetch, get, ipatch, link_format_capacity, patch, post,
    put,
};
use crate::app::App;
use crate::error::{EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, Echo as EchoOpt, EncodedUint, Message, MessageId,
    MissingBlocks, ObserveTransmission, Opt, OptionNumber, OptionsBuilder, ProblemDetails,
    QBlockTransmission, Token, Transmission, Type, decode, encode, encode_uint,
};
use crate::storage::{
    BlockKey, DatagramIo, Endpoint, ExchangeKey, MemoryLayout, MemoryProfile, ObserveKey, profiles,
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
    Response::content(&LARGE).content_format(ContentFormat::OCTET_STREAM)
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
    encode_req_echo(Type::Confirmable, code, path, payload, None, None)
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
    )
}

fn encode_req_ty(
    ty: Type,
    code: Code,
    path: &[&str],
    payload: &[u8],
    no_response: Option<u32>,
) -> ([u8; 256], usize) {
    encode_req_echo(ty, code, path, payload, no_response, None)
}

fn encode_req_echo(
    ty: Type,
    code: Code,
    path: &[&str],
    payload: &[u8],
    no_response: Option<u32>,
    echo: Option<&[u8]>,
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
    let msg = Message::new(ty, code, MessageId::new(0x1001))
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
fn created_response_carries_location_path_and_query() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["items"], &[]);
    let mut app = App::profile::<profiles::Default>()
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
fn push_opt_fails_loud_when_builder_is_full() {
    let mut opts = OptionsBuilder::<1>::new();
    super::push_opt(&mut opts, Opt::uri_path("a")).expect("first");
    assert_eq!(
        super::push_opt(&mut opts, Opt::uri_path("b")),
        Err(EncodeError::OptionsFull)
    );
}

#[test]
fn created_max_location_etag_observe_echo_all_on_wire() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::POST, &["items"], &[]);
    let mut app = App::profile::<profiles::Default>()
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
    assert_eq!(parsed.observe().and_then(Result::ok), Some(0));
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

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    app.transport_mut().inbox = Some((peer, wire, n));
    app.transport_mut().last_send = None;
    app.poll(1).expect("poll");
    let parsed = last_reply(&app);
    assert_eq!(parsed.code, Code::CONTENT);
    assert_eq!(&parsed.payload[..parsed.payload_len], b"off");
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
    app.echo_freshness(ECHO_FRESH_MS);
    app.poll(10).expect("poll");
    let challenge = assert_echo_401(&app, 10);

    let (wire, n) = encode_req_with_echo(Code::PUT, &["leds", "0"], b"1", &challenge);
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
    app.echo_freshness(ECHO_FRESH_MS);
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
    app.echo_freshness(ECHO_FRESH_MS);
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
        .echo_freshness(ECHO_FRESH_MS)
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

    let mut scratch = [0u8; RESPONSE_BODY];
    let response = dispatch_well_known(&site, &mut scratch);
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
    let mut scratch = [0u8; RESPONSE_BODY];
    let response = dispatch_well_known(&site, &mut scratch);
    assert_eq!(response.code(), Code::INTERNAL_SERVER_ERROR);
    assert!(response.payload().is_empty());
    assert_ne!(response.format(), Some(ContentFormat::LINK_FORMAT));
}

/// IANA experimental 65000–65535. Even ⇒ elective (LSB clear); unrecognized
/// critical options can be 4.02, so this number is safe to carry through decode.
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
fn large_get_ships_block2_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["large"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
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
}

#[test]
fn large_get_location_etag_echo_and_block2_all_on_wire() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["loud"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
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

    let second = decode(&app.transport().sends[1][..app.transport().send_lens[1]]).expect("second");
    assert_eq!(second.ty(), Type::NonConfirmable);
    let q1 = second.q_block2().next().expect("Q-Block2").expect("val");
    assert_eq!(q1.num(), 1);
    assert!(!q1.more());
    assert_eq!(second.payload(), &LARGE[1024..]);
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
        .notify(10, &["sensors", "temp"], Response::content(b"obs-n"))
        .expect("notify");
    assert_eq!(
        sent, 1,
        "NSTART=1: one notify per endpoint even with two rows"
    );
    assert_eq!(app.transport().send_n, 1);
}

#[test]
fn notify_fans_out_to_distinct_endpoints() {
    let peer_a = Endpoint::v4([192, 0, 2, 1], 5683);
    let peer_b = Endpoint::v4([192, 0, 2, 3], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_nstart_wire(Code::GET, &["sensors", "temp"], &extra, 0x1001, 0xA1);
    let mut app = App::profile::<profiles::Default>()
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
        .notify(10, &["sensors", "temp"], Response::content(b"obs-n"))
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
fn observe_max_age_expiry_drops_interest() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(WideLoopback {
            inbox: Some((peer, wire, n)),
            ..WideLoopback::default()
        })
        .expect("bind");
    app.poll(0).expect("poll");
    assert!(observe_registered(&app, peer));

    app.poll(5_000).expect("expired");
    assert!(!observe_registered(&app, peer));
}

#[test]
fn observe_source_sends_on_signal_poll() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
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
    let response = app.take_response(call).expect("matched");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
    assert_eq!(response.format(), Some(ContentFormat::TEXT_PLAIN));
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(response.peer(), Some(peer));
    assert_eq!(response.ty(), Some(Type::Acknowledgement));
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
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
    let response = app.take_response(call).expect("matched");
    assert_eq!(response.code(), Code::CHANGED);
    assert_eq!(response.payload(), b"on");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

fn echo_rfc8132_app() -> App<profiles::Default, Echo> {
    App::profile::<profiles::Default>()
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
    let response = app.take_response(call).expect("matched");
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
    let response = app.take_response(call).expect("matched");
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), b"21.5");
    assert_eq!(response.ty(), Some(Type::NonConfirmable));
}

#[test]
fn client_get_sends_query_accept_etag_if_match_and_block2() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Default>()
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
        .block_wise::<false>()
        .route(&["validate"], get(get_tagged))
        .bind(Echo::default())
        .expect("bind");
    let call = app.get("validate").to(peer).send(0).expect("send");
    app.poll(0).expect("server");
    app.poll(0).expect("client");
    let response = app.take_response(call).expect("matched");
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
    let response = app.take_response(call).expect("matched");
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
    let response = app.take_response(call).expect("matched");
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
        .block_wise::<false>()
        .route(&["sensors", "temp"], get(get_obs))
        .bind(Echo::default())
        .expect("bind")
}

fn client_observe_live(app: &App<profiles::Default, Echo>, call: crate::Call) -> bool {
    let key = ObserveKey::new(call.token(), call.peer());
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
    let initial = app.take_response(call).expect("initial");
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
    let note = app.take_response(call).expect("notification");
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
    let response = app.take_response(call).expect("matched");
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
    let response = app.take_response(call).expect("matched after loss");
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
    assert!(app.take_response(call).is_none());
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn poll_progresses_when_rx_saturated() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = App::profile::<profiles::Constrained>()
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
    assert_eq!(response.code(), Code::GATEWAY_TIMEOUT);
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
    assert_eq!(response.code(), Code::GATEWAY_TIMEOUT);
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
    assert_eq!(response.code(), Code::GATEWAY_TIMEOUT);
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
            .expect("untaken reply kept");
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), b"21.5");
    }
    app.get(&["sensors", "temp"])
        .to(peer)
        .send(0)
        .expect("send after take");
}
