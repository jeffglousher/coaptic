use super::*;
use core::{
    future::Future,
    task::{Context, Poll, Waker},
};

fn register(protected: bool) -> (Server, Peer) {
    let mut s = fixed();
    let mut p = Peer::new(&mut s, protected);
    let q = BlockValue::new(0, true, 1).unwrap().encode();
    let opts = [
        Opt::observe_register(),
        Opt::uri_path("obs"),
        Opt::q_block2(&q),
    ];
    s.transport_mut().incoming = Some(
        p.encode(
            &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(700))
                .with_token(key().token())
                .with_options(&opts),
        ),
    );
    s.poll_with(0, |_, _| response(b"initial").etag(b"v0"))
        .unwrap();
    assert!(s.transport_mut().take().is_some());
    assert!(s.transport_mut().take().is_none());
    (s, p)
}

fn chunk(s: &mut Server, p: &Peer, num: u32, terminal: bool) -> (MessageId, Option<u32>) {
    let wire = s.transport_mut().take().expect("next CON chunk");
    let opened = p.open(wire);
    let packet = decode(&opened.bytes[..opened.len]).unwrap();
    assert_eq!(packet.ty(), Type::Confirmable);
    assert_eq!(
        packet.code(),
        if terminal {
            Code::INTERNAL_SERVER_ERROR
        } else {
            Code::CONTENT
        }
    );
    let q = packet.q_block2().next().unwrap().unwrap();
    assert_eq!((q.num(), q.szx(), q.more()), (num, 1, num < 21));
    let start = num as usize * 32;
    assert_eq!(
        packet.payload(),
        &LARGE[start..(start + 32).min(LARGE.len())]
    );
    assert_eq!(packet.etag().next(), Some(b"v1".as_slice()));
    assert_eq!(packet.observe().is_none(), terminal);
    assert!(
        s.transport_mut().take().is_none(),
        "NSTART allows one chunk only"
    );
    (packet.message_id(), packet.observe().map(Result::unwrap))
}

#[test]
fn con_q_observe_crosses_windows_exact_acks_terminal_and_next_epoch() {
    for protected in [false, cfg!(feature = "oscore")] {
        for terminal in [false, true] {
            let (mut s, p) = register(protected);
            let value = if terminal {
                Response::new(Code::INTERNAL_SERVER_ERROR)
                    .payload_borrowed(&LARGE)
                    .etag(b"v1")
            } else {
                large()
            };
            assert_eq!(s.notify_confirmable(5000, &["obs"], value).unwrap(), 1);
            assert_eq!(s.engine_mut().lookup_observe(key()).is_none(), terminal);
            let (mut mid, epoch) = chunk(&mut s, &p, 0, terminal);
            let first = mid;
            s.transport_mut().incoming =
                Some(control(mid.wrapping_add(1000), Type::Acknowledgement));
            s.poll(5001).unwrap();
            assert!(s.transport_mut().take().is_none());
            let mut foreign = control(mid, Type::Acknowledgement);
            foreign.peer = Endpoint::v4([192, 0, 2, 99], 5683);
            s.transport_mut().incoming = Some(foreign);
            s.poll(5002).unwrap();
            assert!(s.transport_mut().take().is_none());
            for num in 1..22 {
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(5010 + u64::from(num)).unwrap();
                let (next, sequence) = chunk(&mut s, &p, num, terminal);
                assert_ne!(next, mid);
                assert_eq!(sequence, epoch);
                mid = next;
                if num == 1 {
                    s.transport_mut().incoming = Some(control(first, Type::Acknowledgement));
                    s.poll(5012).unwrap();
                    assert!(
                        s.transport_mut().take().is_none(),
                        "stale ACK cannot skip a chunk"
                    );
                }
            }
            s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
            s.poll(5100).unwrap();
            assert!(s.transport_mut().take().is_none());
            assert!(
                body(&mut s).is_some(),
                "completed snapshot remains recoverable"
            );
            if !terminal {
                assert_eq!(s.notify_confirmable(5200, &["obs"], large()).unwrap(), 1);
                let (_, next_epoch) = chunk(&mut s, &p, 0, false);
                if !protected {
                    assert_ne!(next_epoch, epoch);
                }
            }
        }
    }
}

