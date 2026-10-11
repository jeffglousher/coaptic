//! Queued receive dispatch under a bounded stream of fresh ingress.
use coaptic::app::{AppStorage, AppStore};
use coaptic::message::{Message, MessageId, Opt, Token, Type, encode};
use coaptic::storage::DatagramIo;
use coaptic::{App, Endpoint, Response, get, profiles, resources};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Tracking;
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1))
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1))
            }
        });
        unsafe { System.realloc(ptr, layout, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Tracking = Tracking;
fn no_alloc(f: impl FnOnce()) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCS.with(|c| c.set(None));
        }
    }
    ALLOCS.with(|c| c.set(Some(0)));
    let reset = Reset;
    f();
    let n = ALLOCS.with(|c| c.get().unwrap());
    drop(reset);
    assert_eq!(n, 0);
}
const OWNER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const FRESH: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
thread_local! {static OLD_CALLS: Cell<usize> = const {Cell::new(0)}; static FRESH_CALLS: Cell<usize> = const {Cell::new(0)};}
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
    fail_send: bool,
    sends: usize,
}
impl DatagramIo for Io {
    type Error = ();
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        Ok(self.incoming.take().map(|f| {
            out[..f.len].copy_from_slice(&f.bytes[..f.len]);
            (f.len, f.peer)
        }))
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        if self.fail_send {
            self.fail_send = false;
            return Err(());
        }
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.last.peer = peer;
        self.sends += 1;
        Ok(bytes.len())
    }
}
type Service<S> = App<profiles::Default, Io, 8, false, S>;
fn packet(mid: u16, path: &str, peer: Endpoint) -> Frame {
    let opts = [Opt::uri_path(path)];
    let mut f = Frame {
        peer,
        ..Frame::default()
    };
    f.len = encode(
        &Message::new(Type::Confirmable, coaptic::Code::GET, MessageId::new(mid))
            .with_token(Token::new(&mid.to_be_bytes()).unwrap())
            .with_options(&opts),
        &mut f.bytes,
    )
    .unwrap();
    f
}
fn fixed() -> Service<AppStore<profiles::Default, false>> {
    App::with_resources(resources::Resources::<profiles::Default>::new())
        .allow_plaintext()
        .deterministic_for_tests()
        .route(
            "old",
            get(|_| {
                OLD_CALLS.with(|v| v.set(v.get() + 1));
                Response::content(b"complete old response")
            }),
        )
        .route(
            "fresh",
            get(|_| {
                FRESH_CALLS.with(|v| v.set(v.get() + 1));
                Response::content(b"complete fresh response")
            }),
        )
        .bind(Io::default())
        .unwrap()
}
fn queued<S: AppStorage>(mut app: Service<S>) {
    OLD_CALLS.with(|v| v.set(0));
    FRESH_CALLS.with(|v| v.set(0));
    no_alloc(|| {
        let tx = app.engine_mut().acquire_tx().unwrap();
        app.engine_mut()
            .write_tx(tx, b"\x40\x01\x00\x01", FRESH)
            .unwrap();
        app.engine_mut()
            .record_pending_con(tx, FRESH, MessageId::new(1), 0, 0)
            .unwrap();
        app.transport_mut().incoming = Some(packet(100, "old", OWNER));
        app.transport_mut().fail_send = true;
        assert!(app.poll(2_000).is_err());
        assert_eq!(OLD_CALLS.with(Cell::get), 0);
        let bound = app.engine().capacities().rx_datagram_slots;
        for n in 0..bound {
            app.transport_mut().incoming = Some(packet(200 + n as u16, "fresh", FRESH));
            app.poll(2_001 + n as u64).unwrap();
        }
        assert_eq!(
            OLD_CALLS.with(Cell::get),
            1,
            "queued owner must progress despite new ingress"
        );
    });
}
#[test]
fn fixed_queued_owner_progresses() {
    queued(fixed());
}

