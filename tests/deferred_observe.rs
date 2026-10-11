//! Deferred initial Observe owns its registration through completion and refusal.
use coaptic::app::AppStorage;
use coaptic::message::ContentFormat;
use coaptic::message::decode;
use coaptic::message::{BlockValue, Message, MessageId, Opt, Token, Type, encode};
use coaptic::resources::Resources;
use coaptic::storage::DatagramIo;
use coaptic::{App, Code, Endpoint, Response};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };

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
    Buffers {
        rx_datagram: (4, 256), tx_datagram: (4, 256),
        rx_body: (2, 1024), tx_body: (2, 1024),
        dedup: 16, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 256],
    len: usize,
    peer: Endpoint,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
            peer: PEER,
        }
    }
}
#[derive(Default)]
struct Io {
    incoming: Option<Frame>,
    output: [Frame; 16],
    len: usize,
    fail_send: bool,
}
impl Io {
    fn take(&mut self) -> Option<Frame> {
        if self.len == 0 {
            return None;
        }
        let frame = self.output[0];
        self.output.copy_within(1..self.len, 0);
        self.len -= 1;
        Some(frame)
    }
}
impl DatagramIo for Io {
    type Error = &'static str;
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some(frame) = self.incoming.take() else {
            return Ok(None);
        };
        out[..frame.len].copy_from_slice(&frame.bytes[..frame.len]);
        Ok(Some((frame.len, frame.peer)))
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        assert!(self.len < self.output.len());
        let frame = &mut self.output[self.len];
        frame.bytes[..bytes.len()].copy_from_slice(bytes);
        frame.len = bytes.len();
        frame.peer = peer;
        self.len += 1;
        if self.fail_send {
            return Err("ambiguous send");
        }
        Ok(bytes.len())
    }
}

type Server<S = coaptic::storage::Memory<Buffers>> = App<Buffers, Io, 1, false, S, 2>;
fn fixed() -> Server {
    fixed_lifetime(247_000)
}
fn fixed_lifetime(lifetime: u64) -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<false>()
            .deferred::<2>(),
    )
    .allow_plaintext()
    .deferred_lifetime(lifetime)
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).observe(source))
    .bind(Io::default())
    .unwrap()
}

struct Peer {
    #[cfg(feature = "oscore")]
    context: Option<coaptic::oscore::SecurityContext>,
}
impl Peer {
    fn new<S: AppStorage>(server: &mut Server<S>, protected: bool) -> Self {
        #[cfg(feature = "oscore")]
        {
            let make = |sender, recipient| {
                coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
                    master_secret: &[0x42; 16],
                    master_salt: &[],
                    sender_id: sender,
                    recipient_id: recipient,
                    id_context: &[],
                })
                .unwrap()
            };
            if protected {
                server.set_oscore(&mut Some(make(&[2], &[1]))).unwrap();
            }
            Self {
                context: protected.then(|| make(&[1], &[2])),
            }
        }
        #[cfg(not(feature = "oscore"))]
        {
            let _ = server;
            assert!(!protected);
            Self {}
        }
    }
    fn encode(&mut self, message: &Message<'_>) -> Frame {
        let mut frame = Frame::default();
        #[cfg(feature = "oscore")]
        if let Some(context) = self.context.as_mut() {
            frame.len = context.protect_request(message, &mut frame.bytes).unwrap();
            return frame;
        }
        frame.len = encode(message, &mut frame.bytes).unwrap();
        frame
    }
    fn open(&self, frame: Frame) -> Frame {
        #[cfg(feature = "oscore")]
        if let Some(context) = self.context.as_ref() {
            let protected = decode(&frame.bytes[..frame.len]).unwrap();
            assert!(protected.oscore().is_some());
            let mut opened = Frame::default();
            let request = context.lookup(protected.token()).unwrap();
            let parsed = context
                .unprotect_response(&protected, request, &mut opened.bytes)
                .unwrap();
            // The parsed inner datagram occupies exactly this bounded prefix.
            let mut opts = coaptic::message::OptionsBuilder::<24>::new();
            for opt in parsed.options() {
                opts.push(Opt::new(opt.number(), opt.value())).unwrap();
            }
            let message = Message::new(parsed.ty(), parsed.code(), parsed.message_id())
                .with_token(parsed.token())
                .with_options(opts.as_slice())
                .with_payload(parsed.payload());
            let mut canonical = Frame::default();
            canonical.len = encode(&message, &mut canonical.bytes).unwrap();
            return canonical;
        }
        frame
    }
}

