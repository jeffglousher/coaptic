use super::{client_c1, server_c1};
use crate::app::{AppStorage, DEFAULT_ROUTES, Method};
use crate::message::{
    Code, ContentFormat, Message, MessageId, Opt, OptionNumber, Token, Type, decode, encode,
};
use crate::oscore::{ReplayCheckpoint, SecurityContext};
use crate::storage::{DatagramIo, Endpoint};
use crate::{App, Request, Response, profiles};
use core::cell::Cell;

type Packet = (Endpoint, [u8; 256], usize);

#[derive(Default)]
struct Events {
    durable: Cell<Option<ReplayCheckpoint>>,
    checkpoint_calls: Cell<usize>,
    receives: Cell<usize>,
    effects: Cell<usize>,
    required_sequence: Cell<Option<u64>>,
}

impl Events {
    fn persist(&self, checkpoint: ReplayCheckpoint) -> bool {
        self.checkpoint_calls.set(self.checkpoint_calls.get() + 1);
        self.durable.set(Some(checkpoint));
        true
    }

    fn assert_durable(&self) {
        let checkpoint = self.durable.get().expect("checkpoint committed before I/O");
        if let Some(sequence) = self.required_sequence.get() {
            assert!(rejects(checkpoint, sequence));
        }
    }
}

struct DurableIo<'a> {
    events: &'a Events,
    inbox: Option<Packet>,
    sent: [Option<Packet>; 8],
    sent_len: usize,
    fail_send: bool,
}

impl<'a> DurableIo<'a> {
    fn new(events: &'a Events) -> Self {
        Self {
            events,
            inbox: None,
            sent: [None; 8],
            sent_len: 0,
            fail_send: false,
        }
    }
}

impl DatagramIo for DurableIo<'_> {
    type Error = &'static str;

    fn recv(&mut self, bytes: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        assert!(self.events.durable.get().is_some());
        self.events.receives.set(self.events.receives.get() + 1);
        let Some((peer, packet, len)) = self.inbox.take() else {
            return Ok(None);
        };
        if len > bytes.len() {
            return Err("receive overflow");
        }
        bytes[..len].copy_from_slice(&packet[..len]);
        Ok(Some((len, peer)))
    }

    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.events.assert_durable();
        if core::mem::take(&mut self.fail_send) {
            return Err("injected send failure");
        }
        assert!(bytes.len() <= 256);
        assert!(self.sent_len < self.sent.len());
        let mut packet = [0; 256];
        packet[..bytes.len()].copy_from_slice(bytes);
        self.sent[self.sent_len] = Some((peer, packet, bytes.len()));
        self.sent_len += 1;
        Ok(bytes.len())
    }
}

fn rejects(checkpoint: ReplayCheckpoint, sequence: u64) -> bool {
    let (left, bits) = checkpoint.parts();
    sequence < left || (sequence - left < 32 && bits & (1 << (sequence - left)) != 0)
}

fn peer() -> Endpoint {
    Endpoint::v4([192, 0, 2, 1], 5683)
}

fn request(sender: &mut SecurityContext, ty: Type, mid: u16) -> Packet {
    let format = ContentFormat::new(60).encode();
    let options = [
        Opt::uri_path("durable"),
        Opt::uri_path("update"),
        Opt::content_format(&format),
        Opt::uri_query("revision=7"),
    ];
    let message = Message::new(ty, Code::PUT, MessageId::new(mid))
        .with_token(Token::new(&[0xa1, 0x04]).unwrap())
        .with_options(&options)
        .with_payload(b"complete durable request");
    let mut packet = [0; 256];
    let len = sender.protect_request(&message, &mut packet).unwrap();
    (peer(), packet, len)
}

