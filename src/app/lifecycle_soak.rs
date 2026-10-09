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
}

struct Io {
    network: Rc<RefCell<Network>>,
    side: usize,
}

fn address(side: usize) -> Endpoint {
    Endpoint::v4([192, 0, 2, side as u8 + 1], 5683)
}

impl DatagramIo for Io {
    type Error = ();
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
        if network.sent % 13 == 0 {
            network.dropped += 1;
        } else {
            let duplicate = network.sent % 17 == 0;
            network.queues[1 - self.side].push_back(bytes.to_vec());
            if duplicate {
                network.duplicated += 1;
                network.queues[1 - self.side].push_back(bytes.to_vec());
            }
        }
        Ok(bytes.len())
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
            client = client.allow_plaintext();
            server = server.allow_plaintext();
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
