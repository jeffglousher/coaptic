extern crate std;

use super::*;
use crate::message::{Code, Message, Opt, Token};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    vec::Vec,
};

const CLIENT: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const SERVER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const OTHER: Endpoint = Endpoint::v4([192, 0, 2, 3], 5683);
const BODY: &[u8] = &[0x5a; 240];

#[derive(Default)]
struct Network {
    a: VecDeque<(Vec<u8>, Endpoint)>,
    b: VecDeque<(Vec<u8>, Endpoint)>,
    sent: Vec<(Endpoint, Endpoint, Vec<u8>)>,
    receives: usize,
    fail: Option<Endpoint>,
}

struct Io {
    local: Endpoint,
    network: Rc<RefCell<Network>>,
}

impl DatagramIo for Io {
    type Error = u8;

    fn recv(&mut self, output: &mut [u8]) -> Result<Option<(usize, Endpoint)>, u8> {
        let mut network = self.network.borrow_mut();
        network.receives += 1;
        let queue = if self.local == CLIENT {
            &mut network.a
        } else {
            &mut network.b
        };
        let Some((bytes, from)) = queue.pop_front() else {
            return Ok(None);
        };
        assert!(bytes.len() <= output.len());
        output[..bytes.len()].copy_from_slice(&bytes);
        Ok(Some((bytes.len(), from)))
    }

    fn send(&mut self, to: Endpoint, bytes: &[u8]) -> Result<usize, u8> {
        let mut network = self.network.borrow_mut();
        network.sent.push((self.local, to, bytes.to_vec()));
        if network.fail == Some(self.local) {
            return Err(9);
        }
        let queue = if to == CLIENT {
            &mut network.a
        } else {
            &mut network.b
        };
        queue.push_back((bytes.to_vec(), self.local));
        Ok(bytes.len())
    }
}

type Owner<'a> = ManagedConnection<'a, Io, profiles::Constrained, 2>;

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

#[test]
fn local_pairing_connects_after_restart_and_keeps_management_on_owner_path() {
    use crate::provisioning::{EnrollmentPolicy, LocalPairing, PairingRecord};
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let local = |device: &Identity, peer: &Identity, store| {
        let record = PairingRecord::unpaired(device, [store; 32]).unwrap();
        let mut pairing = LocalPairing::restore(device, record).unwrap();
        let challenge = pairing.begin(device, 0, entropy).unwrap();
        pairing
            .pair(
                &challenge,
                EnrollmentPolicy::new(peer.peer(), [7; 32], [8; 32]).unwrap(),
                1,
                |_, _| Ok::<_, ()>(()),
            )
            .unwrap();
        pairing
    };
    let pa = local(&a, &b, 3);
    let pb = local(&b, &a, 4);
    let mut a_record = [0; 224];
    let mut b_record = [0; 224];
    let a_len = pa.checkpoint().encode(&mut a_record).unwrap();
    let b_len = pb.checkpoint().encode(&mut b_record).unwrap();
    for _ in 0..2 {
        let pa =
            LocalPairing::restore(&a, PairingRecord::decode(&a_record[..a_len]).unwrap()).unwrap();
        let pb =
            LocalPairing::restore(&b, PairingRecord::decode(&b_record[..b_len]).unwrap()).unwrap();
        let network = Rc::new(RefCell::new(Network::default()));
        let mut client: Owner<'_> = ManagedConnection::from_local_pairing(
            &a,
            &pa,
            SERVER,
            Io {
                local: CLIENT,
                network: network.clone(),
            },
            entropy,
            0,
        )
        .unwrap();
        let mut server: Owner<'_> = ManagedConnection::from_local_pairing(
            &b,
            &pb,
            CLIENT,
            Io {
                local: SERVER,
                network,
            },
            entropy,
            0,
        )
        .unwrap();
        assert_eq!(
            client.policy_validity(),
            PolicyValidity::OfflineLocalStorage
        );
        let now = connect(&mut client, &mut server, 0) + 1;
        let call = client
            .request(Method::Get, "paired", &[], None, now)
            .unwrap();
        client.poll_with(now, idle).unwrap();
        server
            .poll_with(now, |principal, _, _| {
                assert_eq!(*principal, a.peer().principal());
                Response::content(b"durable pairing")
            })
            .unwrap();
        client.poll_with(now, idle).unwrap();
        let mut bytes = [0; 32];
        assert_eq!(
            client
                .take_response_into(call, &mut bytes, now)
                .unwrap()
                .unwrap()
                .unwrap()
                .payload(),
            b"durable pairing"
        );
        assert!(matches!(
            server.revoke(|_, _, _| Ok::<_, ()>(())),
            Err(TrustCommitError::State(TrustError::LocalPolicy))
        ));
        assert!(server.is_connected());
    }
}

const BOOTSTRAP_MID_START: u16 = 0x1000;
const APP_MID_START: u16 = 0x8000;
const MID_STRIDE: u16 = 0x100;
std::thread_local! {
    static MID_SEEDS: Cell<(u16, u16)> = const { Cell::new((BOOTSTRAP_MID_START, APP_MID_START)) };
}

fn reset_mid_seeds() {
    MID_SEEDS.with(|seeds| seeds.set((BOOTSTRAP_MID_START, APP_MID_START)));
}

fn entropy(bytes: &mut [u8]) -> bool {
    if getrandom::fill(bytes).is_err() {
        return false;
    }
    // Success fixtures must not randomly collide with retained bootstrap or
    // prior-epoch MIDs. Only MID seed bytes are controlled: EDHOC key material,
    // Tokens and jitter retain system entropy. Each test owns its thread state.
    if matches!(bytes.len(), 2 | 22) {
        MID_SEEDS.with(|seeds| {
            let (mut bootstrap, mut app) = seeds.get();
            let seed = if bytes.len() == 22 {
                &mut bootstrap
            } else {
                &mut app
            };
            bytes[..2].copy_from_slice(&seed.to_be_bytes());
            *seed = seed
                .checked_add(MID_STRIDE)
                .expect("test MID seeds exhausted");
            assert!(bootstrap < APP_MID_START, "test MID ranges overlap");
            seeds.set((bootstrap, app));
        });
    }
    true
}

fn trust(identity: &Identity, peer: &Identity, store: u8) -> PeerTrust {
    PeerTrust::install(
        identity,
        peer.peer().clone(),
        TrustAnchor::genesis([store; 32]).unwrap(),
        |_, _, _| Ok::<_, u8>(()),
    )
    .unwrap()
}