fn assert_request(events: &Events, request: &Request<'_>, sequence: u64, mid: u16) {
    assert!(rejects(events.durable.get().unwrap(), sequence));
    assert_eq!(request.method(), Some(Method::Put));
    assert_eq!(request.path(), &["durable", "update"]);
    assert_eq!(request.peer(), peer());
    assert_eq!(request.message_id(), MessageId::new(mid));
    assert_eq!(request.token(), Token::new(&[0xa1, 0x04]).unwrap());
    assert_eq!(request.content_format(), Some(Ok(ContentFormat::new(60))));
    let mut query = request.uri_query();
    assert_eq!(query.next(), Some(Ok("revision=7")));
    assert_eq!(query.next(), None);
    assert_eq!(request.payload(), b"complete durable request");
    events.effects.set(events.effects.get() + 1);
}

fn exercise_commit_ordering<S: AppStorage>(
    mut server: App<profiles::Default, DurableIo<'_>, DEFAULT_ROUTES, false, S>,
    events: &Events,
) {
    let mut sender = client_c1();
    let packet = request(&mut sender, Type::Confirmable, 100);
    server.transport_mut().inbox = Some(packet);
    events.required_sequence.set(Some(0));
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            0,
            |checkpoint| {
                if events.checkpoint_calls.get() == 0 {
                    assert_eq!(events.receives.get(), 0);
                    assert_eq!(checkpoint.parts(), (0, 0));
                } else {
                    assert!(rejects(checkpoint, 0));
                }
                events.persist(checkpoint)
            },
            |opened, handle| {
                assert_eq!(events.checkpoint_calls.get(), 2);
                assert!(handle.is_none());
                assert_request(events, &opened, 0, 100);
                Response::changed()
                    .payload_copy(b"committed reply")
                    .content_format(ContentFormat::new(60))
            },
        )
        .unwrap();
    assert_eq!(events.effects.get(), 1);
    let first = server.transport().sent[0].unwrap();
    assert_eq!(first.0, peer());
    let outer = decode(&first.1[..first.2]).unwrap();
    assert!(outer.oscore().is_some());
    let mut scratch = [0; 256];
    let (response, _) = sender
        .unprotect_bound_response(&outer, &mut scratch)
        .unwrap();
    assert_eq!(response.code(), Code::CHANGED);
    assert_eq!(response.content_format(), Some(Ok(ContentFormat::new(60))));
    assert_eq!(response.payload(), b"committed reply");

    let received = events.receives.get();
    assert!(matches!(
        server.poll(1),
        Err(crate::Error::OscoreCheckpointRequired)
    ));
    assert!(matches!(
        server.poll_with(1, |_, _| panic!("unguarded dispatch")),
        Err(crate::Error::OscoreCheckpointRequired)
    ));
    assert_eq!(events.receives.get(), received);

    server.transport_mut().inbox = Some(packet);
    server
        .poll_with_oscore_checkpoint(1, |_| panic!("accepted duplicate does not persist"))
        .unwrap();
    assert_eq!(server.transport().sent[1], Some(first));
    assert_eq!(events.effects.get(), 1);
    server
        .poll_with_oscore_checkpoint(2, |_| panic!("clean idle does not persist"))
        .unwrap();
    assert_eq!(events.checkpoint_calls.get(), 2);
}

#[test]
fn fixed_checkpoint_barrier_commits_before_dispatch_and_ack_and_replays_cached_con() {
    let events = Events::default();
    let server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    exercise_commit_ordering(server, &events);
}