fn held_and_pinned<S: AppStorage>(mut app: Service<S>) {
    OLD_CALLS.with(|v| v.set(0));
    FRESH_CALLS.with(|v| v.set(0));
    no_alloc(|| {
        let call = app.get("remote").to(FRESH).send(0).unwrap();
        let sent = app.transport_mut().last;
        let request = coaptic::message::decode(&sent.bytes[..sent.len]).unwrap();
        let mut response = Frame {
            peer: FRESH,
            ..Frame::default()
        };
        response.len = encode(
            &Message::new(
                Type::Acknowledgement,
                coaptic::Code::CONTENT,
                request.message_id(),
            )
            .with_token(request.token())
            .with_payload(&[0x5a; 256]),
            &mut response.bytes,
        )
        .unwrap();
        app.transport_mut().incoming = Some(response);
        app.poll(1).unwrap();
        assert_eq!(app.engine_mut().rx_occupied(), 1);
        // One unrelated due send fails after reception, retaining request1.
        let tx = app.engine_mut().acquire_tx().unwrap();
        app.engine_mut()
            .write_tx(tx, b"\x40\x01\x00\x01", FRESH)
            .unwrap();
        app.engine_mut()
            .record_pending_con(tx, FRESH, MessageId::new(1), 0, 0)
            .unwrap();
        app.transport_mut().incoming = Some(packet(100, "old", OWNER));
        app.transport_mut().fail_send = true;
        assert!(app.poll(2_000).is_err());
        let pinned = app.engine_mut().acquire_rx().unwrap();
        assert_eq!(pinned.index(), 2);
        let poison = packet(999, "fresh", FRESH);
        app.engine_mut()
            .write_rx(pinned, &poison.bytes[..poison.len], FRESH)
            .unwrap();
        core::mem::forget(app.engine_mut().access_rx(pinned).unwrap());
        // Only slot3 is free: receiving it returns the shared scan cursor to
        // held slot0 every time. Filtering after selection alone starves slot1.
        for n in 0..4 {
            app.transport_mut().incoming = Some(packet(200 + n, "fresh", FRESH));
            app.poll(2_001 + u64::from(n)).unwrap();
        }
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        assert!(app.engine_mut().rx_is_pinned(pinned));
        assert_eq!(FRESH_CALLS.with(Cell::get), 3);
        let mut short = [0x33; 255];
        assert!(app.take_response_into(call, &mut short).is_err());
        assert_eq!(short, [0x33; 255]);
        let mut body = [0; 256];
        let reply = app
            .take_response_into(call, &mut body)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply.payload(), &[0x5a; 256]);
        assert!(app.take_response_into(call, &mut body).unwrap().is_none());
        app.poll(2_006).unwrap();
        assert_eq!(FRESH_CALLS.with(Cell::get), 4);
        assert_eq!(
            app.engine_mut().rx_occupied(),
            1,
            "only pinned bytes remain"
        );
    });
}
#[test]
fn fixed_held_response_and_pinned_slot_do_not_hide_queued_owner() {
    held_and_pinned(fixed());
}

