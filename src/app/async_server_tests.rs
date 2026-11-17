use super::*;
use crate::message::{Opt, Token, encode};
use crate::resources::Resources;
use crate::storage::profiles;
use core::{
    cell::Cell,
    future::pending,
    pin::pin,
    task::{Context, Poll, Waker},
};

struct Io<'a> {
    ready: &'a Cell<bool>,
    accepted: &'a Cell<usize>,
}
impl DatagramTransport for Io<'_> {
    type Error = ();
}
impl AsyncDatagramIo for Io<'_> {
    async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
        pending().await
    }
    async fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        core::future::poll_fn(|_| {
            if self.ready.get() {
                self.accepted.set(self.accepted.get() + 1);
                Poll::Ready(Ok(bytes.len()))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
#[derive(Default)]
struct Timer(Option<u64>);
impl AsyncTimer for Timer {
    fn armed(&self) -> Option<u64> {
        self.0
    }
    fn schedule(&mut self, deadline: Option<u64>) {
        self.0 = deadline;
    }
    fn poll_wait(&mut self, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}
fn handler(_: Request<'_>) -> Response<'static> {
    Response::content(b"value")
}
fn snapshot() -> Response<'static> {
    Response::content(b"value")
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 8], 5683);

#[test]
fn fetch_without_content_format_is_rejected_before_dispatch_in_both_modes() {
    struct Capture<'a>(&'a Cell<([u8; 64], usize)>);
    impl DatagramTransport for Capture<'_> {
        type Error = ();
    }
    impl DatagramIo for Capture<'_> {
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let mut wire = [0; 64];
            wire[..bytes.len()].copy_from_slice(bytes);
            self.0.set((wire, bytes.len()));
            Ok(bytes.len())
        }
    }
    impl AsyncDatagramIo for Capture<'_> {
        async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            pending().await
        }
        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            DatagramIo::send(self, peer, bytes)
        }
    }
    fn forbidden(_: Request<'_>) -> Response<'static> {
        panic!("invalid FETCH reached application handler")
    }
    for asynchronous in [false, true] {
        let sent = Cell::new(([0; 64], 0));
        let mut app = App::profile::<profiles::Constrained>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .allow_unprotected()
            .route("value", fetch(forbidden))
            .bind(Capture(&sent))
            .unwrap();
        let options = [Opt::uri_path("value")];
        let request = Message::new(Type::Confirmable, Code::FETCH, MessageId::new(41))
            .with_token(Token::new(&[1, 2]).unwrap())
            .with_options(&options);
        let mut wire = [0; 64];
        let n = encode(&request, &mut wire).unwrap();
        let rx = app.engine.acquire_rx().unwrap();
        app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
        if asynchronous {
            let mut clock = || Ok::<_, ()>(10);
            let mut timer = Timer::default();
            let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
            assert!(matches!(
                progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Ok(()))
            ));
        } else {
            app.poll(10).unwrap();
        }
        let (reply, n) = sent.get();
        let reply = crate::message::decode(&reply[..n]).unwrap();
        assert_eq!(reply.code(), Code::BAD_REQUEST);
        assert_eq!(reply.message_id(), MessageId::new(41));
        assert_eq!(app.engine.rx_occupied(), 0);
        assert_eq!(app.engine.tx_occupied(), 0);
    }
}

#[test]
fn cancelled_blocked_send_restores_the_borrowed_platform_deadline() {
    struct Scheduled<'a>(&'a Cell<Option<u64>>);
    impl AsyncTimer for Scheduled<'_> {
        fn armed(&self) -> Option<u64> {
            self.0.get()
        }
        fn schedule(&mut self, deadline: Option<u64>) {
            self.0.set(deadline);
        }
        fn poll_wait(&mut self, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }
    for previous in [None, Some(100), Some(25)] {
        let armed = Cell::new(previous);
        let ready = Cell::new(false);
        let accepted = Cell::new(0);
        let mut socket = Io {
            ready: &ready,
            accepted: &accepted,
        };
        let mut timer = Scheduled(&armed);
        let mut clock = || Ok::<_, ()>(10);
        let mut transport = ProtocolTransport {
            io: &mut socket,
            clock: &mut clock,
            timer: &mut timer,
            deadline: 50,
            completed: 0,
        };
        {
            let mut send = pin!(transport.send(PEER, b"reply"));
            assert!(
                send.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            assert_eq!(armed.get(), Some(previous.unwrap_or(50).min(50)));
        }
        assert_eq!(armed.get(), previous);
        assert_eq!(accepted.get(), 0);
    }
}