#[test]
fn caller_selected_datagram_storage_preserves_checkpoint_barrier_and_reply_identity() {
    let events = Events::default();
    let server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind_storage(
            DurableIo::new(&events),
            crate::storage::Memory::<profiles::Default>::new(),
        )
        .unwrap();
    exercise_commit_ordering(server, &events);
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_checkpoint_barrier_preserves_commit_ordering_and_exact_cached_response() {
    let events = Events::default();
    let server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind_alloc(
            DurableIo::new(&events),
            crate::storage::Capacities::from_profile::<profiles::Default>(),
        )
        .unwrap();
    exercise_commit_ordering(server, &events);
}

#[test]
fn checkpoint_preflight_failure_and_required_policy_stop_before_io() {
    let events = Events::default();
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    assert!(matches!(
        server.poll_with_oscore_checkpoint(0, |_| false),
        Err(crate::Error::OscoreCheckpointFailed)
    ));
    assert_eq!(events.receives.get(), 0);
    assert!(matches!(
        server.poll(0),
        Err(crate::Error::OscoreCheckpointRequired)
    ));
    assert_eq!(events.receives.get(), 0);
    server
        .poll_with_oscore_checkpoint(1, |checkpoint| events.persist(checkpoint))
        .unwrap();
    assert_eq!(events.receives.get(), 1);
    assert_eq!(events.checkpoint_calls.get(), 1);

    let result = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .block_wise::<true>()
        .deferred::<1>()
        .routes::<3>()
        .allow_plaintext()
        .bind(super::Loopback::default());
    assert!(matches!(
        result,
        Err(crate::error::BuildError::SecurityRequired)
    ));
}

#[test]
fn failed_request_checkpoint_burns_live_replay_and_refills_before_receiving_retry() {
    for persisted_before_failure in [false, true] {
        let events = Events::default();
        let mut sender = client_c1();
        let packet = request(&mut sender, Type::Confirmable, 101);
        let mut server = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .require_oscore_checkpoint()
            .oscore(server_c1())
            .bind(DurableIo::new(&events))
            .unwrap();
        server
            .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
            .unwrap();
        server.transport_mut().inbox = Some(packet);
        let mut failed = None;
        assert!(matches!(
            server.poll_with_oscore_checkpoint_and_dispatch(
                1,
                |checkpoint| {
                    assert!(rejects(checkpoint, 0));
                    failed = Some(checkpoint);
                    if persisted_before_failure {
                        events.durable.set(Some(checkpoint));
                    }
                    false
                },
                |_, _| panic!("failed persistence must not dispatch"),
            ),
            Err(crate::Error::OscoreCheckpointFailed)
        ));
        assert!(!server.oscore().unwrap().replay_fresh(0));
        assert_eq!(server.engine_mut().rx_occupied(), 0);
        assert_eq!(server.engine_mut().tx_occupied(), 0);
        assert_eq!(server.transport().sent_len, 0);
        assert_eq!(events.effects.get(), 0);
        let failed = failed.unwrap();
        assert_eq!(
            server
                .oscore_mut()
                .unwrap()
                .restore_replay(ReplayCheckpoint::from_parts(0, 0).unwrap()),
            Err(crate::oscore::Error::ReplayRollback)
        );

        let receives = events.receives.get();
        server.transport_mut().inbox = Some(packet);
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                2,
                |checkpoint| {
                    assert_eq!(events.receives.get(), receives);
                    assert_eq!(checkpoint, failed);
                    events.persist(checkpoint)
                },
                |_, _| panic!("live failed ciphertext stays replay-rejected"),
            )
            .unwrap();
        assert_eq!(server.transport().sent_len, 0);

        let next = request(&mut sender, Type::Confirmable, 102);
        events.required_sequence.set(Some(1));
        server.transport_mut().inbox = Some(next);
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                3,
                |checkpoint| {
                    assert!(rejects(checkpoint, 0));
                    assert!(rejects(checkpoint, 1));
                    events.persist(checkpoint)
                },
                |opened, _| {
                    assert_request(&events, &opened, 1, 102);
                    Response::changed()
                },
            )
            .unwrap();
        assert_eq!(events.effects.get(), 1);
        assert_eq!(server.transport().sent_len, 1);
    }
}