fn handler(_: coaptic::Request<'_>) -> Response<'static> {
    response(b"sync")
}
fn source() -> Response<'static> {
    response(b"updated")
}
fn response(body: &'static [u8]) -> Response<'static> {
    Response::content(body).content_format(ContentFormat::TEXT_PLAIN)
}
fn request(peer: &mut Peer, mid: u16, ty: Type, observe: u32) -> Frame {
    let value = coaptic::message::encode_uint(observe);
    let options = [Opt::observe(&value), Opt::uri_path("obs")];
    peer.encode(
        &Message::new(ty, Code::GET, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(&options),
    )
}
fn control(mid: MessageId, ty: Type) -> Frame {
    let mut frame = Frame::default();
    frame.len = encode(&Message::new(ty, Code::EMPTY, mid), &mut frame.bytes).unwrap();
    frame
}
fn begin<S: AppStorage>(
    server: &mut Server<S>,
    peer: &mut Peer,
    ty: Type,
    mid: u16,
    now: u64,
) -> coaptic::app::DeferredReply {
    server.transport_mut().incoming = Some(request(peer, mid, ty, 0));
    let mut handle = None;
    without_allocating(|| {
        server
            .poll_with(now, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap()
    });
    if ty == Type::Confirmable {
        let frame = server.transport_mut().take().unwrap();
        let p = decode(&frame.bytes[..frame.len]).unwrap();
        assert!(p.is_empty_ack());
        assert_eq!(p.message_id(), MessageId::new(mid));
    }
    assert!(server.transport_mut().take().is_none());
    handle.expect("bounded Observe handle")
}
fn content<S: AppStorage>(
    server: &mut Server<S>,
    peer: &Peer,
    body: &[u8],
    seq: u32,
    ty: Type,
) -> MessageId {
    let frame = peer.open(server.transport_mut().take().expect("response"));
    let p = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(p.code(), Code::CONTENT);
    assert_eq!(p.ty(), ty);
    assert_eq!(p.token(), Token::new(b"owner").unwrap());
    assert_eq!(p.payload(), body);
    #[cfg(feature = "oscore")]
    let seq = if peer.context.is_some() { 0 } else { seq };
    assert_eq!(p.observe(), Some(Ok(seq)));
    assert_eq!(p.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
    p.message_id()
}
fn cycle<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    for (index, ty) in [Type::Confirmable, Type::NonConfirmable]
        .into_iter()
        .enumerate()
    {
        let start = index as u64 * 10000;
        let handle = begin(&mut server, &mut peer, ty, 1 + index as u16, start);
        without_allocating(|| {
            server
                .complete(handle, response(b"initial"), start + 1)
                .unwrap()
        });
        let mid = content(&mut server, &peer, b"initial", 0, ty);
        assert!(
            server
                .complete(handle, response(b"stale"), start + 2)
                .is_err()
        );
        if ty == Type::Confirmable {
            server.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
            server.poll(start + 2).unwrap();
        }
        assert_eq!(
            without_allocating(|| server
                .notify(start + 3002, &["obs"], response(b"updated"))
                .unwrap()),
            1
        );
        let update = content(&mut server, &peer, b"updated", 1, Type::NonConfirmable);
        server.transport_mut().incoming = Some(control(update, Type::Reset));
        server.poll(start + 3003).unwrap();
        assert_eq!(
            server
                .notify(start + 6000, &["obs"], response(b"updated"))
                .unwrap(),
            0
        );
    }
}
#[test]
fn deferred_observe_fixed_con_non_full_initial_and_updates() {
    cycle(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_observe_protected_full_initial_and_updates() {
    cycle(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_observe_allocated_exact_budget_has_same_lifecycle() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<false>()
            .deferred::<2>()
    };
    let budget = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let app = App::with_resources(resources().allocated(budget))
            .allow_plaintext()
            .deterministic_for_tests()
            .route("obs", coaptic::get(handler).observe(source))
            .bind(Io::default())
            .unwrap();
        cycle(app, protected);
    }
}
#[test]
fn deferred_observe_ambiguous_send_keeps_format_sequence_and_coalesced_signal() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let handle = begin(&mut server, &mut peer, Type::Confirmable, 1, 0);
        server.transport_mut().fail_send = true;
        assert!(without_allocating(|| server.complete(handle, response(b"initial"), 1)).is_err());
        #[cfg(feature = "oscore")]
        let failed_piv_end = server.oscore().map(|context| context.sender_seq());
        let first = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        assert_eq!(server.engine_mut().tx_occupied(), 0);
        server.transport_mut().fail_send = false;
        assert!(matches!(
            server.complete(handle, Response::content(b"bad-format"), 2),
            Err(coaptic::app::Error::Response(
                coaptic::app::ResponseError::DeferredObserveFormat
            ))
        ));
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.signal(&["obs"]), 1);
        assert_eq!(server.notify(3, &["obs"], response(b"updated")).unwrap(), 0);
        server.poll(3).unwrap();
        assert!(server.transport_mut().take().is_none());
        without_allocating(|| server.complete(handle, response(b"retried"), 4).unwrap());
        #[cfg(feature = "oscore")]
        if let Some(before) = failed_piv_end {
            assert_eq!(server.oscore().unwrap().sender_seq(), before + 1);
        }
        assert_eq!(relation(&mut server).unwrap().seq(), 1);
        let second = content(&mut server, &peer, b"retried", 1, Type::Confirmable);
        assert_ne!(first, second);
        server.transport_mut().incoming = Some(control(second, Type::Acknowledgement));
        server.poll(5).unwrap();
        server.poll(6).unwrap();
        content(&mut server, &peer, b"updated", 2, Type::NonConfirmable);
    }
}

fn key() -> coaptic::storage::ObserveKey {
    coaptic::storage::ObserveKey::new(Token::new(b"owner").unwrap(), PEER)
}
fn relation<S: AppStorage>(server: &mut Server<S>) -> Option<coaptic::storage::ObserveInterest> {
    let engine = server.engine_mut();
    engine
        .lookup_observe(key())
        .and_then(|id| engine.observe_interest(id))
}
fn drain<S: AppStorage>(server: &mut Server<S>) {
    while server.transport_mut().take().is_some() {}
}

#[test]
fn deferred_observe_wrong_control_retry_rst_and_giveup_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        for reset in [false, true] {
            let mut server = fixed();
            let mut peer = Peer::new(&mut server, protected);
            let handle = begin(&mut server, &mut peer, Type::Confirmable, 10, 0);
            server.complete(handle, response(b"initial"), 1).unwrap();
            let original = server.transport_mut().take().unwrap();
            let mid = decode(&original.bytes[..original.len])
                .unwrap()
                .message_id();
            for (index, mut bad) in [
                control(mid, Type::Acknowledgement),
                control(
                    MessageId::new(mid.get().wrapping_add(1)),
                    Type::Acknowledgement,
                ),
                control(mid, Type::Reset),
            ]
            .into_iter()
            .enumerate()
            {
                if index != 1 {
                    bad.peer = Endpoint::v4([192, 0, 2, 9], 5683);
                }
                server.transport_mut().incoming = Some(bad);
                server.poll(2 + index as u64).unwrap();
                assert!(server.engine_mut().lookup_pending_con(mid, PEER).is_some());
                assert!(relation(&mut server).is_some());
            }
            if reset {
                server.transport_mut().incoming = Some(control(mid, Type::Reset));
                server.poll(5).unwrap();
                assert!(relation(&mut server).is_none());
            } else {
                for now in [3001, 9001, 21001, 45001] {
                    without_allocating(|| server.poll(now).unwrap());
                    let retry = server.transport_mut().take().unwrap();
                    assert_eq!(&retry.bytes[..retry.len], &original.bytes[..original.len]);
                }
                server.poll(93001).unwrap();
                assert!(relation(&mut server).is_none());
                assert!(server.transport_mut().take().is_none());
            }
            assert_eq!(server.engine_mut().tx_occupied(), 0);
            let fresh = begin(&mut server, &mut peer, Type::Confirmable, 11, 100000);
            server
                .complete(fresh, response(b"initial"), 100001)
                .unwrap();
            content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        }
    }
}

