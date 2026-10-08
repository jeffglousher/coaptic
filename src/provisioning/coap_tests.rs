extern crate std;

use std::{collections::VecDeque, vec::Vec};

use super::*;

const CLIENT: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const SERVER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const OTHER: Endpoint = Endpoint::v4([192, 0, 2, 3], 5683);

#[derive(Default)]
struct Io {
    incoming: VecDeque<(Endpoint, Vec<u8>)>,
    attempts: Vec<(Endpoint, Vec<u8>)>,
    fail: bool,
    fail_on: Option<usize>,
    short: bool,
    receives: usize,
}

impl DatagramIo for Io {
    type Error = u8;

    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        self.receives += 1;
        Ok(self.incoming.pop_front().map(|(endpoint, bytes)| {
            assert!(bytes.len() <= out.len());
            out[..bytes.len()].copy_from_slice(&bytes);
            (bytes.len(), endpoint)
        }))
    }

    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.attempts.push((endpoint, bytes.to_vec()));
        if self.fail || self.fail_on == Some(self.attempts.len()) {
            Err(9)
        } else if self.short {
            Ok(bytes.len() - 1)
        } else {
            Ok(bytes.len())
        }
    }
}

impl Io {
    fn last(&self) -> Vec<u8> {
        self.attempts.last().unwrap().1.clone()
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
        if out.len() == 32 {
            out[31] = value;
        }
        true
    }
}

fn no_entropy(_: &mut [u8]) -> bool {
    panic!("duplicates and transitions must reuse established keys")
}

fn no_authorization(_: &Principal) -> bool {
    panic!("unauthenticated messages must not invoke authorization")
}

fn finish(
    client: &mut CoapProvisioner<'_>,
    server: &mut CoapProvisioner<'_>,
    client_io: &mut Io,
    server_io: &mut Io,
    now: u64,
) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    client.flush(client_io, now).unwrap();
    let m1 = client_io.last();
    server
        .ingest(&m1, CLIENT, now, entropy(4), no_authorization)
        .unwrap();
    server.flush(server_io, now).unwrap();
    let m2 = server_io.last();
    client
        .ingest(&m2, SERVER, now, no_entropy, |_| true)
        .unwrap();
    client.flush(client_io, now).unwrap();
    let m3 = client_io.last();
    server
        .ingest(&m3, CLIENT, now, no_entropy, |_| true)
        .unwrap();
    assert!(server.take_session().is_none());
    server.flush(server_io, now).unwrap();
    let m4 = server_io.last();
    client
        .ingest(&m4, SERVER, now, no_entropy, |_| true)
        .unwrap();
    (m1, m2, m3, m4)
}

fn recode(
    bytes: &[u8],
    ty: Type,
    mid: MessageId,
    token: Token,
    options: &[Opt<'_>],
    payload: &[u8],
) -> Vec<u8> {
    let parsed = decode(bytes).unwrap();
    let mut output = [0; DATAGRAM_CAPACITY];
    let len = CoapMessage::new(ty, parsed.code(), mid)
        .with_token(token)
        .with_options(options)
        .with_payload(payload)
        .encode(&mut output)
        .unwrap();
    output[..len].to_vec()
}

#[test]
fn sequential_wire_profile_validates_payload_metadata_and_session_identity() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    let (m1, m2, m3, m4) = finish(&mut client, &mut server, &mut client_io, &mut server_io, 0);
    for (request, response, prefix) in [(&m1, &m2, 0xf5), (&m3, &m4, 0x01)] {
        let request = decode(request).unwrap();
        let response = decode(response).unwrap();
        assert!(request_metadata(request));
        assert!(response_metadata(response));
        assert_eq!(request.payload()[0], prefix);
        assert!(!response.payload().is_empty());
        assert_eq!(response.ty(), Type::Acknowledgement);
        assert_eq!(response.message_id(), request.message_id());
        assert_eq!(response.token(), request.token());
    }
    assert_ne!(
        decode(&m1).unwrap().message_id(),
        decode(&m3).unwrap().message_id()
    );
    assert_ne!(decode(&m1).unwrap().token(), decode(&m3).unwrap().token());
    let client_session = client.take_session().unwrap();
    let server_session = server.take_session().unwrap();
    assert_eq!(*client_session.principal(), b.peer().principal());
    assert_eq!(*server_session.principal(), a.peer().principal());
    let (client_context, _) = client_session.into_parts();
    let (server_context, _) = server_session.into_parts();
    assert_eq!(client_context.sender_key(), server_context.recipient_key());
    assert_eq!(client_context.recipient_key(), server_context.sender_key());
    assert_eq!(client_context.common_iv(), server_context.common_iv());
    assert!(client.take_session().is_none());
    assert!(server.take_session().is_none());
    std::println!(
        "CoapProvisioner bytes={}",
        core::mem::size_of::<CoapProvisioner<'_>>()
    );
}