#[test]
fn async_transaction_metadata_has_independent_small_device_ceilings() {
    type Memory = crate::storage::Memory<profiles::Constrained>;
    type Clock = fn() -> Result<u64, ()>;
    #[cfg(feature = "std")]
    std::println!(
        "async metadata bytes: transport={} packet={} rx={} body={} notify={} deferred_observe={}",
        core::mem::size_of::<ProtocolTransport<'_, Io<'_>, Clock, Timer>>(),
        core::mem::size_of::<PacketOwner<'_, Memory>>(),
        core::mem::size_of::<RxOwner<'_, Memory>>(),
        core::mem::size_of::<BodyOwner<'_, Memory>>(),
        core::mem::size_of::<NotifyOwner<'_, Memory>>(),
        core::mem::size_of::<DeferredObserveOwner<'_, Memory>>()
    );
    assert!(core::mem::size_of::<ProtocolTransport<'_, Io<'_>, Clock, Timer>>() <= 40);
    assert!(core::mem::size_of::<PacketOwner<'_, Memory>>() <= 16);
    assert!(core::mem::size_of::<RxOwner<'_, Memory>>() <= 24);
    assert!(core::mem::size_of::<BodyOwner<'_, Memory>>() <= 32);
    assert!(core::mem::size_of::<NotifyOwner<'_, Memory>>() <= 128);
    assert!(core::mem::size_of::<DeferredObserveOwner<'_, Memory>>() <= 24);
    #[cfg(feature = "std")]
    {
        let ready = Cell::new(false);
        let accepted = Cell::new(0);
        let mut app = App::with_resources(
            Resources::<profiles::Constrained>::new()
                .routes::<1>()
                .block_wise::<false>(),
        )
        .allow_unprotected()
        .deterministic_for_tests()
        .route("value", get(handler))
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
        let mut clock = || Ok::<_, ()>(0);
        let mut timer = Timer::default();
        let progress = app.progress_async(0, &mut clock, &mut timer, 100);
        let progress_bytes = core::mem::size_of_val(&progress);
        drop(progress);
        let deadline = app.async_send_deadline(0, 100);
        let engine_bytes = {
            let mut io = ProtocolTransport {
                io: &mut app.io,
                clock: &mut clock,
                timer: &mut timer,
                deadline,
                completed: 0,
            };
            let engine = super::super::poll_engine_async(
                &mut app.engine,
                <profiles::Constrained as crate::app::response::AppAssembled<false>>::provider::<
                    Memory,
                >(&mut app.assembled),
                &mut io,
                &app.site,
                &mut app.ids,
                &mut app.inbox,
                &mut app.lives,
                &mut app.oscore,
                &mut app.oscore_checkpoint,
                None,
                app.echo_policy,
                &mut app.dedup_closed,
                app.full_responses,
                &mut app.deferred,
                app.deferred_lifetime_ms,
                None,
                0,
                None,
                None,
            );
            core::mem::size_of_val(&engine)
        };
        let send_bytes = {
            let mut io = ProtocolTransport {
                io: &mut app.io,
                clock: &mut clock,
                timer: &mut timer,
                deadline,
                completed: 0,
            };
            let job = finish_plain_job(
                &mut app.engine,
                &mut io,
                SlotId::from_index(0),
                PlainJob {
                    tx: SlotId::from_index(0),
                    dest: PEER,
                    pin_tx: false,
                    ready_checked: false,
                },
            );
            core::mem::size_of_val(&job)
        };
        let cold_bytes = {
            let mut io = ProtocolTransport {
                io: &mut app.io,
                clock: &mut clock,
                timer: &mut timer,
                deadline,
                completed: 0,
            };
            let response = Response::content(b"value");
            let meta = super::super::SendResponse {
                dest: PEER,
                ty: Type::Confirmable,
                mid: MessageId::new(1),
                token: Token::EMPTY,
                no_response: crate::message::NoResponse::DEFAULT,
                block2: None,
                q_block2: None,
                q_request: None,
                block1: None,
                oscore: oscore::no_request(),
                request: Code::GET,
            };
            let cold = finish_plain_cold(
                &mut app.engine,
                &mut app.ids,
                &mut app.oscore,
                &mut app.dedup_closed,
                &mut io,
                SlotId::from_index(0),
                meta,
                &response,
                false,
                0,
            );
            core::mem::size_of_val(&cold)
        };
        let dispatch_bytes = {
            let mut io = ProtocolTransport {
                io: &mut app.io,
                clock: &mut clock,
                timer: &mut timer,
                deadline,
                completed: 0,
            };
            let mut checkpoint = None;
            let dispatch = dispatch_rx_async(
                &mut app.engine,
                <profiles::Constrained as crate::app::response::AppAssembled<false>>::provider::<
                    Memory,
                >(&mut app.assembled),
                &mut io,
                &app.site,
                &mut app.inbox,
                &mut app.lives,
                &mut app.ids,
                &mut app.oscore,
                &mut app.oscore_checkpoint,
                &mut checkpoint,
                app.echo_policy,
                &mut app.dedup_closed,
                app.full_responses,
                &mut app.deferred,
                app.deferred_lifetime_ms,
                None,
                0,
                SlotId::from_index(0),
            );
            core::mem::size_of_val(&dispatch)
        };
        std::println!(
            "Constrained routes1 blockwise=false progress_async future={progress_bytes} plain_send={send_bytes} plain_cold={cold_bytes} dispatch_rx={dispatch_bytes} poll_engine={engine_bytes} local_rx_scratch={} optional_local_rx_scratch={}; future size only, not peak stack or whole RAM",
            core::mem::size_of::<<Memory as DatagramSlots>::RxScratch>(),
            core::mem::size_of::<Option<<Memory as DatagramSlots>::RxScratch>>()
        );
        assert!(send_bytes <= 1024, "plain send suspends {send_bytes} bytes");
        assert!(
            progress_bytes <= 12_000,
            "progress_async reserves {progress_bytes} bytes"
        );
    }
}

#[test]
fn ready_send_does_not_arm_the_platform_timer() {
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let armed = Cell::new(0);
    struct CountingTimer<'a>(&'a Cell<usize>, Option<u64>);
    impl AsyncTimer for CountingTimer<'_> {
        fn armed(&self) -> Option<u64> {
            self.1
        }
        fn schedule(&mut self, deadline: Option<u64>) {
            self.1 = deadline;
            if deadline.is_some() {
                self.0.set(self.0.get() + 1);
            }
        }
        fn poll_wait(&mut self, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }
    let mut io = Io {
        ready: &ready,
        accepted: &accepted,
    };
    let mut clock = || Ok::<u64, ()>(10);
    let mut timer = CountingTimer(&armed, None);
    let mut transport = ProtocolTransport {
        io: &mut io,
        clock: &mut clock,
        timer: &mut timer,
        deadline: 10_000,
        completed: 0,
    };
    let mut send = pin!(AsyncDatagramIo::send(&mut transport, PEER, b"hi"));
    assert!(matches!(
        send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(2))
    ));
    assert_eq!(armed.get(), 0);
    assert_eq!(accepted.get(), 1);
}

#[test]
fn waiting_send_arms_one_platform_deadline() {
    let ready = Cell::new(false);
    let accepted = Cell::new(0);
    let armed = Cell::new(0);
    let timer_ready = Cell::new(false);
    struct CountingTimer<'a> {
        armed: &'a Cell<usize>,
        deadline: Option<u64>,
        ready: &'a Cell<bool>,
    }
    impl AsyncTimer for CountingTimer<'_> {
        fn armed(&self) -> Option<u64> {
            self.deadline
        }
        fn schedule(&mut self, deadline: Option<u64>) {
            self.deadline = deadline;
            if deadline.is_some() {
                self.armed.set(self.armed.get() + 1);
            }
        }
        fn poll_wait(&mut self, cx: &mut Context<'_>) -> Poll<()> {
            if self.ready.get() {
                Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    let mut io = Io {
        ready: &ready,
        accepted: &accepted,
    };
    let mut clock = || Ok::<u64, ()>(10);
    let mut timer = CountingTimer {
        armed: &armed,
        deadline: None,
        ready: &timer_ready,
    };
    let mut transport = ProtocolTransport {
        io: &mut io,
        clock: &mut clock,
        timer: &mut timer,
        deadline: 10_000,
        completed: 0,
    };
    let mut send = pin!(AsyncDatagramIo::send(&mut transport, PEER, b"hi"));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(send.as_mut().poll(&mut cx).is_pending());
    assert_eq!(armed.get(), 1);
    assert_eq!(accepted.get(), 0);
    timer_ready.set(true);
    assert!(matches!(
        send.as_mut().poll(&mut cx),
        Poll::Ready(Err(AsyncIoError::Deadline))
    ));
    assert_eq!(accepted.get(), 0);
}

#[test]
fn cancelled_notification_restores_due_and_releases_exact_tx() {
    let ready = Cell::new(false);
    let accepted = Cell::new(0);
    let mut app = App::with_resources(
        Resources::<profiles::Constrained>::new()
            .routes::<1>()
            .block_wise::<false>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("obs", get(handler).observe(snapshot))
    .bind(Io {
        ready: &ready,
        accepted: &accepted,
    })
    .unwrap();
    let resource = app.site.observe_resource(&["obs"]);
    let key = ObserveKey::new(Token::EMPTY, PEER);
    let id = app
        .engine
        .insert_observe(ObserveInterest::new(Token::EMPTY, PEER).with_resource(resource))
        .unwrap();
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    {
        let mut send = pin!(app.notify_confirmable_async(
            10,
            &["obs"],
            Response::content(b"new"),
            &mut clock,
            &mut timer,
            100
        ));
        assert!(
            send.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(accepted.get(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    assert_eq!(app.engine.lookup_observe(key), Some(id));
    assert!(app.engine.observe_interest(id).unwrap().is_pending());
    ready.set(true);
    let mut retry = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
    assert!(matches!(
        retry.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(accepted.get(), 1);
}

#[test]
fn cancelled_deferred_observe_keeps_handle_without_unsent_hold() {
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let mut app = App::with_resources(
        Resources::<profiles::Constrained>::new()
            .routes::<1>()
            .block_wise::<false>()
            .deferred::<1>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("obs", get(handler).observe(snapshot))
    .bind(Io {
        ready: &ready,
        accepted: &accepted,
    })
    .unwrap();
    let observe = crate::message::encode_uint(0);
    let opts = [Opt::observe(&observe), Opt::uri_path("obs")];
    let request =
        Message::new(Type::Confirmable, Code::GET, MessageId::new(21)).with_options(&opts);
    let mut wire = [0; 64];
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    let mut handle = None;
    {
        let mut progress =
            pin!(
                app.progress_with_async(10, &mut clock, &mut timer, 100, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
            );
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let handle = handle.unwrap();
    ready.set(false);
    {
        let mut complete = pin!(app.complete_async(
            handle,
            Response::content(b"value"),
            10,
            &mut clock,
            &mut timer,
            100
        ));
        assert!(
            complete
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(app.engine.tx_occupied(), 0);
    assert_eq!(app.engine.rx_occupied(), 0);
    let row = app.deferred.rows[0].unwrap();
    assert_eq!(row.handle, handle);
    assert!(!row.completed);
    let interest = deferred_observe_owner(&app.engine, row).unwrap();
    assert!(interest.notify_hold().is_none());
    assert!(row.observe.unwrap().failed_mid.is_none());
    assert!(app.cancel_deferred(handle));
    assert!(app.engine.lookup_observe(interest.key()).is_none());
}

#[test]
fn cancelled_partial_deferred_q_keeps_immutable_snapshot() {
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let mut app = App::with_resources(
        Resources::<profiles::Constrained>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<1>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("q", get(handler))
    .bind(Io {
        ready: &ready,
        accepted: &accepted,
    })
    .unwrap();
    let q = BlockValue::new(0, true, 0).unwrap().encode();
    let opts = [Opt::uri_path("q"), Opt::q_block2(&q)];
    let request =
        Message::new(Type::Confirmable, Code::GET, MessageId::new(81)).with_options(&opts);
    let mut wire = [0; 64];
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    let mut handle = None;
    {
        let mut progress =
            pin!(
                app.progress_with_async(10, &mut clock, &mut timer, 100, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
            );
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let handle = handle.unwrap();
    let before = accepted.get();
    let mut clock = || {
        if accepted.get() > before {
            ready.set(false);
        }
        Ok::<_, ()>(10)
    };
    {
        let mut complete = pin!(app.complete_async(
            handle,
            Response::content(&[7; 64]).etag(b"v1"),
            10,
            &mut clock,
            &mut timer,
            100
        ));
        assert!(
            complete
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(accepted.get(), before + 1);
    let (body, _) = app.deferred.rows[0]
        .unwrap()
        .snapshot
        .expect("retained snapshot");
    assert_eq!(app.engine.tx_body_payload(body).unwrap(), &[7; 64]);
    assert_eq!(app.engine.tx_occupied(), 0);
    ready.set(true);
    let mut clock = || Ok::<_, ()>(11);
    {
        let mut changed = pin!(app.complete_async(
            handle,
            Response::content(&[8; 64]).etag(b"v1"),
            11,
            &mut clock,
            &mut timer,
            100
        ));
        assert!(matches!(
            changed
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(Error::Block(BlockTransferError::IdentityMismatch)))
        ));
    }
    assert_eq!(accepted.get(), before + 1);
    assert!(app.cancel_deferred(handle));
    assert!(app.engine.tx_body_payload(body).is_none());
}

#[test]
fn cancelled_deferred_q_followup_keeps_snapshot_for_recovery() {
    struct RecordingIo<'a> {
        inner: Io<'a>,
        packet: &'a Cell<([u8; 128], usize)>,
    }
    impl DatagramTransport for RecordingIo<'_> {
        type Error = ();
    }
    impl AsyncDatagramIo for RecordingIo<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            self.inner.recv(bytes).await
        }
        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let mut packet = [0; 128];
            packet[..bytes.len()].copy_from_slice(bytes);
            self.packet.set((packet, bytes.len()));
            self.inner.send(peer, bytes).await
        }
    }

    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let calls = Cell::new(0);
    let packet = Cell::new(([0; 128], 0));
    let mut app = App::with_resources(
        Resources::<profiles::Constrained>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<1>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("q", get(handler))
    .bind(RecordingIo {
        inner: Io {
            ready: &ready,
            accepted: &accepted,
        },
        packet: &packet,
    })
    .unwrap();
    let q = BlockValue::new(0, false, 0).unwrap().encode();
    let opts = [Opt::uri_path("q"), Opt::q_block2(&q)];
    let request =
        Message::new(Type::Confirmable, Code::GET, MessageId::new(91)).with_options(&opts);
    let mut wire = [0; 64];
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    let mut handle = None;
    {
        let mut progress =
            pin!(
                app.progress_with_async(10, &mut clock, &mut timer, 100, |_, candidate| {
                    calls.set(calls.get() + 1);
                    handle = candidate;
                    Response::deferred()
                })
            );
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let handle = handle.unwrap();
    let payload = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ+-";
    {
        let mut complete = pin!(app.complete_async(
            handle,
            Response::content(payload).etag(b"v1"),
            10,
            &mut clock,
            &mut timer,
            100
        ));
        assert!(matches!(
            complete
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let row = app.deferred.rows[0].unwrap();
    assert!(row.completed);
    let (body, retained) = row.snapshot.expect("completed Q snapshot");
    assert_eq!(app.engine.tx_body_payload(body), Some(payload.as_slice()));
    let before = accepted.get();
    let q = BlockValue::new(2, false, 0).unwrap().encode();
    let opts = [Opt::uri_path("q"), Opt::q_block2(&q)];
    let request =
        Message::new(Type::NonConfirmable, Code::GET, MessageId::new(92)).with_options(&opts);
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    ready.set(false);
    {
        let mut progress =
            pin!(
                app.progress_with_async(10, &mut clock, &mut timer, 100, |_, _| {
                    calls.set(calls.get() + 1);
                    Response::content(b"must not replace the snapshot")
                })
            );
        assert!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(accepted.get(), before);
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    let row = app.deferred.rows[0].unwrap();
    assert_eq!(row.handle, handle);
    assert!(row.completed);
    let (retained_body, metadata) = row.snapshot.unwrap();
    assert_eq!(retained_body, body);
    assert!(metadata.same_header(&retained));
    assert_eq!(app.engine.tx_body_payload(body), Some(payload.as_slice()));
    let (pending_packet, pending_len) = packet.get();
    let pending = crate::message::decode(&pending_packet[..pending_len]).unwrap();
    assert_eq!(pending.code(), Code::CONTENT);
    assert_eq!(pending.payload(), &payload[32..48]);
    assert_eq!(pending.etag().next(), Some(&b"v1"[..]));
    assert_eq!(pending.size2(), Some(Ok(64)));
    assert_eq!(
        pending.q_block2().next(),
        Some(Ok(BlockValue::new(2, true, 0).unwrap()))
    );

    let request =
        Message::new(Type::NonConfirmable, Code::GET, MessageId::new(93)).with_options(&opts);
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    ready.set(true);
    {
        let mut progress =
            pin!(
                app.progress_with_async(10, &mut clock, &mut timer, 100, |_, _| {
                    calls.set(calls.get() + 1);
                    Response::content(b"must not replace the snapshot")
                })
            );
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(accepted.get(), before + 1);
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    let (retry_packet, retry_len) = packet.get();
    // A fresh NON exchange gets a fresh MID; every other wire byte is retained.
    assert_eq!(retry_len, pending_len);
    assert_eq!(&retry_packet[..2], &pending_packet[..2]);
    assert_eq!(&retry_packet[4..retry_len], &pending_packet[4..pending_len]);
    let row = app.deferred.rows[0].unwrap();
    assert_eq!(row.handle, handle);
    assert!(row.completed);
    let (retained_body, metadata) = row.snapshot.unwrap();
    assert_eq!(retained_body, body);
    assert!(metadata.same_header(&retained));
    assert_eq!(app.engine.tx_body_payload(body), Some(payload.as_slice()));
    assert!(!app.cancel_deferred(handle));
    app.deferred.expire(&mut app.engine, row.deadline_ms);
    assert!(app.deferred.rows[0].is_none());
    assert!(app.engine.tx_body_payload(body).is_none());
}

#[test]
fn cancelled_immediate_static_q_releases_snapshot_and_retries_exact_bytes() {
    static BODY: [u8; 64] = *b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ+-";
    fn static_body(_: Request<'_>) -> Response<'static> {
        Response::content(&BODY).etag(b"static")
    }
    struct RecordingIo<'a> {
        inner: Io<'a>,
        second: &'a Cell<([u8; 128], usize)>,
        seen: &'a Cell<u8>,
    }
    impl DatagramTransport for RecordingIo<'_> {
        type Error = ();
    }
    impl AsyncDatagramIo for RecordingIo<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            self.inner.recv(bytes).await
        }
        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let reply = crate::message::decode(bytes).unwrap();
            let block = reply.q_block2().next().unwrap().unwrap();
            let num = block.num() as usize;
            assert!(num < 4);
            assert_eq!(reply.code(), Code::CONTENT);
            assert_eq!(reply.payload(), &BODY[num * 16..(num + 1) * 16]);
            assert_eq!(reply.etag().next(), Some(&b"static"[..]));
            assert_eq!(reply.size2(), Some(Ok(BODY.len() as u32)));
            assert_eq!(block.more(), num != 3);
            if num == 1 {
                let mut packet = [0; 128];
                packet[..bytes.len()].copy_from_slice(bytes);
                self.second.set((packet, bytes.len()));
            }
            let sent = self.inner.send(peer, bytes).await?;
            self.seen.set(self.seen.get() | (1 << num));
            Ok(sent)
        }
    }
    fn exercise<S: AppStorage<Token = Token>, const CALLS: usize>(
        mut app: App<profiles::Constrained, RecordingIo<'_>, 1, true, S, 0, CALLS>,
        ready: &Cell<bool>,
        accepted: &Cell<usize>,
        second: &Cell<([u8; 128], usize)>,
        seen: &Cell<u8>,
    ) {
        ready.set(true);
        accepted.set(0);
        second.set(([0; 128], 0));
        seen.set(0);
        let q = BlockValue::new(0, true, 0).unwrap().encode();
        let opts = [Opt::uri_path("q"), Opt::q_block2(&q)];
        let mut wire = [0; 64];
        let mut timer = Timer::default();
        for attempt in 0..2 {
            let request = Message::new(
                Type::NonConfirmable,
                Code::GET,
                MessageId::new(101 + attempt),
            )
            .with_token(Token::new(b"static").unwrap())
            .with_options(&opts);
            let n = encode(&request, &mut wire).unwrap();
            let rx = app.engine.acquire_rx().unwrap();
            app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
            let mut clock = || {
                if attempt == 0 && accepted.get() != 0 {
                    ready.set(false);
                }
                Ok::<_, ()>(10)
            };
            let (pending, pending_len) = second.get();
            {
                let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
                let result = progress
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()));
                if attempt == 0 {
                    assert!(result.is_pending());
                } else {
                    assert!(matches!(result, Poll::Ready(Ok(()))));
                }
            }
            assert_eq!(app.engine.rx_occupied(), 0);
            assert_eq!(app.engine.tx_occupied(), 0);
            for index in 0..app.engine.capacities().tx_body_slots.unwrap() {
                let id = SlotId::from_index(index);
                assert!(app.engine.tx_body_payload(id).is_none());
                assert!(app.engine.tx_body_transfer(id).is_none());
            }
            if attempt == 0 {
                assert_eq!(accepted.get(), 1);
                assert_eq!(seen.get(), 1);
                assert_ne!(second.get().1, 0);
                ready.set(true);
            } else {
                assert_eq!(accepted.get(), 5);
                assert_eq!(seen.get(), 0b1111);
                let (retry, retry_len) = second.get();
                assert_eq!(retry_len, pending_len);
                // The fresh NON exchange gets new MIDs; payload/options stay exact.
                assert_eq!(&retry[..2], &pending[..2]);
                assert_eq!(&retry[4..retry_len], &pending[4..pending_len]);
            }
        }
    }

    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let second = Cell::new(([0; 128], 0));
    let seen = Cell::new(0);
    let builder = || {
        App::with_resources(
            Resources::<profiles::Constrained>::new()
                .routes::<1>()
                .block_wise::<true>(),
        )
        .allow_unprotected()
        .deterministic_for_tests()
        .route("q", get(static_body))
    };
    let io = || RecordingIo {
        inner: Io {
            ready: &ready,
            accepted: &accepted,
        },
        second: &second,
        seen: &seen,
    };
    exercise(
        builder().bind(io()).unwrap(),
        &ready,
        &accepted,
        &second,
        &seen,
    );
    #[cfg(feature = "alloc")]
    exercise(
        App::with_resources(
            Resources::<profiles::Constrained>::new()
                .routes::<1>()
                .block_wise::<true>()
                .allocated(usize::MAX),
        )
        .allow_unprotected()
        .deterministic_for_tests()
        .route("q", get(static_body))
        .bind(io())
        .unwrap(),
        &ready,
        &accepted,
        &second,
        &seen,
    );
}

#[test]
fn accepted_retry_with_failed_clock_retires_call_without_automatic_resend() {
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .allow_unprotected()
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
    let mut timer = Timer::default();
    let call = {
        let mut send = pin!(app.get("remote").to(PEER).send_async(
            0,
            || Ok::<_, ()>(0),
            &mut timer
        ));
        match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(call)) => call,
            other => panic!("initial send: {other:?}"),
        }
    };
    let mut samples = [Ok(3000), Err(())].into_iter();
    let mut clock = || samples.next().unwrap();
    let mut timer = Timer::default();
    {
        let mut retry = pin!(app.progress_async(3000, &mut clock, &mut timer, 4000));
        assert!(matches!(
            retry.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(Error::Io(DatagramIoError::Io(
                AsyncIoError::ClockAfterSend(())
            ))))
        ));
    }
    assert_eq!(accepted.get(), 2);
    assert_eq!(app.engine.tx_occupied(), 0);
    assert!(matches!(
        app.take_response(call),
        Some(Err(CallFailure::ContinuationFailed))
    ));
    let mut clock = || Ok::<_, ()>(3001);
    let mut progress = pin!(app.progress_async(3001, &mut clock, &mut timer, 4000));
    assert!(matches!(
        progress
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(accepted.get(), 2);
}

#[test]
fn deadline_refusal_preserves_exact_retry_snapshot() {
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .allow_unprotected()
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
    let mut timer = Timer::default();
    {
        let mut send = pin!(app.get("remote").to(PEER).send_async(
            0,
            || Ok::<_, ()>(0),
            &mut timer
        ));
        assert!(matches!(
            send.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(_))
        ));
    }
    let before = app.engine.peek_retransmit(3000).unwrap();
    let mut timer = Timer::default();
    let mut clock = || Ok::<_, ()>(3000);
    {
        let mut retry = pin!(app.progress_async(3000, &mut clock, &mut timer, 3000));
        assert!(matches!(
            retry.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(Error::Io(DatagramIoError::Io(AsyncIoError::Deadline))))
        ));
    }
    assert_eq!(app.engine.peek_retransmit(3000), Some(before));
    assert_eq!(accepted.get(), 1);
}

#[cfg(feature = "oscore")]
#[test]
fn shared_association_barrier_precedes_effects_and_rejects_restored_replay() {
    use crate::oscore::{DeriveParams, SecurityContext};
    let make = |sender: &[u8], recipient: &[u8]| {
        SecurityContext::derive(DeriveParams {
            master_secret: &[7; 16],
            master_salt: &[],
            sender_id: sender,
            recipient_id: recipient,
            id_context: &[],
        })
        .unwrap()
    };
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let build = || {
        App::with_resources(crate::resources::SmallDevice::new().associations::<2>())
            .deterministic_for_tests()
            .require_associations()
            .require_oscore_checkpoint()
            .bind(Io {
                ready: &ready,
                accepted: &accepted,
            })
            .unwrap()
    };
    let mut app = build();
    let owner = app
        .install_association(PEER, [1; 32], 9, &mut Some(make(&[0], &[1])))
        .unwrap();
    let other = app
        .install_association(
            Endpoint::v4([192, 0, 2, 9], 5683),
            [2; 32],
            10,
            &mut Some(make(&[0], &[2])),
        )
        .unwrap();
    let mut sender = make(&[1], &[0]);
    let options = [Opt::uri_path("effect")];
    let request =
        Message::new(Type::Confirmable, Code::POST, MessageId::new(7)).with_options(&options);
    let mut wire = [0; 256];
    let n = sender.protect_request(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    let mut initial = None;
    let mut consumed = None;
    let mut saw_other = false;
    let mut effects = 0;
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    {
        let mut progress = pin!(app.progress_with_association_checkpoint_async(
            10,
            &mut clock,
            &mut timer,
            100,
            |association, checkpoint| {
                if association == Some(owner) {
                    if initial.is_none() {
                        initial = Some(checkpoint);
                        true
                    } else {
                        consumed = Some(checkpoint);
                        false
                    }
                } else {
                    assert_eq!(association, Some(other));
                    saw_other = true;
                    true
                }
            },
            |_, _| {
                effects += 1;
                Response::changed()
            }
        ));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(Error::OscoreCheckpointFailed))
        ));
    }
    assert_eq!(effects, 0);
    assert_eq!(accepted.get(), 0);
    assert!(saw_other);
    assert_ne!(initial, consumed);
    assert_eq!(app.engine.rx_occupied(), 0);
    let mut restored = make(&[0], &[1]);
    restored.restore_replay(consumed.unwrap()).unwrap();
    restored.restore_sender_reservation(64).unwrap();
    restored
        .reserve_sender_sequences(64, |_| Ok::<_, ()>(()))
        .unwrap();
    let mut restarted = build();
    restarted
        .install_association(PEER, [1; 32], 9, &mut Some(restored))
        .unwrap();
    let rx = restarted.engine.acquire_rx().unwrap();
    restarted.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    {
        let mut progress = pin!(restarted.progress_with_association_checkpoint_async(
            10,
            &mut clock,
            &mut timer,
            100,
            |association, _| {
                assert_eq!(association.unwrap().epoch(), 9);
                true
            },
            |_, _| panic!("restored replay must not repeat the effect")
        ));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    assert_eq!(accepted.get(), 0);
}

#[test]
fn already_due_work_does_not_hide_later_send_deadline() {
    let ready = Cell::new(false);
    let accepted = Cell::new(0);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .allow_unprotected()
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
    for (token, due_ms) in [(1, 10), (2, 50)] {
        app.engine
            .insert_observe(
                ObserveInterest::new(Token::new(&[token]).unwrap(), PEER)
                    .with_lifetime(Some(crate::storage::ObserveLifetime::MaxAge { due_ms })),
            )
            .unwrap();
    }
    assert_eq!(app.next_timeout(), Some(10));
    assert_eq!(app.next_timeout_after(Some(10)), Some(50));
    assert_eq!(app.async_send_deadline(10, 100), 50);
    assert_eq!(app.async_send_deadline(10, 30), 30);
}

#[test]
fn expired_non_hold_does_not_keep_idle_app_ready() {
    let ready = Cell::new(false);
    let accepted = Cell::new(0);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .allow_unprotected()
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
    let mut row = ObserveInterest::new(Token::EMPTY, PEER);
    row.record_notify(0, MessageId::new(42), false);
    let until_ms = match row.notify_hold().unwrap() {
        crate::storage::ObserveNotifyHold::Non { until_ms, .. } => until_ms,
        _ => unreachable!(),
    };
    app.engine.insert_observe(row).unwrap();
    assert_eq!(app.next_timeout(), None);
    assert_eq!(app.next_timeout_after(Some(0)), Some(until_ms));
    assert_eq!(app.next_timeout_after(Some(until_ms)), None);
    assert_eq!(app.next_poll_at(until_ms), None);
}

#[test]
fn delayed_non_notification_commits_hold_and_max_age_at_acceptance() {
    let ready = Cell::new(false);
    let accepted = Cell::new(0);
    let now = Cell::new(10);
    let mut app = App::with_resources(crate::resources::SmallDevice::new())
        .deterministic_for_tests()
        .allow_unprotected()
        .route("obs", get(handler).observe(snapshot))
        .bind(Io {
            ready: &ready,
            accepted: &accepted,
        })
        .unwrap();
    let resource = app.site.observe_resource(&["obs"]);
    let id = app
        .engine
        .insert_observe(ObserveInterest::new(Token::EMPTY, PEER).with_resource(resource))
        .unwrap();
    let mut clock = || Ok::<_, ()>(now.get());
    let mut timer = Timer::default();
    {
        let mut notification = pin!(app.notify_async(
            10,
            &["obs"],
            Response::content(b"delayed").max_age(10),
            &mut clock,
            &mut timer,
            10_000
        ));
        assert!(
            notification
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        now.set(5000);
        ready.set(true);
        assert!(matches!(
            notification
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(1))
        ));
    }
    assert_eq!(accepted.get(), 1);
    let interest = app.engine.observe_interest(id).unwrap();
    assert!(matches!(
        interest.notify_hold(),
        Some(crate::storage::ObserveNotifyHold::Non { until_ms: 8000, .. })
    ));
    assert_eq!(interest.lifetime().unwrap().due_ms(), 15_000);
    assert_eq!(app.engine.tx_occupied(), 0);
}

#[test]
fn cancelled_client_con_ack_through_progress_retires_call_and_rejects_replay() {
    struct RecordingIo<'a> {
        inner: Io<'a>,
        last: &'a Cell<([u8; 64], usize)>,
    }
    impl DatagramTransport for RecordingIo<'_> {
        type Error = ();
    }
    impl AsyncDatagramIo for RecordingIo<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            self.inner.recv(bytes).await
        }
        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let mut wire = [0; 64];
            wire[..bytes.len()].copy_from_slice(bytes);
            self.last.set((wire, bytes.len()));
            self.inner.send(peer, bytes).await
        }
    }

    const OTHER_PEER: Endpoint = Endpoint::v4([192, 0, 2, 9], 5683);
    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let last = Cell::new(([0; 64], 0));
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .allow_unprotected()
        .bind(RecordingIo {
            inner: Io {
                ready: &ready,
                accepted: &accepted,
            },
            last: &last,
        })
        .unwrap();
    let mut timer = Timer::default();
    // Keep an unrelated live Call without occupying the ACK's control slot.
    let other = {
        let mut send = pin!(app.get("other").to(OTHER_PEER).non().send_async(
            0,
            || Ok::<_, ()>(0),
            &mut timer
        ));
        match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(call)) => call,
            result => panic!("other call: {result:?}"),
        }
    };
    let call = {
        let mut send = pin!(app.get("remote").to(PEER).send_async(
            0,
            || Ok::<_, ()>(0),
            &mut timer
        ));
        match send.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(call)) => call,
            result => panic!("client call: {result:?}"),
        }
    };
    assert_eq!(accepted.get(), 2);
    assert_eq!(app.engine.tx_occupied(), 1);

    let mid = MessageId::new(70);
    let response = Message::new(Type::Confirmable, Code::CONTENT, mid)
        .with_token(call.token())
        .with_payload(b"reply");
    let mut wire = [0; 64];
    let n = encode(&response, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    ready.set(false);
    let mut clock = || Ok::<_, ()>(10);
    {
        let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
        assert!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let (pending, len) = last.get();
        let ack = crate::message::decode(&pending[..len]).unwrap();
        assert!(ack.is_empty_ack());
        assert_eq!(ack.message_id(), mid);
        assert_eq!(accepted.get(), 2);
    }

    assert!(app.engine.access_rx(rx).is_err());
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    assert_eq!(accepted.get(), 2, "the pending ACK was never accepted");
    assert!(matches!(
        app.take_response(call),
        Some(Err(CallFailure::ContinuationFailed))
    ));
    assert!(app.take_response(other).is_none());

    // An unaccepted ACK must not earn dedup admission or a cached ACK replay.
    let retry_rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(retry_rx, &wire[..n], PEER).unwrap();
    ready.set(true);
    {
        let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let (sent, len) = last.get();
    let reset = crate::message::decode(&sent[..len]).unwrap();
    assert!(
        reset.is_empty_rst(),
        "cancelled response must not replay an ACK"
    );
    assert_eq!(reset.message_id(), mid);
    assert_eq!(accepted.get(), 3);
    assert!(app.engine.access_rx(retry_rx).is_err());
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    assert!(app.take_response(call).is_none());
    assert!(app.take_response(other).is_none());
    assert!(app.cancel(other), "the unrelated Call remains active");
    assert!(matches!(
        app.take_response(other),
        Some(Err(CallFailure::Cancelled))
    ));
}

fn plain_hits() -> &'static core::sync::atomic::AtomicUsize {
    static HITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    &HITS
}

fn plain_handler(_: Request<'_>) -> Response<'static> {
    plain_hits().fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Response::content(b"value")
}

#[test]
fn plain_server_get_answers_once_and_replays_without_the_handler() {
    fn hits() -> &'static core::sync::atomic::AtomicUsize {
        static HITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
        &HITS
    }
    fn handler(_: Request<'_>) -> Response<'static> {
        hits().fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Response::content(b"value")
    }

    let ready = Cell::new(true);
    let accepted = Cell::new(0);
    let last = Cell::new(([0u8; 64], 0usize));
    struct RecordingIo<'a> {
        inner: Io<'a>,
        last: &'a Cell<([u8; 64], usize)>,
    }
    impl DatagramTransport for RecordingIo<'_> {
        type Error = ();
    }
    impl AsyncDatagramIo for RecordingIo<'_> {
        async fn recv(&mut self, bytes: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            self.inner.recv(bytes).await
        }
        async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let mut wire = [0u8; 64];
            let n = bytes.len().min(wire.len());
            wire[..n].copy_from_slice(&bytes[..n]);
            self.last.set((wire, bytes.len()));
            self.inner.send(peer, bytes).await
        }
    }
    hits().store(0, core::sync::atomic::Ordering::Relaxed);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .allow_unprotected()
        .route("value", get(handler))
        .bind(RecordingIo {
            inner: Io {
                ready: &ready,
                accepted: &accepted,
            },
            last: &last,
        })
        .unwrap();
    let token = Token::new(&[9, 8, 7, 6]).unwrap();
    let path = [Opt::uri_path("value")];
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(41))
        .with_token(token)
        .with_options(&path);
    let mut wire = [0u8; 64];
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    let mut clock = || Ok::<_, ()>(10);
    let mut timer = Timer::default();
    {
        let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    assert_eq!(hits().load(core::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(accepted.get(), 1);
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    let (sent, len) = last.get();
    let ack = crate::message::decode(&sent[..len]).unwrap();
    assert_eq!(ack.ty(), Type::Acknowledgement);
    assert_eq!(ack.message_id(), MessageId::new(41));
    assert_eq!(ack.token(), token);
    assert_eq!(ack.payload(), b"value");

    let retry = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(retry, &wire[..n], PEER).unwrap();
    {
        let mut progress = pin!(app.progress_async(10, &mut clock, &mut timer, 100));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    assert_eq!(
        hits().load(core::sync::atomic::Ordering::Relaxed),
        1,
        "dedup replay must not rerun the handler"
    );
    assert_eq!(accepted.get(), 2);
    let (replay, replay_len) = last.get();
    assert_eq!(&replay[..replay_len], &sent[..len]);
}

#[test]
fn plain_server_sync_poll_answers_once_and_replays_without_the_handler() {
    fn hits() -> &'static core::sync::atomic::AtomicUsize {
        static HITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
        &HITS
    }
    fn handler(_: Request<'_>) -> Response<'static> {
        hits().fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Response::content(b"value")
    }

    let accepted = Cell::new(0usize);
    let last = Cell::new(([0u8; 64], 0usize));
    struct SyncIo<'a> {
        accepted: &'a Cell<usize>,
        last: &'a Cell<([u8; 64], usize)>,
    }
    impl DatagramTransport for SyncIo<'_> {
        type Error = ();
    }
    impl DatagramIo for SyncIo<'_> {
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let mut wire = [0u8; 64];
            let n = bytes.len().min(wire.len());
            wire[..n].copy_from_slice(&bytes[..n]);
            self.last.set((wire, bytes.len()));
            self.accepted.set(self.accepted.get() + 1);
            Ok(bytes.len())
        }
    }
    hits().store(0, core::sync::atomic::Ordering::Relaxed);
    let mut app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .allow_unprotected()
        .route("value", get(handler))
        .bind(SyncIo {
            accepted: &accepted,
            last: &last,
        })
        .unwrap();
    let token = Token::new(&[9, 8, 7, 6]).unwrap();
    let path = [Opt::uri_path("value")];
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(41))
        .with_token(token)
        .with_options(&path);
    let mut wire = [0u8; 64];
    let n = encode(&request, &mut wire).unwrap();
    let rx = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
    app.poll(10).unwrap();
    assert_eq!(hits().load(core::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(accepted.get(), 1);
    assert_eq!(app.engine.rx_occupied(), 0);
    assert_eq!(app.engine.tx_occupied(), 0);
    let (sent, len) = last.get();
    let ack = crate::message::decode(&sent[..len]).unwrap();
    assert_eq!(ack.ty(), Type::Acknowledgement);
    assert_eq!(ack.message_id(), MessageId::new(41));
    assert_eq!(ack.token(), token);
    assert_eq!(ack.payload(), b"value");

    let retry = app.engine.acquire_rx().unwrap();
    app.engine.write_rx(retry, &wire[..n], PEER).unwrap();
    app.poll(10).unwrap();
    assert_eq!(
        hits().load(core::sync::atomic::Ordering::Relaxed),
        1,
        "dedup replay must not rerun the handler"
    );
    assert_eq!(accepted.get(), 2);
    let (replay, replay_len) = last.get();
    assert_eq!(&replay[..replay_len], &sent[..len]);
}

#[cfg(feature = "std")]
#[test]
#[ignore = "release timing for one plain server GET"]
fn plain_get_sync_versus_async_cost() {
    use std::time::Instant;
    const N: u32 = 4_000;
    const WARMUP: u32 = 200;
    let path = [Opt::uri_path("value")];
    let token = Token::new(&[1, 2, 3, 4]).unwrap();
    let mut wire = [0u8; 64];
    let prepare = |seq: u32, wire: &mut [u8]| {
        let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(seq as u16))
            .with_token(token)
            .with_options(&path);
        encode(&request, wire).unwrap()
    };
    // Each sample is a new exchange. 247s steps expire the previous dedup row
    // so the measurement is a fresh GET, not a replay.
    let at = |seq: u32| 10 + u64::from(seq) * 300_000;

    struct CaptureIo<'a> {
        accepted: &'a Cell<usize>,
    }
    impl DatagramTransport for CaptureIo<'_> {
        type Error = ();
    }
    impl DatagramIo for CaptureIo<'_> {
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            self.accepted.set(self.accepted.get() + 1);
            Ok(bytes.len())
        }
    }
    impl AsyncDatagramIo for CaptureIo<'_> {
        async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
            pending().await
        }
        async fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            self.accepted.set(self.accepted.get() + 1);
            Ok(bytes.len())
        }
    }

    let accepted = Cell::new(0);
    let mut sync = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .allow_unprotected()
        .route("value", get(plain_handler))
        .bind(CaptureIo {
            accepted: &accepted,
        })
        .unwrap();
    let mut async_app = App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .allow_unprotected()
        .route("value", get(plain_handler))
        .bind(CaptureIo {
            accepted: &accepted,
        })
        .unwrap();
    let now_ms = Cell::new(10u64);
    let mut clock = || Ok::<_, ()>(now_ms.get());
    let mut timer = Timer::default();

    for seq in 0..WARMUP {
        let n = prepare(seq, &mut wire);
        let rx = sync.engine.acquire_rx().unwrap();
        sync.engine.write_rx(rx, &wire[..n], PEER).unwrap();
        sync.poll(at(seq)).unwrap();
    }
    let start = Instant::now();
    for seq in 0..N {
        let n = prepare(WARMUP + seq, &mut wire);
        let rx = sync.engine.acquire_rx().unwrap();
        sync.engine.write_rx(rx, &wire[..n], PEER).unwrap();
        sync.poll(at(WARMUP + seq)).unwrap();
    }
    let sync_ns = start.elapsed().as_secs_f64() * 1e9 / f64::from(N);

    for seq in 0..WARMUP {
        let n = prepare(seq, &mut wire);
        let rx = async_app.engine.acquire_rx().unwrap();
        async_app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
        let now = at(seq);
        now_ms.set(now);
        let mut progress = pin!(async_app.progress_async(now, &mut clock, &mut timer, now + 1_000));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let future = async_app.progress_async(10, &mut clock, &mut timer, 1_000);
    let future_bytes = core::mem::size_of_val(&future);
    drop(future);
    let start = Instant::now();
    for seq in 0..N {
        let n = prepare(WARMUP + seq, &mut wire);
        let rx = async_app.engine.acquire_rx().unwrap();
        async_app.engine.write_rx(rx, &wire[..n], PEER).unwrap();
        let now = at(WARMUP + seq);
        now_ms.set(now);
        let mut progress = pin!(async_app.progress_async(now, &mut clock, &mut timer, now + 1_000));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
    let async_ns = start.elapsed().as_secs_f64() * 1e9 / f64::from(N);
    std::println!(
        "plain GET Constrained blockwise sync {sync_ns:.1} ns/op async {async_ns:.1} ns/op async/sync {:.3} progress_async future {future_bytes} bytes N={N}",
        async_ns / sync_ns
    );
}