#[test]
fn deferred_observe_cancel_expiry_and_supersession_never_resurrect() {
    for protected in [false, cfg!(feature = "oscore")] {
        for mode in 0..4 {
            let mut server = fixed_lifetime(100);
            let mut peer = Peer::new(&mut server, protected);
            let old = begin(&mut server, &mut peer, Type::Confirmable, 20, 0);
            server.transport_mut().fail_send = true;
            assert!(server.complete(old, response(b"initial"), 1).is_err());
            drain(&mut server);
            server.transport_mut().fail_send = false;
            assert_eq!(server.signal(&["obs"]), 1);
            assert_eq!(server.notify(2, &["obs"], response(b"updated")).unwrap(), 0);
            let now = match mode {
                0 => {
                    assert!(server.cancel_deferred(old));
                    3
                }
                1 => {
                    server.poll(100).unwrap();
                    100
                }
                2 => {
                    let next = begin(&mut server, &mut peer, Type::Confirmable, 21, 3);
                    assert!(!server.cancel_deferred(old));
                    assert!(server.complete(old, response(b"stale"), 4).is_err());
                    server.complete(next, response(b"initial"), 5).unwrap();
                    content(&mut server, &peer, b"initial", 0, Type::Confirmable);
                    assert!(relation(&mut server).is_some());
                    assert!(server.complete(old, response(b"stale"), 6).is_err());
                    assert!(relation(&mut server).is_some());
                    continue;
                }
                _ => {
                    server.transport_mut().incoming =
                        Some(request(&mut peer, 21, Type::Confirmable, 1));
                    server.poll(3).unwrap();
                    drain(&mut server);
                    3
                }
            };
            assert!(relation(&mut server).is_none());
            assert!(!server.cancel_deferred(old));
            assert!(server.complete(old, response(b"stale"), now + 1).is_err());
            server.poll(now + 2).unwrap();
            assert!(server.transport_mut().take().is_none());
            let fresh = begin(&mut server, &mut peer, Type::Confirmable, 22, now + 3);
            server
                .complete(fresh, response(b"initial"), now + 4)
                .unwrap();
            content(&mut server, &peer, b"initial", 0, Type::Confirmable);
            assert!(!relation(&mut server).unwrap().is_pending());
        }
    }
}