#[test]
fn restart_after_checkpoint_failure_distinguishes_unwritten_and_committed_state() {
    for persisted_before_failure in [false, true] {
        let events = Events::default();
        let mut sender = client_c1();
        let packet = request(&mut sender, Type::Confirmable, 103);
        {
            let mut server = App::profile::<profiles::Default>()
                .deterministic_for_tests()
                .block_wise::<false>()
                .require_oscore_checkpoint()
                .oscore(server_c1())
                .bind(DurableIo::new(&events))
                .unwrap();
            server
                .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
                .unwrap();
            server.transport_mut().inbox = Some(packet);
            assert!(matches!(
                server.poll_with_oscore_checkpoint_and_dispatch(
                    1,
                    |checkpoint| {
                        if persisted_before_failure {
                            events.durable.set(Some(checkpoint));
                        }
                        false
                    },
                    |_, _| panic!("crashed request must not dispatch"),
                ),
                Err(crate::Error::OscoreCheckpointFailed)
            ));
        }
        let mut context = server_c1();
        let parts = events.durable.get().unwrap().parts();
        context
            .restore_replay(ReplayCheckpoint::from_parts(parts.0, parts.1).unwrap())
            .unwrap();
        let mut restarted = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .require_oscore_checkpoint()
            .oscore(context)
            .bind(DurableIo::new(&events))
            .unwrap();
        restarted.transport_mut().inbox = Some(packet);
        events.required_sequence.set(Some(0));
        restarted
            .poll_with_oscore_checkpoint_and_dispatch(
                2,
                |checkpoint| events.persist(checkpoint),
                |opened, _| {
                    assert!(!persisted_before_failure);
                    assert_request(&events, &opened, 0, 103);
                    Response::changed()
                },
            )
            .unwrap();
        assert_eq!(events.effects.get(), usize::from(!persisted_before_failure));
        assert_eq!(
            restarted.transport().sent_len,
            usize::from(!persisted_before_failure)
        );
        assert!(!restarted.oscore().unwrap().replay_fresh(0));
    }
}

#[test]
fn failed_ack_after_durable_dispatch_replays_without_second_effect_or_checkpoint() {
    let events = Events::default();
    let mut sender = client_c1();
    let packet = request(&mut sender, Type::Confirmable, 104);
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    server.transport_mut().inbox = Some(packet);
    server.transport_mut().fail_send = true;
    events.required_sequence.set(Some(0));
    assert!(
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                0,
                |checkpoint| events.persist(checkpoint),
                |opened, _| {
                    assert_request(&events, &opened, 0, 104);
                    Response::changed().payload_copy(b"once")
                },
            )
            .is_err()
    );
    assert_eq!(events.effects.get(), 1);
    assert_eq!(events.checkpoint_calls.get(), 2);
    assert_eq!(server.transport().sent_len, 0);
    server.transport_mut().inbox = Some(packet);
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            1,
            |_| panic!("duplicate does not persist"),
            |_, _| panic!("duplicate does not dispatch"),
        )
        .unwrap();
    assert_eq!(server.transport().sent_len, 1);
    assert_eq!(events.effects.get(), 1);
    let response = server.transport().sent[0].unwrap();
    let mut scratch = [0; 256];
    let (opened, _) = sender
        .unprotect_bound_response(&decode(&response.1[..response.2]).unwrap(), &mut scratch)
        .unwrap();
    assert_eq!(opened.code(), Code::CHANGED);
    assert_eq!(opened.payload(), b"once");
}