fn pair<'a>(a: &'a Identity, b: &'a Identity) -> (Owner<'a>, Owner<'a>, Rc<RefCell<Network>>) {
    let network = Rc::new(RefCell::new(Network::default()));
    let client = Owner::new(
        a,
        trust(a, b, 1),
        SERVER,
        Io {
            local: CLIENT,
            network: network.clone(),
        },
        entropy,
        PolicyValidity::OfflineProtectedAnchor,
        0,
    )
    .unwrap();
    let server = Owner::new(
        b,
        trust(b, a, 2),
        CLIENT,
        Io {
            local: SERVER,
            network: network.clone(),
        },
        entropy,
        PolicyValidity::OfflineProtectedAnchor,
        0,
    )
    .unwrap();
    (client, server, network)
}

fn idle(_: &Principal, _: Request<'_>, _: Option<ManagedDeferredReply>) -> Response<'static> {
    panic!("bootstrap must not dispatch application effects")
}

fn connect(client: &mut Owner<'_>, server: &mut Owner<'_>, now: u64) -> u64 {
    client.connect(now).unwrap();
    for step in 0..12 {
        client.poll_with(now + step, idle).unwrap();
        server.poll_with(now + step, idle).unwrap();
        if client.is_connected()
            && server.is_connected()
            && !client.recovery.as_ref().unwrap().has_candidate()
            && !server.recovery.as_ref().unwrap().has_candidate()
        {
            return now + step;
        }
    }
    panic!("paired managed handshake did not complete")
}

#[test]
fn managed_scheduling_queries_do_not_drive_io_or_advance_time() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, _, network) = pair(&a, &b);
    assert_eq!(client.next_timeout(), None);
    assert_eq!(client.next_poll_at(0), None);
    client.connect(10).unwrap();
    assert_eq!(client.next_timeout(), Some(10));
    assert_eq!(client.next_poll_at(20), Some(20));
    assert_eq!(client.last_now, 10);
    assert_eq!(network.borrow().receives, 0);
    assert!(network.borrow().sent.is_empty());
    client.poll(10).unwrap();
    assert!(client.next_timeout().unwrap() > 10);
    assert_eq!(client.next_poll_at(10), client.next_timeout());
}

#[test]
fn managed_scheduling_expires_policy_and_clock_before_io_then_stops_waking() {
    for rollback in [false, true] {
        let a = identity(1, 1);
        let b = identity(2, 2);
        let (mut client, _, network) = pair(&a, &b);
        client.validity = PolicyValidity::OnlineUntil {
            accepted_ms: 0,
            deadline_ms: 50,
        };
        client.poll(10).unwrap();
        assert_eq!(client.next_timeout(), Some(50));
        let receives = network.borrow().receives;
        let now = if rollback { 9 } else { 50 };
        assert_eq!(client.next_poll_at(now), Some(now));
        assert!(matches!(
            client.poll(now),
            Err(ManagedError::Clock | ManagedError::PolicyExpired)
        ));
        assert_eq!(network.borrow().receives, receives);
        assert!(client.is_blocked());
        assert_eq!(client.next_timeout(), None);
        assert_eq!(client.next_poll_at(now), None);
    }
}

#[test]
fn managed_scheduling_omits_paused_app_timers_but_keeps_queued_ingress_ready() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    client
        .request(Method::Post, "work", &[], None, now)
        .unwrap();
    let mut deferred = None;
    server
        .poll_with(now, |_, _, handle| {
            deferred = handle;
            Response::deferred()
        })
        .unwrap();
    assert!(deferred.is_some());
    client.poll(now).unwrap();
    let due = match server.state.as_ref().unwrap() {
        State::Established(app) => app.next_timeout().unwrap(),
        _ => unreachable!(),
    };
    assert!(server.next_timeout().unwrap() <= due);
    server.connect(due - 1).unwrap();
    server.poll_with(due, idle).unwrap();
    assert!(server.recovery.as_ref().unwrap().has_candidate());
    assert!(server.next_timeout().unwrap() > due);
    assert!(server.next_poll_at(due).unwrap() > due);
    state_io(server.state.as_mut().unwrap()).queued = Some((0, OTHER));
    let receives = network.borrow().receives;
    assert_eq!(server.next_poll_at(due), Some(due));
    assert_eq!(network.borrow().receives, receives);
    state_io(server.state.as_mut().unwrap()).queued = None;
    server
        .recovery
        .as_mut()
        .unwrap()
        .discard_candidate(due)
        .unwrap();
    assert_eq!(server.next_poll_at(due), Some(due));
}

#[test]
fn final_error_after_handoff_cannot_revoke_the_installed_managed_app() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let generation = server.trust().checkpoint().anchor();
    let id = server
        .recovery
        .as_ref()
        .unwrap()
        .active_connection_id()
        .unwrap();
    let mut payload = id.lakers().as_cbor().to_vec();
    payload.extend_from_slice(&[0, 0xf5]);
    let options = [
        Opt::uri_path(".well-known"),
        Opt::uri_path("edhoc"),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[65]),
    ];
    let mut bytes = [0; DATAGRAM_BYTES];
    let len = Message::new(Type::Confirmable, Code::POST, MessageId::new(30_000))
        .with_token(Token::from_checked(b"late"))
        .with_options(&options)
        .with_payload(&payload)
        .encode(&mut bytes)
        .unwrap();
    network
        .borrow_mut()
        .b
        .push_back((bytes[..len].to_vec(), CLIENT));
    assert!(matches!(
        server.poll_with(now, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::PeerAbort(super::super::PeerErrorCode::ReservedSuccess)
        )))
    ));
    assert!(server.is_connected());
    assert_eq!(server.trust().checkpoint().anchor(), generation);
    assert_eq!(
        server.recovery.as_ref().unwrap().active_connection_id(),
        Some(id)
    );
    assert!(!server.recovery.as_ref().unwrap().has_candidate());
    let reply = network.borrow_mut().a.pop_front().unwrap().0;
    assert_eq!(decode(&reply).unwrap().code(), Code::CHANGED);
    let call = client
        .request(Method::Get, "alive", &[], None, now + 1)
        .unwrap();
    client.poll_with(now + 1, idle).unwrap();
    server
        .poll_with(now + 1, |principal, _, _| {
            assert_eq!(principal, &a.peer().principal());
            Response::content(b"authorized")
        })
        .unwrap();
    client.poll_with(now + 1, idle).unwrap();
    let mut body = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut body, now + 1)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"authorized"
    );
}

