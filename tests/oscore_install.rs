//! Checked context installation preserves caller-owned staged state.
#![cfg(feature = "oscore")]
use coaptic::app::{AppStorage, AppStore, OscoreInstallError};
use coaptic::message::{Message, MessageId, Token, Type, decode};
use coaptic::oscore::{DeriveParams, SecurityContext};
use coaptic::resources::Resources;
use coaptic::storage::{DatagramIo, ExchangeEntry, ExchangeKey, SlotPool, Storage};
use coaptic::{App, Code, Endpoint, Response};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static EFFECTS: Cell<usize> = const { Cell::new(0) };
}
struct TrackingAllocator;
// Test instrumentation counts only this thread and delegates to the system allocator.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;
fn without_allocating<R>(work: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ALLOCATIONS.with(|c| c.set(None));
        }
    }
    ALLOCATIONS.with(|c| c.set(Some(0)));
    let guard = Guard;
    let result = work();
    let allocations = ALLOCATIONS.with(|c| c.get().unwrap());
    drop(guard);
    assert_eq!(
        allocations, 0,
        "poll/request work must not allocate or grow"
    );
    result
}

coaptic::memory_profile! { Buffers {
    rx_datagram: (2, 512), tx_datagram: (2, 512),
    rx_body: (1, 2048), tx_body: (1, 2048), dedup: 2, observe: 2,
}}
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 512],
    len: usize,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }
}
#[derive(Default)]
struct Io {
    input: Option<Frame>,
    last: Frame,
    sends: usize,
    receives: usize,
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
impl coaptic::storage::DatagramTransport for Io {
    type Error = core::convert::Infallible;
}

impl DatagramIo for Io {
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        self.receives += 1;
        Ok(self.input.take().map(|f| {
            out[..f.len].copy_from_slice(&f.bytes[..f.len]);
            (f.len, PEER)
        }))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.sends += 1;
        Ok(bytes.len())
    }
}
type TestApp<S> = App<Buffers, Io, 1, true, S, 2>;
fn context(server: bool, key: u8) -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: &[key; 16],
        master_salt: &[],
        sender_id: if server { &[2] } else { &[1] },
        recipient_id: if server { &[1] } else { &[2] },
        id_context: &[],
    })
    .unwrap()
}
fn fixed() -> TestApp<AppStore<Buffers, true>> {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .bind(Io::default())
    .unwrap()
}
fn snapshot(c: &SecurityContext) -> ([u8; 16], u64, Option<u64>, (u64, u32)) {
    (
        *c.sender_key(),
        c.sender_seq(),
        c.sender_reservation_end(),
        c.replay_checkpoint().parts(),
    )
}
fn busy<S: AppStorage>(app: &mut TestApp<S>, candidate: &mut Option<SecurityContext>) {
    let old = snapshot(app.oscore().unwrap());
    let staged = snapshot(candidate.as_ref().unwrap());
    let io = (
        app.transport().sends,
        app.transport().receives,
        app.transport().last.bytes,
    );
    without_allocating(|| {
        assert!(matches!(
            app.set_oscore(candidate),
            Err(OscoreInstallError::Busy)
        ))
    });
    assert_eq!(snapshot(app.oscore().unwrap()), old);
    assert_eq!(snapshot(candidate.as_ref().unwrap()), staged);
    assert_eq!(
        (
            app.transport().sends,
            app.transport().receives,
            app.transport().last.bytes
        ),
        io
    );
}
fn pool<S: Storage>(s: &mut S, which: usize) -> &mut dyn SlotPool {
    match which {
        0 => s.rx_datagram(),
        1 => s.tx_datagram(),
        2 => s.rx_body().unwrap(),
        3 => s.tx_body().unwrap(),
        4 => s.dedup(),
        5 => s.observe(),
        _ => unreachable!(),
    }
}
fn storage_campaign<S: AppStorage>(mut app: TestApp<S>) {
    assert!(matches!(
        app.set_oscore(&mut None),
        Err(OscoreInstallError::MissingCandidate)
    ));
    app.set_oscore(&mut Some(context(false, 24))).unwrap();
    for which in 0..6 {
        let mut candidate = Some(context(false, 35));
        candidate.as_mut().unwrap().set_sender_seq(123).unwrap();
        candidate
            .as_mut()
            .unwrap()
            .restore_replay(coaptic::oscore::ReplayCheckpoint::from_parts(60, 5).unwrap())
            .unwrap();
        let id = pool(app.engine_mut().storage_mut(), which)
            .acquire()
            .unwrap();
        match which {
            0 => {
                app.engine_mut().write_rx(id, &[0x31; 17], PEER).unwrap();
            }
            1 => {
                app.engine_mut().write_tx(id, &[0x52; 19], PEER).unwrap();
            }
            3 => {
                let mut access = app.engine_mut().access_tx_body_mut(id).unwrap();
                access.bytes_mut()[..900].fill(0x73);
                access.set_len(900).unwrap();
            }
            _ => {}
        }
        let count = pool(app.engine_mut().storage_mut(), which).occupied_count();
        busy(&mut app, &mut candidate);
        match which {
            0 => assert_eq!(
                app.engine_mut().access_rx(id).unwrap().as_bytes(),
                &[0x31; 17]
            ),
            1 => assert_eq!(
                app.engine_mut().access_tx(id).unwrap().as_bytes(),
                &[0x52; 19]
            ),
            3 => assert_eq!(app.engine().tx_body_payload(id).unwrap(), &[0x73; 900]),
            _ => {}
        }
        assert_eq!(
            pool(app.engine_mut().storage_mut(), which).occupied_count(),
            count
        );
        pool(app.engine_mut().storage_mut(), which)
            .release(id)
            .unwrap();
        without_allocating(|| {
            app.set_oscore(&mut candidate).unwrap();
        });
        assert!(candidate.is_none());
        assert_eq!(app.oscore().unwrap().sender_seq(), 123);
    }
    // Exchange occupancy is independent of the released TX slot.
    let id = app.engine_mut().acquire_tx().unwrap();
    let token = Token::new(&[9]).unwrap();
    let row = ExchangeEntry::new(token, PEER, MessageId::new(4), id);
    app.engine_mut().storage_mut().insert_exchange(row).unwrap();
    app.engine_mut().release_tx(id).unwrap();
    let mut candidate = Some(context(false, 45));
    busy(&mut app, &mut candidate);
    assert_eq!(
        app.engine_mut()
            .storage_mut()
            .take_exchange(ExchangeKey::new(token, PEER)),
        Some(row)
    );
    app.set_oscore(&mut candidate).unwrap();
    assert!(candidate.is_none());
}
#[test]
fn oscore_install_fixed_pool_and_exchange_refusal_reclaims_without_allocation() {
    storage_campaign(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn oscore_install_allocated_exact_budget_has_same_refusal_and_recovery() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>()
    };
    let bytes = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    let app = App::with_resources(resources().allocated(bytes))
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    storage_campaign(app);
}
fn protected_campaign<S: AppStorage>(mut client: TestApp<S>) {
    client.set_oscore(&mut Some(context(false, 24))).unwrap();
    let mut server = fixed();
    server.set_oscore(&mut Some(context(true, 35))).unwrap();
    let old = client.get("value").non().to(PEER).send(0).unwrap();
    server.transport_mut().input = Some(client.transport().last);
    without_allocating(|| {
        server
            .poll_with(0, |_, _| panic!("old key must not dispatch"))
            .unwrap()
    });
    assert_eq!(server.transport().sends, 0);
    let mut candidate = Some(context(false, 35));
    busy(&mut client, &mut candidate);
    assert!(client.cancel(old));
    busy(&mut client, &mut candidate);
    assert!(client.take_response(old).unwrap().is_err());
    client.set_oscore(&mut candidate).unwrap();
    assert!(candidate.is_none());
    let call = without_allocating(|| {
        client
            .get("value")
            .non()
            .deadline(100)
            .to(PEER)
            .send(1)
            .unwrap()
    });
    server.transport_mut().input = Some(client.transport().last);
    without_allocating(|| {
        server
            .poll_with(1, |_, _| Response::content(b"fresh protected body"))
            .unwrap()
    });
    client.transport_mut().input = Some(server.transport().last);
    without_allocating(|| client.poll(2).unwrap());
    let mut next = Some(context(false, 45));
    busy(&mut client, &mut next);
    assert_eq!(
        client.take_response(call).unwrap().unwrap().payload(),
        b"fresh protected body"
    );
    // Collected response backing must not cause permanent Busy.
    without_allocating(|| {
        client.set_oscore(&mut next).unwrap();
    });
    assert!(next.is_none());
    let call = client
        .get("value")
        .non()
        .deadline(4)
        .to(PEER)
        .send(3)
        .unwrap();
    client.poll(4).unwrap();
    let mut next = Some(context(false, 55));
    busy(&mut client, &mut next);
    assert!(client.take_response(call).unwrap().is_err());
    client.set_oscore(&mut next).unwrap();
}
#[test]
fn oscore_install_fixed_old_key_refusal_call_collection_deadline_and_recovery() {
    protected_campaign(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn oscore_install_allocated_protected_call_recovery() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>()
    };
    let bytes = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    protected_campaign(
        App::with_resources(resources().allocated(bytes))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap(),
    );
}
#[test]
fn oscore_install_error_diagnostics_are_compact_and_contain_no_context() {
    assert_eq!(core::mem::size_of::<OscoreInstallError>(), 1);
    assert_eq!(format!("{:?}", OscoreInstallError::Busy), "Busy");
    assert_eq!(
        format!("{}", OscoreInstallError::MissingCandidate),
        "no OSCORE context was staged"
    );
}