#[test]
fn loss_uses_exact_cached_requests_four_retries_and_precise_deadlines() {
    let a = identity(1, 0);
    let mut client =
        CoapProvisioner::start(&a, identity(2, 1).peer(), SERVER, 0, entropy(3)).unwrap();
    let mut io = Io::default();
    assert_eq!(client.next_deadline(), Some(0));
    client.flush(&mut io, 0).unwrap();
    let first = io.last();
    for (deadline, next) in [(2000, 6000), (6000, 14000), (14000, 30000), (30000, 62000)] {
        assert_eq!(client.next_deadline(), Some(deadline));
        let before = io.attempts.len();
        assert_eq!(client.flush(&mut io, deadline - 1).unwrap(), Status::Idle);
        assert_eq!(io.attempts.len(), before);
        assert_eq!(client.flush(&mut io, deadline).unwrap(), Status::Progress);
        assert_eq!(io.last(), first);
        assert_eq!(client.next_deadline(), Some(next));
    }
    assert_eq!(io.attempts.len(), 5);
    assert_eq!(client.flush(&mut io, 62000), Err(PollError::Timeout));
    assert_eq!(io.attempts.len(), 5);
    assert_eq!(client.next_deadline(), None);
}

#[test]
fn late_polling_does_not_extend_the_transmit_span() {
    let a = identity(1, 0);
    let mut client =
        CoapProvisioner::start(&a, identity(2, 1).peer(), SERVER, 0, entropy(3)).unwrap();
    let mut io = Io::default();
    client.flush(&mut io, 0).unwrap();
    client.flush(&mut io, 29_999).unwrap();
    assert_eq!(io.attempts.len(), 2);
    assert_eq!(client.next_deadline(), Some(62000));
    client.flush(&mut io, 45_001).unwrap();
    assert_eq!(io.attempts.len(), 2);
    assert_eq!(client.flush(&mut io, 62000), Err(PollError::Timeout));
}

#[test]
fn initial_timeout_randomization_preserves_upper_bound_and_distinct_operations() {
    let a = identity(1, 0);
    let mut client = CoapProvisioner::start(&a, identity(2, 1).peer(), SERVER, 0, |out| {
        out.fill(0xff);
        if out.len() == 32 {
            out.fill(0);
            out[31] = 3;
        }
        true
    })
    .unwrap();
    let mut io = Io::default();
    client.flush(&mut io, 0).unwrap();
    assert_eq!(client.next_deadline(), Some(3000));
    let Role::Client(client) = &client.role else {
        unreachable!()
    };
    assert_ne!(client.ids.first.token, client.ids.second.token);
    assert_eq!(client.ids.first.mid.get(), u16::MAX);
    assert_eq!(client.ids.second.mid.get(), 0);
}

#[test]
fn duplicate_message1_reuses_response_without_entropy_or_authorization() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client.flush(&mut client_io, 0).unwrap();
    let m1 = client_io.last();
    server
        .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let first = server_io.last();
    server
        .ingest(&m1, CLIENT, 2000, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut server_io, 2000).unwrap();
    assert_eq!(server_io.last(), first);
    assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
}

#[test]
fn completion_keeps_both_exact_response_caches_after_session_handoff() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    let (m1, m2, m3, m4) = finish(&mut client, &mut server, &mut client_io, &mut server_io, 0);
    assert!(server.take_session().is_some());
    for (request, response) in [(&m1, &m2), (&m3, &m4)] {
        assert_eq!(
            server
                .ingest(request, CLIENT, 2000, no_entropy, no_authorization)
                .unwrap(),
            Status::Progress
        );
        assert_eq!(
            server.flush(&mut server_io, 2000).unwrap(),
            Status::Complete
        );
        assert_eq!(server_io.last(), *response);
    }
    assert!(server.take_session().is_none());
    assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    let sends = server_io.attempts.len();
    assert_eq!(
        server.flush(&mut server_io, EXCHANGE_LIFETIME_MS).unwrap(),
        Status::Expired
    );
    assert_eq!(server.next_deadline(), None);
    assert_eq!(
        server
            .ingest(
                &m3,
                CLIENT,
                EXCHANGE_LIFETIME_MS,
                no_entropy,
                no_authorization
            )
            .unwrap(),
        Status::Expired
    );
    assert_eq!(server_io.attempts.len(), sends);
}