#[test]
fn initiator_error_ack_routes_without_content_format_and_releases_nstart() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let generation = client.trust().checkpoint().anchor();
    client.connect(now).unwrap();
    client.poll_with(now, idle).unwrap();
    server.poll_with(now, idle).unwrap();
    *network
        .borrow_mut()
        .a
        .front_mut()
        .unwrap()
        .0
        .last_mut()
        .unwrap() ^= 1;
    assert!(matches!(
        client.poll_with(now + 1, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::Provisioning(super::super::Error::Authentication)
        )))
    ));
    assert!(client.is_connected());
    assert_eq!(client.trust().checkpoint().anchor(), generation);
    assert!(client.recovery.as_ref().unwrap().has_candidate());
    assert!(matches!(
        client.request(Method::Get, "alive", &[], None, now + 1),
        Err(ManagedError::Busy)
    ));
    assert!(matches!(
        server.poll_with(now + 2, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::PeerAbort(super::super::PeerErrorCode::Unspecified)
        )))
    ));
    let reply = network.borrow().a.front().unwrap().0.clone();
    let p = decode(&reply).unwrap();
    assert_eq!(p.code(), Code::CHANGED);
    assert!(p.options().next().is_none());
    assert!(p.payload().is_empty());
    client.poll_with(now + 3, idle).unwrap();
    assert!(!client.recovery.as_ref().unwrap().has_candidate());
    let call = client
        .request(Method::Get, "alive", &[], None, now + 3)
        .unwrap();
    client.poll_with(now + 3, idle).unwrap();
    server
        .poll_with(now + 3, |principal, _, _| {
            assert_eq!(principal, &a.peer().principal());
            Response::content(b"authorized")
        })
        .unwrap();
    client.poll_with(now + 3, idle).unwrap();
    let mut body = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut body, now + 3)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"authorized"
    );
    assert!(client.is_connected());
    assert!(server.is_connected());
}

#[test]
fn peer_error_managed_routing_preserves_authorized_app_and_allows_explicit_recovery() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let generation = client.trust().checkpoint().anchor();
    client.connect(now).unwrap();
    client.poll_with(now, idle).unwrap();
    let (m1, _) = network.borrow_mut().b.pop_front().unwrap();
    let p = decode(&m1).unwrap();
    let options = [Opt::new(OptionNumber::CONTENT_FORMAT, &[64])];
    let mut bytes = [0; DATAGRAM_BYTES];
    let len = Message::new(Type::Confirmable, Code::BAD_REQUEST, MessageId::new(30_000))
        .with_token(p.token())
        .with_options(&options)
        .with_payload(&[1, 0x63, b'b', b'a', b'd'])
        .encode(&mut bytes)
        .unwrap();
    network
        .borrow_mut()
        .a
        .push_back((bytes[..len].to_vec(), SERVER));
    assert!(matches!(
        client.poll_with(now + 1, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::PeerAbort(super::super::PeerErrorCode::Unspecified)
        )))
    ));
    assert!(client.is_connected());
    assert_eq!(client.trust().checkpoint().anchor(), generation);
    assert!(!client.recovery.as_ref().unwrap().has_candidate());
    assert_eq!(client.peer_error().unwrap().diagnostic(), Some("bad"));
    assert_eq!(
        network.borrow_mut().b.pop_front().unwrap().0,
        [0x60, 0, 0x75, 0x30]
    );
    let call = client
        .request(Method::Get, "alive", &[], None, now + 2)
        .unwrap();
    // Retained error ACKs are transport work, not a second outgoing CON.
    let ack_count = network
        .borrow()
        .sent
        .iter()
        .filter(|(from, to, packet)| {
            *from == CLIENT && *to == SERVER && packet == &[0x60, 0, 0x75, 0x30]
        })
        .count();
    network
        .borrow_mut()
        .a
        .push_back((bytes[..len].to_vec(), SERVER));
    client.poll_with(now + 2, idle).unwrap();
    assert_eq!(
        network
            .borrow()
            .sent
            .iter()
            .filter(|(from, to, packet)| {
                *from == CLIENT && *to == SERVER && packet == &[0x60, 0, 0x75, 0x30]
            })
            .count(),
        ack_count + 1
    );
    server
        .poll_with(now + 2, |principal, _, _| {
            assert_eq!(principal, &a.peer().principal());
            Response::content(b"authorized")
        })
        .unwrap();
    client.poll_with(now + 2, idle).unwrap();
    let mut body = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut body, now + 2)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"authorized"
    );
    assert!(matches!(
        client.connect(now + 4),
        Err(ManagedError::Recovery(RecoveryError::Capacity))
    ));
    connect(
        &mut client,
        &mut server,
        now + 2 + super::super::CoapProvisioner::EXCHANGE_LIFETIME_MS,
    );
    assert!(client.is_connected());
}

fn snapshot() -> Response<'static> {
    Response::content(b"snapshot").max_age(10)
}

#[test]
fn manages_a_fresh_exchange_and_collects_complete_protected_payload_and_metadata() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client
        .request(
            Method::Post,
            "telemetry/device-a",
            b"temperature=24",
            Some(ContentFormat::TEXT_PLAIN),
            now,
        )
        .unwrap();
    let before = network.borrow().receives;
    let mut effects = 0;
    server
        .poll_with(now, |principal, request, _| {
            assert_eq!(principal, &a.peer().principal());
            assert_eq!(request.method(), Some(Method::Post));
            assert_eq!(request.path(), &["telemetry", "device-a"]);
            assert_eq!(request.payload(), b"temperature=24");
            effects += 1;
            Response::content(BODY)
                .content_format(ContentFormat::OCTET_STREAM)
                .max_age(42)
                .etag(b"v1")
        })
        .unwrap();
    assert_eq!(network.borrow().receives - before, 1);
    client.poll_with(now, idle).unwrap();
    assert_eq!(effects, 1);
    let mut small = [0xa5; 16];
    assert!(matches!(
        client.take_response_into(call, &mut small, now),
        Err(ManagedError::ResponseBuffer(
            ResponseBufferError::TooSmall {
                required: 240,
                available: 16
            }
        ))
    ));
    assert_eq!(small, [0xa5; 16]);
    let mut output = [0; 300];
    let response = client
        .take_response_into(call, &mut output, now)
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.payload(), BODY);
    assert!(!response.payload_truncated());
    assert_eq!(response.format(), Some(ContentFormat::OCTET_STREAM));
    assert_eq!(response.max_age_secs(), Some(42));
    assert_eq!(
        response
            .received_options()
            .find(|option| option.number() == OptionNumber::ETAG)
            .unwrap()
            .value(),
        b"v1"
    );
    let packets = &network.borrow().sent;
    let protected = packets
        .iter()
        .filter(|(_, _, bytes)| {
            decode(bytes)
                .unwrap()
                .options()
                .any(|option| option.number() == OptionNumber::OSCORE)
        })
        .count();
    assert_eq!(protected, 2);
    assert!(
        packets
            .iter()
            .all(|(_, _, bytes)| !bytes.windows(14).any(|part| part == b"temperature=24"))
    );
}