#[test]
fn deferred_observe_empty_ack_failure_duplicate_and_cross_app_authority() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, false);
    let frame = request(&mut peer, 30, Type::Confirmable, 0);
    server.transport_mut().incoming = Some(frame);
    server.transport_mut().fail_send = true;
    let mut handle = None;
    let mut calls = 0;
    assert!(
        server
            .poll_with(0, |_, candidate| {
                calls += 1;
                handle = candidate;
                Response::deferred()
            })
            .is_err()
    );
    drain(&mut server);
    server.transport_mut().fail_send = false;
    server.transport_mut().incoming = Some(frame);
    server
        .poll_with(1, |_, _| {
            calls += 1;
            response(b"unexpected")
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert!(
        decode(&server.transport_mut().take().unwrap().bytes[..4])
            .unwrap()
            .is_empty_ack()
    );
    let handle = handle.unwrap();
    let mut foreign = fixed();
    assert!(foreign.complete(handle, response(b"wrong app"), 2).is_err());
    assert!(foreign.transport_mut().take().is_none());
    server.complete(handle, response(b"initial"), 2).unwrap();
    content(&mut server, &peer, b"initial", 0, Type::Confirmable);
}

#[test]
fn deferred_observe_capacity_fallback_and_unsupported_registration_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let occupied = coaptic::storage::ObserveInterest::new(
            Token::new(b"other").unwrap(),
            Endpoint::v4([192, 0, 2, 3], 5683),
        );
        server.engine_mut().insert_observe(occupied).unwrap();
        let handle = begin(&mut server, &mut peer, Type::Confirmable, 40, 0);
        server.complete(handle, response(b"ordinary"), 1).unwrap();
        let frame = peer.open(server.transport_mut().take().unwrap());
        let p = decode(&frame.bytes[..frame.len]).unwrap();
        assert_eq!(p.payload(), b"ordinary");
        assert_eq!(p.observe(), None);
        assert_eq!(p.code(), Code::CONTENT);
        assert!(relation(&mut server).is_none());
        server.transport_mut().incoming = Some(control(p.message_id(), Type::Acknowledgement));
        server.poll(2).unwrap();
        server.engine_mut().take_observe(occupied.key());
        for (index, code, block, q, path) in [
            (0, Code::FETCH, false, false, "obs"),
            (1, Code::GET, true, false, "obs"),
            (2, Code::GET, false, true, "obs"),
            (3, Code::GET, false, false, "missing"),
        ] {
            let b = BlockValue::new(0, false, 0).unwrap().encode();
            let zero = coaptic::message::EncodedUint::new(0);
            let mut opts = coaptic::message::OptionsBuilder::<4>::new();
            opts.push(Opt::observe(&zero)).unwrap();
            opts.push(Opt::uri_path(path)).unwrap();
            let cf = ContentFormat::TEXT_PLAIN.encode();
            if code == Code::FETCH {
                opts.push(Opt::content_format(&cf)).unwrap();
            }
            if block {
                opts.push(Opt::block2(&b)).unwrap();
            }
            if q {
                opts.push(Opt::q_block2(&b)).unwrap();
            }
            server.transport_mut().incoming = Some(
                peer.encode(
                    &Message::new(Type::Confirmable, code, MessageId::new(41 + index))
                        .with_token(Token::new(b"owner").unwrap())
                        .with_options(opts.as_slice()),
                ),
            );
            let mut dispatched = false;
            server
                .poll_with(3 + u64::from(index), |_, candidate| {
                    dispatched = true;
                    assert!(candidate.is_none());
                    Response::deferred()
                })
                .unwrap();
            assert!(dispatched, "case {index}");
            let frame = peer.open(server.transport_mut().take().unwrap());
            let p = decode(&frame.bytes[..frame.len]).unwrap();
            assert_eq!(p.code(), Code::SERVICE_UNAVAILABLE);
            assert!(p.observe().is_none());
            assert!(relation(&mut server).is_none());
        }
        let fresh = begin(&mut server, &mut peer, Type::Confirmable, 50, 10);
        server.complete(fresh, response(b"initial"), 11).unwrap();
        content(&mut server, &peer, b"initial", 0, Type::Confirmable);
    }
}