#[test]
fn sequential_request_can_reuse_the_token_with_a_new_mid() {
    for token in [None, Some(Token::EMPTY)] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
        let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
        let mut client_io = Io::default();
        let mut server_io = Io::default();
        if let Some(token) = token {
            let Role::Client(state) = &mut client.role else {
                unreachable!()
            };
            let parsed = decode(state.request.as_slice()).unwrap();
            let options: Vec<_> = parsed.options().collect();
            let request = recode(
                state.request.as_slice(),
                parsed.ty(),
                parsed.message_id(),
                token,
                &options,
                parsed.payload(),
            );
            state.operation.token = token;
            state.ids.first.token = token;
            state.request = Wire::copy(&request).unwrap();
        }
        client.flush(&mut client_io, 0).unwrap();
        let m1 = client_io.last();
        let first = decode(&m1).unwrap();
        server
            .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
            .unwrap();
        server.flush(&mut server_io, 0).unwrap();
        let m2 = server_io.last();
        let options: Vec<_> = first.options().collect();
        let collision = recode(
            &m1,
            first.ty(),
            first.message_id(),
            Token::from_checked(&[99]),
            &options,
            first.payload(),
        );
        assert_eq!(
            server
                .ingest(&collision, CLIENT, 0, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
        assert_eq!(server.flush(&mut server_io, 0).unwrap(), Status::Idle);
        client.ingest(&m2, SERVER, 0, no_entropy, |_| true).unwrap();
        let Role::Client(state) = &mut client.role else {
            unreachable!()
        };
        let parsed = decode(state.request.as_slice()).unwrap();
        let options: Vec<_> = parsed.options().collect();
        let reused = recode(
            state.request.as_slice(),
            parsed.ty(),
            parsed.message_id(),
            first.token(),
            &options,
            parsed.payload(),
        );
        state.operation.token = first.token();
        state.ids.second.token = first.token();
        state.request = Wire::copy(&reused).unwrap();
        client.flush(&mut client_io, 0).unwrap();
        let m3 = client_io.last();
        let third = decode(&m3).unwrap();
        assert!(request_metadata(third));
        assert_eq!(third.payload()[0], 0x01);
        assert_eq!(third.token(), first.token());
        assert_ne!(third.message_id(), first.message_id());
        assert_eq!(
            server.ingest(&m3, CLIENT, 0, no_entropy, |_| true).unwrap(),
            Status::Progress
        );
        server.flush(&mut server_io, 0).unwrap();
        let m4 = server_io.last();
        let fourth = decode(&m4).unwrap();
        assert!(response_metadata(fourth));
        assert_eq!(fourth.token(), third.token());
        assert_eq!(fourth.message_id(), third.message_id());
        client.ingest(&m4, SERVER, 0, no_entropy, |_| true).unwrap();
        let (client_context, client_principal) = client.take_session().unwrap().into_parts();
        let (server_context, server_principal) = server.take_session().unwrap().into_parts();
        assert_eq!(client_principal, b.peer().principal());
        assert_eq!(server_principal, a.peer().principal());
        assert_eq!(client_context.sender_key(), server_context.recipient_key());
        assert_eq!(client_context.recipient_key(), server_context.sender_key());
        assert_eq!(client_context.common_iv(), server_context.common_iv());
        for (request, response) in [(&m1, &m2), (&m3, &m4)] {
            assert_eq!(
                server
                    .ingest(request, CLIENT, 2000, no_entropy, no_authorization)
                    .unwrap(),
                Status::Progress
            );
            server.flush(&mut server_io, 2000).unwrap();
            assert_eq!(server_io.last(), *response);
        }
        assert_eq!(server.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    }
}

#[test]
fn endpoint_mid_token_and_complete_request_bytes_isolate_cached_responses() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client.flush(&mut client_io, 0).unwrap();
    let m1 = client_io.last();
    server
        .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let original = server_io.last();
    assert_eq!(
        server
            .ingest(&m1, OTHER, 0, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    let parsed = decode(&m1).unwrap();
    let options: Vec<_> = parsed.options().collect();
    let mut payload = parsed.payload().to_vec();
    payload.push(0);
    let changed_body = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &options,
        &payload,
    );
    let changed_token = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        Token::from_checked(&[99]),
        &options,
        parsed.payload(),
    );
    let changed_mid = recode(
        &m1,
        parsed.ty(),
        parsed.message_id().wrapping_add(7),
        parsed.token(),
        &options,
        parsed.payload(),
    );
    for changed in [&changed_body, &changed_token, &changed_mid] {
        assert_eq!(
            server
                .ingest(changed, CLIENT, 0, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
        assert_eq!(server.flush(&mut server_io, 0).unwrap(), Status::Idle);
    }
    assert_eq!(server_io.attempts.len(), 1);
    server
        .ingest(&m1, CLIENT, 0, no_entropy, no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    assert_eq!(server_io.last(), original);
}

#[test]
fn invalid_request_path_format_code_and_payload_prefix_preserve_listening_state() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut io = Io::default();
    client.flush(&mut io, 0).unwrap();
    let m1 = io.last();
    let parsed = decode(&m1).unwrap();
    let good: Vec<_> = parsed.options().collect();
    let bad_path = [
        Opt::new(OptionNumber::URI_PATH, b".well-known"),
        Opt::new(OptionNumber::URI_PATH, b"other"),
        good[2],
    ];
    let bad_format = [
        good[0],
        good[1],
        Opt::new(OptionNumber::CONTENT_FORMAT, &[64]),
    ];
    let repeated_format = [good[0], good[1], good[2], good[2]];
    let mut bad_prefix = parsed.payload().to_vec();
    bad_prefix[0] = 0x01;
    let wrong_path = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &bad_path,
        parsed.payload(),
    );
    let wrong_format = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &bad_format,
        parsed.payload(),
    );
    let repeated_format = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &repeated_format,
        parsed.payload(),
    );
    let wrong_prefix = recode(
        &m1,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &good,
        &bad_prefix,
    );
    let mut wrong_code = m1.clone();
    wrong_code[1] = 1;
    let non = recode(
        &m1,
        Type::NonConfirmable,
        parsed.message_id(),
        parsed.token(),
        &good,
        parsed.payload(),
    );
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    for bad in [
        &wrong_path,
        &wrong_format,
        &repeated_format,
        &wrong_prefix,
        &wrong_code,
        &non,
    ] {
        assert_eq!(
            server
                .ingest(bad, CLIENT, 0, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
        assert_eq!(server.next_deadline(), None);
    }
    assert_eq!(
        server
            .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
            .unwrap(),
        Status::Progress
    );
}

#[test]
fn invalid_response_metadata_does_not_consume_valid_client_state() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client.flush(&mut client_io, 0).unwrap();
    server
        .ingest(&client_io.last(), CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let m2 = server_io.last();
    let parsed = decode(&m2).unwrap();
    let good: Vec<_> = parsed.options().collect();
    let bad_format = [Opt::new(OptionNumber::CONTENT_FORMAT, &[65])];
    let mid = recode(
        &m2,
        parsed.ty(),
        parsed.message_id().wrapping_add(1),
        parsed.token(),
        &good,
        parsed.payload(),
    );
    let token = recode(
        &m2,
        parsed.ty(),
        parsed.message_id(),
        Token::from_checked(&[2]),
        &good,
        parsed.payload(),
    );
    let format = recode(
        &m2,
        parsed.ty(),
        parsed.message_id(),
        parsed.token(),
        &bad_format,
        parsed.payload(),
    );
    let mut code = m2.clone();
    code[1] = 69;
    assert_eq!(
        client
            .ingest(&m2, OTHER, 0, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    for bad in [&mid, &token, &format, &code] {
        assert_eq!(
            client
                .ingest(bad, SERVER, 0, no_entropy, no_authorization)
                .unwrap(),
            Status::Ignored
        );
    }
    assert!(client.take_session().is_none());
    assert_eq!(
        client.ingest(&m2, SERVER, 0, no_entropy, |_| true).unwrap(),
        Status::Progress
    );
    client.flush(&mut client_io, 0).unwrap();
    assert_eq!(decode(&client_io.last()).unwrap().payload()[0], 0x01);
}

#[test]
fn separate_confirmable_replies_are_acknowledged_only_after_authentication() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client.flush(&mut client_io, 0).unwrap();
    let m1 = client_io.last();
    server
        .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let m2 = server_io.last();
    let parsed = decode(&m2).unwrap();
    let options: Vec<_> = parsed.options().collect();
    let separate = recode(
        &m2,
        Type::Confirmable,
        MessageId::new(900),
        parsed.token(),
        &options,
        parsed.payload(),
    );
    client
        .ingest(&separate, SERVER, 0, no_entropy, |_| true)
        .unwrap();
    client.flush(&mut client_io, 0).unwrap();
    assert_eq!(client_io.attempts.len(), 3);
    let ack = decode(&client_io.attempts[1].1).unwrap();
    assert!(ack.is_empty_ack());
    assert_eq!(ack.message_id().get(), 900);
    let m3 = client_io.last();
    client
        .ingest(&separate, SERVER, 0, no_entropy, no_authorization)
        .unwrap();
    client.flush(&mut client_io, 0).unwrap();
    assert_eq!(client_io.attempts.len(), 4);
    assert_eq!(client_io.last(), client_io.attempts[1].1);
    server.ingest(&m3, CLIENT, 0, no_entropy, |_| true).unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let m4 = server_io.last();
    let parsed = decode(&m4).unwrap();
    let options: Vec<_> = parsed.options().collect();
    let separate4 = recode(
        &m4,
        Type::Confirmable,
        MessageId::new(901),
        parsed.token(),
        &options,
        parsed.payload(),
    );
    client
        .ingest(&separate4, SERVER, 0, no_entropy, |_| true)
        .unwrap();
    assert!(client.take_session().is_none());
    client.flush(&mut client_io, 0).unwrap();
    assert!(client.take_session().is_some());
    client
        .ingest(&separate4, SERVER, 2000, no_entropy, no_authorization)
        .unwrap();
    client.flush(&mut client_io, 2000).unwrap();
    assert_eq!(decode(&client_io.last()).unwrap().message_id().get(), 901);
}

#[test]
fn empty_ack_stops_retransmission_while_wrong_mid_ack_is_ignored() {
    let a = identity(1, 0);
    let mut client =
        CoapProvisioner::start(&a, identity(2, 1).peer(), SERVER, 0, entropy(3)).unwrap();
    let mut io = Io::default();
    client.flush(&mut io, 0).unwrap();
    let mid = decode(&io.last()).unwrap().message_id();
    let mut ack = [0; 4];
    CoapMessage::empty_ack(mid.wrapping_add(1))
        .encode(&mut ack)
        .unwrap();
    assert_eq!(
        client
            .ingest(&ack, SERVER, 1, no_entropy, no_authorization)
            .unwrap(),
        Status::Ignored
    );
    assert_eq!(client.next_deadline(), Some(2000));
    CoapMessage::empty_ack(mid).encode(&mut ack).unwrap();
    client
        .ingest(&ack, SERVER, 1, no_entropy, no_authorization)
        .unwrap();
    assert_eq!(client.next_deadline(), Some(EXCHANGE_LIFETIME_MS));
    client.flush(&mut io, 62000).unwrap();
    assert_eq!(io.attempts.len(), 1);
    assert_eq!(
        client.flush(&mut io, EXCHANGE_LIFETIME_MS),
        Err(PollError::Timeout)
    );
}

#[test]
fn older_request_duplicates_cannot_displace_an_unsent_final_confirmation() {
    for fail_first in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
        let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
        let mut client_io = Io::default();
        let mut server_io = Io::default();
        client.flush(&mut client_io, 0).unwrap();
        let m1 = client_io.last();
        server
            .ingest(&m1, CLIENT, 0, entropy(4), no_authorization)
            .unwrap();
        server.flush(&mut server_io, 0).unwrap();
        let m2 = server_io.last();
        client.ingest(&m2, SERVER, 0, no_entropy, |_| true).unwrap();
        client.flush(&mut client_io, 0).unwrap();
        let m3 = client_io.last();
        server.ingest(&m3, CLIENT, 0, no_entropy, |_| true).unwrap();
        assert!(server.take_session().is_none());
        if fail_first {
            server_io.fail = true;
            assert_eq!(server.flush(&mut server_io, 0), Err(PollError::Io(9)));
            server_io.fail = false;
            assert!(server.take_session().is_none());
        }
        server
            .ingest(&m1, CLIENT, 0, no_entropy, no_authorization)
            .unwrap();
        server_io.fail_on = Some(server_io.attempts.len() + 2);
        let before = server_io.attempts.len();
        assert_eq!(server.flush(&mut server_io, 0), Err(PollError::Io(9)));
        assert_eq!(server_io.attempts[before].1, m2);
        let m4 = server_io.last();
        assert_eq!(
            decode(&m4).unwrap().message_id(),
            decode(&m3).unwrap().message_id()
        );
        assert!(server.take_session().is_none());
        assert_eq!(server.next_deadline(), Some(0));
        server_io.fail_on = None;
        server.flush(&mut server_io, 1).unwrap();
        assert_eq!(server_io.last(), m4);
        assert_eq!(
            *server.take_session().unwrap().principal(),
            a.peer().principal()
        );
        client.ingest(&m4, SERVER, 1, no_entropy, |_| true).unwrap();
        assert_eq!(
            *client.take_session().unwrap().principal(),
            b.peer().principal()
        );
    }
}

#[test]
fn older_response_duplicates_cannot_displace_an_unsent_final_acknowledgement() {
    for fail_first in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
        let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
        let mut client_io = Io::default();
        let mut server_io = Io::default();
        client.flush(&mut client_io, 0).unwrap();
        server
            .ingest(&client_io.last(), CLIENT, 0, entropy(4), no_authorization)
            .unwrap();
        server.flush(&mut server_io, 0).unwrap();
        let response2 = server_io.last();
        let parsed = decode(&response2).unwrap();
        let options: Vec<_> = parsed.options().collect();
        let separate2 = recode(
            &response2,
            Type::Confirmable,
            MessageId::new(900),
            parsed.token(),
            &options,
            parsed.payload(),
        );
        client
            .ingest(&separate2, SERVER, 0, no_entropy, |_| true)
            .unwrap();
        client.flush(&mut client_io, 0).unwrap();
        server
            .ingest(&client_io.last(), CLIENT, 0, no_entropy, |_| true)
            .unwrap();
        server.flush(&mut server_io, 0).unwrap();
        let response4 = server_io.last();
        let parsed = decode(&response4).unwrap();
        let options: Vec<_> = parsed.options().collect();
        let separate4 = recode(
            &response4,
            Type::Confirmable,
            MessageId::new(901),
            parsed.token(),
            &options,
            parsed.payload(),
        );
        client
            .ingest(&separate4, SERVER, 0, no_entropy, |_| true)
            .unwrap();
        assert!(client.take_session().is_none());
        if fail_first {
            client_io.fail = true;
            assert_eq!(client.flush(&mut client_io, 0), Err(PollError::Io(9)));
            client_io.fail = false;
            assert!(client.take_session().is_none());
        }
        client
            .ingest(&separate2, SERVER, 0, no_entropy, no_authorization)
            .unwrap();
        client_io.fail_on = Some(client_io.attempts.len() + 2);
        let before = client_io.attempts.len();
        assert_eq!(client.flush(&mut client_io, 0), Err(PollError::Io(9)));
        let older_ack = decode(&client_io.attempts[before].1).unwrap();
        assert!(older_ack.is_empty_ack());
        assert_eq!(older_ack.message_id().get(), 900);
        let final_ack = client_io.last();
        assert_eq!(decode(&final_ack).unwrap().message_id().get(), 901);
        assert!(client.take_session().is_none());
        assert_eq!(client.next_deadline(), Some(0));
        client_io.fail_on = None;
        client.flush(&mut client_io, 1).unwrap();
        assert_eq!(client_io.last(), final_ack);
        assert_eq!(
            *client.take_session().unwrap().principal(),
            b.peer().principal()
        );
    }
}

#[test]
fn failed_and_short_sends_preserve_exact_message4_and_withhold_session() {
    for short in [false, true] {
        let a = identity(1, 0);
        let b = identity(2, 1);
        let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
        let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
        let mut client_io = Io::default();
        let mut server_io = Io::default();
        client.flush(&mut client_io, 0).unwrap();
        server
            .ingest(&client_io.last(), CLIENT, 0, entropy(4), no_authorization)
            .unwrap();
        server.flush(&mut server_io, 0).unwrap();
        client
            .ingest(&server_io.last(), SERVER, 0, no_entropy, |_| true)
            .unwrap();
        client.flush(&mut client_io, 0).unwrap();
        server
            .ingest(&client_io.last(), CLIENT, 0, no_entropy, |_| true)
            .unwrap();
        server_io.fail = !short;
        server_io.short = short;
        let error = server.flush(&mut server_io, 0).unwrap_err();
        assert_eq!(
            error,
            if short {
                PollError::ShortSend
            } else {
                PollError::Io(9)
            }
        );
        let failed = server_io.last();
        assert!(server.take_session().is_none());
        server_io.fail = false;
        server_io.short = false;
        server.flush(&mut server_io, 1).unwrap();
        assert_eq!(server_io.last(), failed);
        assert!(server.take_session().is_some());
        client
            .ingest(&server_io.last(), SERVER, 1, no_entropy, |_| true)
            .unwrap();
        assert!(client.take_session().is_some());
    }
}

#[test]
fn wire_valid_malformed_authentication_consumes_state_without_ack_or_session() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client.flush(&mut client_io, 0).unwrap();
    server
        .ingest(&client_io.last(), CLIENT, 0, entropy(4), no_authorization)
        .unwrap();
    server.flush(&mut server_io, 0).unwrap();
    let good = server_io.last();
    let parsed = decode(&good).unwrap();
    let options: Vec<_> = parsed.options().collect();
    let mut payload = parsed.payload().to_vec();
    *payload.last_mut().unwrap() ^= 1;
    let bad = recode(
        &good,
        Type::Confirmable,
        MessageId::new(900),
        parsed.token(),
        &options,
        &payload,
    );
    let error = client
        .ingest(&bad, SERVER, 0, no_entropy, no_authorization)
        .unwrap_err();
    assert!(matches!(error, PollError::Provisioning(_)));
    assert_eq!(
        client.ingest(&good, SERVER, 0, no_entropy, no_authorization),
        Err(error)
    );
    assert!(client.take_session().is_none());
    assert_eq!(client_io.attempts.len(), 1);
    assert_eq!(client.next_deadline(), None);
}