#[cfg(feature = "alloc")]
fn allocated() -> Service<coaptic::storage::AllocMemory> {
    App::with_resources(resources::Resources::<profiles::Default>::new().allocated(usize::MAX))
        .allow_plaintext()
        .deterministic_for_tests()
        .route(
            "old",
            get(|_| {
                OLD_CALLS.with(|v| v.set(v.get() + 1));
                Response::content(b"complete old response")
            }),
        )
        .route(
            "fresh",
            get(|_| {
                FRESH_CALLS.with(|v| v.set(v.get() + 1));
                Response::content(b"complete fresh response")
            }),
        )
        .bind(Io::default())
        .unwrap()
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_queued_owner_progresses() {
    queued(allocated());
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_held_response_and_pinned_slot_do_not_hide_queued_owner() {
    held_and_pinned(allocated());
}

fn duplicate<S: AppStorage>(mut app: Service<S>) {
    OLD_CALLS.with(|v| v.set(0));
    no_alloc(|| {
        let tx = app.engine_mut().acquire_tx().unwrap();
        app.engine_mut()
            .write_tx(tx, b"\x40\x01\x00\x01", FRESH)
            .unwrap();
        app.engine_mut()
            .record_pending_con(tx, FRESH, MessageId::new(1), 0, 0)
            .unwrap();
        let old = packet(100, "old", OWNER);
        app.transport_mut().incoming = Some(old);
        app.transport_mut().fail_send = true;
        assert!(app.poll(2_000).is_err());
        app.transport_mut().incoming = Some(old);
        app.poll(2_001).unwrap();
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        let reply = app.transport_mut().last;
        let parsed = coaptic::message::decode(&reply.bytes[..reply.len]).unwrap();
        assert_eq!(parsed.message_id(), MessageId::new(100));
        assert_eq!(parsed.payload(), b"complete old response");
        assert_eq!(reply.peer, OWNER);
        for now in 2_002..2_008 {
            app.transport_mut().incoming = Some(old);
            app.poll(now).unwrap();
            assert_eq!(OLD_CALLS.with(Cell::get), 1);
            let cached = app.transport_mut().last;
            assert_eq!(&cached.bytes[..cached.len], &reply.bytes[..reply.len]);
            assert_eq!(cached.peer, OWNER);
        }
        app.poll(2_008).unwrap();
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        assert_eq!(app.engine_mut().rx_occupied(), 0);
    });
}
#[test]
fn fixed_queued_and_fresh_duplicates_dispatch_once() {
    duplicate(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_queued_and_fresh_duplicates_dispatch_once() {
    duplicate(allocated());
}

#[cfg(feature = "oscore")]
fn context(sender: &[u8], recipient: &[u8]) -> coaptic::oscore::SecurityContext {
    coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x28; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
#[cfg(feature = "oscore")]
fn protected<S: AppStorage>(mut app: Service<S>) {
    OLD_CALLS.with(|v| v.set(0));
    FRESH_CALLS.with(|v| v.set(0));
    app.set_oscore(context(&[2], &[1]));
    let mut peer = context(&[1], &[2]);
    no_alloc(|| {
        let plain = packet(100, "old", OWNER);
        let parsed = coaptic::message::decode(&plain.bytes[..plain.len]).unwrap();
        let options = [Opt::uri_path("old")];
        let message = Message::new(Type::Confirmable, coaptic::Code::GET, parsed.message_id())
            .with_token(parsed.token())
            .with_options(&options);
        let mut old = Frame::default();
        old.len = peer.protect_request(&message, &mut old.bytes).unwrap();
        let mut verifier = context(&[2], &[1]);
        let mut scratch = [0; 1472];
        let (_, reference) = verifier
            .unprotect_request(
                &coaptic::message::decode(&old.bytes[..old.len]).unwrap(),
                &mut scratch,
            )
            .unwrap();
        let tx = app.engine_mut().acquire_tx().unwrap();
        app.engine_mut()
            .write_tx(tx, b"\x40\x01\x00\x01", FRESH)
            .unwrap();
        app.engine_mut()
            .record_pending_con(tx, FRESH, MessageId::new(1), 0, 0)
            .unwrap();
        app.transport_mut().incoming = Some(old);
        app.transport_mut().fail_send = true;
        assert!(app.poll(2_000).is_err());
        let mut bad = Frame {
            peer: FRESH,
            ..Frame::default()
        };
        bad.bytes[0] = 0xff;
        bad.len = 1;
        app.transport_mut().incoming = Some(bad);
        app.poll(2_001).unwrap();
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        let response = app.transport_mut().last;
        assert_eq!(response.peer, OWNER);
        let outer = coaptic::message::decode(&response.bytes[..response.len]).unwrap();
        assert_eq!(outer.message_id(), MessageId::new(100));
        assert_eq!(outer.token(), parsed.token());
        let inner = peer
            .unprotect_response(&outer, reference, &mut scratch)
            .unwrap();
        assert_eq!(inner.payload(), b"complete old response");
        assert_eq!(inner.code(), coaptic::Code::CONTENT);
        for now in 2_002..2_007 {
            app.transport_mut().incoming = Some(bad);
            app.poll(now).unwrap();
        }
        app.poll(2_007).unwrap();
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        assert_eq!(FRESH_CALLS.with(Cell::get), 0);
        assert_eq!(app.engine_mut().rx_occupied(), 0);
    });
}
#[cfg(feature = "oscore")]
#[test]
fn fixed_protected_queued_response_is_complete() {
    protected(fixed());
}
#[cfg(all(feature = "alloc", feature = "oscore"))]
#[test]
fn allocated_protected_queued_response_is_complete() {
    protected(allocated());
}

fn saturated<S: AppStorage>(mut app: Service<S>) {
    OLD_CALLS.with(|v| v.set(0));
    FRESH_CALLS.with(|v| v.set(0));
    no_alloc(|| {
        for n in 0..4 {
            let rx = app.engine_mut().acquire_rx().unwrap();
            let frame = packet(100 + n, "old", OWNER);
            app.engine_mut()
                .write_rx(rx, &frame.bytes[..frame.len], OWNER)
                .unwrap();
        }
        let fresh = packet(200, "fresh", FRESH);
        app.transport_mut().incoming = Some(fresh);
        assert!(matches!(app.poll(1), Err(coaptic::app::Error::Saturated)));
        assert!(
            app.transport_mut().incoming.is_some(),
            "saturation must leave transport unread"
        );
        assert_eq!(OLD_CALLS.with(Cell::get), 1);
        for now in 2..=5 {
            app.poll(now).unwrap();
        }
        assert_eq!(OLD_CALLS.with(Cell::get), 4);
        assert_eq!(FRESH_CALLS.with(Cell::get), 1);
        assert_eq!(app.engine_mut().rx_occupied(), 0);
    });
}
#[test]
fn fixed_saturation_still_dispatches_then_recovers() {
    saturated(fixed());
}
#[cfg(feature = "alloc")]
#[test]
fn allocated_saturation_still_dispatches_then_recovers() {
    saturated(allocated());
}
