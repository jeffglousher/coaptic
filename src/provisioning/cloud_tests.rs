extern crate std;

use std::{cell::Cell, vec::Vec};

use super::*;
use crate::message::Token;
use crate::provisioning::{CoapProvisioner, Initiator, TrustAnchor};

const SERVER: Endpoint = Endpoint::v4([192, 0, 2, 254], 5683);
const FIRST: Endpoint = Endpoint::v4([192, 0, 2, 1], 49152);
const SECOND: Endpoint = Endpoint::v4([192, 0, 2, 2], 49152);
const THIRD: Endpoint = Endpoint::v4([192, 0, 2, 3], 49152);

struct Registry {
    entries: Vec<PeerTrust>,
    lookups: Cell<usize>,
}

impl AdmissionRegistry for Registry {
    fn resolve(&self, reference: CredentialReference) -> Option<&PeerTrust> {
        self.lookups.set(self.lookups.get() + 1);
        let mut matches = self
            .entries
            .iter()
            .filter(|trust| trust.checkpoint().peer().kid() == reference.kid());
        let entry = matches.next()?;
        if matches.next().is_some() {
            None
        } else {
            Some(entry)
        }
    }
}

impl Registry {
    fn new(server: &Identity, peers: &[&Identity]) -> Self {
        let entries = peers
            .iter()
            .enumerate()
            .map(|(index, peer)| {
                PeerTrust::install(
                    server,
                    peer.peer(),
                    TrustAnchor::genesis([(index + 1) as u8; 32]).unwrap(),
                    |_, _, _| Ok::<_, ()>(()),
                )
                .unwrap()
            })
            .collect();
        Self {
            entries,
            lookups: Cell::new(0),
        }
    }
}

#[derive(Default)]
struct Io {
    sent: Vec<(Endpoint, Vec<u8>)>,
    fail: bool,
    short: bool,
}

impl DatagramIo for Io {
    type Error = u8;

    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        panic!("shared admission must never receive from the listener")
    }

    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.sent.push((endpoint, bytes.to_vec()));
        if self.fail {
            Err(9)
        } else if self.short {
            Ok(bytes.len() - 1)
        } else {
            Ok(bytes.len())
        }
    }
}