thread_local! { static BLOCK_SEND: Cell<bool> = const { Cell::new(false) }; }
impl coaptic::storage::AsyncDatagramIo for Io {
    async fn recv(&mut self, out: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        match DatagramIo::recv(self, out)? {
            Some(frame) => Ok(frame),
            None => core::future::pending().await,
        }
    }
    async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if BLOCK_SEND.with(Cell::get) {
            core::future::pending::<()>().await;
        }
        DatagramIo::send(self, peer, bytes)
    }
}
struct Timer;
impl coaptic::platform::AsyncTimer for Timer {
    async fn wait_until(&mut self, _: u64) {
        core::future::pending::<()>().await;
    }
}

#[cfg(feature = "oscore")]
fn protected_admission<S: AppStorage<Token = coaptic::message::Token>>(mut server: Server<S>) {
    let mut peer = Peer::new(&mut server, true);
    let held = core::array::from_fn::<_, 3, _>(|_| server.engine_mut().acquire_tx().unwrap());
    let options = [Opt::uri_path("obs")];
    let message = Message::new(Type::Confirmable, Code::GET, MessageId::new(801))
        .with_token(Token::new(b"pressure").unwrap())
        .with_options(&options);
    server.transport_mut().incoming = Some(peer.encode(&message));
    let mut effects = 0;
    assert!(
        server
            .poll_with_oscore_checkpoint_and_dispatch(
                0,
                |_| false,
                |_, _| {
                    effects += 1;
                    response(b"effect")
                }
            )
            .is_err()
    );
    assert!(server.transport_mut().take().is_none());
    assert_eq!(server.metrics().admission_refusal_sent, 0);
    let mut checkpoints = 0;
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            1,
            |_| {
                checkpoints += 1;
                true
            },
            |_, _| {
                effects += 1;
                response(b"effect")
            },
        )
        .unwrap();
    assert!(checkpoints >= 2);
    let opened = peer.open(server.transport_mut().take().unwrap());
    assert_eq!(
        decode(&opened.bytes[..opened.len]).unwrap().code(),
        Code::SERVICE_UNAVAILABLE
    );
    assert_eq!(effects, 0);
    assert_eq!(server.metrics().admission_refusal_sent, 1);
    assert_eq!(server.engine_mut().tx_occupied(), 3);
    assert_eq!(server.engine_mut().rx_occupied(), 0);
    let unauthenticated = Message::new(Type::Confirmable, Code::GET, MessageId::new(803))
        .with_token(Token::new(b"pressure").unwrap())
        .with_options(&options);
    let mut tampered = peer.encode(&unauthenticated);
    tampered.bytes[tampered.len - 1] ^= 1;
    server.transport_mut().incoming = Some(tampered);
    server.poll_with_oscore_checkpoint(2, |_| true).unwrap();
    assert!(server.transport_mut().take().is_none());
    assert_eq!(server.metrics().admission_refusal_sent, 1);
    for slot in held {
        server.engine_mut().release_tx(slot).unwrap();
    }
    let fresh = Message::new(Type::Confirmable, Code::GET, MessageId::new(802))
        .with_token(Token::new(b"pressure").unwrap())
        .with_options(&options);
    server.transport_mut().incoming = Some(peer.encode(&fresh));
    server
        .poll_with_oscore_checkpoint_and_dispatch(
            3,
            |_| true,
            |_, _| {
                effects += 1;
                response(b"effect")
            },
        )
        .unwrap();
    assert_eq!(effects, 1);
    let opened = peer.open(server.transport_mut().take().unwrap());
    assert_eq!(
        decode(&opened.bytes[..opened.len]).unwrap().payload(),
        b"effect"
    );
}

#[cfg(feature = "oscore")]
#[test]
fn admission_protected_refusal_obeys_checkpoint_and_authentication() {
    protected_admission(fixed());
}

#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn admission_allocated_protected_refusal_has_same_contract() {
    let resources = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .deferred::<2>();
    let server = App::with_resources(resources.allocated(usize::MAX))
        .allow_unprotected()
        .deterministic_for_tests()
        .route("obs", coaptic::get(handler).observe(source))
        .bind(Io::default())
        .unwrap();
    protected_admission(server);
}