#[test]
fn revocation_cancels_queued_bootstrap_and_application_retries_before_any_io() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client
        .request(Method::Get, "config", &[], None, now)
        .unwrap();
    assert!(matches!(client.connect(now), Err(ManagedError::Busy)));
    client.revoke(|_, _, _| Ok::<_, u8>(())).unwrap();
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(!client.is_connected());
    assert!(matches!(
        client.poll_with(now + 10_000, idle),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    assert!(matches!(
        client.request(Method::Get, "config", &[], None, now + 10_000),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    assert!(matches!(
        client.cancel(call, now + 10_000),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    let mut output = [0; 32];
    assert!(matches!(
        client.take_response_into(call, &mut output, now + 10_000),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);
    assert!(client.recovery.is_none());

    let (mut waiting, _, network) = pair(&a, &b);
    waiting.connect(0).unwrap();
    waiting.revoke(|_, _, _| Ok::<_, u8>(())).unwrap();
    assert!(waiting.poll_with(0, idle).is_err());
    assert!(network.borrow().sent.is_empty());
    assert_eq!(network.borrow().receives, 0);
}

#[test]
fn revocation_blocks_cached_replies_and_deferred_effects_and_completion() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client
        .request(Method::Post, "actuator", b"set=4", None, now)
        .unwrap();
    let request = network.borrow().sent.last().unwrap().2.clone();
    let mut deferred = None;
    server
        .poll_with(now, |_, _, handle| {
            deferred = handle;
            Response::deferred()
        })
        .unwrap();
    let deferred = deferred.unwrap();
    client.poll_with(now, idle).unwrap();
    let mut effects = 0;
    server
        .with_authorized_effect(deferred, now, |principal| {
            assert_eq!(principal, &a.peer().principal());
            effects += 1;
        })
        .unwrap();
    assert_eq!(effects, 1);
    network.borrow_mut().b.push_back((request, CLIENT));
    server.revoke(|_, _, _| Ok::<_, u8>(())).unwrap();
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(server.poll_with(now + 1, idle).is_err());
    assert!(
        server
            .with_authorized_effect(deferred, now + 1, |_| effects += 1)
            .is_err()
    );
    assert!(
        server
            .complete(deferred, Response::changed(), now + 1)
            .is_err()
    );
    assert_eq!(effects, 1);
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);
    assert!(client.cancel(call, now + 1).unwrap());
}

#[test]
fn managed_observe_mid_collision_refuses_before_io_and_recovers_capacity() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    server.observable("temperature", snapshot).unwrap();
    let now = connect(&mut client, &mut server, 0) + 1;
    let refusals = client.app_mut().unwrap().client_capacity() + 1;
    // Either endpoint can install first. Reserve both deterministic App seed
    // ranges as live bootstrap MIDs, forcing more refusals than Call capacity.
    let io = state_io(client.state.as_mut().unwrap());
    for first in [APP_MID_START, APP_MID_START + MID_STRIDE] {
        for offset in 0..refusals {
            let slot = io.bootstrap.iter_mut().find(|slot| slot.is_none()).unwrap();
            *slot = Some((MessageId::new(first + offset as u16), now + LIFETIME));
        }
    }
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    for _ in 0..refusals {
        assert!(matches!(
            client.observe("temperature", now),
            Err(ManagedError::Application(crate::app::Error::Io(
                crate::storage::DatagramIoError::Io(ManagedIoError::MessageIdReserved)
            )))
        ));
        assert_eq!(network.borrow().sent.len(), sends);
        assert_eq!(network.borrow().receives, receives);
        assert_eq!(client.app_mut().unwrap().engine_mut().tx_occupied(), 0);
        assert_eq!(client.app_mut().unwrap().engine_mut().rx_occupied(), 0);
    }
    let call = client.observe("temperature", now).unwrap();
    assert_eq!(network.borrow().sent.len(), sends + 1);
    server.poll_with(now, |_, _, _| snapshot()).unwrap();
    client.poll_with(now, idle).unwrap();
    let mut output = [0; 32];
    let response = client
        .take_response_into(call, &mut output, now)
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.observe_seq(), Some(0));
    assert_eq!(response.payload(), b"snapshot");
    assert!(client.cancel(call, now).unwrap());
    assert_eq!(client.app_mut().unwrap().engine_mut().tx_occupied(), 0);
    assert_eq!(client.app_mut().unwrap().engine_mut().rx_occupied(), 0);
}

#[test]
fn observer_notifications_cross_the_same_gate_and_do_not_survive_regrant() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    server.observable("temperature", snapshot).unwrap();
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client.observe("temperature", now).unwrap();
    server.poll_with(now, |_, _, _| snapshot()).unwrap();
    client.poll_with(now, idle).unwrap();
    let mut output = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut output, now)
            .unwrap()
            .unwrap()
            .unwrap()
            .observe_seq(),
        Some(0)
    );
    assert_eq!(
        server
            .notify(
                &["temperature"],
                Response::content(b"new").max_age(10),
                now + 1
            )
            .unwrap(),
        1
    );
    server.revoke(|_, _, _| Ok::<_, u8>(())).unwrap();
    let sends = network.borrow().sent.len();
    assert!(
        server
            .notify(&["temperature"], snapshot(), now + 2)
            .is_err()
    );
    assert!(server.poll_with(now + 50_000, idle).is_err());
    assert_eq!(network.borrow().sent.len(), sends);
    server
        .replace(a.peer().clone(), true, |_, _, _| Ok::<_, u8>(()))
        .unwrap();
    assert!(!server.is_connected());
    assert!(matches!(
        server.notify(&["temperature"], snapshot(), now + 50_001),
        Err(ManagedError::Trust(TrustError::StaleGrant))
    ));
}

#[test]
fn ambiguous_policy_commit_and_failed_authority_refresh_fail_closed() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0);
    assert!(matches!(
        client.replace(b.peer().clone(), true, |_, _, _| Err(7)),
        Err(TrustCommitError::Persistence(7))
    ));
    assert!(client.trust().is_blocked());
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(matches!(
        client.poll_with(now + 1, idle),
        Err(ManagedError::Trust(TrustError::Blocked))
    ));
    assert!(matches!(
        client.connect(now + 1),
        Err(ManagedError::Trust(TrustError::Blocked))
    ));
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);

    assert!(matches!(
        server.refresh(now + 1, |_| Err::<(PeerTrust, PolicyValidity), _>(8)),
        Err(ManagedRefreshError::Authority(8))
    ));
    assert!(!server.is_connected());
    assert!(matches!(
        server.poll_with(now + 1, idle),
        Err(ManagedError::Trust(TrustError::Blocked))
    ));
    let checkpoint = server.trust().checkpoint().clone();
    let anchor = checkpoint.anchor();
    server
        .refresh(now + 1, |identity| {
            PeerTrust::restore(identity, checkpoint, anchor)
                .map(|trust| (trust, PolicyValidity::OfflineProtectedAnchor))
        })
        .unwrap();
    assert!(!server.is_connected());
    assert!(matches!(
        server.request(Method::Get, "fresh", &[], None, now + 2),
        Err(ManagedError::Trust(TrustError::StaleGrant))
    ));
}

