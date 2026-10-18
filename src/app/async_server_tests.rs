use super::*;
use crate::message::{Opt, Token, encode};
use crate::resources::Resources;
use crate::storage::profiles;
use core::{
    cell::Cell,
    future::pending,
    task::{Context, Waker},
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
struct Timer;
impl AsyncTimer for Timer {
    async fn wait_until(&mut self, _: u64) {
        pending().await
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
fn async_transaction_metadata_has_independent_small_device_ceilings() {
    type Memory = crate::storage::Memory<profiles::Constrained>;
    type Clock = fn() -> Result<u64, ()>;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
    let mut timer = Timer;
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