#[test]
fn deferred_dispatch_commits_before_ack_and_restart_refuses_uncompleted_request() {
    let events = Events::default();
    let mut sender = client_c1();
    let packet = request(&mut sender, Type::Confirmable, 105);
    {
        let mut server = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<false>()
            .require_oscore_checkpoint()
            .deferred::<1>()
            .oscore(server_c1())
            .bind(DurableIo::new(&events))
            .unwrap();
        server.transport_mut().inbox = Some(packet);
        events.required_sequence.set(Some(0));
        let mut handle = None;
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                0,
                |checkpoint| events.persist(checkpoint),
                |opened, candidate| {
                    assert_request(&events, &opened, 0, 105);
                    handle = candidate;
                    Response::deferred()
                },
            )
            .unwrap();
        assert!(handle.is_some());
        let ack = server.transport().sent[0].unwrap();
        assert!(decode(&ack.1[..ack.2]).unwrap().is_empty_ack());
        server.transport_mut().inbox = Some(packet);
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                1,
                |_| panic!("pending duplicate does not persist"),
                |_, _| panic!("pending duplicate does not dispatch"),
            )
            .unwrap();
        assert_eq!(server.transport().sent[1], Some(ack));
    }
    let mut context = server_c1();
    context
        .restore_replay(events.durable.get().unwrap())
        .unwrap();
    let mut restarted = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .deferred::<1>()
        .oscore(context)
        .bind(DurableIo::new(&events))
        .unwrap();
    restarted.transport_mut().inbox = Some(packet);
    restarted
        .poll_with_oscore_checkpoint_and_dispatch(
            2,
            |checkpoint| events.persist(checkpoint),
            |_, _| panic!("durably accepted request cannot repeat after restart"),
        )
        .unwrap();
    assert_eq!(restarted.transport().sent_len, 0);
    assert_eq!(events.effects.get(), 1);
}

#[test]
fn corruption_plaintext_and_empty_controls_do_not_cross_request_checkpoint_barrier() {
    let events = Events::default();
    let mut sender = client_c1();
    let packet = request(&mut sender, Type::Confirmable, 106);
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    server
        .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
        .unwrap();
    let before = server.oscore().unwrap().replay_checkpoint();
    let mut corrupt = packet;
    corrupt.1[corrupt.2 - 1] ^= 1;
    server.transport_mut().inbox = Some(corrupt);
    server
        .poll_with_oscore_checkpoint(1, |_| panic!("corruption does not persist"))
        .unwrap();
    assert_eq!(server.oscore().unwrap().replay_checkpoint(), before);
    assert_eq!(server.transport().sent_len, 0);

    for (ty, code, mid) in [
        (Type::Confirmable, Code::PUT, 107),
        (Type::Acknowledgement, Code::EMPTY, 108),
        (Type::Reset, Code::EMPTY, 109),
        (Type::Confirmable, Code::EMPTY, 110),
    ] {
        let message = Message::new(ty, code, MessageId::new(mid));
        let mut bytes = [0; 256];
        let n = encode(&message, &mut bytes).unwrap();
        server.transport_mut().inbox = Some((peer(), bytes, n));
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                2,
                |_| panic!("unprotected traffic does not persist"),
                |_, _| panic!("unprotected traffic must not dispatch"),
            )
            .unwrap();
        assert_eq!(server.oscore().unwrap().replay_checkpoint(), before);
    }
    assert_eq!(events.checkpoint_calls.get(), 1);
    assert_eq!(events.effects.get(), 0);
    server.transport_mut().inbox = Some(packet);
    events.required_sequence.set(Some(0));
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            3,
            |checkpoint| events.persist(checkpoint),
            |opened, _| {
                assert_request(&events, &opened, 0, 106);
                Response::changed()
            },
        )
        .unwrap();
    assert_eq!(events.effects.get(), 1);
    assert_eq!(events.checkpoint_calls.get(), 2);
}

