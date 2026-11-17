//! Seeded compound client/server lifecycle campaign with four concurrent calls.
//! Recreates both endpoints with fresh security contexts every 32 rounds and
//! combines packet loss, duplication, Observe, block bodies, cancellation and
//! deadlines. Logical time advances without sleeping.

use crate::storage::{DatagramIo, Endpoint, SlotId};
use crate::{App, CallFailure, Code, Request, Response, get, profiles, put};
extern crate std;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::{println, vec::Vec};

const BODY: [u8; 2000] = [0x5a; 2000];
const SEED: u64 = 0x202_8613_7641;
const EPOCHS: usize = 128;
const ROUNDS: usize = 32;

#[derive(Default)]
struct Network {
    queues: [VecDeque<Vec<u8>>; 2],
    sent: usize,
    dropped: usize,
    duplicated: usize,
    blackout: bool,
    capture: bool,
    frames: [Vec<Vec<u8>>; 2],
}

struct Io {
    network: Rc<RefCell<Network>>,
    side: usize,
}

fn address(side: usize) -> Endpoint {
    Endpoint::v4([192, 0, 2, side as u8 + 1], 5683)
}

impl crate::storage::DatagramTransport for Io {
    type Error = ();
}

impl DatagramIo for Io {
    fn recv(&mut self, buffer: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some(bytes) = self.network.borrow_mut().queues[self.side].pop_front() else {
            return Ok(None);
        };
        if bytes.len() > buffer.len() {
            return Err(());
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(Some((bytes.len(), address(1 - self.side))))
    }
    fn send(&mut self, destination: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        assert_eq!(destination, address(1 - self.side));
        let mut network = self.network.borrow_mut();
        network.sent += 1;
        if network.capture {
            network.frames[self.side].push(bytes.to_vec());
        }
        if network.blackout || network.sent.is_multiple_of(13) {
            network.dropped += 1;
        } else {
            let duplicate = network.sent.is_multiple_of(17);
            network.queues[1 - self.side].push_back(bytes.to_vec());
            if duplicate {
                network.duplicated += 1;
                network.queues[1 - self.side].push_back(bytes.to_vec());
            }
        }
        Ok(bytes.len())
    }
}

impl crate::storage::AsyncDatagramIo for Io {
    async fn recv(&mut self, buffer: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        match DatagramIo::recv(self, buffer)? {
            Some(frame) => Ok(frame),
            None => core::future::pending().await,
        }
    }

