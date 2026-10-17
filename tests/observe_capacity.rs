//! Shared Observe resources: exact registration/refusal and lifecycle recovery.
use coaptic::app::AppStorage;
use coaptic::message::{MessageId, Token, Type, decode};
use coaptic::storage::{DatagramIo, ObserveInterest, ObserveKey};
use coaptic::{App, Code, Endpoint, Response, get, resources};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static IO_CALLS: Cell<usize> = const { Cell::new(0) };
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
coaptic::memory_profile! {
    Pools {
        rx_datagram: (2, 512), tx_datagram: (32, 512),
        rx_body: (1, 1024), tx_body: (1, 1024),
        dedup: 32, observe: 4,
    }
}
const BODY: &[u8] = b"complete observable representation";
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 512],
    len: usize,
    peer: Endpoint,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
            peer: peer(1),
        }
    }
}
#[derive(Default)]
struct Io {
    incoming: Option<Frame>,
    last: Frame,
    sends: usize,
    notifications: [Frame; 8],
    notification_count: usize,
}
impl DatagramIo for Io {
    type Error = core::convert::Infallible;
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        IO_CALLS.with(|c| c.set(c.get() + 1));
        Ok(self.incoming.take().map(|f| {
            out[..f.len].copy_from_slice(&f.bytes[..f.len]);
            (f.len, f.peer)
        }))
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        IO_CALLS.with(|c| c.set(c.get() + 1));
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.last.peer = peer;
        self.sends += 1;
        let message = decode(bytes).unwrap();
        if message.ty() == Type::Confirmable && message.code() == Code::CONTENT {
            assert_eq!(message.payload(), BODY);
            if self.notification_count < self.notifications.len() {
                self.notifications[self.notification_count] = self.last;
            }
            self.notification_count += 1;
        }
        Ok(bytes.len())
    }
}
fn peer(id: u8) -> Endpoint {
    Endpoint::v4([192, 0, 2, id], 5683)
}
fn key(id: u8) -> ObserveKey {
    ObserveKey::new(Token::new(&[id]).unwrap(), peer(id))
}
type Server<const N: usize, S> =
    App<coaptic::profiles::WithObserve<Pools, N>, Io, 8, false, S, 0, 0>;