#[test]
fn authenticated_malformed_inner_replay_mutation_is_durable_before_next_io() {
    let events = Events::default();
    let mut sender = client_c1();
    let original = request(&mut sender, Type::Confirmable, 111);
    let outer = decode(&original.1[..original.2]).unwrap();
    let header = crate::oscore::OscoreHeader::parse(outer.oscore().unwrap()).unwrap();
    let piv = header.piv.unwrap();
    let aad = crate::oscore::aead::Aad::new(sender.sender_id(), piv.as_bytes()).unwrap();
    let mut ciphertext = [0; 10];
    ciphertext[..2].copy_from_slice(&[Code::PUT.as_raw(), 0xf0]);
    let n = crate::oscore::aead::seal_in_place(
        sender.sender_key(),
        &sender.request_nonce(piv),
        aad.as_bytes(),
        2,
        &mut ciphertext,
    )
    .unwrap();
    let options = [Opt::new(OptionNumber::OSCORE, outer.oscore().unwrap())];
    let malformed = Message::new(outer.ty(), outer.code(), outer.message_id())
        .with_token(outer.token())
        .with_options(&options)
        .with_payload(&ciphertext[..n]);
    let mut bytes = [0; 256];
    let len = encode(&malformed, &mut bytes).unwrap();
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    server
        .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
        .unwrap();
    server.transport_mut().inbox = Some((peer(), bytes, len));
    assert!(matches!(
        server.poll_with_oscore_checkpoint_and_dispatch(
            1,
            |checkpoint| {
                assert!(rejects(checkpoint, 0));
                false
            },
            |_, _| panic!("malformed authenticated request must not dispatch"),
        ),
        Err(crate::Error::OscoreCheckpointFailed)
    ));
    assert!(!server.oscore().unwrap().replay_fresh(0));
    assert_eq!(server.engine_mut().rx_occupied(), 0);
    assert_eq!(server.transport().sent_len, 0);
    let receives = events.receives.get();
    server.transport_mut().inbox = Some(original);
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            2,
            |checkpoint| {
                assert_eq!(events.receives.get(), receives);
                assert!(rejects(checkpoint, 0));
                events.persist(checkpoint)
            },
            |_, _| panic!("malformed authenticated PIV remains burned"),
        )
        .unwrap();
    assert_eq!(server.transport().sent_len, 0);
    assert_eq!(events.effects.get(), 0);
}

#[test]
fn context_access_and_replacement_require_checkpoint_before_receiving() {
    let events = Events::default();
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    server
        .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
        .unwrap();
    server.oscore_mut().unwrap().replay_accept(7);
    let receives = events.receives.get();
    server
        .poll_with_oscore_checkpoint(1, |checkpoint| {
            assert_eq!(events.receives.get(), receives);
            assert!(rejects(checkpoint, 7));
            events.persist(checkpoint)
        })
        .unwrap();
    server.set_oscore(server_c1());
    let receives = events.receives.get();
    server
        .poll_with_oscore_checkpoint(2, |checkpoint| {
            assert_eq!(events.receives.get(), receives);
            assert_eq!(checkpoint.parts(), (0, 0));
            events.persist(checkpoint)
        })
        .unwrap();
    assert_eq!(events.checkpoint_calls.get(), 3);
    assert!(matches!(
        server.poll(3),
        Err(crate::Error::OscoreCheckpointRequired)
    ));
}

