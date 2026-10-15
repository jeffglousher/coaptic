//! Finite source-backed CON supersession; no sustained availability claim.
use coaptic::app::{AppStorage, AppStore};
use coaptic::message::{MessageId, Type, decode};
use coaptic::storage::{DatagramIo, ObserveKey, PendingCon};
use coaptic::{App, Endpoint, Response, get, profiles, resources};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };

}
struct Tracking;
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.realloc(p, layout, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Tracking = Tracking;
fn no_alloc<R>(f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.with(|c| c.set(None));
        }
    }
    ALLOCATIONS.with(|c| c.set(Some(0)));
    let reset = Reset;
    let result = f();
    let n = ALLOCATIONS.with(|c| c.get().unwrap());
    drop(reset);
    assert_eq!(n, 0);
    result
}

thread_local! {
    static VERSION: Cell<u8> = const { Cell::new(0) };
    static FAIL_ENTROPY: Cell<bool> = const { Cell::new(false) };
}
const OWNER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const OTHER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const LARGE: &[u8] = &[7; 2000];
fn source() -> Response<'static> {
    match VERSION.with(Cell::get) {
        0 => Response::content(b"initial complete").etag(b"v0"),
        1 => Response::content(b"intermediate complete").etag(b"v1"),
        2 => Response::content(b"latest complete representation").etag(b"v2"),
        3 => Response::content(LARGE),
        4 => Response::not_found(),
        _ => {
            Response::content(b"changed format").content_format(coaptic::ContentFormat::TEXT_PLAIN)
        }
    }
    .max_age(5)
}
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 1472],
    len: usize,
    peer: Endpoint,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 1472],
            len: 0,
            peer: OWNER,
        }
    }
}
#[derive(Default)]
struct Io {
    incoming: Option<Frame>,
    last: Frame,
    sends: usize,
}
impl DatagramIo for Io {
    type Error = core::convert::Infallible;
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        Ok(self.incoming.take().map(|f| {
            out[..f.len].copy_from_slice(&f.bytes[..f.len]);
            (f.len, f.peer)
        }))
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.last.peer = peer;
        self.sends += 1;
        Ok(bytes.len())
    }
}
type Server<S> = App<profiles::Default, Io, 8, false, S>;
type Client = Server<AppStore<profiles::Default, false>>;
#[cfg(feature = "oscore")]
fn context(sender: &[u8], recipient: &[u8]) -> coaptic::oscore::SecurityContext {
    coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x38; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
fn client(protected: bool) -> Client {
    let builder = App::with_resources(resources::Resources::<profiles::Default>::new())
        .deterministic_for_tests();
    #[cfg(feature = "oscore")]
    let builder = if protected {
        builder.oscore(context(&[1], &[2]))
    } else {
        builder.allow_plaintext()
    };
    #[cfg(not(feature = "oscore"))]
    let builder = {
        assert!(!protected);
        builder.allow_plaintext()
    };
    builder.bind(Io::default()).unwrap()
}
fn random(out: &mut [u8]) -> bool {
    out.fill(0x53);
    !FAIL_ENTROPY.with(Cell::get)
}
fn reset() {
    VERSION.with(|v| v.set(0));
    FAIL_ENTROPY.with(|f| f.set(false));
}
fn fixed(protected: bool) -> Client {
    reset();
    let builder = App::with_resources(resources::Resources::<profiles::Default>::new())
        .randomness(random)
        .route("value", get(|_| source()).observe(source));
    #[cfg(feature = "oscore")]
    let builder = if protected {
        builder.oscore(context(&[2], &[1]))
    } else {
        builder.allow_plaintext()
    };
    #[cfg(not(feature = "oscore"))]
    let builder = {
        assert!(!protected);
        builder.allow_plaintext()
    };
    builder.bind(Io::default()).unwrap()
}
#[cfg(feature = "alloc")]
fn allocated(protected: bool) -> Server<coaptic::storage::AllocMemory> {
    reset();
    let config = resources::Resources::<profiles::Default>::new().allocated(usize::MAX);
    let required = config.required_heap_bytes().unwrap();
    let short = resources::Resources::<profiles::Default>::new().allocated(required - 1);
    assert!(matches!(
        App::with_resources(short)
            .allow_plaintext()
            .randomness(random)
            .bind(Io::default()),
        Err(coaptic::BuildError::HeapBudgetExceeded { .. })
    ));
    let builder =
        App::with_resources(resources::Resources::<profiles::Default>::new().allocated(required))
            .randomness(random)
            .route("value", get(|_| source()).observe(source));
    #[cfg(feature = "oscore")]
    let builder = if protected {
        builder.oscore(context(&[2], &[1]))
    } else {
        builder.allow_plaintext()
    };
    #[cfg(not(feature = "oscore"))]
    let builder = {
        assert!(!protected);
        builder.allow_plaintext()
    };
    builder.bind(Io::default()).unwrap()
}
fn pending<S: AppStorage>(app: &Server<S>, key: ObserveKey) -> PendingCon {
    let row = app
        .engine()
        .observe_interest(app.engine().lookup_observe(key).unwrap())
        .unwrap();
    let mid = row.notify_hold().unwrap().con_mid().unwrap();
    let tx = app.engine().lookup_pending_con(mid, OWNER).unwrap();
    app.engine().pending_con(tx).unwrap()
}
fn control<S: AppStorage>(
    app: &mut Server<S>,
    mid: MessageId,
    rst: bool,
    peer: Endpoint,
    now: u64,
) {
    app.transport_mut().incoming = Some(control_frame(mid, rst, peer));
    app.poll(now).unwrap();
}
fn control_frame(mid: MessageId, rst: bool, peer: Endpoint) -> Frame {
    let m = mid.get();
    let mut frame = Frame {
        len: 4,
        peer,
        ..Frame::default()
    };
    frame.bytes[..4].copy_from_slice(&[if rst { 0x70 } else { 0x60 }, 0, (m >> 8) as u8, m as u8]);
    frame
}
fn deliver<S: AppStorage>(
    server: &Server<S>,
    client: &mut Client,
    call: coaptic::Call,
    body: &[u8],
    now: u64,
) {
    client.transport_mut().incoming = Some(server.transport().last);
    client.poll(now).unwrap();
    assert_eq!(client.take_response(call).unwrap().unwrap().payload(), body);
}
fn registered<S: AppStorage>(
    server: &mut Server<S>,
    client: &mut Client,
) -> (coaptic::Call, ObserveKey) {
    let call = client.get("value").observe().to(OWNER).send(0).unwrap();
    server.transport_mut().incoming = Some(client.transport().last);
    server.poll(1).unwrap();
    deliver(server, client, call, b"initial complete", 2);
    let key = ObserveKey::new(call.token(), OWNER);
    assert_eq!(
        server.notify_confirmable(3, &["value"], source()).unwrap(),
        1
    );
    (call, key)
}
fn lifecycle<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (call, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        let first_wire = server.transport().last;
        // The combined storage primitive refuses inconsistent ownership before
        // touching either the bytes or the relation.
        let row_id = server.engine().lookup_observe(key).unwrap();
        let original = server.engine().observe_interest(row_id).unwrap();
        let new_mid = MessageId::new(first.message_id().get().wrapping_add(1));
        let mut replacement = original;
        replacement.record_notify(4, new_mid, true);
        let mut candidate = first_wire;
        candidate.bytes[2..4].copy_from_slice(&new_mid.get().to_be_bytes());
        for case in 0..3 {
            let mut bytes = candidate;
            let (before, after, expected) = match case {
                0 => (
                    original.with_seq(original.seq() + 1),
                    replacement,
                    coaptic::storage::PendingReplaceError::Stale,
                ),
                1 => (
                    original,
                    original,
                    coaptic::storage::PendingReplaceError::InvalidMessage,
                ),
                _ => {
                    assert_ne!(bytes.bytes[0] & 15, 0);
                    bytes.bytes[4] ^= 1;
                    (
                        original,
                        replacement,
                        coaptic::storage::PendingReplaceError::InvalidMessage,
                    )
                }
            };
            assert_eq!(
                coaptic::storage::ObserveSlots::replace_observe_con(
                    server.engine_mut().storage_mut(),
                    first,
                    before,
                    after,
                    &bytes.bytes[..bytes.len]
                ),
                Err(expected)
            );
            assert_eq!(server.engine().observe_interest(row_id), Some(original));
            assert_eq!(pending(&server, key), first);
            assert_eq!(
                coaptic::storage::DatagramSlots::tx_payload(
                    server.engine().storage(),
                    first.tx_slot()
                ),
                Some(&first_wire.bytes[..first_wire.len])
            );
        }
        #[cfg(feature = "oscore")]
        let sender_before = server.oscore().map(|ctx| ctx.sender_seq());
        #[cfg(feature = "oscore")]
        let binding_before = server
            .engine()
            .observe_interest(server.engine().lookup_observe(key).unwrap())
            .unwrap()
            .oscore();
        deliver(&server, &mut client, call, b"initial complete", 4);
        let mut held = [None; 4];
        for item in &mut held {
            *item = server.engine_mut().acquire_tx();
        }
        assert!(server.engine_mut().acquire_tx().is_none());
        VERSION.with(|v| v.set(1));
        assert_eq!(server.signal(&["value"]), 1);
        VERSION.with(|v| v.set(2));
        assert_eq!(server.signal(&["value"]), 1);
        let due = first.rto().next_timeout_ms();
        let sent = server.transport().sends;
        server.poll(due - 1).unwrap();
        assert_eq!(server.transport().sends, sent);
        server.poll(due).unwrap();
        let next = pending(&server, key);
        assert_ne!(next.message_id(), first.message_id());
        assert_eq!(next.tx_slot(), first.tx_slot());
        assert_eq!(next.rto(), first.rto().next_attempt(due).unwrap());
        let row = server
            .engine()
            .observe_interest(server.engine().lookup_observe(key).unwrap())
            .unwrap();
        assert_eq!(row.superseded_con_mids(), &[first.message_id()]);
        assert!(!row.is_pending());
        #[cfg(feature = "oscore")]
        {
            assert_eq!(row.oscore(), binding_before);
            assert_eq!(
                server.oscore().map(|ctx| ctx.sender_seq()),
                sender_before.map(|n| n + 1)
            );
        }
        assert_eq!(
            decode(&server.transport().last.bytes[..server.transport().last.len])
                .unwrap()
                .ty(),
            Type::Confirmable
        );
        #[cfg(feature = "oscore")]
        if protected {
            assert!(
                decode(&server.transport().last.bytes[..server.transport().last.len])
                    .unwrap()
                    .oscore()
                    .is_some()
            );
        }
        deliver(
            &server,
            &mut client,
            call,
            b"latest complete representation",
            due + 1,
        );
        let next_wire = server.transport().last;
        assert_ne!(
            &first_wire.bytes[..first_wire.len],
            &next_wire.bytes[..next_wire.len]
        );
        control(&mut server, first.message_id(), false, OWNER, due + 2);
        control(&mut server, next.message_id(), false, OTHER, due + 3);
        control(&mut server, first.message_id(), true, OTHER, due + 4);
        assert_eq!(pending(&server, key), next);
        // No signal: retransmit exactly the same representation/ciphertext.
        server.poll(next.rto().next_timeout_ms()).unwrap();
        #[cfg(feature = "oscore")]
        assert_eq!(
            server.oscore().map(|ctx| ctx.sender_seq()),
            sender_before.map(|n| n + 1)
        );
        assert_eq!(
            &server.transport().last.bytes[..server.transport().last.len],
            &next_wire.bytes[..next_wire.len]
        );
        // A current-chain old CON RST cancels the current packet and observation.
        control(
            &mut server,
            first.message_id(),
            true,
            OWNER,
            next.rto().next_timeout_ms() + 1,
        );
        assert!(server.engine().lookup_observe(key).is_none());
        assert!(server.engine().pending_con(next.tx_slot()).is_none());
        for slot in held.into_iter().flatten() {
            server.engine_mut().release_tx(slot).unwrap();
        }
    });
}
#[test]
fn fixed_plaintext_supersession() {
    lifecycle(fixed(false), false);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_plaintext_supersession() {
    lifecycle(allocated(false), false);
}
#[cfg(feature = "oscore")]
#[test]
fn fixed_protected_supersession() {
    lifecycle(fixed(true), true);
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_supersession() {
    lifecycle(allocated(true), true);
}

fn refusal<S: AppStorage>(mut server: Server<S>, protected: bool, mode: u8) {
    let mut client = client(protected);
    no_alloc(|| {
        let (call, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        let wire = server.transport().last;
        let id = server.engine().lookup_observe(key).unwrap();
        let original = server.engine().observe_interest(id).unwrap();
        VERSION.with(|v| v.set(if mode == 6 { 2 } else { mode }));
        assert_eq!(server.signal(&["value"]), 1);
        FAIL_ENTROPY.with(|f| f.set(mode == 6));
        let due = first.rto().next_timeout_ms();
        let error = server.poll(due).unwrap_err();
        if mode == 6 {
            assert!(matches!(error, coaptic::Error::Identity(_)));
        } else if mode == 3 {
            assert!(matches!(error, coaptic::Error::Message(_)));
        } else {
            assert_eq!(error, coaptic::Error::ObserveSupersessionUnsupported);
        }
        let failed = pending(&server, key);
        assert_eq!(failed.message_id(), first.message_id());
        assert_eq!(failed.rto(), first.rto().next_attempt(due).unwrap());
        assert_eq!(
            &server.transport().last.bytes[..server.transport().last.len],
            &wire.bytes[..wire.len]
        );
        let retained = server.engine().observe_interest(id).unwrap();
        assert_eq!(retained.seq(), original.seq());
        assert_eq!(retained.notify_hold(), original.notify_hold());
        assert!(retained.superseded_con_mids().is_empty());
        assert!(retained.is_pending());
        #[cfg(feature = "oscore")]
        assert_eq!(retained.oscore(), original.oscore());
        FAIL_ENTROPY.with(|f| f.set(false));
        VERSION.with(|v| v.set(2));
        let retry = failed.rto().next_timeout_ms();
        server.poll(retry).unwrap();
        assert_eq!(pending(&server, key).rto().attempts(), 2);
        deliver(
            &server,
            &mut client,
            call,
            b"latest complete representation",
            retry + 1,
        );
        let latest = pending(&server, key);
        control(&mut server, latest.message_id(), false, OWNER, retry + 2);
        let row = server.engine().observe_interest(id).unwrap();
        assert!(row.superseded_con_mids().is_empty());
        assert!(row.notify_hold().is_none());
        assert!(server.engine().pending_con(latest.tx_slot()).is_none());
        // The completed chain's old reset cannot cancel a later registration.
        control(&mut server, first.message_id(), true, OWNER, retry + 3);
        assert!(server.engine().lookup_observe(key).is_some());
    });
}
#[test]
fn fixed_refusal_and_recovery() {
    for mode in [3, 4, 5, 6] {
        refusal(fixed(false), false, mode);
    }
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_refusal_and_recovery() {
    for mode in [3, 4, 5, 6] {
        refusal(allocated(false), false, mode);
    }
}
#[cfg(feature = "oscore")]
#[test]
fn protected_refusal_and_recovery() {
    for mode in [3, 4, 5, 6] {
        refusal(fixed(true), true, mode);
    }
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_refusal_and_recovery() {
    for mode in [3, 4, 5, 6] {
        refusal(allocated(true), true, mode);
    }
}

fn finite_chain<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (_call, key) = registered(&mut server, &mut client);
        let mut current = pending(&server, key);
        for attempt in 1..=coaptic::message::Transmission::MAX_RETRANSMIT {
            VERSION.with(|v| v.set(1 + (attempt % 2)));
            server.signal(&["value"]);
            let due = current.rto().next_timeout_ms();
            server.poll(due).unwrap();
            let next = pending(&server, key);
            assert_eq!(next.rto(), current.rto().next_attempt(due).unwrap());
            let row = server
                .engine()
                .observe_interest(server.engine().lookup_observe(key).unwrap())
                .unwrap();
            assert_eq!(row.superseded_con_mids().len(), usize::from(attempt));
            control(&mut server, current.message_id(), false, OWNER, due + 1);
            assert_eq!(pending(&server, key), next);
            current = next;
        }
        server.signal(&["value"]);
        let sent = server.transport().sends;
        server.poll(current.rto().next_timeout_ms()).unwrap();
        assert_eq!(server.transport().sends, sent);
        assert!(server.engine().lookup_observe(key).is_none());
        assert!(server.engine().pending_con(current.tx_slot()).is_none());
    });
}
#[test]
fn fixed_finite_chain() {
    finite_chain(fixed(false), false);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_finite_chain() {
    finite_chain(allocated(false), false);
}
#[cfg(feature = "oscore")]
#[test]
fn protected_finite_chain() {
    finite_chain(fixed(true), true);
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_finite_chain() {
    finite_chain(allocated(true), true);
}

#[cfg(feature = "oscore")]
fn exhausted<S: AppStorage>(mut server: Server<S>) {
    let mut client = client(true);
    no_alloc(|| {
        let (_call, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        let wire = server.transport().last;
        let id = server.engine().lookup_observe(key).unwrap();
        let original = server.engine().observe_interest(id).unwrap();
        server
            .oscore_mut()
            .unwrap()
            .set_sender_seq(1u64 << 40)
            .unwrap();
        VERSION.with(|v| v.set(2));
        server.signal(&["value"]);
        let due = first.rto().next_timeout_ms();
        assert_eq!(
            server.poll(due),
            Err(coaptic::Error::Oscore(
                coaptic::oscore::Error::SequenceExhausted
            ))
        );
        assert_eq!(server.oscore().unwrap().sender_seq(), 1u64 << 40);
        assert_eq!(pending(&server, key).message_id(), first.message_id());
        assert_eq!(
            &server.transport().last.bytes[..server.transport().last.len],
            &wire.bytes[..wire.len]
        );
        let retained = server.engine().observe_interest(id).unwrap();
        assert_eq!(retained.oscore(), original.oscore());
        assert_eq!(retained.notify_hold(), original.notify_hold());
        control(&mut server, first.message_id(), false, OWNER, due + 1);
        assert!(server.engine().pending_con(first.tx_slot()).is_none());
    });
}
#[cfg(feature = "oscore")]
#[test]
fn exhausted_sender_retains_authenticated_retry() {
    exhausted(fixed(true));
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_exhausted_sender_retains_authenticated_retry() {
    exhausted(allocated(true));
}

#[cfg(feature = "alloc")]
#[test]
fn metadata_accounting() {
    let mut c = coaptic::storage::Capacities::from_profile::<profiles::Default>();
    c.observe_entries = 0;
    let empty = coaptic::storage::AllocMemory::required_heap_bytes(c).unwrap();
    c.observe_entries = 4;
    let full = coaptic::storage::AllocMemory::required_heap_bytes(c).unwrap();
    assert_eq!(
        full - empty,
        4 * (core::mem::size_of::<Option<coaptic::storage::ObserveInterest>>() + 2)
    );
    println!(
        "ObserveInterest={} OptionObserveInterest={} DefaultRequestedHeap={full}",
        core::mem::size_of::<coaptic::storage::ObserveInterest>(),
        core::mem::size_of::<Option<coaptic::storage::ObserveInterest>>()
    );
}

fn replace_registration<S: AppStorage>(mut server: Server<S>, protected: bool, cancel: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (call, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        VERSION.with(|v| v.set(2));
        server.signal(&["value"]);
        let due = first.rto().next_timeout_ms();
        server.poll(due).unwrap();
        let replaced = pending(&server, key);
        deliver(
            &server,
            &mut client,
            call,
            b"latest complete representation",
            due + 1,
        );
        // Same MID at another endpoint belongs to an unrelated exchange.
        let unrelated = server.engine_mut().acquire_tx().unwrap();
        assert_eq!(
            server.engine_mut().record_pending_con(
                unrelated,
                OTHER,
                replaced.message_id(),
                due + 2,
                0
            ),
            Some(unrelated)
        );
        let unrelated_pending = server.engine().pending_con(unrelated).unwrap();
        let next_call = if cancel {
            client
                .get("value")
                .deregister_call(call)
                .to(OWNER)
                .send(due + 3)
                .unwrap()
        } else {
            client
                .get("value")
                .reregister_call(call)
                .to(OWNER)
                .send(due + 3)
                .unwrap()
        };
        server.transport_mut().incoming = Some(client.transport().last);
        server.poll(due + 4).unwrap();
        assert!(
            server
                .engine()
                .lookup_pending_con(replaced.message_id(), OWNER)
                .is_none()
        );
        assert_eq!(
            server.engine().pending_con(unrelated),
            Some(unrelated_pending)
        );
        deliver(
            &server,
            &mut client,
            next_call,
            b"latest complete representation",
            due + 5,
        );
        assert_eq!(server.engine().lookup_observe(key).is_none(), cancel);
        if cancel {
            let fresh = client
                .get("value")
                .observe()
                .to(OWNER)
                .send(due + 6)
                .unwrap();
            server.transport_mut().incoming = Some(client.transport().last);
            server.poll(due + 7).unwrap();
            deliver(
                &server,
                &mut client,
                fresh,
                b"latest complete representation",
                due + 8,
            );
        }
        // A completed chain cannot lend its rejection ownership to a new row.
        control(&mut server, first.message_id(), true, OWNER, due + 9);
        control(&mut server, replaced.message_id(), true, OWNER, due + 10);
        assert_eq!(
            server
                .notify_confirmable(due + 11, &["value"], source())
                .unwrap(),
            1
        );
        let new_key = if cancel {
            let parsed =
                decode(&server.transport().last.bytes[..server.transport().last.len]).unwrap();
            ObserveKey::new(parsed.token(), OWNER)
        } else {
            key
        };
        let new_pending = pending(&server, new_key);
        control(&mut server, new_pending.message_id(), true, OTHER, due + 12);
        assert_eq!(pending(&server, new_key), new_pending);
        control(&mut server, new_pending.message_id(), true, OWNER, due + 13);
        assert!(server.engine().lookup_observe(new_key).is_none());
        assert!(server.engine().pending_con(new_pending.tx_slot()).is_none());
        assert_eq!(
            server.engine().pending_con(unrelated),
            Some(unrelated_pending)
        );
        server.engine_mut().release_tx(unrelated).unwrap();
    });
}
#[test]
fn fixed_cancel_and_reregister() {
    for cancel in [false, true] {
        replace_registration(fixed(false), false, cancel);
    }
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_cancel_and_reregister() {
    for cancel in [false, true] {
        replace_registration(allocated(false), false, cancel);
    }
}
#[cfg(feature = "oscore")]
#[test]
fn protected_cancel_and_reregister() {
    for cancel in [false, true] {
        replace_registration(fixed(true), true, cancel);
    }
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_cancel_and_reregister() {
    for cancel in [false, true] {
        replace_registration(allocated(true), true, cancel);
    }
}

fn refused_staging_ingress<S: AppStorage>(mut server: Server<S>, protected: bool, rst: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (_, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        VERSION.with(|v| v.set(4)); // terminal cannot replace the pending CON
        server.signal(&["value"]);
        server.transport_mut().incoming = Some(control_frame(first.message_id(), rst, OWNER));
        assert_eq!(
            server.poll(first.rto().next_timeout_ms()),
            Err(coaptic::Error::ObserveSupersessionUnsupported)
        );
        assert!(
            server
                .engine()
                .lookup_pending_con(first.message_id(), OWNER)
                .is_none()
        );
        if rst {
            assert!(server.engine().lookup_observe(key).is_none());
        }
    });
}
#[test]
fn fixed_staging_refusal_still_dispatches_control() {
    for rst in [false, true] {
        refused_staging_ingress(fixed(false), false, rst);
    }
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_staging_refusal_still_dispatches_control() {
    for rst in [false, true] {
        refused_staging_ingress(allocated(false), false, rst);
    }
}
#[cfg(feature = "oscore")]
#[test]
fn protected_staging_refusal_still_dispatches_control() {
    for rst in [false, true] {
        refused_staging_ingress(fixed(true), true, rst);
    }
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_staging_refusal_still_dispatches_control() {
    for rst in [false, true] {
        refused_staging_ingress(allocated(true), true, rst);
    }
}

fn pinned_retains_row<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (call, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        let row_id = server.engine().lookup_observe(key).unwrap();
        let wire = server.transport().last;
        // Leaking the advanced read guard deliberately retains the pin while
        // regaining the owner. This is not an App happy-path access pattern.
        core::mem::forget(server.engine_mut().access_tx(first.tx_slot()).unwrap());
        VERSION.with(|v| v.set(2));
        server.signal(&["value"]);
        let original = server.engine().observe_interest(row_id).unwrap();
        #[cfg(feature = "oscore")]
        let sender_before = server.oscore().map(|ctx| ctx.sender_seq());
        let due = first.rto().next_timeout_ms();
        assert_eq!(
            server.poll(due),
            Err(coaptic::Error::Io(coaptic::storage::DatagramIoError::Slot(
                coaptic::storage::SlotError::Pinned
            )))
        );
        assert_eq!(server.engine().observe_interest(row_id), Some(original));
        assert_eq!(
            &server.transport().last.bytes[..server.transport().last.len],
            &wire.bytes[..wire.len]
        );
        let retained = pending(&server, key);
        assert_eq!(retained.message_id(), first.message_id());
        assert_eq!(retained.rto(), first.rto().next_attempt(due).unwrap());
        #[cfg(feature = "oscore")]
        assert_eq!(
            server.oscore().map(|ctx| ctx.sender_seq()),
            sender_before.map(|n| n + 1)
        );
        client
            .get("value")
            .deregister_call(call)
            .to(OWNER)
            .send(due + 1)
            .unwrap();
        server.transport_mut().incoming = Some(client.transport().last);
        assert_eq!(
            server.poll(due + 2),
            Err(coaptic::Error::Slot(coaptic::storage::SlotError::Pinned))
        );
        assert_eq!(server.engine().observe_interest(row_id), Some(original));
        assert_eq!(pending(&server, key), retained);
        // Failed cancellation consumed and released its RX; the pool is reusable.
        let mut slots = [None; 4];
        for slot in &mut slots {
            *slot = Some(server.engine_mut().acquire_rx().unwrap());
        }
        assert!(server.engine_mut().acquire_rx().is_none());
        for slot in slots.into_iter().flatten() {
            server.engine_mut().release_rx(slot).unwrap();
        }
    });
}
#[test]
fn fixed_pinned_replacement_and_cancel_refuse_atomically() {
    pinned_retains_row(fixed(false), false);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_pinned_replacement_and_cancel_refuse_atomically() {
    pinned_retains_row(allocated(false), false);
}
#[cfg(feature = "oscore")]
#[test]
fn protected_pinned_replacement_and_cancel_refuse_atomically() {
    pinned_retains_row(fixed(true), true);
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_pinned_replacement_and_cancel_refuse_atomically() {
    pinned_retains_row(allocated(true), true);
}

fn pinned_delayed_reset<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut client = client(protected);
    no_alloc(|| {
        let (_, key) = registered(&mut server, &mut client);
        let first = pending(&server, key);
        VERSION.with(|v| v.set(2));
        server.signal(&["value"]);
        let due = first.rto().next_timeout_ms();
        server.poll(due).unwrap();
        let current = pending(&server, key);
        let row_id = server.engine().lookup_observe(key).unwrap();
        let row = server.engine().observe_interest(row_id).unwrap();
        assert_eq!(row.superseded_con_mids(), &[first.message_id()]);
        let bytes = server.transport().last;
        core::mem::forget(server.engine_mut().access_tx(current.tx_slot()).unwrap());
        control(&mut server, first.message_id(), true, OTHER, due + 1);
        assert_eq!(server.engine().observe_interest(row_id), Some(row));
        assert_eq!(pending(&server, key), current);
        server.transport_mut().incoming = Some(control_frame(first.message_id(), true, OWNER));
        assert_eq!(
            server.poll(due + 2),
            Err(coaptic::Error::Slot(coaptic::storage::SlotError::Pinned))
        );
        assert_eq!(server.engine().observe_interest(row_id), Some(row));
        assert_eq!(pending(&server, key), current);
        assert_eq!(
            coaptic::storage::DatagramSlots::tx_payload(
                server.engine().storage(),
                current.tx_slot()
            ),
            Some(&bytes.bytes[..bytes.len])
        );
        let mut slots = [None; 4];
        for slot in &mut slots {
            *slot = Some(server.engine_mut().acquire_rx().unwrap());
        }
        assert!(server.engine_mut().acquire_rx().is_none());
        for slot in slots.into_iter().flatten() {
            server.engine_mut().release_rx(slot).unwrap();
        }
    });
}
#[test]
fn fixed_pinned_delayed_reset_preserves_chain_and_releases_rx() {
    pinned_delayed_reset(fixed(false), false);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_pinned_delayed_reset_preserves_chain_and_releases_rx() {
    pinned_delayed_reset(allocated(false), false);
}
#[cfg(feature = "oscore")]
#[test]
fn protected_pinned_delayed_reset_preserves_chain_and_releases_rx() {
    pinned_delayed_reset(fixed(true), true);
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_pinned_delayed_reset_preserves_chain_and_releases_rx() {
    pinned_delayed_reset(allocated(true), true);
}
