//! Site and [`App::poll`] against a loopback [`DatagramIo`].

use super::{Reply, Request, Site, get, post};
use crate::app::App;
use crate::message::{
    Code, ContentFormat, EncodedUint, Message, MessageId, Opt, OptionsBuilder, Token, Type, decode,
    encode,
};
use crate::storage::{DatagramIo, Endpoint, profiles};

fn get_temp(_req: Request<'_>) -> Reply {
    Reply::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
}

fn get_led(_: Request<'_>) -> Reply {
    // Demo payload. Real LED state is firmware-owned, not an App bag.
    Reply::content(b"off")
}

fn put_led(_req: Request<'_>) -> Reply {
    Reply::changed()
}

fn post_led(_: Request<'_>) -> Reply {
    Reply::changed()
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
    assert_eq!(last_reply(&app).code, Code::NOT_FOUND);
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
    assert_eq!(last_reply(&app).code, Code::METHOD_NOT_ALLOWED);
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
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);
    assert_eq!(site.dispatch(req).payload(), b"off");
}

#[test]
fn route_path_splits_static_uri() {
    let mut site = Site::<4>::new();
    site.route_path("/sensors/temp", get(get_temp));
    let (wire, n) = encode_req(Code::GET, &["sensors", "temp"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CONTENT);
}

#[test]
fn replacing_route_changes_handler() {
    let mut site = Site::<2>::new();
    site.route(&["leds", "0"], get(get_led));
    site.route(&["leds", "0"], post(post_led));

    let (wire, n) = encode_req(Code::GET, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::METHOD_NOT_ALLOWED);

    let (wire, n) = encode_req(Code::POST, &["leds", "0"], &[]);
    let parsed = decode(&wire[..n]).expect("decode");
    let req =
        Request::from_decoded(parsed, Endpoint::v4([192, 0, 2, 1], 5683), slot0()).expect("req");
    assert_eq!(site.dispatch(req).code(), Code::CHANGED);
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
        .content_format(ContentFormat::LINK_FORMAT)
        .max_age(60)
        .observe(3)
        .etag(b"ab");
    assert_eq!(cf.format(), Some(ContentFormat::LINK_FORMAT));
    assert_eq!(cf.max_age_secs(), Some(60));
    assert_eq!(cf.observe_seq(), Some(3));
    assert_eq!(cf.etag_bytes(), Some(&b"ab"[..]));
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