#[test]
fn deferred_observe_datagram_bounds_refuse_without_registration_then_recover() {
    static LARGE: [u8; 300] = [b'x'; 300];
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let handle = begin(&mut server, &mut peer, Type::Confirmable, 60, 0);
        for now in 1..8 {
            assert!(
                without_allocating(|| server.complete(
                    handle,
                    Response::content(&LARGE).content_format(ContentFormat::TEXT_PLAIN),
                    now
                ))
                .is_err()
            );
            assert!(server.transport_mut().take().is_none());
            assert!(relation(&mut server).is_none());
            assert_eq!(server.engine_mut().tx_occupied(), 0);
        }
        server.complete(handle, response(b"initial"), 8).unwrap();
        content(&mut server, &peer, b"initial", 0, Type::Confirmable);
    }
}

#[test]
fn deferred_observe_unattempted_cancel_and_expiry_release_old_signal() {
    for expiry in [false, true] {
        let mut server = fixed_lifetime(100);
        let mut peer = Peer::new(&mut server, false);
        let first = begin(&mut server, &mut peer, Type::Confirmable, 70, 0);
        server.complete(first, response(b"initial"), 1).unwrap();
        let mid = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        server.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        server.poll(2).unwrap();
        let pending = begin(&mut server, &mut peer, Type::Confirmable, 71, 3);
        assert_eq!(server.signal(&["obs"]), 1);
        server.poll(4).unwrap();
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.notify(4, &["obs"], response(b"updated")).unwrap(), 0);
        if !expiry {
            assert!(server.cancel_deferred(pending));
        }
        server.poll(if expiry { 103 } else { 5 }).unwrap();
        content(&mut server, &peer, b"updated", 1, Type::NonConfirmable);
        assert!(server.complete(pending, response(b"stale"), 104).is_err());
        assert!(relation(&mut server).is_some());
    }
}

#[test]
fn deferred_observe_nstart_refusal_preserves_live_owner_then_ack_recovers() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, false);
    let first = begin(&mut server, &mut peer, Type::Confirmable, 80, 0);
    server.complete(first, response(b"initial"), 1).unwrap();
    let original = server.transport_mut().output[0];
    let mid = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
    let pending = begin(&mut server, &mut peer, Type::Confirmable, 81, 2);
    assert!(server.complete(pending, response(b"next"), 3).is_err());
    assert_eq!(relation(&mut server).unwrap().seq(), 0);
    assert!(server.engine_mut().lookup_pending_con(mid, PEER).is_some());
    assert!(server.transport_mut().take().is_none());
    assert_eq!(server.signal(&["obs"]), 1);
    server.poll(3001).unwrap();
    let retry = server.transport_mut().take().unwrap();
    assert_eq!(&retry.bytes[..retry.len], &original.bytes[..original.len]);
    assert_eq!(relation(&mut server).unwrap().seq(), 0);
    assert!(relation(&mut server).unwrap().is_pending());
    server.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
    server.poll(3002).unwrap();
    server.complete(pending, response(b"next"), 3003).unwrap();
    content(&mut server, &peer, b"next", 1, Type::Confirmable);
}