#[test]
fn authenticated_fresh_handoff_invalidates_old_call_and_deferred_handle_epochs() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let old_call = client
        .request(Method::Post, "work", &[], None, now)
        .unwrap();
    let mut old_deferred = None;
    server
        .poll_with(now, |_, _, handle| {
            old_deferred = handle;
            Response::deferred()
        })
        .unwrap();
    client.poll_with(now, idle).unwrap();
    let old_deferred = old_deferred.unwrap();
    let now = connect(&mut client, &mut server, now + 1) + 1;
    let mut output = [0; 32];
    assert!(matches!(
        client.take_response_into(old_call, &mut output, now),
        Err(ManagedError::StaleHandle)
    ));
    assert!(matches!(
        client.cancel(old_call, now),
        Err(ManagedError::StaleHandle)
    ));
    assert!(matches!(
        server.complete(old_deferred, Response::changed(), now),
        Err(ManagedError::StaleHandle)
    ));
    let sends = network.borrow().sent.len();
    let mut effects = 0;
    assert!(matches!(
        server.with_authorized_effect(old_deferred, now, |_| effects += 1),
        Err(ManagedError::StaleHandle)
    ));
    assert_eq!(effects, 0);
    assert_eq!(network.borrow().sent.len(), sends);
}

#[test]
fn unauthenticated_candidate_failure_preserves_the_old_authenticated_app() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let attacker = identity(3, 1);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    let (initiator, first) =
        super::super::Initiator::start(&attacker, b.peer().clone(), entropy).unwrap();
    let bytes = [0; DATAGRAM_BYTES];
    let mut packet = bytes;
    let encoded = ContentFormat::new(65).encode();
    let options = [
        Opt::uri_path(".well-known"),
        Opt::uri_path("edhoc"),
        Opt::content_format(&encoded),
    ];
    let mid = MessageId::new(30_000);
    let payload: Vec<_> = core::iter::once(0xf5)
        .chain(first.as_bytes().iter().copied())
        .collect();
    let len = Message::new(Type::Confirmable, Code::POST, mid)
        .with_token(Token::from_checked(b"attacker"))
        .with_options(&options)
        .with_payload(&payload)
        .encode(&mut packet)
        .unwrap();
    network
        .borrow_mut()
        .b
        .push_back((packet[..len].to_vec(), CLIENT));
    server.poll_with(now, idle).unwrap();
    assert!(server.is_connected());
    assert!(server.recovery.as_ref().unwrap().has_candidate());
    let message2 = network.borrow_mut().a.pop_front().unwrap().0;
    let decoded = decode(&message2).unwrap();
    let (confirmation, third) = initiator
        .receive_message_2(
            &super::super::Message::from_slice(decoded.payload()).unwrap(),
            |_| true,
        )
        .unwrap();
    let payload: Vec<_> = confirmation
        .peer_connection_id()
        .lakers()
        .as_cbor()
        .iter()
        .copied()
        .chain(third.as_bytes().iter().copied())
        .collect();
    let len = Message::new(Type::Confirmable, Code::POST, mid.wrapping_add(1))
        .with_token(Token::from_checked(b"attacke2"))
        .with_options(&options)
        .with_payload(&payload)
        .encode(&mut packet)
        .unwrap();
    network
        .borrow_mut()
        .b
        .push_back((packet[..len].to_vec(), CLIENT));
    assert!(matches!(
        server.poll_with(now + 1, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::Provisioning(_)
        )))
    ));
    assert!(server.is_connected());
    assert!(!server.recovery.as_ref().unwrap().has_candidate());
    let refusal = network.borrow_mut().a.pop_front().unwrap().0;
    let refused = decode(&refusal).unwrap();
    assert_eq!(refused.code(), Code::BAD_REQUEST);
    assert_eq!(refused.payload(), b"\x01\x6cEDHOC failed");
    let call = client
        .request(Method::Get, "still-authorized", &[], None, now + 2)
        .unwrap();
    server
        .poll_with(now + 2, |principal, _, _| {
            assert_eq!(principal, &a.peer().principal());
            Response::content(b"alive")
        })
        .unwrap();
    client.poll_with(now + 2, idle).unwrap();
    let mut output = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut output, now + 2)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"alive"
    );
}

#[test]
fn transport_failures_and_wrong_endpoint_do_not_create_an_authenticated_owner() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    network.borrow_mut().fail = Some(CLIENT);
    client.connect(0).unwrap();
    assert!(matches!(
        client.poll_with(0, idle),
        Err(ManagedError::Recovery(RecoveryError::Bootstrap(
            super::super::PollError::Io(ManagedIoError::Transport(9))
        )))
    ));
    let request = network.borrow().sent.last().unwrap().2.clone();
    network.borrow_mut().fail = None;
    network.borrow_mut().b.push_back((request, OTHER));
    server.poll_with(0, idle).unwrap();
    assert!(!server.is_connected());
    assert!(!server.recovery.as_ref().unwrap().has_candidate());
    client.poll_with(1, idle).unwrap();
    server.poll_with(1, idle).unwrap();
    client.poll_with(2, idle).unwrap();
    server.poll_with(2, idle).unwrap();
    client.poll_with(3, idle).unwrap();
    assert!(client.is_connected());
    assert!(server.is_connected());
}

