//! Site and [`App::poll`] against a loopback [`DatagramIo`].

use super::{Error, Request, Response, Site, get, post, put};
use crate::app::App;
use crate::error::{EncodeError, SlotMessageError};
use crate::message::{
    BlockValue, Code, ContentFormat, EncodedUint, Message, MessageId, Opt, OptionsBuilder,
    ProblemDetails, Token, Type, decode, encode,
};
use crate::storage::{BlockKey, DatagramIo, Endpoint, ObserveKey, profiles};

const LARGE: [u8; 2000] = [b'A'; 2000];
const WIRE: usize = 1472;

fn get_temp(_req: Request<'_>) -> Response {
    Response::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
}

fn get_led(_: Request<'_>) -> Response {
    // Demo payload. Real LED state is firmware-owned, not an App bag.
    Response::content(b"off")
}

fn put_led(_req: Request<'_>) -> Response {
    Response::changed()
}

fn post_led(_: Request<'_>) -> Response {
    Response::changed()
}

fn put_body(req: Request<'_>) -> Response {
    if req.has_body() {
        Response::changed().payload_copy(req.body().unwrap_or(&[]))
    } else {
        Response::changed().payload_copy(req.payload())
    }
}

fn get_large(_: Request<'_>) -> Response {
    Response::content(&LARGE).content_format(ContentFormat::OCTET_STREAM)
}

fn get_obs(_req: Request<'_>) -> Response {
    Response::content(b"obs-0")
        .content_format(ContentFormat::TEXT_PLAIN)
        .max_age(5)
        .observe(0)
}

fn obs_snapshot() -> Response {
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

fn app_with_site(io: Loopback) -> App<profiles::Default, Loopback> {
    App::profile::<profiles::Default>()
        .block_wise(false)
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
}

fn last_reply(app: &App<profiles::Default, Loopback>) -> LastReply {
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
        Some(u32::from(crate::NoResponse::SUPPRESS_2)),
    );
    let mut app = app_with_site(Loopback {
        inbox: Some((peer, wire, n)),
        last_send: None,
    });
    app.poll(0).expect("poll");
    assert!(app.transport().last_send.is_none());
}

#[test]
fn well_known_core_lists_registered_paths() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_req(Code::GET, &[".well-known", "core"], &[]);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(false)
        .route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_led))
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
    assert_eq!(body, "</sensors/temp>,</leds/0>");
}

#[test]
fn site_dispatch_table() {
    let mut site = Site::<4>::new();
    site.route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_led));

    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);
    assert_eq!(site.dispatch(req).payload(), b"off");
}

#[test]
fn route_path_splits_static_uri() {
    let mut site = Site::<4>::new();
    site.route_path("/sensors/temp", get(get_temp));
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);
}

#[test]
fn replacing_route_changes_handler() {
    let mut site = Site::<2>::new();
    site.route(&["leds", "0"], get(get_led));
    site.route(&["leds", "0"], post(post_led));

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::METHOD_NOT_ALLOWED);

    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req = Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), None).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CHANGED);
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
    let size1_enc = req.size1.map(crate::encode_uint);
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
        .block_wise(true)
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
fn block1_complete_exposes_body() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let first = [b'A'; 16];
    let second = *b"REST";
    let (wire, n) = encode_req_block1(Code::PUT, &["leds", "0"], &first, 0, true, 16, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
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
        .block_wise(true)
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
        .block_wise(true)
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
        .block_wise(true)
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
    app.poll(2).expect("recover");
    let parsed = last_reply(&app);
    assert_eq!(parsed.ty, Type::NonConfirmable);
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
fn qblock1_apply_error_is_request_entity_incomplete() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let payload = [0xABu8; 16];
    let (wire, n) = encode_block_req(q_block1(&payload, 0, true, 0x1001, 32));
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
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
}

#[test]
fn qblock2_recover_sent_from_poll() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let token = Token::new(&[0xA1]).expect("token");
    let key = BlockKey::new(token, peer);
    let body: [u8; 40] = core::array::from_fn(|i| (i + 7) as u8);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
        .bind(Loopback::default())
        .expect("bind");
    match app.engine_mut() {
        crate::app::EngineMut::BlockWise(engine) => {
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
        }
        crate::app::EngineMut::Datagram(_) => panic!("expected body pools"),
    }
    app.poll(0).expect("poll recover");
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
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
        .bind(Loopback::default())
        .expect("bind");
    match app.engine_mut() {
        crate::app::EngineMut::BlockWise(_) => {}
        crate::app::EngineMut::Datagram(_) => panic!("expected body pools"),
    }
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
    let token = Token::new(&[0xA1]).expect("token");
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