    async fn send(&mut self, destination: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        DatagramIo::send(self, destination, bytes)
    }
}

fn value(_: Request<'_>) -> Response<'static> {
    Response::content(b"value").observe(0)
}
fn snapshot() -> Response<'static> {
    Response::content(b"changed")
}
fn large(_: Request<'_>) -> Response<'static> {
    Response::content(&BODY)
}
fn upload(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.body().unwrap_or(request.payload()), BODY);
    Response::changed()
}

fn campaign(secured: bool) {
    let mut random = SEED;
    let (mut completed, mut cancelled, mut expired, mut attempts, mut refused, mut remote_refused) =
        (0, 0, 0, 0, 0, 0);
    let (mut dropped, mut duplicated) = (0, 0);
    let mut poll_refused = 0;
    for epoch in 0..EPOCHS {
        let network = Rc::new(RefCell::new(Network::default()));
        let mut client = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>();
        let mut server = App::profile::<profiles::Default>()
            .deterministic_for_tests()
            .block_wise::<true>()
            .route("value", get(value).observe(snapshot))
            .route("obs0", get(value).observe(snapshot))
            .route("obs1", get(value).observe(snapshot))
            .route("obs2", get(value).observe(snapshot))
            .route("obs3", get(value).observe(snapshot))
            .route("large", get(large))
            .route("upload", put(upload));
        if !secured {
            client = client.allow_unprotected();
            server = server.allow_unprotected();
        }
        #[cfg(feature = "oscore")]
        if secured {
            let context = |sender, recipient| {
                crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
                    master_secret: b"lifecycle qualification fixture",
                    master_salt: &[],
                    sender_id: sender,
                    recipient_id: recipient,
                    id_context: &(epoch as u64).to_le_bytes(),
                })
                .unwrap()
            };
            client = client.oscore(context(&[1], &[2]));
            server = server.oscore(context(&[2], &[1]));
        }
        let mut client = client
            .bind(Io {
                network: network.clone(),
                side: 0,
            })
            .unwrap();
        let mut server = server
            .bind(Io {
                network: network.clone(),
                side: 1,
            })
            .unwrap();
        let _ = epoch;
        let mut now = 0;
        for _ in 0..ROUNDS {
            let mut pending = Vec::new();
            for path in ["obs0", "obs1", "obs2", "obs3"] {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let kind = random % 4;
                let outgoing = match kind {
                    0 => client.get("value"),
                    1 => client.get("large"),
                    2 => client.put("upload").payload(&BODY),
                    _ => client.get(path).observe(),
                };
                attempts += 1;
                let call = match outgoing.to(address(1)).deadline(now + 80).send(now) {
                    Ok(call) => call,
                    Err(crate::app::Error::Saturated) => {
                        refused += 1;
                        continue;
                    }
                    Err(error) => panic!("unexpected admission refusal: {error:?}"),
                };
                let cancel = random & 8 != 0;
                if cancel {
                    assert!(client.cancel(call));
                }
                pending.push((call, kind, cancel));
            }
            for tick in 0..20 {
                now += 5;
                match server.poll(now) {
                    Ok(()) => {}
                    Err(crate::app::Error::Saturated) => poll_refused += 1,
                    Err(error) => panic!("unexpected server poll failure: {error:?}"),
                }
                if tick == 2 {
                    for path in ["obs0", "obs1", "obs2", "obs3"] {
                        match server.notify(now, &[path], Response::content(b"changed")) {
                            Ok(_) | Err(crate::app::Error::Saturated) => {}
                            Err(error) => panic!("unexpected notification refusal: {error:?}"),
                        }
                    }
                }
                client.poll(now).unwrap();
                pending.retain(|(call, kind, cancel)| {
                    let Some(response) = client.take_response(*call) else {
                        return true;
                    };
                    match response {
                        Ok(response) => {
                            assert!(!cancel);
                            let subscribed = response.observe_seq().is_some();
                            if [136, 163].contains(&response.code().as_raw()) {
                                assert_eq!(*kind, 2);
                                remote_refused += 1;
                                return false;
                            }
                            if *kind == 2 {
                                assert_eq!(response.code(), Code::CHANGED);
                            } else if *kind == 1 {
                                assert_eq!(response.code(), Code::CONTENT);
                                assert_eq!(response.body().unwrap_or(response.payload()), BODY);
                            } else {
                                assert_eq!(response.code(), Code::CONTENT);
                                assert!(
                                    [b"value".as_slice(), b"changed".as_slice()]
                                        .contains(&response.payload())
                                );
                            }
                            completed += 1;
                            if *kind == 3 && subscribed {
                                assert!(client.cancel(*call));
                                assert_eq!(
                                    client.take_response(*call).unwrap().unwrap_err(),
                                    CallFailure::Cancelled
                                );
                            }
                        }
                        Err(CallFailure::Cancelled) => {
                            assert!(
                                *cancel,
                                "unexpected cancellation: kind={kind} call={call:?}"
                            );
                            cancelled += 1;
                        }
                        Err(CallFailure::DeadlineExceeded) => {
                            assert!(!cancel);
                            expired += 1;
                        }
                        Err(error) => panic!("unexpected call failure: {error:?}"),
                    }
                    false
                });
            }
            assert!(
                pending.is_empty(),
                "epoch={epoch} now={now} pending={pending:?}"
            );
            assert_eq!(client.engine_mut().tx_occupied(), 0);
            assert_eq!(client.engine_mut().rx_occupied(), 0);
            for index in 0..client.engine().capacities().rx_body_slots.unwrap() {
                assert!(
                    client
                        .engine()
                        .rx_body_transfer(SlotId::from_index(index))
                        .is_none()
                );
            }
            for index in 0..client.engine().capacities().tx_body_slots.unwrap() {
                assert!(
                    client
                        .engine()
                        .tx_body_transfer(SlotId::from_index(index))
                        .is_none()
                );
            }
        }
        dropped += network.borrow().dropped;
        duplicated += network.borrow().duplicated;
    }
    assert_eq!(attempts, EPOCHS * ROUNDS * 4);
    assert_eq!(
        completed + cancelled + expired + refused + remote_refused,
        attempts
    );
    assert!(completed > 1000 && cancelled > 1000 && expired > 100);
    assert!(dropped > 100 && duplicated > 100);
    println!(
        "lifecycle seed={SEED:x} secured={secured} epochs={EPOCHS} rounds={ROUNDS} attempts={attempts} completed={completed} cancelled={cancelled} expired={expired} refused={refused} remote_refused={remote_refused} poll_refused={poll_refused} dropped={dropped} duplicated={duplicated}"
    );
}