fn request<const N: usize, S: AppStorage>(
    app: &mut Server<N, S>,
    id: u8,
    mid: u16,
    cancel: bool,
    now: u64,
) -> Frame {
    let mut f = Frame {
        peer: peer(id),
        ..Frame::default()
    };
    f.bytes[..5].copy_from_slice(&[0x41, 1, (mid >> 8) as u8, mid as u8, id]);
    let start = if cancel {
        f.bytes[5..7].copy_from_slice(&[0x61, 1]);
        7
    } else {
        f.bytes[5] = 0x60;
        6
    };
    f.bytes[start..start + 6].copy_from_slice(b"\x55value");
    f.len = start + 6;
    app.transport_mut().incoming = Some(f);
    let sends = app.transport().sends;
    app.poll(now).unwrap();
    assert_eq!(app.transport().sends, sends + 1);
    let reply = app.transport().last;
    let p = decode(&reply.bytes[..reply.len]).unwrap();
    assert_eq!(reply.peer, peer(id));
    assert_eq!(p.ty(), Type::Acknowledgement);
    assert_eq!(p.message_id(), MessageId::new(mid));
    assert_eq!(p.token(), key(id).token());
    assert_eq!(p.code(), Code::CONTENT);
    assert_eq!(p.payload(), BODY);
    reply
}
fn admitted(f: Frame) -> bool {
    decode(&f.bytes[..f.len]).unwrap().observe().is_some()
}
fn lifecycle<const N: usize, S: AppStorage>(mut app: Server<N, S>, count: usize) {
    no_alloc(|| {
        assert_eq!(app.client_capacity(), 0);
        assert_eq!(app.engine().capacities().observe_entries, count);
        for id in 1..=count as u8 {
            assert!(admitted(request(&mut app, id, id as u16, false, 0)));
        }
        assert!(!admitted(request(&mut app, 20, 20, false, 1)));
        for id in 1..=count as u8 {
            assert!(app.engine().lookup_observe(key(id)).is_some());
        }
        if count == 0 {
            return;
        }
        // Max-Age expiration marks representation freshness, not cancellation.
        for _ in 0..count {
            app.poll(2_000).unwrap();
        }
        for id in 1..=count as u8 {
            assert!(app.engine().lookup_observe(key(id)).is_some());
        }
        assert!(!admitted(request(&mut app, 20, 21, false, 2_001)));
        assert!(!admitted(request(&mut app, 1, 22, true, 2_002)));
        assert!(app.engine().lookup_observe(key(1)).is_none());
        // Client-role rows consume the same physical table; no reserved server area.
        let client = ObserveInterest::new_client(key(30).token(), peer(30));
        assert!(app.engine_mut().insert_observe(client).is_some());
        assert!(!admitted(request(&mut app, 20, 23, false, 2_003)));
        assert!(app.engine_mut().take_observe(client.key()).is_some());
        assert!(admitted(request(&mut app, 20, 24, false, 2_004)));
        // All owners receive a complete CON; each has a distinct endpoint (NSTART=1).
        assert_eq!(
            app.notify_confirmable(2_005, &["value"], Response::content(BODY).max_age(1))
                .unwrap(),
            count
        );
        let notify = app.transport().last;
        assert_eq!(app.transport().notification_count, count);
        let mut owners = [None; 6];
        for (index, frame) in app.transport().notifications[..count].iter().enumerate() {
            let message = decode(&frame.bytes[..frame.len]).unwrap();
            assert!(message.observe().is_some());
            let owner = ObserveKey::new(message.token(), frame.peer);
            assert!(app.engine().lookup_observe(owner).is_some());
            assert!(!owners[..index].contains(&Some(owner)));
            owners[index] = Some(owner);
        }
        let p = decode(&notify.bytes[..notify.len]).unwrap();
        assert_eq!(p.ty(), Type::Confirmable);
        assert_eq!(p.payload(), BODY);
        let notified_key = ObserveKey::new(p.token(), notify.peer);
        let mid = p.message_id().get();
        let mut rst = Frame {
            peer: peer(99),
            len: 4,
            ..Frame::default()
        };
        rst.bytes[..4].copy_from_slice(&[0x70, 0, (mid >> 8) as u8, mid as u8]);
        app.transport_mut().incoming = Some(rst);
        app.poll(2_006).unwrap();
        assert!(app.engine().lookup_observe(notified_key).is_some());
        rst.peer = notify.peer;
        app.transport_mut().incoming = Some(rst);
        app.poll(2_007).unwrap();
        assert!(app.engine().lookup_observe(notified_key).is_none());
        assert!(admitted(request(&mut app, 40, 25, false, 2_008)));
        // Reclaimed row can notify too. Retained other-owner CONs remain pending.
        assert_eq!(
            app.notify_confirmable(2_009, &["value"], Response::content(BODY).max_age(1))
                .unwrap(),
            1
        );
        // Poll each finite retry round: a late poll schedules the next retry
        // from that poll's time rather than inventing missed transmissions.
        for round in 1..=5 {
            for _ in 0..count + 2 {
                app.poll(round * 1_000_000).unwrap();
            }
        }
        for id in [2, 3, 4, 5, 6, 20, 40] {
            assert!(app.engine().lookup_observe(key(id)).is_none());
        }
        assert!(admitted(request(&mut app, 50, 26, false, 5_000_001)));
    });
}
#[test]
fn fixed_shared_observe_lifecycle() {
    macro_rules! run {
        ($n:expr) => {{
            let app = no_alloc(|| {
                App::with_resources(
                    resources::Resources::<Pools>::new()
                        .observe_entries::<$n>()
                        .client_calls::<0>(),
                )
                .allow_unprotected()
                .deterministic_for_tests()
                .route(
                    "value",
                    get(|_| Response::content(BODY).observe(0).max_age(1)),
                )
                .bind(Io::default())
                .unwrap()
            });
            lifecycle(app, $n);
        }};
    }
    run!(0);
    run!(1);
    run!(6);
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_shared_observe_lifecycle_and_budget() {
    for count in [0, 1, 6] {
        let config = || {
            resources::Resources::<Pools>::new()
                .observe_entries::<4>()
                .client_calls::<0>()
                .allocated(usize::MAX)
                .observe_entries(count)
        };
        let required = config().required_heap_bytes().unwrap();
        let make = |limit| {
            resources::Resources::<Pools>::new()
                .observe_entries::<4>()
                .client_calls::<0>()
                .allocated(limit)
                .observe_entries(count)
        };
        let before = IO_CALLS.with(Cell::get);
        let err = no_alloc(|| {
            App::with_resources(make(required - 1))
                .allow_unprotected()
                .deterministic_for_tests()
                .bind(Io::default())
                .err()
                .unwrap()
        });
        assert_eq!(IO_CALLS.with(Cell::get), before);
        assert_eq!(
            err,
            coaptic::BuildError::HeapBudgetExceeded {
                required,
                limit: required - 1
            }
        );
        let app = App::with_resources(make(required))
            .allow_unprotected()
            .deterministic_for_tests()
            .route(
                "value",
                get(|_| Response::content(BODY).observe(0).max_age(1)),
            )
            .bind(Io::default())
            .unwrap();
        lifecycle(app, count);
    }
}