fn observe_campaign<S: AppStorage>(mut client: TestApp<S>) {
    client.set_oscore(&mut Some(context(false, 24))).unwrap();
    let mut server = fixed();
    server.set_oscore(&mut Some(context(true, 24))).unwrap();
    let call = client.get("value").observe().to(PEER).send(0).unwrap();
    server.transport_mut().input = Some(client.transport().last);
    server
        .poll_with(1, |_, _| Response::content(b"observed").observe(0))
        .unwrap();
    client.transport_mut().input = Some(server.transport().last);
    client.poll(2).unwrap();
    assert_eq!(
        client.take_response(call).unwrap().unwrap().payload(),
        b"observed"
    );
    let mut next = Some(context(false, 35));
    busy(&mut client, &mut next);
    let mut server_next = Some(context(true, 35));
    busy(&mut server, &mut server_next);
    assert!(client.cancel(call));
    assert!(client.take_response(call).unwrap().is_err());
    client.set_oscore(&mut next).unwrap();
    assert!(
        server
            .engine_mut()
            .remove_observe(coaptic::storage::ObserveKey::new(call.token(), PEER))
    );
    // Cached request ciphertext is still old-epoch work after removing Observe.
    busy(&mut server, &mut server_next);
    server.poll(300_000).unwrap();
    server.set_oscore(&mut server_next).unwrap();
}
#[test]
fn oscore_install_observe_and_cached_reply_require_old_epoch_drain() {
    observe_campaign(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn oscore_install_allocated_observe_requires_drain() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>()
    };
    let bytes = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    observe_campaign(
        App::with_resources(resources().allocated(bytes))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap(),
    );
}
#[test]
fn oscore_install_retained_binding_and_checkpoint_refusal_are_unchanged() {
    let mut app = fixed();
    app.set_oscore(&mut Some(context(false, 24))).unwrap();
    let mut commits = 0;
    app.poll_with_oscore_checkpoint(0, |_| {
        commits += 1;
        true
    })
    .unwrap();
    assert_eq!(commits, 1);
    let mut next = Some(context(false, 35));
    let slot = app.engine_mut().acquire_rx().unwrap();
    busy(&mut app, &mut next);
    app.engine_mut().release_rx(slot).unwrap();
    app.poll_with_oscore_checkpoint(1, |_| {
        commits += 1;
        true
    })
    .unwrap();
    assert_eq!(commits, 1, "refusal must preserve clean checkpoint");
    let token = Token::new(&[7]).unwrap();
    let msg = Message::new(Type::NonConfirmable, Code::GET, MessageId::new(7)).with_token(token);
    let mut wire = [0; 512];
    app.oscore_mut()
        .unwrap()
        .protect_request(&msg, &mut wire)
        .unwrap();
    assert!(app.oscore().unwrap().lookup(token).is_some());
    busy(&mut app, &mut next);
    app.oscore_mut().unwrap().take(token).unwrap();
    app.set_oscore(&mut next).unwrap();
    assert!(next.is_none());
    assert!(matches!(
        app.poll(2),
        Err(coaptic::Error::OscoreCheckpointRequired)
    ));
    app.poll_with_oscore_checkpoint(2, |_| {
        commits += 1;
        true
    })
    .unwrap();
    assert_eq!(commits, 2);
}
fn deferred_campaign<S: AppStorage>(mut server: TestApp<S>) {
    server.set_oscore(&mut Some(context(true, 24))).unwrap();
    let mut client = fixed();
    client.set_oscore(&mut Some(context(false, 24))).unwrap();
    let mut previous_handle = None;
    for finish in [false, true] {
        let now = if finish { 400_000 } else { 0 };
        let call = client
            .get("value")
            .block2(coaptic::message::BlockValue::from_size(0, false, 256).unwrap())
            .to(PEER)
            .send(now)
            .unwrap();
        server.transport_mut().input = Some(client.transport().last);
        let mut handle = None;
        server
            .poll_with(now, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        let handle = handle.unwrap();
        if let Some(stale) = previous_handle {
            assert_ne!(stale, handle);
            assert!(!server.cancel_deferred(stale));
            assert!(
                server
                    .complete(stale, Response::content(b"stale"), now)
                    .is_err()
            );
        }
        previous_handle = Some(handle);
        let mut next = Some(context(true, 24));
        busy(&mut server, &mut next);
        if finish {
            server
                .complete(handle, Response::content(&[3; 2000]), now + 1)
                .unwrap();
            let reply =
                decode(&server.transport().last.bytes[..server.transport().last.len]).unwrap();
            assert!(reply.oscore().is_some());
            busy(&mut server, &mut next);
            let mut ack = Frame::default();
            ack.bytes[..4].copy_from_slice(&[
                0x60,
                0,
                server.transport().last.bytes[2],
                server.transport().last.bytes[3],
            ]);
            ack.len = 4;
            server.transport_mut().input = Some(ack);
            server.poll(now + 2).unwrap();
            busy(&mut server, &mut next);
            assert!(
                !server.cancel_deferred(handle),
                "completed snapshot requires expiry"
            );
        } else {
            assert!(server.cancel_deferred(handle));
            busy(&mut server, &mut next);
        }
        assert!(client.cancel(call));
        assert!(client.take_response(call).unwrap().is_err());
        for tick in 0..8 {
            server.poll(now + 300_000 + tick).unwrap();
        }
        let counts: [usize; 6] =
            core::array::from_fn(|i| pool(server.engine_mut().storage_mut(), i).occupied_count());
        assert!(
            server.set_oscore(&mut next).is_ok(),
            "finish={finish} pools={counts:?}"
        );
        assert!(next.is_none());
    }
}
#[test]
fn oscore_install_deferred_handle_snapshot_cancel_expiry_and_recovery() {
    deferred_campaign(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn oscore_install_allocated_deferred_snapshot_requires_drain() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>()
    };
    let bytes = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    deferred_campaign(
        App::with_resources(resources().allocated(bytes))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap(),
    );
}

#[test]
fn oscore_install_plaintext_work_refuses_first_attach_without_fallback_change() {
    let mut app = fixed();
    let call = app.get("value").non().to(PEER).send(0).unwrap();
    let mut candidate = Some(context(false, 24));
    let before = snapshot(candidate.as_ref().unwrap());
    let sends = app.transport().sends;
    assert!(matches!(
        app.set_oscore(&mut candidate),
        Err(OscoreInstallError::Busy)
    ));
    assert!(app.oscore().is_none());
    assert_eq!(snapshot(candidate.as_ref().unwrap()), before);
    assert_eq!(app.transport().sends, sends);
    assert!(app.cancel(call));
    assert!(app.take_response(call).unwrap().is_err());
    app.set_oscore(&mut candidate).unwrap();
    assert!(candidate.is_none());
    let fresh = app.get("value").non().to(PEER).send(1).unwrap();
    assert_ne!(fresh.token(), call.token());
    assert!(app.take_response(call).is_none());
    assert!(
        decode(&app.transport().last.bytes[..app.transport().last.len])
            .unwrap()
            .oscore()
            .is_some()
    );
}