#[test]
fn admission_async_cancelled_refusal_releases_control_slot() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let held = core::array::from_fn::<_, 3, _>(|_| server.engine_mut().acquire_tx().unwrap());
        let options = [Opt::uri_path("obs")];
        let message = Message::new(Type::Confirmable, Code::GET, MessageId::new(900))
            .with_token(Token::new(b"pressure").unwrap())
            .with_options(&options);
        let frame = peer.encode(&message);
        let rx = server.engine_mut().acquire_rx().unwrap();
        server
            .engine_mut()
            .write_rx(rx, &frame.bytes[..frame.len], frame.peer)
            .unwrap();
        let mut clock = || Ok::<u64, ()>(0);
        let mut timer = Timer;
        BLOCK_SEND.with(|value| value.set(true));
        {
            let mut work = core::pin::pin!(server.progress_async(0, &mut clock, &mut timer, 1000));
            assert!(
                work.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        BLOCK_SEND.with(|value| value.set(false));
        assert_eq!(server.engine_mut().tx_occupied(), 3);
        assert_eq!(server.engine_mut().rx_occupied(), 0);
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.metrics().admission_refusal_sent, 0);
        assert_eq!(server.metrics().admission_refusal_unsent, 1);
        for slot in held {
            server.engine_mut().release_tx(slot).unwrap();
        }
    }
}