#[test]
fn compound_lifecycle_soak_plain() {
    campaign(false);
}

#[cfg(feature = "oscore")]
#[test]
fn compound_lifecycle_soak_oscore() {
    campaign(true);
}

// This is software datagram-loss injection, not Wi-Fi or physical qualification.
// Both Apps and security contexts stay alive across the complete blackout.
fn blackout_recovery(secured: bool, asynchronous: bool) {
    use core::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    struct Timer;
    impl crate::platform::AsyncTimer for Timer {
        async fn wait_until(&mut self, _: u64) {
            core::future::pending().await
        }
    }
    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = core::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("finite in-memory operation unexpectedly blocked"),
        }
    }
    std::thread_local! {
        static EFFECTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    fn effect(request: Request<'_>) -> Response<'static> {
        assert_eq!(request.payload(), b"apply");
        EFFECTS.with(|effects| effects.set(effects.get() + 1));
        Response::changed()
    }
    macro_rules! send {
        ($request:expr, $now:expr) => {{
            if asynchronous {
                ready($request.send_async($now, || Ok::<_, ()>($now), &mut Timer))
                    .map_err(|error| std::format!("{error:?}"))
            } else {
                $request
                    .send($now)
                    .map_err(|error| std::format!("{error:?}"))
            }
        }};
    }
    macro_rules! progress {
        ($app:expr, $now:expr) => {{
            if asynchronous {
                let mut timer = Timer;
                {
                    let mut wait =
                        core::pin::pin!($app.wait_async($now, &mut timer, core::future::pending()));
                    if let Poll::Ready(Err(error)) =
                        wait.as_mut().poll(&mut Context::from_waker(Waker::noop()))
                    {
                        panic!("receive failed: {error:?}");
                    }
                }
                ready($app.progress_async($now, &mut || Ok::<_, ()>($now), &mut timer, $now + 100))
                    .unwrap();
            } else {
                $app.poll($now).unwrap();
            }
        }};
    }
    let network = Rc::new(RefCell::new(Network {
        blackout: true,
        capture: true,
        ..Network::default()
    }));
    let mut client = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .client_calls::<2>();
    let mut server = App::profile::<profiles::Default>()
        .deterministic_for_tests()
        .block_wise::<false>()
        .route("effect", crate::post(effect))
        .route("value", get(value));
    if !secured {
        client = client.allow_unprotected();
        server = server.allow_unprotected();
    }
    #[cfg(feature = "oscore")]
    if secured {
        let context = |sender, recipient| {
            crate::oscore::SecurityContext::derive(crate::oscore::DeriveParams {
                master_secret: b"finite blackout recovery fixture",
                master_salt: &[],
                sender_id: sender,
                recipient_id: recipient,
                id_context: b"blackout",
            })
            .unwrap()
        };
        client = client.oscore(context(&[1], &[2]));
        server = server.oscore(context(&[2], &[1]));
    }
    let mut client = client
        .bind(Io {
            network: network.clone(),
            side: 0,
        })
        .unwrap();
    let mut server = server
        .bind(Io {
            network: network.clone(),
            side: 1,
        })
        .unwrap();
    EFFECTS.with(|effects| effects.set(0));
    let call = send!(
        client
            .post("effect")
            .payload(b"apply")
            .to(address(1))
            .deadline(20_000),
        0
    )
    .unwrap();
    let original = network.borrow().frames[0][0].clone();
    let cancelled = send!(client.get("value").non().to(address(1)).deadline(50), 0).unwrap();
    let before_refusal = network.borrow().sent;
    if asynchronous {
        assert!(matches!(
            ready(client.get("full").non().to(address(1)).send_async(
                0,
                || Ok::<_, ()>(0),
                &mut Timer
            )),
            Err(crate::app::AsyncCallError::Request(
                crate::app::Error::Saturated
            ))
        ));
    } else {
        assert!(matches!(
            client.get("full").non().to(address(1)).send(0),
            Err(crate::app::Error::Saturated)
        ));
    }
    assert_eq!(
        network.borrow().sent,
        before_refusal,
        "capacity refusal precedes I/O"
    );
    assert!(client.cancel(cancelled));
    assert!(matches!(
        client.take_response(cancelled),
        Some(Err(CallFailure::Cancelled))
    ));
    let expired = send!(client.get("value").non().to(address(1)).deadline(50), 1).unwrap();
    progress!(client, 50);
    assert!(matches!(
        client.take_response(expired),
        Some(Err(CallFailure::DeadlineExceeded))
    ));
    let due = client.next_timeout().unwrap();
    progress!(client, due);
    assert_eq!(
        network.borrow().frames[0].last().unwrap(),
        &original,
        "a lost retry must retain exact encoded bytes, including OSCORE ciphertext"
    );
    assert_eq!(EFFECTS.with(std::cell::Cell::get), 0);
    assert!(client.take_response(call).is_none());
    network.borrow_mut().blackout = false;
    let reconnected_at = client.next_timeout().unwrap();
    assert!(reconnected_at > due, "loss cannot reset the retry schedule");
    progress!(client, reconnected_at);
    assert_eq!(network.borrow().frames[0].last().unwrap(), &original);
    progress!(server, reconnected_at);
    progress!(client, reconnected_at);
    assert_eq!(
        client.take_response(call).unwrap().unwrap().code(),
        Code::CHANGED
    );
    assert_eq!(EFFECTS.with(std::cell::Cell::get), 1);
    // Deliver the same request after response collection. Cached replay may
    // answer it, but neither endpoint may resurrect application delivery.
    network.borrow_mut().queues[1].push_back(original);
    progress!(server, reconnected_at + 1);
    progress!(client, reconnected_at + 1);
    assert_eq!(EFFECTS.with(std::cell::Cell::get), 1);
    assert!(client.take_response(call).is_none());
    let fresh = send!(client.get("value").non().to(address(1)), reconnected_at + 2).unwrap();
    progress!(server, reconnected_at + 2);
    progress!(client, reconnected_at + 2);
    assert_eq!(
        client.take_response(fresh).unwrap().unwrap().payload(),
        b"value"
    );
    assert_eq!(client.engine_mut().tx_occupied(), 0);
    assert_eq!(client.engine_mut().rx_occupied(), 0);
    println!(
        "software blackout recovery secured={secured} async={asynchronous} retries=2 effects=1 cancellation=1 deadline=1 capacity_refusal=1 fresh_completion=1"
    );
}

#[test]
fn finite_blackout_reconnect_sync_and_async_unprotected() {
    for asynchronous in [false, true] {
        blackout_recovery(false, asynchronous);
    }
}

#[cfg(feature = "oscore")]
#[test]
fn finite_blackout_reconnect_sync_and_async_oscore() {
    for asynchronous in [false, true] {
        blackout_recovery(true, asynchronous);
    }
}