impl Io {
    fn last(&self) -> &[u8] {
        &self.sent.last().unwrap().1
    }
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

fn entropy(value: u8) -> impl FnMut(&mut [u8]) -> bool {
    move |out| {
        out.fill(0);
        out[out.len() - 1] = value;
        true
    }
}

fn no_entropy(_: &mut [u8]) -> bool {
    panic!("cached transitions must not request fresh entropy")
}

fn request(payload: &[u8], mid: u16, token: &[u8], echo: Option<&[u8]>) -> Vec<u8> {
    let options = [
        Opt::new(OptionNumber::URI_PATH, b".well-known"),
        Opt::new(OptionNumber::URI_PATH, b"edhoc"),
        Opt::new(OptionNumber::CONTENT_FORMAT, &[65]),
        Opt::new(OptionNumber::ECHO, echo.unwrap_or(&[])),
    ];
    let mut out = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::con(Code::POST, MessageId::new(mid), Token::new(token).unwrap())
        .with_options(&options[..if echo.is_some() { 4 } else { 3 }])
        .with_payload(payload)
        .encode(&mut out)
        .unwrap();
    out[..len].to_vec()
}

fn m1(client: &Identity, server: &Identity, mid: u16, echo: Option<&[u8]>) -> Vec<u8> {
    let (_, message) = Initiator::start(client, server.peer(), entropy(3)).unwrap();
    let mut payload = Vec::from([0xf5]);
    payload.extend_from_slice(message.as_bytes());
    request(&payload, mid, &[1, 2, 3], echo)
}

fn check_response(request: &[u8], response: &[u8], code: Code) {
    let request = decode(request).unwrap();
    let response = decode(response).unwrap();
    assert_eq!(response.ty(), Type::Acknowledgement);
    assert_eq!(response.code(), code);
    assert_eq!(response.message_id(), request.message_id());
    assert_eq!(response.token(), request.token());
    let mut options = response.options();
    let option = options.next().unwrap();
    if code == Code::UNAUTHORIZED {
        assert_eq!(option.number(), OptionNumber::ECHO);
        assert_eq!(option.value().len(), ECHO_LEN);
        assert!(response.payload().is_empty());
    } else {
        assert_eq!(option.number(), OptionNumber::CONTENT_FORMAT);
        assert_eq!(decode_uint16(option.value()), Ok(64));
        assert!(!response.payload().is_empty());
    }
    assert!(options.next().is_none());
}

struct ExchangeResult {
    client: Session,
    server: AdmittedSession,
    m1: Vec<u8>,
    m2: Vec<u8>,
    m3: Vec<u8>,
    m4: Vec<u8>,
}

fn finish<const A: usize, const P: usize, const C: usize, const S: usize>(
    cloud: &mut CloudAdmission<'_, A, P, C, S>,
    registry: &Registry,
    client_identity: &Identity,
    endpoint: Endpoint,
    now: u64,
) -> ExchangeResult {
    let mut client = CoapProvisioner::start(
        client_identity,
        cloud.identity.peer(),
        SERVER,
        now,
        entropy(3),
    )
    .unwrap();
    let mut client_io = Io::default();
    let mut cloud_io = Io::default();
    client.flush(&mut client_io, now).unwrap();
    let original = client_io.last().to_vec();
    let before_lookup = registry.lookups.get();
    let retained_checks = cloud
        .retained
        .iter()
        .flatten()
        .filter(|item| now < item.expires)
        .count();
    assert_eq!(
        cloud.ingest(&original, endpoint, now, registry, no_entropy),
        Ok(Status::Progress)
    );
    assert_eq!(registry.lookups.get(), before_lookup + retained_checks);
    cloud.flush(&mut cloud_io, now, registry).unwrap();
    check_response(&original, cloud_io.last(), Code::UNAUTHORIZED);
    let challenge = cloud_io.last().to_vec();
    client
        .ingest(&challenge, SERVER, now, no_entropy, |_| {
            panic!("Echo cannot authorize")
        })
        .unwrap();
    client.flush(&mut client_io, now).unwrap();
    let m1 = client_io.last().to_vec();
    let original_parsed = decode(&original).unwrap();
    let retry = decode(&m1).unwrap();
    assert_eq!(original_parsed.payload(), retry.payload());
    assert_ne!(original_parsed.message_id(), retry.message_id());
    assert_eq!(original_parsed.token(), retry.token());
    assert_eq!(retry.echo(), decode(&challenge).unwrap().echo());
    cloud
        .ingest(&m1, endpoint, now, registry, entropy(4))
        .unwrap();
    cloud.flush(&mut cloud_io, now, registry).unwrap();
    let m2 = cloud_io.last().to_vec();
    check_response(&m1, &m2, Code::CHANGED);
    client
        .ingest(&m2, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut client_io, now).unwrap();
    let m3 = client_io.last().to_vec();
    assert_eq!(
        cloud.ingest(&m3, endpoint, now, registry, no_entropy),
        Ok(Status::Complete)
    );
    assert!(cloud.take_session(now, registry).unwrap().is_none());
    cloud.flush(&mut cloud_io, now, registry).unwrap();
    let m4 = cloud_io.last().to_vec();
    check_response(&m3, &m4, Code::CHANGED);
    client
        .ingest(&m4, SERVER, now, no_entropy, |_| true)
        .unwrap();
    let server = cloud.take_session(now, registry).unwrap().unwrap();
    let client = client.take_session().unwrap();
    ExchangeResult {
        client,
        server,
        m1,
        m2,
        m3,
        m4,
    }
}

fn prepare_ready<const A: usize, const P: usize, const C: usize, const S: usize>(
    cloud: &mut CloudAdmission<'_, A, P, C, S>,
    registry: &Registry,
    client: &Identity,
    endpoint: Endpoint,
    now: u64,
) -> (crate::provisioning::InitiatorConfirm, Vec<u8>) {
    let (initiator, raw_m1) = Initiator::start(client, cloud.identity.peer(), entropy(3)).unwrap();
    let mut payload = Vec::from([0xf5]);
    payload.extend_from_slice(raw_m1.as_bytes());
    let token = cloud.echo(endpoint, now);
    let m1 = request(&payload, 10, &[1], Some(&token));
    cloud
        .ingest(&m1, endpoint, now, registry, entropy(4))
        .unwrap();
    let mut io = Io::default();
    cloud.flush(&mut io, now, registry).unwrap();
    let message = Message::from_slice(decode(io.last()).unwrap().payload()).unwrap();
    let (confirm, raw_m3) = initiator.receive_message_2(&message, |_| true).unwrap();
    let mut payload = confirm.peer_connection_id().lakers().as_cbor().to_vec();
    payload.extend_from_slice(raw_m3.as_bytes());
    let m3 = request(&payload, 11, &[2], None);
    cloud
        .ingest(&m3, endpoint, now, registry, no_entropy)
        .unwrap();
    (confirm, m3)
}

#[test]
fn final_send_failure_and_revocation_never_expose_ready_keys_or_renew_deadlines() {
    let server = identity(254, 254);
    let client = identity(1, 1);
    let mut registry = Registry::new(&server, &[&client]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let (_, m3) = prepare_ready(&mut cloud, &registry, &client, FIRST, 0);
    let id = cloud.pending[0].as_ref().unwrap().local_id;
    let expires = cloud.pending[0].as_ref().unwrap().expires;
    let mut io = Io {
        fail: true,
        ..Io::default()
    };
    assert_eq!(cloud.flush(&mut io, 0, &registry), Err(CloudError::Io(9)));
    let m4 = io.last().to_vec();
    check_response(&m3, &m4, Code::CHANGED);
    assert!(cloud.take_session(0, &registry).unwrap().is_none());
    io.fail = false;
    io.short = true;
    assert_eq!(
        cloud.flush(&mut io, 0, &registry),
        Err(CloudError::ShortSend)
    );
    assert_eq!(io.last(), m4);
    assert_eq!(cloud.pending[0].as_ref().unwrap().expires, expires);
    registry.entries[0]
        .revoke(|_, _, _| Ok::<_, ()>(()))
        .unwrap();
    io.short = false;
    let attempts = io.sent.len();
    assert_eq!(cloud.flush(&mut io, 1, &registry), Ok(Status::Idle));
    assert_eq!(io.sent.len(), attempts);
    assert!(cloud.take_session(1, &registry).unwrap().is_none());
    assert!(cloud.reserves_connection_id(id));
    assert_eq!(
        cloud.ingest(&m3, FIRST, 1, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    cloud.flush(&mut io, expires, &registry).unwrap();
    assert!(!cloud.reserves_connection_id(id));
}

#[test]
fn retained_saturation_delays_handoff_without_evicting_active_or_completed_state() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let second = identity(2, 2);
    let registry = Registry::new(&server, &[&first, &second]);
    let mut cloud =
        CloudAdmission::<3, 2, 1, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let first_result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let first_id = first_result.server.association().local_id;
    let (confirm, second_m3) = prepare_ready(&mut cloud, &registry, &second, SECOND, 1_000);
    let second_id = confirm.peer_connection_id();
    let mut io = Io::default();
    cloud.flush(&mut io, 1_000, &registry).unwrap();
    let second_m4 = io.last().to_vec();
    check_response(&second_m3, &second_m4, Code::CHANGED);
    assert!(cloud.take_session(1_000, &registry).unwrap().is_none());
    assert!(cloud.reserves_connection_id(first_id));
    assert!(cloud.reserves_connection_id(second_id));
    assert_eq!(
        cloud.retained[0].as_ref().unwrap().association.local_id,
        first_id
    );
    cloud
        .ingest(&first_result.m3, FIRST, 1_000, &registry, no_entropy)
        .unwrap();
    cloud.flush(&mut io, 1_000, &registry).unwrap();
    assert_eq!(io.last(), first_result.m4);
    let second_session = cloud
        .take_session(EXCHANGE_LIFETIME_MS, &registry)
        .unwrap()
        .unwrap();
    assert_eq!(second_session.association().local_id, second_id);
    assert!(cloud.reserves_connection_id(first_id));
    assert_eq!(cloud.active.iter().flatten().count(), 2);
    assert_eq!(
        cloud.retained[0].as_ref().unwrap().association.local_id,
        second_id
    );
    assert_eq!(
        cloud.retained[0].as_ref().unwrap().expires,
        1_000 + EXCHANGE_LIFETIME_MS
    );
    let session = confirm
        .receive_message_4(
            &Message::from_slice(decode(&second_m4).unwrap().payload()).unwrap(),
            |_| true,
        )
        .unwrap();
    assert_eq!(session.principal(), &server.peer().principal());
}

#[test]
fn forged_routing_identifier_selects_only_candidate_and_never_authenticates_payload() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let second = identity(2, 2);
    let registry = Registry::new(&server, &[&first, &second]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let first_result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let second_result = finish(&mut cloud, &registry, &second, SECOND, 0);
    let second_association = *second_result.server.association();
    let (mut first_context, _) = first_result.client.into_parts();
    let (second_session, _) = second_result.server.into_parts();
    let (mut second_context, _) = second_session.into_parts();
    let message = CoapMessage::con(Code::POST, MessageId::new(80), Token::from_checked(&[5, 6]))
        .with_payload(b"device one payload cannot become device two");
    let mut original = [0; 384];
    let len = first_context
        .protect_request(&message, &mut original)
        .unwrap();
    let parsed = decode(&original[..len]).unwrap();
    let original_header = parsed.oscore().unwrap();
    let mut header = original_header.to_vec();
    let id_len = first_context.sender_id().len();
    let prefix = header.len() - id_len;
    header[prefix..].copy_from_slice(second_association.local_id.as_bytes());
    let options = [Opt::oscore(&header)];
    let mut forged = [0; 384];
    let len = CoapMessage::new(parsed.ty(), parsed.code(), parsed.message_id())
        .with_token(parsed.token())
        .with_options(&options)
        .with_payload(parsed.payload())
        .encode(&mut forged)
        .unwrap();
    assert_eq!(
        cloud.route_oscore(&forged[..len], SECOND, 0, &registry),
        Ok(Some(second_association))
    );
    let mut clear = [0; 384];
    assert!(
        second_context
            .unprotect_request(&decode(&forged[..len]).unwrap(), &mut clear)
            .is_err()
    );
    let options = [Opt::oscore(&header), Opt::oscore(&header)];
    let len = CoapMessage::new(parsed.ty(), parsed.code(), parsed.message_id())
        .with_token(parsed.token())
        .with_options(&options)
        .with_payload(parsed.payload())
        .encode(&mut forged)
        .unwrap();
    assert_eq!(
        cloud.route_oscore(&forged[..len], SECOND, 0, &registry),
        Ok(None)
    );
}

#[test]
fn two_devices_share_listener_with_distinct_authenticated_contexts_and_exact_metadata() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let second = identity(2, 2);
    let registry = Registry::new(&server, &[&first, &second]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let a = finish(&mut cloud, &registry, &first, FIRST, 0);
    registry.lookups.set(0);
    let b = finish(&mut cloud, &registry, &second, SECOND, 0);
    let a_route = *a.server.association();
    let b_route = *b.server.association();
    assert_ne!(a_route.local_id, b_route.local_id);
    assert_eq!(a_route.grant.principal(), &first.peer().principal());
    assert_eq!(b_route.grant.principal(), &second.peer().principal());
    let (a_session, _) = a.server.into_parts();
    let (b_session, _) = b.server.into_parts();
    let (mut a_client, _) = a.client.into_parts();
    let (mut b_client, _) = b.client.into_parts();
    let (mut a_server, _) = a_session.into_parts();
    let (mut b_server, _) = b_session.into_parts();
    assert_eq!(a_client.sender_key(), a_server.recipient_key());
    assert_eq!(b_client.sender_key(), b_server.recipient_key());
    assert_ne!(a_client.sender_key(), b_client.sender_key());
    for (client, owner, endpoint, route, payload) in [
        (
            &mut a_client,
            &mut a_server,
            FIRST,
            a_route,
            b"first full telemetry payload".as_slice(),
        ),
        (
            &mut b_client,
            &mut b_server,
            SECOND,
            b_route,
            b"second full telemetry payload".as_slice(),
        ),
    ] {
        let options = [Opt::new(OptionNumber::URI_PATH, b"telemetry")];
        let message = CoapMessage::con(
            Code::POST,
            MessageId::new(100),
            Token::from_checked(&[7, 8]),
        )
        .with_options(&options)
        .with_payload(payload);
        let mut protected = [0; 384];
        let len = client.protect_request(&message, &mut protected).unwrap();
        assert_eq!(
            cloud
                .route_oscore(&protected[..len], endpoint, 0, &registry)
                .unwrap(),
            Some(route)
        );
        assert_eq!(
            cloud
                .route_oscore(&protected[..len], THIRD, 0, &registry)
                .unwrap(),
            None
        );
        let mut clear = [0; 384];
        let (decoded, _) = owner
            .unprotect_request(&decode(&protected[..len]).unwrap(), &mut clear)
            .unwrap();
        assert_eq!(decoded.code(), Code::POST);
        assert_eq!(decoded.message_id(), message.message_id());
        assert_eq!(decoded.token(), message.token());
        assert_eq!(
            decoded.get_option(OptionNumber::URI_PATH).unwrap().value(),
            b"telemetry"
        );
        assert_eq!(decoded.payload(), payload);
    }
}

#[test]
fn duplicate_loss_short_send_and_changed_operation_preserve_exact_bytes_and_reservations() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let registry = Registry::new(&server, &[&first]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let association = *result.server.association();
    let mut io = Io::default();
    cloud
        .ingest(&result.m3, FIRST, 1, &registry, no_entropy)
        .unwrap();
    io.fail = true;
    assert_eq!(cloud.flush(&mut io, 1, &registry), Err(CloudError::Io(9)));
    assert_eq!(io.last(), result.m4);
    io.fail = false;
    io.short = true;
    assert_eq!(
        cloud.flush(&mut io, 1, &registry),
        Err(CloudError::ShortSend)
    );
    assert_eq!(io.last(), result.m4);
    io.short = false;
    cloud.flush(&mut io, 1, &registry).unwrap();
    assert_eq!(io.last(), result.m4);
    let mut changed = result.m3.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert_eq!(
        cloud.ingest(&changed, FIRST, 1, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert_eq!(
        cloud.ingest(&result.m3, SECOND, 1, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert_eq!(cloud.flush(&mut io, 1, &registry), Ok(Status::Idle));
    cloud
        .ingest(&result.m1, FIRST, 1, &registry, no_entropy)
        .unwrap();
    cloud.flush(&mut io, 1, &registry).unwrap();
    assert_eq!(io.last(), result.m2);
    assert!(cloud.reserves_message_id(decode(&result.m1).unwrap().message_id(), FIRST, 1));
    assert!(cloud.reserves_message_id(decode(&result.m3).unwrap().message_id(), FIRST, 1));
    assert!(cloud.release(association));
    assert!(!cloud.release(association));
    assert!(cloud.reserves_connection_id(association.local_id));
    cloud
        .flush(&mut io, EXCHANGE_LIFETIME_MS, &registry)
        .unwrap();
    assert!(!cloud.reserves_connection_id(association.local_id));
}

#[test]
fn echo_proves_exact_address_freshness_before_entropy_and_registry_lookup() {
    let server = identity(254, 254);
    let client = identity(1, 1);
    let registry = Registry::new(&server, &[&client]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let original = m1(&client, &server, 10, None);
    cloud
        .ingest(&original, FIRST, 0, &registry, no_entropy)
        .unwrap();
    let mut io = Io::default();
    cloud.flush(&mut io, 0, &registry).unwrap();
    let echo = decode(io.last()).unwrap().echo().unwrap().to_vec();
    let retry = request(
        decode(&original).unwrap().payload(),
        11,
        &[1, 2, 3],
        Some(&echo),
    );
    assert_eq!(
        cloud.ingest(&retry, SECOND, 0, &registry, no_entropy),
        Ok(Status::Progress)
    );
    assert!(cloud.pending.iter().all(Option::is_none));
    cloud.flush(&mut io, 0, &registry).unwrap();
    let mut corrupt = echo.clone();
    corrupt[8] ^= 1;
    let forged = request(
        decode(&original).unwrap().payload(),
        12,
        &[1, 2, 3],
        Some(&corrupt),
    );
    cloud
        .ingest(&forged, FIRST, 0, &registry, no_entropy)
        .unwrap();
    assert!(cloud.pending.iter().all(Option::is_none));
    cloud.flush(&mut io, 0, &registry).unwrap();
    cloud
        .ingest(&retry, FIRST, 10_000, &registry, no_entropy)
        .unwrap();
    assert!(cloud.pending.iter().all(Option::is_none));
    assert_eq!(registry.lookups.get(), 0);
    assert!(!cloud.valid_echo(Some(&cloud.echo(FIRST, 10_001)), FIRST, 10_000));
    let scoped = Endpoint::v6_scoped([1; 16], 5683, 2);
    let token = cloud.echo(scoped, 10_000);
    assert!(!cloud.valid_echo(Some(&token), Endpoint::v6_scoped([1; 16], 5683, 3), 10_000));
    assert!(!cloud.valid_echo(Some(&token), Endpoint::v6_scoped([1; 16], 5684, 2), 10_000));
}

fn deferred_result(
    client: &Identity,
    server: &Identity,
    trusted: Option<crate::provisioning::PinnedPeer>,
) -> Result<(Session, Message), Error> {
    let (initiator, m1) = Initiator::start(client, server.peer(), entropy(3)).unwrap();
    let (responder, m2) =
        RegistryResponder::receive_message_1(server, ConnectionId::RESPONDER, &m1, entropy(4))
            .unwrap();
    let (_, m3) = initiator.receive_message_2(&m2, |_| true).unwrap();
    responder.receive_message_3(
        &m3,
        |_| trusted,
        |_| panic!("unverified possession must not authorize"),
    )
}

#[test]
fn references_cannot_authorize_unknown_ambiguous_or_substituted_credentials() {
    let server = identity(254, 254);
    let client = identity(1, 1);
    assert!(matches!(
        deferred_result(&client, &server, None),
        Err(Error::Authentication)
    ));
    assert!(matches!(
        deferred_result(&client, &server, Some(identity(2, 1).peer())),
        Err(Error::Authentication)
    ));
    assert!(matches!(
        deferred_result(&client, &server, Some(identity(2, 2).peer())),
        Err(Error::Authentication)
    ));
    assert!(matches!(
        deferred_result(&client, &server, Some(server.peer())),
        Err(Error::Authentication)
    ));
    let duplicate_kid = identity(2, 1);
    let registry = Registry::new(&server, &[&client, &duplicate_kid]);
    assert!(registry.resolve(CredentialReference(1)).is_none());
}

#[test]
fn revocation_stops_m4_cached_replies_handoff_and_routes_while_retaining_active_cid() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let mut registry = Registry::new(&server, &[&first]);
    let mut cloud =
        CloudAdmission::<2, 2, 2, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let association = *result.server.association();
    cloud
        .ingest(&result.m3, FIRST, 1, &registry, no_entropy)
        .unwrap();
    registry.entries[0]
        .revoke(|_, _, _| Ok::<_, ()>(()))
        .unwrap();
    let mut io = Io::default();
    assert_eq!(cloud.flush(&mut io, 1, &registry), Ok(Status::Idle));
    assert!(io.sent.is_empty());
    assert_eq!(
        cloud.ingest(&result.m1, FIRST, 1, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.reserves_connection_id(association.local_id));
    let (mut context, _) = result.client.into_parts();
    let mut out = [0; 384];
    let message =
        CoapMessage::con(Code::POST, MessageId::new(90), Token::EMPTY).with_payload(b"revoked");
    let len = context.protect_request(&message, &mut out).unwrap();
    assert_eq!(
        cloud.route_oscore(&out[..len], FIRST, 1, &registry),
        Ok(None)
    );
    registry.entries[0]
        .replace(&server, first.peer(), true, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(
        cloud.route_oscore(&out[..len], FIRST, 1, &registry),
        Ok(None)
    );
    assert!(cloud.release(association));
}

#[test]
fn fixed_global_source_and_storage_caps_refuse_without_eviction_or_heavy_work() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let registry = Registry::new(&server, &[&first]);
    let limits = AdmissionLimits {
        global_datagrams: 2,
        source_datagrams: 1,
        ..AdmissionLimits::default()
    };
    let mut cloud = CloudAdmission::<2, 2, 2, 2>::new(&server, 0, limits, entropy(9)).unwrap();
    let request = m1(&first, &server, 10, None);
    let mut io = Io::default();
    cloud
        .ingest(&request, FIRST, 0, &registry, no_entropy)
        .unwrap();
    cloud.flush(&mut io, 0, &registry).unwrap();
    assert_eq!(
        cloud.ingest(
            &request,
            Endpoint::v4([192, 0, 2, 1], 49153),
            0,
            &registry,
            no_entropy
        ),
        Ok(Status::Ignored)
    );
    cloud
        .ingest(&request, SECOND, 0, &registry, no_entropy)
        .unwrap();
    cloud.flush(&mut io, 0, &registry).unwrap();
    assert_eq!(
        cloud.ingest(&request, THIRD, 0, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.pending.iter().all(Option::is_none));
    assert_eq!(registry.lookups.get(), 0);
    assert_eq!(io.sent.len(), 2);
    assert_eq!(
        cloud.ingest(&request, THIRD, 1_000, &registry, no_entropy),
        Ok(Status::Progress)
    );
}

#[test]
fn active_capacity_stays_reserved_after_completion_cache_expiry() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let registry = Registry::new(&server, &[&first]);
    let mut cloud =
        CloudAdmission::<1, 1, 1, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let result = finish(&mut cloud, &registry, &first, FIRST, 0);
    let association = *result.server.association();
    let request = m1(&first, &server, 90, None);
    assert_eq!(
        cloud.ingest(
            &request,
            SECOND,
            EXCHANGE_LIFETIME_MS,
            &registry,
            no_entropy
        ),
        Ok(Status::Ignored)
    );
    assert!(cloud.retained.iter().all(Option::is_none));
    assert!(cloud.reserves_connection_id(association.local_id));
    assert!(cloud.release(association));
    assert_eq!(
        cloud.ingest(
            &request,
            SECOND,
            EXCHANGE_LIFETIME_MS,
            &registry,
            no_entropy
        ),
        Ok(Status::Progress)
    );
}

#[test]
fn failed_message3_is_quarantined_and_provisional_capacity_cannot_be_overwritten() {
    let server = identity(254, 254);
    let first = identity(1, 1);
    let wrong = identity(2, 1);
    let registry = Registry::new(&server, &[&first]);
    let mut cloud =
        CloudAdmission::<2, 1, 1, 4>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let (initiator, raw_m1) = Initiator::start(&wrong, server.peer(), entropy(3)).unwrap();
    let mut payload = Vec::from([0xf5]);
    payload.extend_from_slice(raw_m1.as_bytes());
    let echo = cloud.echo(FIRST, 0);
    let first_request = request(&payload, 10, &[1], Some(&echo));
    cloud
        .ingest(&first_request, FIRST, 0, &registry, entropy(4))
        .unwrap();
    let mut io = Io::default();
    cloud.flush(&mut io, 0, &registry).unwrap();
    let (_, message3) = initiator
        .receive_message_2(
            &Message::from_slice(decode(io.last()).unwrap().payload()).unwrap(),
            |_| true,
        )
        .unwrap();
    let id = cloud.pending[0].as_ref().unwrap().local_id;
    let mut payload = id.lakers().as_cbor().to_vec();
    payload.extend_from_slice(message3.as_bytes());
    let m3 = request(&payload, 11, &[2], None);
    assert_eq!(
        cloud.ingest(&m3, FIRST, 0, &registry, no_entropy),
        Err(CloudError::Provisioning(Error::Authentication))
    );
    let lookups = registry.lookups.get();
    assert_eq!(
        cloud.ingest(&m3, FIRST, 1_000, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert_eq!(registry.lookups.get(), lookups);
    assert!(cloud.reserves_connection_id(id));
    let other = m1(&first, &server, 12, None);
    assert_eq!(
        cloud.ingest(&other, SECOND, 1_000, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    assert!(cloud.take_session(1_000, &registry).unwrap().is_none());
    assert_eq!(
        cloud.ingest(&other, SECOND, EXCHANGE_LIFETIME_MS, &registry, no_entropy),
        Ok(Status::Progress)
    );
    assert!(!cloud.reserves_connection_id(id));
}

#[test]
fn invalid_configuration_entropy_clock_and_namespace_fail_closed() {
    let server = identity(254, 254);
    assert!(matches!(
        CloudAdmission::<0, 1, 1, 1>::new(&server, 0, AdmissionLimits::default(), no_entropy),
        Err(CloudError::Capacity)
    ));
    assert!(matches!(
        CloudAdmission::<1, 1, 1, 1>::new(&server, 0, AdmissionLimits::default(), |_| false),
        Err(CloudError::Provisioning(Error::Entropy))
    ));
    assert!(matches!(
        CloudAdmission::<1, 1, 1, 1>::new(&server, 0, AdmissionLimits::default(), |_| true),
        Err(CloudError::Provisioning(Error::Entropy))
    ));
    let registry = Registry::new(&server, &[]);
    let mut cloud =
        CloudAdmission::<1, 1, 1, 1>::new(&server, 1, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let mut io = Io::default();
    assert_eq!(cloud.flush(&mut io, 0, &registry), Err(CloudError::Clock));
    assert_eq!(cloud.flush(&mut io, 1, &registry), Err(CloudError::Clock));
    assert!(io.sent.is_empty());
    assert_eq!(cloud.next_deadline(), None);
    let mut cloud =
        CloudAdmission::<1, 1, 1, 1>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    cloud.next_id = MAX_COUNTER;
    let id = cloud.allocate_id(ConnectionId::INITIATOR).unwrap();
    assert_eq!(id.as_bytes(), &[0xff; 7]);
    assert_eq!(
        cloud.allocate_id(ConnectionId::INITIATOR),
        Err(CloudError::Identifiers)
    );
}

#[test]
fn default_capacity_supports_64_active_associations_with_fixed_storage() {
    let server = identity(254, 254);
    let identities: Vec<_> = (1..=64).map(|value| identity(value, value)).collect();
    let refs: Vec<_> = identities.iter().collect();
    let registry = Registry::new(&server, &refs);
    let mut cloud =
        CloudAdmission::<64, 4, 8, 16>::new(&server, 0, AdmissionLimits::default(), entropy(9))
            .unwrap();
    let mut associations = Vec::new();
    for (index, identity) in identities.iter().enumerate() {
        registry.lookups.set(0);
        let now = index as u64 * EXCHANGE_LIFETIME_MS;
        let endpoint = Endpoint::v4([192, 0, 2, (index + 1) as u8], 49152);
        let result = finish(&mut cloud, &registry, identity, endpoint, now);
        let association = *result.server.association();
        assert_eq!(association.grant.principal(), &identity.peer().principal());
        assert!(!associations.contains(&association.local_id));
        associations.push(association.local_id);
    }
    assert_eq!(cloud.active.iter().flatten().count(), 64);
    assert!(
        associations
            .into_iter()
            .all(|id| cloud.reserves_connection_id(id))
    );
    let now = 64 * EXCHANGE_LIFETIME_MS;
    let request = m1(&identities[0], &server, 90, None);
    assert_eq!(
        cloud.ingest(&request, THIRD, now, &registry, no_entropy),
        Ok(Status::Ignored)
    );
    std::println!(
        "CloudAdmission<64,4,8,16> bytes={}; cryptographic stack, registry, Apps and transport are additional",
        core::mem::size_of::<CloudAdmission<'_>>()
    );
}
