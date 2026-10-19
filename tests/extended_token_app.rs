use coaptic::message::{MessageId, Opt, Type, decode_with_token, extended};
use coaptic::storage::{DatagramIo, DatagramTransport, Endpoint};
use coaptic::{App, Code, Request, Response, get};

type LongToken = extended::Token<32>;
coaptic::memory_profile! { Wide {
    token: LongToken,
    rx_datagram: (4, 256), tx_datagram: (4, 256),
    rx_body: (1, 1024), tx_body: (1, 1024), dedup: 4, observe: 2,
} }
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);

struct Io {
    incoming: Option<([u8; 256], usize)>,
    wire: [u8; 256],
    len: usize,
}
impl Default for Io {
    fn default() -> Self {
        Self {
            incoming: None,
            wire: [0; 256],
            len: 0,
        }
    }
}
impl DatagramTransport for Io {
    type Error = &'static str;
}
impl DatagramIo for Io {
    fn recv(&mut self, output: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some((wire, n)) = self.incoming.take() else {
            return Ok(None);
        };
        output[..n].copy_from_slice(&wire[..n]);
        Ok(Some((n, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.wire[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        Ok(bytes.len())
    }
}
impl Io {
    fn receive(&mut self, message: extended::Message<'_>) {
        let mut wire = [0; 256];
        let n = message.encode(&mut wire).unwrap();
        self.incoming = Some((wire, n));
    }
}

fn wide_handler(request: Request<'_, LongToken>) -> Response<'static, LongToken> {
    assert_eq!(request.token().len(), 32);
    Response::content(b"wide")
}

#[test]
fn extended_app_routes_and_echoes_complete_token() {
    let mut app = App::profile::<Wide>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .allow_unprotected()
        .route("value", get(wide_handler))
        .bind(Io::default())
        .unwrap();
    let mut token = [3; 32];
    token[31] = 91;
    app.transport_mut().receive(extended::Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: MessageId::new(8),
        token: &token,
        options: &[Opt::uri_path("value")],
        payload: &[],
    });
    app.poll(0).unwrap();
    let reply = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
    assert_eq!(reply.token(), token);
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.payload(), b"wide");
}

#[test]
fn extended_client_matches_full_token_and_reclaims_cancelled_call() {
    let mut app = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    let call = app.get("value").to(PEER).send(0).unwrap();
    assert_eq!(call.token().len(), 32);
    let request =
        decode_with_token::<LongToken>(&app.transport().wire[..app.transport().len]).unwrap();
    let mid = request.message_id();
    let mut wrong = [0; 32];
    wrong.copy_from_slice(call.token().as_bytes());
    wrong[31] ^= 1;
    app.transport_mut().receive(extended::Message {
        ty: Type::Acknowledgement,
        code: Code::CONTENT,
        id: mid,
        token: &wrong,
        options: &[],
        payload: b"wrong",
    });
    app.poll(1).unwrap();
    assert!(app.take_response(call).is_none());
    app.transport_mut().receive(extended::Message {
        ty: Type::Acknowledgement,
        code: Code::CONTENT,
        id: mid,
        token: call.token().as_bytes(),
        options: &[],
        payload: b"right",
    });
    app.poll(2).unwrap();
    let response = app.take_response(call).unwrap().unwrap();
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(response.payload(), b"right");
    let cancelled = app.get("value").to(PEER).send(3).unwrap();
    assert!(app.cancel(cancelled));
    assert!(!app.cancel(cancelled));
    let next = app.get("value").to(PEER).send(4).unwrap();
    assert_ne!(next.token(), cancelled.token());
}

#[test]
fn token_length_configuration_refuses_capacity_and_entropy_reduction() {
    for length in [0, 7, 33, 65805] {
        let result = App::profile::<Wide>()
            .block_wise::<false>()
            .token_length(length)
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(Io::default());
        assert!(matches!(result, Err(coaptic::BuildError::TokenLength)));
    }
    let mut app = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .allow_unprotected()
        .bind(Io::default())
        .unwrap();
    assert_eq!(app.get("value").to(PEER).send(0).unwrap().token().len(), 32);
}

fn capacity_refusal<S: coaptic::app::AppStorage<Token = LongToken>>(
    mut app: App<Wide, Io, 8, false, S>,
) {
    let large = [0x6b; 64];
    app.transport_mut().receive(extended::Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: MessageId::new(91),
        token: &large,
        options: &[Opt::uri_path("value")],
        payload: &[],
    });
    app.poll(0).unwrap();
    let reply = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
    assert_eq!(reply.code(), Code::BAD_REQUEST);
    assert_eq!(reply.ty(), Type::Acknowledgement);
    assert_eq!(reply.token(), large);
    assert_eq!(app.engine_mut().rx_occupied(), 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);

    app.transport_mut().receive(extended::Message {
        ty: Type::Confirmable,
        code: Code::GET,
        id: MessageId::new(92),
        token: &[0x6b; 32],
        options: &[Opt::uri_path("value")],
        payload: &[],
    });
    app.poll(1).unwrap();
    let reply = extended::decode(&app.transport().wire[..app.transport().len]).unwrap();
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.token(), [0x6b; 32]);
}

#[test]
fn oversized_token_refusal_echoes_borrowed_identity_then_recovers() {
    let app = App::profile::<Wide>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .allow_unprotected()
        .route("value", get(wide_handler))
        .bind(Io::default())
        .unwrap();
    capacity_refusal(app);
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_token_capacity_refusal_preserves_exact_identity_and_recovers() {
    let mut capacities = coaptic::storage::Capacities::from_profile::<Wide>();
    capacities.rx_body_slots = None;
    capacities.rx_body_bytes = None;
    capacities.tx_body_slots = None;
    capacities.tx_body_bytes = None;
    let app = App::profile::<Wide>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .allow_unprotected()
        .route("value", get(wide_handler))
        .bind_alloc(Io::default(), capacities)
        .unwrap();
    capacity_refusal(app);
}

#[cfg(feature = "oscore")]
fn context<K: coaptic::message::OwnedToken>(
    sender: &[u8],
    recipient: &[u8],
) -> coaptic::oscore::BoundedSecurityContext<4, K> {
    coaptic::oscore::BoundedSecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x42; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}

#[cfg(feature = "oscore")]
#[test]
fn protected_extended_token_roundtrip() {
    let mut client = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .oscore(context(&[1], &[2]))
        .bind(Io::default())
        .unwrap();
    let mut server = App::profile::<Wide>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context(&[2], &[1]))
        .route("value", get(wide_handler))
        .bind(Io::default())
        .unwrap();
    let call = client.get("value").to(PEER).send(0).unwrap();
    server.transport_mut().incoming = Some((client.transport().wire, client.transport().len));
    server.poll(0).unwrap();
    client.transport_mut().incoming = Some((server.transport().wire, server.transport().len));
    client.poll(1).unwrap();
    let response = client.take_response(call).unwrap().unwrap();
    assert_eq!(response.token(), Some(call.token()));
    assert_eq!(response.payload(), b"wide");
}

#[cfg(feature = "oscore")]
fn must_not_dispatch(_: Request<'_>) -> Response<'static> {
    panic!("over-capacity request reached handler")
}

#[cfg(feature = "oscore")]
#[test]
fn protected_capacity_refusal_authenticates_echoes_and_replays_exact_ciphertext() {
    let mut client = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .oscore(context(&[1], &[2]))
        .bind(Io::default())
        .unwrap();
    let mut server = App::profile::<coaptic::profiles::Default>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context(&[2], &[1]))
        .route("value", get(must_not_dispatch))
        .bind(Io::default())
        .unwrap();
    let call = client.get("value").to(PEER).send(0).unwrap();
    let original = (client.transport().wire, client.transport().len);
    let mut tampered = original;
    tampered.0[tampered.1 - 1] ^= 1;
    server.transport_mut().incoming = Some(tampered);
    server.poll(0).unwrap();
    assert_eq!(server.transport().len, 0);
    server.transport_mut().incoming = Some(original);
    server.poll(1).unwrap();
    let reply = (server.transport().wire, server.transport().len);
    let outer = extended::decode(&reply.0[..reply.1]).unwrap();
    assert_eq!(outer.token(), call.token().as_bytes());
    assert!(
        outer
            .options()
            .any(|option| option.number() == coaptic::message::OptionNumber::OSCORE)
    );
    server.transport_mut().incoming = Some(original);
    server.poll(2).unwrap();
    assert_eq!(
        &server.transport().wire[..server.transport().len],
        &reply.0[..reply.1]
    );
    client.transport_mut().incoming = Some(reply);
    client.poll(3).unwrap();
    let response = client.take_response(call).unwrap().unwrap();
    assert_eq!(response.code(), Code::BAD_REQUEST);
    assert_eq!(response.token(), Some(call.token()));
}

#[cfg(feature = "oscore")]
#[test]
fn protected_capacity_refusal_waits_for_replay_checkpoint() {
    let mut client = App::profile::<Wide>()
        .block_wise::<false>()
        .token_length(32)
        .deterministic_for_tests()
        .oscore(context(&[1], &[2]))
        .bind(Io::default())
        .unwrap();
    let mut server = App::profile::<coaptic::profiles::Default>()
        .block_wise::<false>()
        .deterministic_for_tests()
        .oscore(context(&[2], &[1]))
        .require_oscore_checkpoint()
        .route("value", get(must_not_dispatch))
        .bind(Io::default())
        .unwrap();
    let failed = client.get("value").to(PEER).send(0).unwrap();
    let original = (client.transport().wire, client.transport().len);
    server.transport_mut().incoming = Some(original);
    let mut commits = 0;
    assert!(matches!(
        server.poll_with_oscore_checkpoint(0, |_| {
            commits += 1;
            commits == 1
        }),
        Err(coaptic::app::Error::OscoreCheckpointFailed)
    ));
    assert_eq!(commits, 2);
    assert_eq!(server.transport().len, 0);
    assert_eq!(server.engine_mut().rx_occupied(), 0);
    assert_eq!(server.engine_mut().tx_occupied(), 0);

    // An ambiguous persistence failure never rolls back authenticated replay
    // state. Recover persistence, then use a fresh exchange to make progress.
    server.transport_mut().incoming = Some(original);
    server.poll_with_oscore_checkpoint(1, |_| true).unwrap();
    assert_eq!(server.transport().len, 0);
    assert!(client.cancel(failed));
    let fresh = client.get("value").to(PEER).send(2).unwrap();
    server.transport_mut().incoming = Some((client.transport().wire, client.transport().len));
    server.poll_with_oscore_checkpoint(2, |_| true).unwrap();
    client.transport_mut().incoming = Some((server.transport().wire, server.transport().len));
    client.poll(3).unwrap();
    let response = client.take_response(fresh).unwrap().unwrap();
    assert_eq!(response.code(), Code::BAD_REQUEST);
    assert_eq!(response.token(), Some(fresh.token()));
    assert_eq!(server.engine_mut().rx_occupied(), 0);
    assert_eq!(server.engine_mut().tx_occupied(), 0);
}