#[test]
fn backward_clock_and_overflow_are_terminal_before_fresh_entropy() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    assert!(matches!(
        CoapProvisioner::start(&a, b.peer(), SERVER, u64::MAX, no_entropy),
        Err(PollError::Clock)
    ));
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 10, entropy(3)).unwrap();
    assert_eq!(client.flush(&mut Io::default(), 9), Err(PollError::Clock));
    assert_eq!(client.next_deadline(), None);
    let mut valid = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut io = Io::default();
    valid.flush(&mut io, 0).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, u64::MAX);
    assert_eq!(
        server.ingest(&io.last(), CLIENT, u64::MAX, no_entropy, no_authorization),
        Err(PollError::Clock)
    );
    assert_eq!(server.next_deadline(), None);
}

#[test]
fn poll_receives_one_datagram_and_uses_the_cached_response() {
    let a = identity(1, 0);
    let b = identity(2, 1);
    let mut client = CoapProvisioner::start(&a, b.peer(), SERVER, 0, entropy(3)).unwrap();
    let mut server = CoapProvisioner::listen(&b, a.peer(), CLIENT, 0);
    let mut client_io = Io::default();
    let mut server_io = Io::default();
    client
        .poll(&mut client_io, 0, no_entropy, no_authorization)
        .unwrap();
    server_io.incoming.push_back((CLIENT, client_io.last()));
    server_io.incoming.push_back((OTHER, client_io.last()));
    server
        .poll(&mut server_io, 0, entropy(4), no_authorization)
        .unwrap();
    assert_eq!(server_io.receives, 1);
    assert_eq!(server_io.incoming.len(), 1);
    client_io.incoming.push_back((SERVER, server_io.last()));
    client
        .poll(&mut client_io, 0, no_entropy, |_| true)
        .unwrap();
    server_io.incoming.clear();
    server_io.incoming.push_back((CLIENT, client_io.last()));
    assert_eq!(
        server
            .poll(&mut server_io, 0, no_entropy, |_| true)
            .unwrap(),
        Status::Complete
    );
    client_io.incoming.push_back((SERVER, server_io.last()));
    assert_eq!(
        client
            .poll(&mut client_io, 0, no_entropy, |_| true)
            .unwrap(),
        Status::Complete
    );
    assert!(client.take_session().is_some());
    assert!(server.take_session().is_some());
}
