use super::*;
use crate::app::{App, get};
use crate::message::{BlockValue, Message, MessageId, Opt, OptionsBuilder, Token, decode};
use crate::profiles;
use crate::storage::{DatagramIo, DatagramTransport, Endpoint};
use core::cell::Cell;

const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const DISCOVERY: &[&str] = &[".well-known", "core"];
const ROUTES: [&str; 16] = [
    "ep/00", "ep/01", "ep/02", "ep/03", "ep/04", "ep/05", "ep/06", "ep/07", "ep/08", "ep/09",
    "ep/10", "ep/11", "ep/12", "ep/13", "ep/14", "ep/15",
];
const CATALOG: &[u8] = concat!(
    "</ep/00>,</ep/01>,</ep/02>,</ep/03>,</ep/04>,</ep/05>,</ep/06>,</ep/07>,",
    "</ep/08>,</ep/09>,</ep/10>,</ep/11>,</ep/12>,</ep/13>,</ep/14>,</ep/15>"
)
.as_bytes();

fn handler(_: Request<'_>) -> Response<'static> {
    Response::content(b"route")
}

fn request_wire(
    code: Code,
    path: &[&str],
    block: Option<BlockValue>,
    mid: u16,
) -> ([u8; 256], usize) {
    let mut options = OptionsBuilder::<3>::new();
    for segment in path {
        options.push(Opt::uri_path(segment)).unwrap();
    }
    let block = block.map(BlockValue::encode);
    if let Some(ref block) = block {
        options.push(Opt::block2(block)).unwrap();
    }
    let mut wire = [0; 256];
    let len = Message::con(code, MessageId::new(mid), Token::EMPTY)
        .with_options(options.as_slice())
        .encode(&mut wire)
        .unwrap();
    (wire, len)
}

fn dispatch<'a, const N: usize>(
    site: &Site<N>,
    code: Code,
    path: &[&str],
    catalog: impl FnOnce() -> &'a mut [u8],
) -> Response<'a> {
    let (wire, len) = request_wire(code, path, None, 1);
    let request = Request::from_decoded(decode(&wire[..len]).unwrap(), PEER, None).unwrap();
    site.dispatch_with_catalog(request, catalog)
}

#[test]
fn lazy_catalog_supplier_is_unused_outside_builtin_discovery_get() {
    let mut site = Site::<2>::new();
    site.route("value", get(handler)).well_known_core();
    for (code, path, expected) in [
        (Code::GET, &["value"][..], Code::CONTENT),
        (Code::POST, &["value"][..], Code::METHOD_NOT_ALLOWED),
        (Code::GET, &["missing"][..], Code::NOT_FOUND),
        (Code::POST, DISCOVERY, Code::METHOD_NOT_ALLOWED),
    ] {
        let response = dispatch(&site, code, path, || panic!("unused catalog requested"));
        assert_eq!(response.code(), expected);
    }

    site.route(DISCOVERY, get(handler));
    let response = dispatch(&site, Code::GET, DISCOVERY, || {
        panic!("registered discovery route must win")
    });
    assert_eq!(response.payload(), b"route");
    assert_eq!(
        dispatch(&site, Code::POST, DISCOVERY, || {
            panic!("registered route's method refusal must win")
        })
        .code(),
        Code::METHOD_NOT_ALLOWED
    );

    let disabled = Site::<0>::new();
    assert_eq!(
        dispatch(&disabled, Code::GET, DISCOVERY, || {
            panic!("disabled discovery requested catalog")
        })
        .code(),
        Code::NOT_FOUND
    );
}

#[test]
fn lazy_catalog_supplier_runs_once_and_retains_non_inline_bytes() {
    let mut site = Site::<16>::new();
    for path in ROUTES {
        site.route(path, get(handler));
    }
    site.well_known_core();
    let calls = Cell::new(0);
    let mut scratch = None;
    let response = dispatch(&site, Code::GET, DISCOVERY, || {
        calls.set(calls.get() + 1);
        scratch
            .get_or_insert_with(LinkFormatScratch::<16>::new)
            .as_mut()
    });
    assert_eq!(calls.get(), 1);
    assert!(CATALOG.len() > INLINE_PAYLOAD);
    assert_eq!(response.payload(), CATALOG);
    assert_eq!(
        response.etag_bytes(),
        Some(catalog_etag(CATALOG).as_slice())
    );
    assert_eq!(response.format(), Some(ContentFormat::LINK_FORMAT));
}

#[derive(Default)]
struct Io {
    incoming: Option<([u8; 256], usize)>,
    sent: Option<([u8; 256], usize)>,
}

impl DatagramTransport for Io {
    type Error = ();
}

impl DatagramIo for Io {
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        let Some((wire, len)) = self.incoming.take() else {
            return Ok(None);
        };
        out[..len].copy_from_slice(&wire[..len]);
        Ok(Some((len, PEER)))
    }

    fn send(&mut self, _: Endpoint, wire: &[u8]) -> Result<usize, ()> {
        let mut packet = [0; 256];
        packet[..wire.len()].copy_from_slice(wire);
        self.sent = Some((packet, wire.len()));
        Ok(wire.len())
    }
}

#[test]
fn lazy_catalog_app_sends_non_inline_borrowed_classic_blocks() {
    let mut builder = App::profile::<profiles::Default>()
        .routes::<16>()
        .block_wise::<true>()
        .deterministic_for_tests();
    for path in ROUTES {
        builder = builder.route(path, get(handler));
    }
    let first = BlockValue::from_size(0, false, 64).unwrap();
    let mut app = builder
        .well_known_core()
        .allow_unprotected()
        .bind(Io {
            incoming: Some(request_wire(Code::GET, DISCOVERY, Some(first), 1)),
            ..Io::default()
        })
        .unwrap();
    app.poll(0).unwrap();
    let (wire, len) = app.transport().sent.unwrap();
    let response = decode(&wire[..len]).unwrap();
    assert_eq!(response.payload(), &CATALOG[..64]);
    assert!(response.block2().unwrap().unwrap().more());
    let tag: [u8; 4] = response.etag().next().unwrap().try_into().unwrap();

    let next = BlockValue::from_size(1, false, 64).unwrap();
    app.transport_mut().incoming = Some(request_wire(Code::GET, DISCOVERY, Some(next), 2));
    app.poll(1).unwrap();
    let (wire, len) = app.transport().sent.unwrap();
    let response = decode(&wire[..len]).unwrap();
    assert_eq!(response.payload(), &CATALOG[64..128]);
    assert_eq!(response.etag().next(), Some(tag.as_slice()));

    let last = BlockValue::from_size(2, false, 64).unwrap();
    app.transport_mut().incoming = Some(request_wire(Code::GET, DISCOVERY, Some(last), 3));
    app.poll(2).unwrap();
    let (wire, len) = app.transport().sent.unwrap();
    let response = decode(&wire[..len]).unwrap();
    assert_eq!(response.payload(), &CATALOG[128..]);
    assert!(!response.block2().unwrap().unwrap().more());
    assert_eq!(response.etag().next(), Some(tag.as_slice()));
}
