//! Independently selected duplicate-reply history preserves bounded App policy.
use coaptic::app::AppStorage;
use coaptic::message::{Message, MessageId, Opt, Token, Transmission, Type, decode, encode};
use coaptic::resources::Resources;
use coaptic::storage::{DatagramIo, DedupEntry, DedupKey};
use coaptic::{App, Code, Endpoint, Response, post};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static IO_CALLS: Cell<usize> = const { Cell::new(0) };
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

coaptic::memory_profile! {
    Pools {
        rx_datagram: (8, 512), tx_datagram: (8, 512),
        rx_body: (1, 1024), tx_body: (1, 1024),
        dedup: 4, observe: 8,
    }
}
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
    incoming: Option<Frame>,
    last: Frame,
    sends: usize,
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
impl DatagramIo for Io {
    type Error = core::convert::Infallible;
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        IO_CALLS.with(|c| c.set(c.get() + 1));
        let Some(frame) = self.incoming.take() else {
            return Ok(None);
        };
        out[..frame.len].copy_from_slice(&frame.bytes[..frame.len]);
        Ok(Some((frame.len, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        IO_CALLS.with(|c| c.set(c.get() + 1));
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.sends += 1;
        Ok(bytes.len())
    }
}

type Server<const N: usize, S> = App<coaptic::profiles::WithDedup<Pools, N>, Io, 8, false, S>;
fn effect(_: coaptic::Request<'_>) -> Response<'static> {
    EFFECTS.with(|c| c.set(c.get() + 1));
    Response::changed()
}
fn wire(mid: u16) -> Frame {
    let mut frame = Frame::default();
    frame.len = encode(
        &Message::new(Type::Confirmable, Code::POST, MessageId::new(mid))
            .with_token(Token::new(&[1]).unwrap())
            .with_options(&[Opt::uri_path("value")]),
        &mut frame.bytes,
    )
    .unwrap();
    frame
}
fn deliver<const N: usize, S: AppStorage>(app: &mut Server<N, S>, frame: Frame, now: u64) -> Frame {
    let sends = app.transport().sends;
    app.transport_mut().incoming = Some(frame);
    app.poll(now).unwrap();
    assert_eq!(app.transport().sends, sends + 1);
    app.transport().last
}
fn key(mid: u16) -> DedupKey {
    DedupKey::new(MessageId::new(mid), PEER)
}
fn lifecycle<const N: usize, S: AppStorage>(mut app: Server<N, S>, count: usize) {
    EFFECTS.with(|c| c.set(0));
    without_allocating(|| {
        assert_eq!(app.engine().capacities().dedup_entries, count);
        let first = deliver(&mut app, wire(100), 0);
        assert_eq!(
            decode(&first.bytes[..first.len]).unwrap().code(),
            Code::CHANGED
        );
        let replay = deliver(&mut app, wire(100), 1);
        if count == 0 {
            assert!(decode(&replay.bytes[..replay.len]).unwrap().is_empty_ack());
        } else {
            assert_eq!(&first.bytes[..first.len], &replay.bytes[..replay.len]);
        }
        assert_eq!(EFFECTS.with(Cell::get), 1);
        for mid in 101..100 + count as u16 {
            deliver(&mut app, wire(mid), u64::from(mid));
        }
        if count > 0 {
            assert!(app.engine().lookup_dedup(key(100)).is_some());
            for mid in 100..100 + count as u16 {
                deliver(&mut app, wire(mid), 200);
            }
            assert_eq!(EFFECTS.with(Cell::get), count);
            // Existing oldest-row eviction is preserved, not expanded exactly-once storage.
            deliver(&mut app, wire(200), 201);
            assert!(app.engine().lookup_dedup(key(100)).is_none());
            assert!(app.engine().lookup_dedup(key(200)).is_some());
            let effects = EFFECTS.with(Cell::get);
            deliver(&mut app, wire(200), 202);
            assert_eq!(EFFECTS.with(Cell::get), effects);
        }
        let now = u64::from(Transmission::EXCHANGE_LIFETIME_MS) + 300;
        app.poll(now).unwrap();
        assert!(app.engine().lookup_dedup(key(100)).is_none());
        assert!(app.engine().lookup_dedup(key(200)).is_none());
        let effects = EFFECTS.with(Cell::get);
        deliver(&mut app, wire(100), now + 1);
        assert_eq!(EFFECTS.with(Cell::get), effects + 1);
        deliver(&mut app, wire(100), now + 2);
        assert_eq!(EFFECTS.with(Cell::get), effects + 1);
    });
}
#[test]
fn fixed_dedup_capacity_replay_eviction_expiry_and_zero_fallback() {
    macro_rules! run {
        ($n:expr) => {{
            let app = without_allocating(|| {
                App::with_resources(Resources::<Pools>::new().dedup_entries::<$n>())
                    .allow_plaintext()
                    .deterministic_for_tests()
                    .route("value", post(effect))
                    .bind(Io::default())
                    .unwrap()
            });
            lifecycle(app, $n);
        }};
    }
    run!(0);
    run!(1);
    run!(12);
}
#[test]
fn dedup_wrapper_preserves_other_dimensions_and_composes() {
    let before = Resources::<Pools>::new().pool_capacities();
    let mut after = Resources::<Pools>::new()
        .dedup_entries::<12>()
        .pool_capacities();
    assert_eq!(after.dedup_entries, 12);
    after.dedup_entries = before.dedup_entries;
    assert_eq!(after, before);
    let c = Resources::<Pools>::new()
        .dedup_entries::<12>()
        .observe_entries::<6>()
        .client_calls::<2>()
        .security_requests::<3>()
        .routes::<1>()
        .deferred::<1>();
    assert_eq!(c.pool_capacities().dedup_entries, 12);
    assert_eq!(c.pool_capacities().observe_entries, 6);
    assert_eq!(c.client_capacity(), 2);
    assert_eq!(c.security_request_capacity(), 3);
    assert_eq!(c.route_capacity(), 1);
    assert_eq!(c.deferred_capacity(), 1);
    assert_eq!(
        Resources::<Pools>::new()
            .observe_entries::<6>()
            .dedup_entries::<12>()
            .pool_capacities(),
        c.pool_capacities()
    );
}
#[test]
fn nonexpiring_row_refuses_cache_replacement_without_second_effect_then_recovers() {
    let mut app = App::with_resources(Resources::<Pools>::new().dedup_entries::<1>())
        .allow_plaintext()
        .deterministic_for_tests()
        .route("value", post(effect))
        .bind(Io::default())
        .unwrap();
    EFFECTS.with(|c| c.set(0));
    without_allocating(|| {
        app.engine_mut()
            .insert_dedup(DedupEntry::new(MessageId::new(99), PEER))
            .unwrap();
        deliver(&mut app, wire(100), 0);
        let reply = deliver(&mut app, wire(100), 1);
        assert!(decode(&reply.bytes[..reply.len]).unwrap().is_empty_ack());
        assert_eq!(EFFECTS.with(Cell::get), 1);
        assert!(app.engine().lookup_dedup(key(99)).is_some());
        assert!(app.engine().lookup_dedup(key(100)).is_none());
        app.engine_mut().remove_dedup(key(99));
        let fresh = deliver(&mut app, wire(101), 2);
        assert_eq!(
            decode(&fresh.bytes[..fresh.len]).unwrap().code(),
            Code::CHANGED
        );
        deliver(&mut app, wire(101), 3);
        assert_eq!(EFFECTS.with(Cell::get), 2);
    });
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_dedup_capacity_exact_budget_refusal_and_recovery() {
    for count in [0, 1, 12] {
        let config = |limit| {
            Resources::<Pools>::new()
                .dedup_entries::<4>()
                .allocated(limit)
                .dedup_entries(count)
        };
        let required = config(usize::MAX).required_heap_bytes().unwrap();
        let before = IO_CALLS.with(Cell::get);
        let error = without_allocating(|| {
            App::with_resources(config(required - 1))
                .allow_plaintext()
                .deterministic_for_tests()
                .bind(Io::default())
                .err()
                .unwrap()
        });
        assert_eq!(
            error,
            coaptic::BuildError::HeapBudgetExceeded {
                required,
                limit: required - 1
            }
        );
        assert_eq!(IO_CALLS.with(Cell::get), before);
        let app = App::with_resources(config(required))
            .allow_plaintext()
            .deterministic_for_tests()
            .route("value", post(effect))
            .bind(Io::default())
            .unwrap();
        lifecycle(app, count);
    }
    let huge = Resources::<Pools>::new()
        .allocated(usize::MAX)
        .dedup_entries(usize::MAX);
    assert_eq!(
        huge.required_heap_bytes(),
        Err(coaptic::BuildError::CapacityOverflow)
    );
    let before = IO_CALLS.with(Cell::get);
    let error = without_allocating(|| {
        App::with_resources(huge)
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Io::default())
            .err()
            .unwrap()
    });
    assert_eq!(error, coaptic::BuildError::CapacityOverflow);
    assert_eq!(IO_CALLS.with(Cell::get), before);
}

#[cfg(feature = "oscore")]
fn context(sender: &[u8], recipient: &[u8]) -> coaptic::oscore::SecurityContext {
    coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x64; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
#[cfg(feature = "oscore")]
fn protected<const N: usize, S: AppStorage>(mut server: Server<N, S>, count: usize) {
    EFFECTS.with(|c| c.set(0));
    let mut client = App::with_resources(Resources::<Pools>::new())
        .oscore(context(&[1], &[2]))
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    without_allocating(|| {
        let rounds = count.max(1);
        let mut first = Frame::default();
        for i in 0..rounds + 1 {
            let now = if i == rounds {
                u64::from(Transmission::EXCHANGE_LIFETIME_MS) + 100
            } else {
                i as u64 * 10
            };
            if i == rounds {
                server.poll(now).unwrap();
                // Expired response cache does not relax OSCORE replay protection.
                server.transport_mut().incoming = Some(first);
                server.poll(now).unwrap();
                assert_eq!(EFFECTS.with(Cell::get), rounds);
            }
            let call = client.post("value").to(PEER).send(now).unwrap();
            let request = client.transport().last;
            if i == 0 {
                first = request;
            }
            let response = deliver(&mut server, request, now);
            assert!(
                decode(&response.bytes[..response.len])
                    .unwrap()
                    .oscore()
                    .is_some()
            );
            let retry = deliver(&mut server, request, now + 1);
            if count == 0 {
                assert!(decode(&retry.bytes[..retry.len]).unwrap().is_empty_ack());
            } else {
                assert_eq!(&retry.bytes[..retry.len], &response.bytes[..response.len]);
            }
            assert_eq!(EFFECTS.with(Cell::get), i + 1);
            client.transport_mut().incoming = Some(response);
            client.poll(now + 2).unwrap();
            let mut body = [0; 32];
            let reply = client
                .take_response_into(call, &mut body)
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(reply.code(), Code::CHANGED);
        }
    });
}
#[cfg(feature = "oscore")]
#[test]
fn protected_fixed_dedup_replay_and_expiry_preserve_authentication() {
    macro_rules! run {
        ($n:expr) => {{
            let server = App::with_resources(Resources::<Pools>::new().dedup_entries::<$n>())
                .oscore(context(&[2], &[1]))
                .deterministic_for_tests()
                .route("value", post(effect))
                .bind(Io::default())
                .unwrap();
            protected(server, $n);
        }};
    }
    run!(0);
    run!(1);
    run!(12);
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn protected_allocated_dedup_exact_budget_has_same_behavior() {
    for count in [0, 1, 12] {
        let config = |limit| {
            Resources::<Pools>::new()
                .dedup_entries::<4>()
                .allocated(limit)
                .dedup_entries(count)
        };
        let required = config(usize::MAX).required_heap_bytes().unwrap();
        let server = App::with_resources(config(required))
            .oscore(context(&[2], &[1]))
            .deterministic_for_tests()
            .route("value", post(effect))
            .bind(Io::default())
            .unwrap();
        protected(server, count);
    }
}
#[test]
fn dedup_metadata_is_exact_and_other_storage_is_unchanged() {
    use coaptic::storage::{DedupTable, MemoryProfile};
    use core::mem::size_of;
    type Zero = coaptic::profiles::WithDedup<Pools, 0>;
    type One = coaptic::profiles::WithDedup<Pools, 1>;
    type Twelve = coaptic::profiles::WithDedup<Pools, 12>;
    assert_eq!(
        size_of::<<Zero as MemoryProfile>::Dedup>(),
        size_of::<DedupTable<0>>()
    );
    assert_eq!(
        size_of::<<One as MemoryProfile>::Dedup>(),
        size_of::<DedupTable<1>>()
    );
    assert_eq!(
        size_of::<<Twelve as MemoryProfile>::Dedup>(),
        size_of::<DedupTable<12>>()
    );
    assert_eq!(
        size_of::<<Twelve as MemoryProfile>::RxScratch>(),
        size_of::<<Pools as MemoryProfile>::RxScratch>()
    );
    assert_eq!(
        size_of::<<Twelve as MemoryProfile>::TxScratch>(),
        size_of::<<Pools as MemoryProfile>::TxScratch>()
    );
    #[cfg(feature = "alloc")]
    {
        let bytes = |n| {
            Resources::<Pools>::new()
                .allocated(usize::MAX)
                .dedup_entries(n)
                .required_heap_bytes()
                .unwrap()
        };
        let per_entry = size_of::<Option<DedupEntry>>() + 2 * size_of::<bool>();
        assert_eq!(bytes(1) - bytes(0), per_entry);
        assert_eq!(bytes(12) - bytes(0), 12 * per_entry);
    }
}

fn large_effect(_: coaptic::Request<'_>) -> Response<'static> {
    EFFECTS.with(|c| c.set(c.get() + 1));
    Response::changed().payload_copy_full(&[0x5a; DedupEntry::REPLAY_MAX + 16])
}
fn tx_pressure<S: AppStorage>(mut server: Server<12, S>) {
    EFFECTS.with(|c| c.set(0));
    without_allocating(|| {
        for mid in 1..=12 {
            let reply = deliver(&mut server, wire(mid), u64::from(mid));
            assert_eq!(
                decode(&reply.bytes[..reply.len]).unwrap().payload(),
                &[0x5a; DedupEntry::REPLAY_MAX + 16]
            );
        }
        assert_eq!(server.engine_mut().tx_occupied(), 8);
        assert!(server.engine().lookup_dedup(key(1)).is_some());
        let first = deliver(&mut server, wire(1), 13);
        assert!(decode(&first.bytes[..first.len]).unwrap().is_empty_ack());
        let last = deliver(&mut server, wire(12), 14);
        assert_eq!(
            decode(&last.bytes[..last.len]).unwrap().payload(),
            &[0x5a; DedupEntry::REPLAY_MAX + 16]
        );
        assert_eq!(EFFECTS.with(Cell::get), 12);
        server
            .poll(u64::from(Transmission::EXCHANGE_LIFETIME_MS) + 20)
            .unwrap();
        assert_eq!(server.engine_mut().tx_occupied(), 0);
    });
}
#[test]
fn more_history_does_not_reserve_more_wire_storage() {
    let config = Resources::<Pools>::new().dedup_entries::<12>();
    let server = App::with_resources(config)
        .allow_plaintext()
        .deterministic_for_tests()
        .route("value", post(large_effect))
        .bind(Io::default())
        .unwrap();
    tx_pressure(server);
    #[cfg(feature = "alloc")]
    {
        let config = Resources::<Pools>::new()
            .dedup_entries::<12>()
            .allocated(usize::MAX);
        let required = config.required_heap_bytes().unwrap();
        let server = App::with_resources(
            Resources::<Pools>::new()
                .dedup_entries::<12>()
                .allocated(required),
        )
        .allow_plaintext()
        .deterministic_for_tests()
        .route("value", post(large_effect))
        .bind(Io::default())
        .unwrap();
        tx_pressure(server);
    }
}