#[test]
fn deferred_observe_deferred_capacity_and_zero_slots_refuse_without_effects() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, false);
    let mut handles = [None; 2];
    for (index, token) in [b"one".as_slice(), b"two".as_slice(), b"three".as_slice()]
        .into_iter()
        .enumerate()
    {
        let zero = coaptic::message::EncodedUint::new(0);
        let opts = [Opt::observe(&zero), Opt::uri_path("obs")];
        server.transport_mut().incoming = Some(
            peer.encode(
                &Message::new(
                    Type::Confirmable,
                    Code::GET,
                    MessageId::new(90 + index as u16),
                )
                .with_token(Token::new(token).unwrap())
                .with_options(&opts),
            ),
        );
        server
            .poll_with(index as u64, |_, candidate| {
                if index < 2 {
                    handles[index] = candidate;
                    assert!(candidate.is_some());
                } else {
                    assert!(candidate.is_none());
                }
                Response::deferred()
            })
            .unwrap();
        let frame = server.transport_mut().take().unwrap();
        let parsed = decode(&frame.bytes[..frame.len]).unwrap();
        if index < 2 {
            assert!(parsed.is_empty_ack());
        } else {
            assert_eq!(parsed.code(), Code::SERVICE_UNAVAILABLE);
        }
        assert!(relation(&mut server).is_none());
    }
    for handle in handles.into_iter().flatten() {
        assert!(server.cancel_deferred(handle));
    }
    let fresh = begin(&mut server, &mut peer, Type::Confirmable, 94, 4);
    server.complete(fresh, response(b"initial"), 5).unwrap();
    content(&mut server, &peer, b"initial", 0, Type::Confirmable);
    let mut zero = App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<false>()
            .deferred::<0>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).observe(source))
    .bind(Io::default())
    .unwrap();
    zero.transport_mut().incoming = Some(request(&mut peer, 95, Type::Confirmable, 0));
    zero.poll_with(0, |_, candidate| {
        assert!(candidate.is_none());
        Response::deferred()
    })
    .unwrap();
    let frame = zero.transport_mut().take().unwrap();
    assert_eq!(
        decode(&frame.bytes[..frame.len]).unwrap().code(),
        Code::SERVICE_UNAVAILABLE
    );
    assert!(zero.engine_mut().lookup_observe(key()).is_none());
}

#[cfg(feature = "oscore")]
#[test]
fn deferred_observe_protected_actual_size_and_reservation_refusal_recover() {
    static NEAR: [u8; 239] = [b'n'; 239];
    let mut plain = fixed();
    let mut plain_peer = Peer::new(&mut plain, false);
    let plain_handle = begin(&mut plain, &mut plain_peer, Type::Confirmable, 100, 0);
    plain
        .complete(
            plain_handle,
            Response::content(&NEAR).content_format(ContentFormat::TEXT_PLAIN),
            1,
        )
        .unwrap();
    content(&mut plain, &plain_peer, &NEAR, 0, Type::Confirmable);
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, true);
    let handle = begin(&mut server, &mut peer, Type::Confirmable, 100, 0);
    server
        .oscore_mut()
        .unwrap()
        .restore_sender_reservation(0)
        .unwrap();
    for now in 1..4 {
        assert!(matches!(
            server.complete(handle, response(b"initial"), now),
            Err(coaptic::app::Error::Oscore(_))
        ));
        assert!(relation(&mut server).is_none());
        assert_eq!(server.engine_mut().tx_occupied(), 0);
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.oscore().unwrap().sender_seq(), 0);
    }
    server
        .oscore_mut()
        .unwrap()
        .reserve_sender_sequences(10, |_| Ok::<_, ()>(()))
        .unwrap();
    assert!(
        server
            .complete(
                handle,
                Response::content(&NEAR).content_format(ContentFormat::TEXT_PLAIN),
                4
            )
            .is_err()
    );
    assert!(relation(&mut server).is_none());
    assert_eq!(server.engine_mut().tx_occupied(), 0);
    assert!(server.transport_mut().take().is_none());
    let spent = server.oscore().unwrap().sender_seq();
    server.complete(handle, response(b"initial"), 5).unwrap();
    assert_eq!(server.oscore().unwrap().sender_seq(), spent + 1);
    content(&mut server, &peer, b"initial", 0, Type::Confirmable);
}