#[test]
fn con_q_observe_cancelled_continuation_retries_same_block_without_skipping() {
    for protected in [false, cfg!(feature = "oscore")] {
        let (mut s, p) = register(protected);
        s.notify_confirmable(5000, &["obs"], large()).unwrap();
        let (mid, epoch) = chunk(&mut s, &p, 0, false);
        let ack = control(mid, Type::Acknowledgement);
        let rx = s.engine_mut().acquire_rx().unwrap();
        s.engine_mut()
            .write_rx(rx, &ack.bytes[..ack.len], ack.peer)
            .unwrap();
        let mut clock = || Ok::<u64, ()>(5100);
        let mut timer = Timer;
        BLOCK_SEND.with(|b| b.set(true));
        {
            let mut progress =
                core::pin::pin!(s.progress_async(5100, &mut clock, &mut timer, 6000));
            assert!(
                progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        BLOCK_SEND.with(|b| b.set(false));
        assert!(s.transport_mut().take().is_none());
        assert_eq!(s.next_poll_at(5100), Some(5100));
        {
            let mut progress =
                core::pin::pin!(s.progress_async(5100, &mut clock, &mut timer, 6000));
            assert!(matches!(
                progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Ok(()))
            ));
        }
        assert_eq!(chunk(&mut s, &p, 1, false).1, epoch);
    }
}

#[test]
fn admission_continue_and_refusal_replay_exact_ciphertext_without_packet_pin() {
    for protected in [false, cfg!(feature = "oscore")] {
        for refuse in [false, true] {
            let mut server = fixed();
            let mut peer = Peer::new(&mut server, protected);
            let block = BlockValue::new(0, true, 0).unwrap().encode();
            let options = [Opt::uri_path("obs"), Opt::block1(&block)];
            let message = Message::new(Type::Confirmable, Code::PUT, MessageId::new(910))
                .with_token(Token::new(b"upload").unwrap())
                .with_options(&options)
                .with_payload(&[1; 16]);
            let frame = peer.encode(&message);
            let held = if refuse {
                Some(core::array::from_fn::<_, 3, _>(|_| {
                    server.engine_mut().acquire_tx().unwrap()
                }))
            } else {
                None
            };
            server.transport_mut().incoming = Some(frame);
            server
                .poll_with(0, |_, _| {
                    panic!("no upload completion or overload side effects")
                })
                .unwrap();
            let first = server.transport_mut().take().unwrap();
            assert_eq!(
                server.engine_mut().tx_occupied(),
                if refuse { 3 } else { 0 }
            );
            // The peer lost the response, so it retries the exact original request.
            server.transport_mut().incoming = Some(frame);
            server
                .poll_with(1, |_, _| panic!("duplicate cannot dispatch"))
                .unwrap();
            let replay = server.transport_mut().take().unwrap();
            assert_eq!(&first.bytes[..first.len], &replay.bytes[..replay.len]);
            assert_eq!(
                server.engine_mut().tx_occupied(),
                if refuse { 3 } else { 0 }
            );
            let opened = peer.open(replay);
            assert_eq!(
                decode(&opened.bytes[..opened.len]).unwrap().code(),
                if refuse {
                    Code::SERVICE_UNAVAILABLE
                } else {
                    Code::CONTINUE
                }
            );
            if let Some(held) = held {
                for slot in held {
                    server.engine_mut().release_tx(slot).unwrap();
                }
            }
        }
    }
}

#[test]
fn con_q_observe_recovery_retains_bounded_selection_until_exact_ack() {
    for protected in [false, cfg!(feature = "oscore")] {
        for suffix in [false, true] {
            let (mut s, mut p) = register(protected);
            s.notify_confirmable(5000, &["obs"], large()).unwrap();
            for num in 0..22 {
                let (mid, _) = chunk(&mut s, &p, num, false);
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(5010 + u64::from(num)).unwrap();
            }
            let first = BlockValue::new(1, suffix, 1).unwrap().encode();
            let third = BlockValue::new(3, false, 1).unwrap().encode();
            let other_set = BlockValue::new(11, false, 1).unwrap().encode();
            let single = BlockValue::new(1, false, 1).unwrap().encode();
            let original = body(&mut s);
            s.transport_mut().incoming = Some(
                p.encode(
                    &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(899))
                        .with_token(key().token())
                        .with_options(&[
                            Opt::uri_path("obs"),
                            Opt::q_block2(&single),
                            Opt::q_block2(&other_set),
                        ]),
                ),
            );
            s.poll_with(5900, |_, _| panic!("capacity refusal retains snapshot"))
                .unwrap();
            let refused = p.open(s.transport_mut().take().expect("bounded recovery refusal"));
            let refused = decode(&refused.bytes[..refused.len]).unwrap();
            assert_eq!(refused.code(), Code::SERVICE_UNAVAILABLE);
            assert_eq!(refused.max_age(), Some(Ok(1)));
            assert!(refused.observe().is_none());
            assert_eq!(body(&mut s), original);
            let mut options = coaptic::message::OptionsBuilder::<3>::new();
            options.push(Opt::uri_path("obs")).unwrap();
            options.push(Opt::q_block2(&first)).unwrap();
            if !suffix {
                options.push(Opt::q_block2(&third)).unwrap();
            }
            s.transport_mut().incoming = Some(
                p.encode(
                    &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(900))
                        .with_token(key().token())
                        .with_options(options.as_slice()),
                ),
            );
            without_allocating(|| {
                s.poll_with(6000, |_, _| panic!("retained recovery snapshot"))
                    .unwrap()
            });
            let selected: &[u32] = if suffix {
                &[1, 2, 3, 4, 5, 6, 7, 8, 9]
            } else {
                &[1, 3]
            };
            for (index, &num) in selected.iter().enumerate() {
                let wire = s.transport_mut().take().expect("selected CON payload");
                let frame = p.open(wire);
                let message = decode(&frame.bytes[..frame.len]).unwrap();
                assert_eq!(message.ty(), Type::Confirmable);
                assert_eq!(message.q_block2().next().unwrap().unwrap().num(), num);
                assert!(
                    message.observe().is_none(),
                    "missing-block responses never carry Observe"
                );
                assert_eq!(
                    message.payload(),
                    &LARGE[num as usize * 32..(num as usize + 1) * 32]
                );
                assert!(s.transport_mut().take().is_none());
                let mid = message.message_id();
                s.transport_mut().incoming =
                    Some(control(mid.wrapping_add(500), Type::Acknowledgement));
                s.poll(6010 + index as u64 * 2).unwrap();
                assert!(s.transport_mut().take().is_none());
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(6011 + index as u64 * 2).unwrap();
            }
            assert!(
                s.transport_mut().take().is_none(),
                "selection must not advance to another set"
            );
            assert!(s.engine_mut().lookup_observe(key()).is_some());
            s.notify_confirmable(10000, &["obs"], large()).unwrap();
            chunk(&mut s, &p, 0, false);
        }
    }
}