#[test]
fn failed_checkpoint_precedes_block1_admission_and_preserves_partial_body_on_refill() {
    const BODY: &[u8; 32] = b"0123456789abcdefABCDEFGHIJKLMNOP";
    fn block(sender: &mut SecurityContext, num: u32, mid: u16) -> Packet {
        let block = crate::message::BlockValue::from_size(num, num == 0, 16)
            .unwrap()
            .encode();
        let format = ContentFormat::new(60).encode();
        let size = crate::message::encode_uint(32);
        let options = [
            Opt::uri_path("durable"),
            Opt::uri_path("update"),
            Opt::content_format(&format),
            Opt::uri_query("revision=7"),
            Opt::block1(&block),
            Opt::size1(&size),
            Opt::request_tag(b"upload"),
        ];
        let message = Message::new(Type::Confirmable, Code::PUT, MessageId::new(mid))
            .with_token(Token::new(&[0xa1, 0x04]).unwrap())
            .with_options(&options)
            .with_payload(&BODY[num as usize * 16..(num as usize + 1) * 16]);
        let mut bytes = [0; 256];
        let len = sender.protect_request(&message, &mut bytes).unwrap();
        (peer(), bytes, len)
    }

    let events = Events::default();
    let mut sender = client_c1();
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .require_oscore_checkpoint()
        .oscore(server_c1())
        .bind(DurableIo::new(&events))
        .unwrap();
    server
        .poll_with_oscore_checkpoint(0, |checkpoint| events.persist(checkpoint))
        .unwrap();
    server.transport_mut().inbox = Some(block(&mut sender, 0, 112));
    assert!(matches!(
        server.poll_with_oscore_checkpoint(1, |_| false),
        Err(crate::Error::OscoreCheckpointFailed)
    ));
    assert_eq!(server.transport().sent_len, 0);
    let first = server.engine_mut().acquire_rx_body().unwrap();
    let second = server.engine_mut().acquire_rx_body().unwrap();
    assert!(server.engine_mut().acquire_rx_body().is_none());
    server.engine_mut().release_rx_body(first).unwrap();
    server.engine_mut().release_rx_body(second).unwrap();
    server
        .poll_with_oscore_checkpoint(2, |checkpoint| events.persist(checkpoint))
        .unwrap();

    events.required_sequence.set(Some(1));
    server.transport_mut().inbox = Some(block(&mut sender, 0, 113));
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            3,
            |checkpoint| events.persist(checkpoint),
            |_, _| panic!("incomplete body must not dispatch"),
        )
        .unwrap();
    assert_eq!(server.transport().sent_len, 1);
    let body_slot = (0..2)
        .map(crate::storage::SlotId::from_index)
        .find(|id| server.engine().rx_body_transfer(*id).is_some())
        .unwrap();
    let retained = server.engine().rx_body_transfer(body_slot).unwrap();
    assert_eq!(
        server.engine().rx_body_payload(body_slot),
        Some(&BODY[..16])
    );
    server.transport_mut().inbox = Some(block(&mut sender, 1, 114));
    assert!(matches!(
        server.poll_with_oscore_checkpoint_and_dispatch(
            4,
            |_| false,
            |_, _| panic!("uncommitted final fragment must not dispatch"),
        ),
        Err(crate::Error::OscoreCheckpointFailed)
    ));
    assert_eq!(server.engine().rx_body_transfer(body_slot), Some(retained));
    assert_eq!(
        server.engine().rx_body_payload(body_slot),
        Some(&BODY[..16])
    );
    assert_eq!(server.transport().sent_len, 1);

    events.required_sequence.set(Some(3));
    server.transport_mut().inbox = Some(block(&mut sender, 1, 115));
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            5,
            |checkpoint| events.persist(checkpoint),
            |opened, _| {
                events.assert_durable();
                assert_eq!(opened.method(), Some(Method::Put));
                assert_eq!(opened.path(), &["durable", "update"]);
                assert_eq!(opened.peer(), peer());
                assert_eq!(opened.token(), Token::new(&[0xa1, 0x04]).unwrap());
                assert_eq!(opened.content_format(), Some(Ok(ContentFormat::new(60))));
                assert_eq!(opened.body(), Some(&BODY[..]));
                let mut query = opened.uri_query();
                assert_eq!(query.next(), Some(Ok("revision=7")));
                assert_eq!(query.next(), None);
                events.effects.set(events.effects.get() + 1);
                Response::changed().payload_copy(BODY)
            },
        )
        .unwrap();
    assert_eq!(events.effects.get(), 1);
    assert!(server.engine().rx_body_transfer(body_slot).is_none());
    assert_eq!(server.transport().sent_len, 2);
    let reply = server.transport().sent[1].unwrap();
    let mut scratch = [0; 256];
    let (opened, _) = sender
        .unprotect_bound_response(&decode(&reply.1[..reply.2]).unwrap(), &mut scratch)
        .unwrap();
    assert_eq!(opened.code(), Code::CHANGED);
    assert_eq!(opened.payload(), BODY);
}