#[test]
fn bounded_mid_ownership_refuses_bootstrap_app_and_retired_session_collisions() {
    reset_mid_seeds();
    let network = Rc::new(RefCell::new(Network::default()));
    let mut io = ConnectionIo::new(
        Some(Io {
            local: CLIENT,
            network: network.clone(),
        }),
        0,
    );
    io.epoch = 1;
    let packet = |mid| {
        let mut bytes = [0; 32];
        let n = Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
            .encode(&mut bytes)
            .unwrap();
        bytes[..n].to_vec()
    };
    io.send(SERVER, &packet(100)).unwrap();
    io.send(SERVER, &packet(103)).unwrap();
    io.bootstrap_mode = true;
    assert!(matches!(
        io.send(SERVER, &packet(102)),
        Err(ManagedIoError::MessageIdReserved)
    ));
    io.send(SERVER, &packet(200)).unwrap();
    io.bootstrap_mode = false;
    assert!(matches!(
        io.send(SERVER, &packet(200)),
        Err(ManagedIoError::MessageIdReserved)
    ));
    io.epoch = 2;
    assert!(matches!(
        io.send(SERVER, &packet(100)),
        Err(ManagedIoError::MessageIdReserved)
    ));
    let sends = network.borrow().sent.len();
    io.prune(LIFETIME);
    io.send(SERVER, &packet(100)).unwrap();
    assert_eq!(network.borrow().sent.len(), sends + 1);
}

#[test]
fn transport_reservation_capacity_refuses_before_send_and_recovers_after_expiry() {
    reset_mid_seeds();
    let network = Rc::new(RefCell::new(Network::default()));
    let mut io = ConnectionIo::new(
        Some(Io {
            local: CLIENT,
            network: network.clone(),
        }),
        0,
    );
    let send = |io: &mut ConnectionIo<Io>, mid| {
        let mut bytes = [0; 8];
        let len = Message::con(Code::GET, MessageId::new(mid), Token::EMPTY)
            .encode(&mut bytes)
            .unwrap();
        io.send(SERVER, &bytes[..len])
    };

    io.bootstrap_mode = true;
    for mid in 0..BOOTSTRAP_MIDS as u16 {
        send(&mut io, mid).unwrap();
    }
    let sends = network.borrow().sent.len();
    assert!(matches!(
        send(&mut io, BOOTSTRAP_MIDS as u16),
        Err(ManagedIoError::Capacity)
    ));
    assert_eq!(network.borrow().sent.len(), sends);
    send(&mut io, 0).unwrap(); // Retransmission retains its original reservation.

    io.bootstrap_mode = false;
    for epoch in 1..=MID_RANGES as u64 {
        io.epoch = epoch;
        send(&mut io, 100 + epoch as u16).unwrap();
    }
    io.epoch += 1;
    assert!(!io.next_epoch_available());
    let sends = network.borrow().sent.len();
    assert!(matches!(send(&mut io, 200), Err(ManagedIoError::Capacity)));
    assert_eq!(network.borrow().sent.len(), sends);

    io.prune(LIFETIME - 1);
    assert!(!io.next_epoch_available());
    io.prune(LIFETIME);
    assert!(io.next_epoch_available());
    send(&mut io, 200).unwrap();
    io.bootstrap_mode = true;
    send(&mut io, 0).unwrap();
    assert_eq!(network.borrow().sent.len(), sends + 2);
}

#[test]
fn revoked_owner_never_reads_an_already_queued_datagram() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    client
        .request(Method::Put, "config", b"version=2", None, now)
        .unwrap();
    state_io(server.state.as_mut().unwrap()).receive().unwrap();
    assert!(state_io(server.state.as_mut().unwrap()).queued.is_some());
    server
        .replace(a.peer().clone(), true, |_, _, _| Ok::<_, u8>(()))
        .unwrap();
    assert!(state_io(server.state.as_mut().unwrap()).queued.is_none());
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(matches!(
        server.request(Method::Get, "config", &[], None, now),
        Err(ManagedError::Trust(TrustError::StaleGrant))
    ));
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);
}

#[test]
fn online_lease_expires_before_any_request_poll_notification_or_deferred_effect() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    client.validity = PolicyValidity::OnlineUntil {
        accepted_ms: 0,
        deadline_ms: 100,
    };
    server.validity = client.validity;
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client
        .request(Method::Post, "work", &[], None, now)
        .unwrap();
    let mut deferred = None;
    server
        .poll_with(now, |_, _, handle| {
            deferred = handle;
            Response::deferred()
        })
        .unwrap();
    client.poll_with(now, idle).unwrap();
    let deferred = deferred.unwrap();
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(matches!(
        client.poll_with(100, idle),
        Err(ManagedError::PolicyExpired)
    ));
    let mut effects = 0;
    assert!(matches!(
        server.with_authorized_effect(deferred, 100, |_| effects += 1),
        Err(ManagedError::PolicyExpired)
    ));
    assert!(matches!(
        client.request(Method::Get, "config", &[], None, 100),
        Err(ManagedError::PolicyExpired)
    ));
    assert!(matches!(
        server.notify(&["temperature"], snapshot(), 100),
        Err(ManagedError::PolicyExpired)
    ));
    assert!(matches!(
        server.complete(deferred, Response::changed(), 100),
        Err(ManagedError::PolicyExpired)
    ));
    let mut output = [0; 32];
    assert!(matches!(
        client.take_response_into(call, &mut output, 100),
        Err(ManagedError::PolicyExpired)
    ));
    assert!(matches!(
        client.cancel(call, 100),
        Err(ManagedError::PolicyExpired)
    ));
    assert_eq!(effects, 0);
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);
    assert!(client.is_blocked());
    assert!(server.is_blocked());
}

#[test]
fn clock_rollback_and_expired_refresh_cannot_resurrect_a_connection() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 10);
    let record = client.trust().checkpoint().clone();
    let anchor = record.anchor();
    let sends = network.borrow().sent.len();
    assert!(matches!(
        client.poll_with(now - 1, idle),
        Err(ManagedError::Clock)
    ));
    assert!(!client.is_connected());
    assert!(client.is_blocked());
    assert!(matches!(
        client.poll_with(now + 1, idle),
        Err(ManagedError::Trust(TrustError::Blocked))
    ));
    assert!(matches!(
        client.refresh(now + 2, |identity| PeerTrust::restore(
            identity,
            record.clone(),
            anchor
        )
        .map(|trust| (
            trust,
            PolicyValidity::OnlineUntil {
                accepted_ms: now,
                deadline_ms: now + 2
            }
        ))),
        Err(ManagedRefreshError::PolicyExpired)
    ));
    assert!(matches!(
        client.refresh(now + 2, |identity| PeerTrust::restore(
            identity,
            record.clone(),
            anchor
        )
        .map(|trust| (
            trust,
            PolicyValidity::OnlineUntil {
                accepted_ms: now + 3,
                deadline_ms: now + 100
            }
        ))),
        Err(ManagedRefreshError::Clock)
    ));
    assert!(client.is_blocked());
    assert_eq!(network.borrow().sent.len(), sends);
    client
        .refresh(now + 3, |identity| {
            PeerTrust::restore(identity, record, anchor).map(|trust| {
                (
                    trust,
                    PolicyValidity::OnlineUntil {
                        accepted_ms: now + 3,
                        deadline_ms: now + 100,
                    },
                )
            })
        })
        .unwrap();
    assert!(!client.is_connected());
    connect(&mut client, &mut server, now + 4);
    assert!(client.is_connected());
}

