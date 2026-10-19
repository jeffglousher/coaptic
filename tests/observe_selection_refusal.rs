//! A refused follow-up cannot retire or replace an established selection.
use coaptic::message::{BlockValue, Message, MessageId, Opt, Token, Type, decode, encode};
use coaptic::storage::{DatagramIo, DatagramTransport, ObserveKey};
use coaptic::{App, Code, ContentFormat, Endpoint, Request, Response};
use std::collections::VecDeque;

coaptic::memory_profile! { Buffers {
    rx_datagram: (4, 256), tx_datagram: (4, 256),
    rx_body: (2, 1024), tx_body: (2, 1024), dedup: 8, observe: 4,
} }
const PEER: Endpoint = Endpoint::v4([127, 0, 0, 1], 5684);
#[derive(Default)]
struct Io {
    incoming: Option<Vec<u8>>,
    output: VecDeque<Vec<u8>>,
}
impl DatagramTransport for Io {
    type Error = ();
}
impl DatagramIo for Io {
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        let Some(bytes) = self.incoming.take() else {
            return Ok(None);
        };
        out[..bytes.len()].copy_from_slice(&bytes);
        Ok(Some((bytes.len(), PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        self.output.push_back(bytes.to_vec());
        Ok(bytes.len())
    }
}
fn selected(request: Request<'_>) -> Response<'static> {
    let body: &'static [u8] = match request.uri_query().next() {
        Some(Ok("pick=left")) => &[b'a'; 96],
        Some(Ok("pick=right")) => &[b'm'; 96],
        _ => return Response::bad_request(),
    };
    Response::content(body).content_format(ContentFormat::TEXT_PLAIN)
}
fn request(mid: u16, token: &[u8], query: &str, register: bool, followup: bool) -> Vec<u8> {
    let block = BlockValue::new(1, false, 1).unwrap().encode();
    let mut options = Vec::new();
    if register {
        options.push(Opt::observe_register());
    }
    options.push(Opt::uri_path("obs"));
    options.push(Opt::uri_query(query));
    if followup {
        options.push(Opt::block2(&block));
    }
    let message = Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
        .with_token(Token::new(token).unwrap())
        .with_options(&options);
    let mut bytes = [0; 256];
    let len = encode(&message, &mut bytes).unwrap();
    bytes[..len].to_vec()
}

#[test]
fn refused_selected_followup_preserves_both_registered_relations() {
    let mut server = App::with_resources(
        coaptic::resources::Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("obs", coaptic::get(selected).observe_selected(selected))
    .bind(Io::default())
    .unwrap();
    for (mid, token, query, expected) in [
        (1, b"left", "pick=left", Code::CONTENT),
        (2, b"rght", "pick=right", Code::CONTENT),
        (3, b"left", "pick=invalid", Code::BAD_REQUEST),
        (4, b"left", "pick=left", Code::CONTENT),
    ] {
        server.transport_mut().incoming = Some(request(mid, token, query, true, false));
        server.poll(u64::from(mid)).unwrap();
        let wire = server.transport_mut().output.pop_front().unwrap();
        let response = decode(&wire).unwrap();
        assert_eq!(response.code(), expected);
        assert_eq!(response.observe().is_some(), expected.is_success());
    }
    let key = |token: &[u8]| ObserveKey::new(Token::new(token).unwrap(), PEER);
    let left = server.engine_mut().lookup_observe(key(b"left")).unwrap();
    let right = server.engine_mut().lookup_observe(key(b"rght")).unwrap();
    let left_before = server.engine_mut().observe_interest(left).unwrap();
    let right_before = server.engine_mut().observe_interest(right).unwrap();
    server.transport_mut().incoming = Some(request(5, b"left", "pick=right", false, true));
    server.poll(5).unwrap();
    let wire = server.transport_mut().output.pop_front().unwrap();
    assert_eq!(decode(&wire).unwrap().code(), Code::BAD_REQUEST);
    assert_eq!(
        server.engine_mut().observe_interest(left),
        Some(left_before)
    );
    assert_eq!(
        server.engine_mut().observe_interest(right),
        Some(right_before)
    );
    assert_eq!(server.signal(&["obs"]), 2);
    for now in 6..12 {
        server.poll(now).unwrap();
    }
    // RFC 7641 section 4.5 limits simultaneous NON notifications to the
    // endpoint, even though these are two different selected relations.
    assert_eq!(server.transport_mut().output.len(), 1);
    let pending = [left, right]
        .into_iter()
        .filter(|id| {
            server
                .engine_mut()
                .observe_interest(*id)
                .unwrap()
                .is_pending()
        })
        .count();
    assert_eq!(pending, 1, "the paced relation remains live and pending");
    let next = 6 + u64::from(coaptic::message::ObserveTransmission::NON_TIMEOUT_MS);
    server.poll(next - 1).unwrap();
    assert_eq!(server.transport_mut().output.len(), 1);
    server.poll(next).unwrap();
    let mut seen = Vec::new();
    while let Some(wire) = server.transport_mut().output.pop_front() {
        let notification = decode(&wire).unwrap();
        assert_eq!(notification.code(), Code::CONTENT);
        assert!(notification.observe().is_some());
        let token = notification.token();
        assert_eq!(
            notification.payload(),
            if token == Token::new(b"left").unwrap() {
                &[b'a'; 96]
            } else {
                &[b'm'; 96]
            }
        );
        seen.push(token);
    }
    assert_eq!(seen.len(), 2);
    assert!(seen.contains(&Token::new(b"left").unwrap()));
    assert!(seen.contains(&Token::new(b"rght").unwrap()));
}