#[cfg(feature = "oscore")]
#[test]
fn deferred_observe_unauthenticated_and_malformed_requests_cannot_revoke_handle() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, true);
    let handle = begin(&mut server, &mut peer, Type::Confirmable, 110, 0);
    let mut other = fixed();
    let mut attacker = Peer::new(&mut other, true);
    attacker
        .context
        .as_mut()
        .unwrap()
        .set_sender_seq(10)
        .unwrap();
    let mut plain_peer = Peer::new(&mut fixed(), false);
    let plain = request(&mut plain_peer, 111, Type::Confirmable, 0);
    let mut tampered = request(&mut attacker, 112, Type::Confirmable, 0);
    tampered.bytes[tampered.len - 1] ^= 1;
    let zero = coaptic::message::EncodedUint::new(0);
    let opts = [
        Opt::observe(&zero),
        Opt::uri_path("obs"),
        Opt::new(coaptic::message::OptionNumber::BLOCK2, &[0, 0, 0, 0]),
    ];
    let malformed = attacker.encode(
        &Message::new(Type::Confirmable, Code::GET, MessageId::new(113))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(&opts),
    );
    for (index, frame) in [plain, tampered, malformed].into_iter().enumerate() {
        server.transport_mut().incoming = Some(frame);
        server
            .poll_with(index as u64 + 1, |_, _| {
                panic!("invalid request dispatched")
            })
            .unwrap();
        drain(&mut server);
        assert!(relation(&mut server).is_none());
        assert_eq!(server.engine_mut().rx_occupied(), 0);
    }
    server.complete(handle, response(b"initial"), 5).unwrap();
    content(&mut server, &peer, b"initial", 0, Type::Confirmable);
}

coaptic::memory_profile! {
    PairBuffers {rx_datagram:(4,256),tx_datagram:(4,256),rx_body:(2,1024),tx_body:(2,1024),dedup:16,observe:2,}
}
#[test]
fn deferred_observe_blocked_owner_does_not_starve_other_relation() {
    let mut server = App::with_resources(
        Resources::<PairBuffers>::new()
            .routes::<1>()
            .block_wise::<false>()
            .deferred::<2>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).observe(source))
    .bind(Io::default())
    .unwrap();
    let mut peer = Peer::new(&mut fixed(), false);
    server.transport_mut().incoming = Some(request(&mut peer, 120, Type::Confirmable, 0));
    let mut handle = None;
    server
        .poll_with(0, |_, candidate| {
            handle = candidate;
            Response::deferred()
        })
        .unwrap();
    server.transport_mut().take().unwrap();
    server.transport_mut().fail_send = true;
    assert!(
        server
            .complete(handle.unwrap(), response(b"initial"), 1)
            .is_err()
    );
    server.transport_mut().take().unwrap();
    server.transport_mut().fail_send = false;
    let engine = server.engine_mut();
    let first = engine
        .observe_interest(engine.lookup_observe(key()).unwrap())
        .unwrap();
    let other = coaptic::storage::ObserveInterest::new(
        Token::new(b"second").unwrap(),
        Endpoint::v4([192, 0, 2, 3], 5683),
    )
    .with_resource(first.resource())
    .with_content_format(Some(ContentFormat::TEXT_PLAIN));
    engine.insert_observe(other).unwrap();
    assert_eq!(server.signal(&["obs"]), 2);
    server.poll(2).unwrap();
    server.poll(3).unwrap();
    let frame = server
        .transport_mut()
        .take()
        .expect("other observer progresses");
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(parsed.token(), Token::new(b"second").unwrap());
    assert_eq!(parsed.payload(), b"updated");
    assert_eq!(parsed.observe(), Some(Ok(1)));
    assert!(server.transport_mut().take().is_none());
    let id = server.engine_mut().lookup_observe(key()).unwrap();
    assert!(
        server
            .engine_mut()
            .observe_interest(id)
            .unwrap()
            .is_pending()
    );
    assert!(server.cancel_deferred(handle.unwrap()));
}