#[test]
fn deferred_effect_invocation_is_consumed_before_failure_and_rejects_expiry_or_cancel() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, _) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0) + 1;
    client
        .request(Method::Post, "work", &[], None, now)
        .unwrap();
    let mut deferred = None;
    server
        .poll_with(now, |_, _, handle| {
            deferred = handle;
            Response::deferred()
        })
        .unwrap();
    let deferred = deferred.unwrap();
    let mut effects = 0;
    assert_eq!(
        server
            .with_authorized_effect(deferred, now, |_| {
                effects += 1;
                Err::<(), _>(9)
            })
            .unwrap(),
        Err(9)
    );
    assert!(matches!(
        server.with_authorized_effect(deferred, now + 1, |_| effects += 1),
        Err(ManagedError::StaleHandle)
    ));
    assert!(server.cancel_deferred(deferred, now + 1).unwrap());
    assert!(matches!(
        server.with_authorized_effect(deferred, now + 1, |_| effects += 1),
        Err(ManagedError::StaleHandle)
    ));
    assert_eq!(effects, 1);
    client.poll_with(now + 1, idle).unwrap();
    client
        .request(Method::Post, "later", &[], None, now + 2)
        .unwrap();
    let mut later = None;
    server
        .poll_with(now + 2, |_, _, handle| {
            later = handle;
            Response::deferred()
        })
        .unwrap();
    let later = later.unwrap();
    assert!(matches!(
        server.with_authorized_effect(later, now + 2 + LIFETIME, |_| effects += 1),
        Err(ManagedError::StaleHandle)
    ));
    assert_eq!(effects, 1);
}

#[test]
fn externally_admitted_sessions_cannot_start_an_independent_bootstrap() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (initiator, first) = super::super::Initiator::start(&a, b.peer().clone(), entropy).unwrap();
    let (responder, second) =
        super::super::Responder::receive_message_1(&b, a.peer().clone(), &first, entropy).unwrap();
    let (confirmation, third) = initiator.receive_message_2(&second, |_| true).unwrap();
    let (server_session, fourth) = responder.receive_message_3(&third, |_| true).unwrap();
    let client_session = confirmation.receive_message_4(&fourth, |_| true).unwrap();
    let network = Rc::new(RefCell::new(Network::default()));
    let client_trust = trust(&a, &b, 1);
    let client_grant = client_trust.grant(&a).unwrap();
    let server_trust = trust(&b, &a, 2);
    let server_grant = server_trust.grant(&b).unwrap();
    let mut client = Owner::from_admitted_session(
        &a,
        client_trust,
        client_grant,
        client_session,
        SERVER,
        Io {
            local: CLIENT,
            network: network.clone(),
        },
        entropy,
        PolicyValidity::OfflineProtectedAnchor,
        0,
    )
    .unwrap();
    let mut server = Owner::from_admitted_session(
        &b,
        server_trust,
        server_grant,
        server_session,
        CLIENT,
        Io {
            local: SERVER,
            network: network.clone(),
        },
        entropy,
        PolicyValidity::OfflineProtectedAnchor,
        0,
    )
    .unwrap();
    assert!(client.is_connected());
    assert!(client.recovery.is_none());
    assert!(matches!(
        client.connect(0),
        Err(ManagedError::ExternalBootstrap)
    ));
    let call = client
        .request(Method::Get, "admitted", &[], None, 0)
        .unwrap();
    server
        .poll_with(0, |principal, _, _| {
            assert_eq!(principal, &a.peer().principal());
            Response::content(b"managed")
        })
        .unwrap();
    client.poll_with(0, idle).unwrap();
    let mut output = [0; 32];
    assert_eq!(
        client
            .take_response_into(call, &mut output, 0)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"managed"
    );
    server
        .replace(a.peer().clone(), true, |_, _, _| Ok::<_, u8>(()))
        .unwrap();
    assert!(!server.is_connected());
    assert!(matches!(
        server.poll_with(1, idle),
        Err(ManagedError::Trust(TrustError::StaleGrant))
    ));
    assert!(server.recovery.is_none());
}

#[test]
fn fresh_policy_revocation_is_installed_as_a_tombstone_and_blocks_all_io() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let now = connect(&mut client, &mut server, 0);
    let mut policy = PeerTrust::restore(
        &a,
        client.trust.checkpoint().clone(),
        client.trust.checkpoint().anchor(),
    )
    .unwrap();
    policy.revoke(|_, _, _| Ok::<_, u8>(())).unwrap();
    let current = policy.checkpoint().anchor();
    client
        .refresh(now + 1, |_| {
            Ok::<_, u8>((
                policy,
                PolicyValidity::OnlineUntil {
                    accepted_ms: now + 1,
                    deadline_ms: now + 100,
                },
            ))
        })
        .unwrap();
    assert!(!client.trust().checkpoint().enabled());
    assert_eq!(client.trust().checkpoint().anchor(), current);
    assert!(!client.is_connected());
    let sends = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert!(matches!(
        client.poll_with(now + 2, idle),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    assert!(matches!(
        client.connect(now + 2),
        Err(ManagedError::Trust(TrustError::Unauthorized))
    ));
    assert_eq!(network.borrow().sent.len(), sends);
    assert_eq!(network.borrow().receives, receives);
}

#[test]
fn online_validity_rejects_acceptance_clock_rollback_and_queued_bootstrap_expiry() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let network = Rc::new(RefCell::new(Network::default()));
    assert!(matches!(
        Owner::new(
            &a,
            trust(&a, &b, 1),
            SERVER,
            Io {
                local: CLIENT,
                network: network.clone()
            },
            entropy,
            PolicyValidity::OnlineUntil {
                accepted_ms: 10,
                deadline_ms: 20
            },
            9
        ),
        Err(ManagedError::Clock)
    ));
    let mut client = Owner::new(
        &a,
        trust(&a, &b, 1),
        SERVER,
        Io {
            local: CLIENT,
            network: network.clone(),
        },
        entropy,
        PolicyValidity::OnlineUntil {
            accepted_ms: 10,
            deadline_ms: 20,
        },
        10,
    )
    .unwrap();
    client.connect(10).unwrap();
    assert!(matches!(
        client.poll_with(20, idle),
        Err(ManagedError::PolicyExpired)
    ));
    assert!(client.recovery.is_none());
    assert!(client.is_blocked());
    assert!(network.borrow().sent.is_empty());
    assert_eq!(network.borrow().receives, 0);
}