fn last_wide(app: &App<profiles::Default, WideLoopback>) -> crate::ParsedMessage<'_> {
    let n = app.transport().send_n;
    assert!(n > 0, "expected a send");
    decode(&app.transport().sends[n - 1][..app.transport().send_lens[n - 1]]).expect("decode")
}

#[test]
fn large_get_ships_block2_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["large"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
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
fn large_get_without_body_pools_fails_clearly() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let (wire, n) = encode_wide(Code::GET, &["large"], &[], 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(false)
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
        .block_wise(true)
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

fn observe_registered(app: &App<profiles::Default, WideLoopback>, peer: Endpoint) -> bool {
    let key = ObserveKey::new(Token::new(&[0xA1]).expect("token"), peer);
    match app.engine() {
        crate::app::EngineRef::Datagram(engine) => engine.lookup_observe(key).is_some(),
        crate::app::EngineRef::BlockWise(engine) => engine.lookup_observe(key).is_some(),
    }
}

#[test]
fn observe_register_notify_deregister() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(false)
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
fn observe_max_age_expiry_drops_interest() {
    let peer = Endpoint::v4([192, 0, 2, 1], 5683);
    let extra = [Opt::observe_register()];
    let (wire, n) = encode_wide(Code::GET, &["sensors", "temp"], &extra, 0x1001);
    let mut app = App::profile::<profiles::Default>()
        .block_wise(false)
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
        .block_wise(false)
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
        .block_wise(false)
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
    assert!(app.take_reply(call).is_none());
    app.poll(0).expect("server handle");
    assert!(app.take_reply(call).is_none());
    app.poll(0).expect("client match");
    let reply = app.take_reply(call).expect("matched");
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.payload(), b"21.5");
    assert_eq!(reply.content_format(), Some(ContentFormat::TEXT_PLAIN));
    assert_eq!(reply.token(), call.token());
    assert_eq!(reply.peer(), peer);
    assert_eq!(reply.ty(), Type::Acknowledgement);
    assert!(app.take_reply(call).is_none());
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
    let reply = app.take_reply(call).expect("matched");
    assert_eq!(reply.code(), Code::CHANGED);
    assert_eq!(reply.payload(), b"on");
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
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
    let reply = app.take_reply(call).expect("matched");
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.payload(), b"21.5");
    assert_eq!(reply.ty(), Type::NonConfirmable);
}

#[test]
fn client_get_unknown_path_is_problem_details() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = echo_app();
    let call = app.get(&["nope"]).to(peer).send(0).expect("send");
    app.poll(0).expect("server handle");
    app.poll(0).expect("client match");
    let reply = app.take_reply(call).expect("matched");
    assert_eq!(reply.code(), Code::NOT_FOUND);
    let details = reply.problem_details().expect("cbor");
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

fn pipe_app() -> App<profiles::Default, Pipe> {
    App::profile::<profiles::Default>()
        .block_wise(true)
        .route(&["large"], get(get_large))
        .bind(Pipe::default())
        .expect("bind")
}

fn poll_until_reply(app: &mut App<profiles::Default, Pipe>, call: crate::Call) -> crate::Reply {
    for t in 0u64..8 {
        app.poll(t).expect("poll");
        if let Some(reply) = app.take_reply(call) {
            return reply;
        }
    }
    panic!("client Block2/Q-Block2 did not complete");
}

#[test]
fn client_get_block2_assembles_body_without_slot_id() {
    let peer = Endpoint::v4([192, 0, 2, 2], 5683);
    let mut app = pipe_app();
    let call = app.get(&["large"]).to(peer).send(0).expect("send");
    let reply = poll_until_reply(&mut app, call);
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.content_format(), Some(ContentFormat::OCTET_STREAM));
    assert!(reply.has_body());
    assert_eq!(reply.body().expect("assembled"), &LARGE[..]);
    assert_eq!(reply.token(), call.token());
    assert_eq!(reply.peer(), peer);
    assert!(app.take_reply(call).is_none());
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
    let reply = poll_until_reply(&mut app, call);
    assert_eq!(reply.code(), Code::CONTENT);
    assert!(reply.has_body());
    assert_eq!(reply.body().expect("assembled"), &LARGE[..]);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}