#[test]
fn deferred_observe_latest_ambiguous_mid_rst_revokes_only_current_owner() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let handle = begin(&mut server, &mut peer, Type::Confirmable, 130, 0);
        server.transport_mut().fail_send = true;
        assert!(server.complete(handle, response(b"initial"), 1).is_err());
        let first = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        assert!(server.complete(handle, response(b"next"), 2).is_err());
        let second = content(&mut server, &peer, b"next", 1, Type::Confirmable);
        assert_ne!(first, second);
        server.transport_mut().fail_send = false;
        let mut wrong_peer = control(second, Type::Reset);
        wrong_peer.peer = Endpoint::v4([192, 0, 2, 9], 5683);
        let mut malformed = control(second, Type::Reset);
        malformed.bytes[0] |= 1;
        malformed.bytes[4] = 1;
        malformed.len = 5;
        for (index, frame) in [
            control(first, Type::Reset),
            wrong_peer,
            control(second, Type::Acknowledgement),
            control(MessageId::new(second.get().wrapping_add(1)), Type::Reset),
            malformed,
        ]
        .into_iter()
        .enumerate()
        {
            server.transport_mut().incoming = Some(frame);
            server.poll(3 + index as u64).unwrap();
            assert_eq!(relation(&mut server).unwrap().seq(), 1);
            assert!(server.transport_mut().take().is_none());
        }
        assert!(
            server
                .complete(handle, Response::content(b"changed format"), 9)
                .is_err()
        );
        server.transport_mut().incoming = Some(control(second, Type::Reset));
        server.poll(10).unwrap();
        assert!(relation(&mut server).is_none());
        assert!(!server.cancel_deferred(handle));
        assert!(server.complete(handle, response(b"stale"), 11).is_err());
        assert!(server.transport_mut().take().is_none());
        let next = begin(&mut server, &mut peer, Type::Confirmable, 131, 12);
        server.complete(next, response(b"initial"), 13).unwrap();
        let successful = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        server.transport_mut().incoming = Some(control(second, Type::Reset));
        server.poll(14).unwrap();
        assert!(relation(&mut server).is_some());
        assert!(
            server
                .engine_mut()
                .lookup_pending_con(successful, PEER)
                .is_some()
        );
    }
}

#[test]
fn deferred_observe_large_initial_without_etag_refuses_available_body_storage() {
    static LARGE: [u8; 300] = [b'b'; 300];
    let mut server = App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).observe(source))
    .bind(Io::default())
    .unwrap();
    let mut peer = Peer::new(&mut fixed(), false);
    server.transport_mut().incoming = Some(request(&mut peer, 140, Type::Confirmable, 0));
    let mut handle = None;
    server
        .poll_with(0, |_, candidate| {
            handle = candidate;
            Response::deferred()
        })
        .unwrap();
    server.transport_mut().take().unwrap();
    assert!(
        without_allocating(|| server.complete(
            handle.unwrap(),
            Response::content(&LARGE).content_format(ContentFormat::TEXT_PLAIN),
            1
        ))
        .is_err()
    );
    assert!(server.transport_mut().take().is_none());
    assert!(server.engine_mut().lookup_observe(key()).is_none());
    assert_eq!(server.engine_mut().tx_occupied(), 0);
    server
        .complete(handle.unwrap(), response(b"initial"), 2)
        .unwrap();
    let frame = server.transport_mut().take().unwrap();
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(parsed.payload(), b"initial");
    assert_eq!(parsed.observe(), Some(Ok(0)));
    assert!(parsed.block2().is_none());
}

#[test]
fn deferred_observe_ambiguous_rst_authority_expires_before_mid_reuse() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed_lifetime(1_000_000);
        let mut peer = Peer::new(&mut server, protected);
        let handle = begin(&mut server, &mut peer, Type::Confirmable, 150, 0);
        server.transport_mut().fail_send = true;
        assert!(server.complete(handle, response(b"initial"), 1).is_err());
        let old = content(&mut server, &peer, b"initial", 0, Type::Confirmable);
        server.transport_mut().fail_send = false;
        let expires = 1 + u64::from(coaptic::message::Transmission::EXCHANGE_LIFETIME_MS);
        server.transport_mut().incoming = Some(control(old, Type::Reset));
        server.poll(expires).unwrap();
        assert!(
            relation(&mut server).is_some(),
            "expired MID reset authority must not revoke a long-lived handle"
        );
        server.transport_mut().fail_send = true;
        assert!(
            server
                .complete(handle, response(b"retried"), expires + 1)
                .is_err()
        );
        let current = content(&mut server, &peer, b"retried", 1, Type::Confirmable);
        server.transport_mut().fail_send = false;
        server.transport_mut().incoming = Some(control(current, Type::Reset));
        server.poll(expires + 2).unwrap();
        assert!(relation(&mut server).is_none());
        assert!(
            server
                .complete(handle, response(b"stale"), expires + 3)
                .is_err()
        );
    }
}