#[test]
fn unconfigured_observe_paths_cannot_share_an_unknown_resource_identity() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, _) = pair(&a, &b);
    server.observable("configured", snapshot).unwrap();
    let now = connect(&mut client, &mut server, 0) + 1;
    let call = client.observe("unconfigured", now).unwrap();
    server
        .poll_with(now, |_, _, _| snapshot().observe(0))
        .unwrap();
    client.poll(now).unwrap();
    let mut output = [0; 32];
    let response = client
        .take_response_into(call, &mut output, now)
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.code(), Code::CONTENT);
    assert_eq!(response.observe_seq(), None);
    assert!(matches!(
        server.notify(&["unconfigured"], snapshot(), now),
        Err(ManagedError::Path)
    ));
    assert_eq!(server.notify(&["configured"], snapshot(), now).unwrap(), 0);
}

#[test]
fn desired_subscription_refreshes_reconnects_and_stops_at_grant_change() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    server.observable_manual("identity").unwrap();
    let mut desired = ManagedSubscription::new("identity");
    let mut now = connect(&mut client, &mut server, 0) + 1;
    let mut output = [0; 32];
    for round in 0..3 {
        assert!(client.poll_subscription(&mut desired, now).unwrap());
        server
            .poll_with(now, |_, request, _| {
                assert!(request.is_observe_register());
                Response::content(b"owner identity").max_age(1).observe(0)
            })
            .unwrap();
        client.poll(now).unwrap();
        let response = client
            .take_subscription_response_into(&mut desired, &mut output, now)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.payload(), b"owner identity");
        assert!(response.observe_seq().is_some());
        assert!(!client.poll_subscription(&mut desired, now + 1).unwrap());
        now += 16_001;
        if round == 1 {
            now = connect(&mut client, &mut server, now) + 1;
        }
    }
    network.borrow_mut().fail = Some(CLIENT);
    assert!(matches!(
        client.poll_subscription(&mut desired, now),
        Err(ManagedError::Application(crate::app::Error::Io(_)))
    ));
    network.borrow_mut().fail = None;
    assert!(!client.poll_subscription(&mut desired, now + 999).unwrap());
    now += 1000;
    assert!(client.poll_subscription(&mut desired, now).unwrap());
    server
        .poll_with(now, |_, _, _| Response::content(b"recovered").observe(0))
        .unwrap();
    client.poll(now).unwrap();
    assert_eq!(
        client
            .take_subscription_response_into(&mut desired, &mut output, now)
            .unwrap()
            .unwrap()
            .unwrap()
            .payload(),
        b"recovered"
    );
    client
        .replace(b.peer().clone(), true, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    server
        .replace(a.peer().clone(), true, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    now = connect(&mut client, &mut server, now) + 1;
    let sent = network.borrow().sent.len();
    assert!(matches!(
        client.poll_subscription(&mut desired, now),
        Err(ManagedError::StaleHandle)
    ));
    assert_eq!(network.borrow().sent.len(), sent);
    client.stop_subscription(&mut desired, now).unwrap();
    assert!(desired.is_stopped());
    assert!(
        !client
            .poll_subscription(&mut desired, now + 10_000)
            .unwrap()
    );
    assert_eq!(network.borrow().sent.len(), sent);
    assert!(core::mem::size_of::<ManagedSubscription>() <= 192);
}

#[test]
fn subscription_scheduling_retries_without_io_and_stops() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, network) = pair(&a, &b);
    let mut desired = ManagedSubscription::new("identity");
    assert_eq!(client.next_subscription_poll_at(&desired, 0), None);
    let now = connect(&mut client, &mut server, 0) + 1;
    assert_eq!(client.next_subscription_poll_at(&desired, now), Some(now));
    network.borrow_mut().fail = Some(CLIENT);
    assert!(client.poll_subscription(&mut desired, now).is_err());
    let sent = network.borrow().sent.len();
    let receives = network.borrow().receives;
    assert_eq!(
        client.next_subscription_poll_at(&desired, now),
        Some(now + 1000)
    );
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 999),
        Some(now + 1000)
    );
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 1001),
        Some(now + 1001)
    );
    assert_eq!(client.last_now, now);
    assert_eq!(network.borrow().sent.len(), sent);
    assert_eq!(network.borrow().receives, receives);
    network.borrow_mut().fail = None;
    assert!(client.poll_subscription(&mut desired, now + 1000).unwrap());
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 1000),
        Some(now + 31_000)
    );
    assert_eq!(client.next_subscription_poll_at(&desired, now), Some(now));
    client.stop_subscription(&mut desired, now + 1000).unwrap();
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 40_000),
        None
    );
    let desired = ManagedSubscription::new("identity");
    client.refresh_blocked = true;
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 40_000),
        None
    );
}

#[test]
fn subscription_scheduling_resumes_after_recovery_and_reports_changed_grant() {
    reset_mid_seeds();
    let a = identity(1, 1);
    let b = identity(2, 2);
    let (mut client, mut server, _) = pair(&a, &b);
    server.observable_manual("identity").unwrap();
    let mut desired = ManagedSubscription::new("identity");
    let now = connect(&mut client, &mut server, 0) + 1;
    assert!(client.poll_subscription(&mut desired, now).unwrap());
    server
        .poll_with(now, |_, _, _| {
            Response::content(b"identity").max_age(60).observe(0)
        })
        .unwrap();
    client.poll(now).unwrap();
    let mut output = [0; 32];
    client
        .take_subscription_response_into(&mut desired, &mut output, now)
        .unwrap()
        .unwrap()
        .unwrap();
    let due = client.next_subscription_poll_at(&desired, now).unwrap();
    assert!((now + 65_000..=now + 75_000).contains(&due));
    client.connect(now + 1).unwrap();
    assert_eq!(client.next_subscription_poll_at(&desired, due), None);
    client
        .recovery
        .as_mut()
        .unwrap()
        .discard_candidate(now + 1)
        .unwrap();
    assert_eq!(
        client.next_subscription_poll_at(&desired, now + 1),
        Some(due)
    );
    let now = connect(&mut client, &mut server, now + 2) + 1;
    assert_eq!(client.next_subscription_poll_at(&desired, now), Some(now));
    client
        .replace(b.peer().clone(), true, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    server
        .replace(a.peer().clone(), true, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(client.next_subscription_poll_at(&desired, now), None);
    let now = connect(&mut client, &mut server, now + 1) + 1;
    assert_eq!(client.next_subscription_poll_at(&desired, now), Some(now));
    assert!(matches!(
        client.poll_subscription(&mut desired, now),
        Err(ManagedError::StaleHandle)
    ));
}
