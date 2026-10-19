//! App declines observations whose request selection is not retained.
use coaptic::app::AppStorage;
use coaptic::message::ContentFormat;
use coaptic::message::decode;
use coaptic::message::{Message, MessageId, Opt, Token, Type, encode};
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
impl coaptic::storage::DatagramTransport for Io {
    type Error = &'static str;
}

impl DatagramIo for Io {
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

type Server<S = coaptic::storage::Memory<Buffers>> = App<Buffers, Io, 2, false, S, 2>;
fn fixed() -> Server {
    fixed_lifetime(247_000)
}
fn fixed_lifetime(lifetime: u64) -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<2>()
            .block_wise::<false>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deferred_lifetime(lifetime)
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).fetch(handler).observe(source))
    .route("other", coaptic::fetch(handler).observe(source))
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

thread_local! { static SOURCE_CALLS: Cell<usize> = const { Cell::new(0) }; }
fn handler(request: coaptic::Request<'_>) -> Response<'static> {
    Response::content(
        if request.payload() == b"b" || request.uri_query().next() == Some(Ok("b")) {
            b"selected-b"
        } else {
            b"selected-a"
        },
    )
    .content_format(ContentFormat::TEXT_PLAIN)
    .observe(77)
}
fn source() -> Response<'static> {
    SOURCE_CALLS.with(|n| n.set(n.get() + 1));
    Response::content(b"selected-a").content_format(ContentFormat::TEXT_PLAIN)
}
fn request(
    peer: &mut Peer,
    code: Code,
    mid: u16,
    token: &[u8],
    path: &str,
    observation: Option<u32>,
    body: &[u8],
) -> Frame {
    let value = coaptic::message::encode_uint(observation.unwrap_or(0));
    let format = coaptic::message::encode_uint(0);
    let mut options = coaptic::message::OptionsBuilder::<3>::new();
    if observation.is_some() {
        options.push(Opt::observe(&value)).unwrap();
    }
    options.push(Opt::uri_path(path)).unwrap();
    if code == Code::FETCH {
        options.push(Opt::content_format(&format)).unwrap();
    }
    peer.encode(
        &Message::new(Type::NonConfirmable, code, MessageId::new(mid))
            .with_token(Token::new(token).unwrap())
            .with_options(options.as_slice())
            .with_payload(body),
    )
}
fn take<S: AppStorage>(
    server: &mut Server<S>,
    peer: &Peer,
    token: &[u8],
    body: &[u8],
    observed: bool,
) {
    let f = peer.open(server.transport_mut().take().unwrap());
    let p = decode(&f.bytes[..f.len]).unwrap();
    assert_eq!(p.code(), Code::CONTENT);
    assert_eq!(p.token(), Token::new(token).unwrap());
    assert_eq!(p.payload(), body);
    assert_eq!(p.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
    assert_eq!(p.observe().is_some(), observed);
    assert!(server.transport_mut().take().is_none());
}
fn cycle<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    SOURCE_CALLS.with(|n| n.set(0));
    for round in 0..20u16 {
        let body = if round % 2 == 0 { b"a" } else { b"b" };
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            round + 1,
            body,
            "obs",
            Some(0),
            body,
        ));
        without_allocating(|| server.poll(u64::from(round) * 2).unwrap());
        take(
            &mut server,
            &peer,
            body,
            if body == b"b" {
                b"selected-b"
            } else {
                b"selected-a"
            },
            false,
        );
        assert_eq!(server.signal(&["obs"]), 0);
        assert_eq!(
            server
                .notify(
                    u64::from(round) * 2 + 1,
                    &["obs"],
                    Response::content(b"wrong")
                )
                .unwrap(),
            0
        );
        without_allocating(|| server.poll(u64::from(round) * 2 + 1).unwrap());
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.engine_mut().rx_occupied(), 0);
        assert_eq!(server.engine_mut().tx_occupied(), 0);
    }
    assert_eq!(SOURCE_CALLS.with(Cell::get), 0);
    // Plain FETCH and deregistration still deliver the actual selected representation.
    for (i, observation) in [None, Some(1)].into_iter().enumerate() {
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            40 + i as u16,
            b"b",
            "obs",
            observation,
            b"b",
        ));
        without_allocating(|| server.poll(40 + i as u64).unwrap());
        take(&mut server, &peer, b"b", b"selected-b", false);
    }
    // GET observation remains available after repeated declined registrations.
    server.transport_mut().incoming =
        Some(request(&mut peer, Code::GET, 50, b"a", "obs", Some(0), b""));
    without_allocating(|| server.poll(50).unwrap());
    take(&mut server, &peer, b"a", b"selected-a", true);
    SOURCE_CALLS.with(|n| n.set(0));
    assert_eq!(server.signal(&["obs"]), 1);
    without_allocating(|| server.poll(5000).unwrap());
    take(&mut server, &peer, b"a", b"selected-a", true);
    assert_eq!(SOURCE_CALLS.with(Cell::get), 1);
    // A new same-owner FETCH on another route retires the GET relation.
    server.transport_mut().incoming = Some(request(
        &mut peer,
        Code::FETCH,
        51,
        b"a",
        "other",
        Some(0),
        b"b",
    ));
    without_allocating(|| server.poll(5001).unwrap());
    take(&mut server, &peer, b"a", b"selected-b", false);
    assert_eq!(server.signal(&["obs"]), 0);
    assert_eq!(server.signal(&["other"]), 0);
    without_allocating(|| server.poll(9000).unwrap());
    assert!(server.transport_mut().take().is_none());
    assert_eq!(SOURCE_CALLS.with(Cell::get), 1);
    server.transport_mut().incoming =
        Some(request(&mut peer, Code::GET, 52, b"a", "obs", Some(0), b""));
    without_allocating(|| server.poll(9001).unwrap());
    take(&mut server, &peer, b"a", b"selected-a", true);
}
#[test]
fn fetch_observe_fixed_declines_selected_updates_and_get_recovers() {
    cycle(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn fetch_observe_protected_declines_without_stale_source_and_get_recovers() {
    cycle(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn fetch_observe_allocated_exact_budget_matches_fixed_refusal_recovery() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<2>()
            .block_wise::<false>()
            .deferred::<2>()
    };
    let budget = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let server = App::with_resources(resources().allocated(budget))
            .allow_unprotected()
            .deterministic_for_tests()
            .route("obs", coaptic::get(handler).fetch(handler).observe(source))
            .route("other", coaptic::fetch(handler).observe(source))
            .bind(Io::default())
            .unwrap();
        cycle(server, protected);
    }
}
#[test]
fn fetch_observe_explicit_response_without_source_is_declined() {
    let mut server = App::with_resources(
        Resources::<Buffers>::new()
            .routes::<2>()
            .block_wise::<false>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("obs", coaptic::fetch(handler))
    .route("other", coaptic::get(handler))
    .bind(Io::default())
    .unwrap();
    let mut peer = Peer::new(&mut server, false);
    server.transport_mut().incoming = Some(request(
        &mut peer,
        Code::FETCH,
        1,
        b"b",
        "obs",
        Some(0),
        b"b",
    ));
    without_allocating(|| server.poll(0).unwrap());
    take(&mut server, &peer, b"b", b"selected-b", false);
    assert_eq!(server.signal(&["obs"]), 0);
    assert_eq!(
        server
            .notify(1, &["obs"], Response::content(b"wrong"))
            .unwrap(),
        0
    );
    assert!(
        server
            .engine()
            .lookup_observe(coaptic::storage::ObserveKey::new(
                Token::new(b"b").unwrap(),
                PEER
            ))
            .is_none()
    );
}
#[test]
fn fetch_observe_foreign_owner_cannot_cancel_get_then_exact_owner_retires() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 1, b"a", "obs", Some(0), b""));
        server.poll(0).unwrap();
        take(&mut server, &peer, b"a", b"selected-a", true);
        let key = coaptic::storage::ObserveKey::new(Token::new(b"a").unwrap(), PEER);
        let id = server.engine().lookup_observe(key).unwrap();
        let original = server.engine().observe_interest(id);
        for (index, foreign_endpoint) in [false, true].into_iter().enumerate() {
            let token = if foreign_endpoint { b"a" } else { b"b" };
            let mut frame = request(
                &mut peer,
                Code::FETCH,
                2 + index as u16,
                token,
                "other",
                Some(0),
                b"b",
            );
            if foreign_endpoint {
                frame.peer = Endpoint::v4([192, 0, 2, 3], 5683);
            }
            server.transport_mut().incoming = Some(frame);
            without_allocating(|| server.poll(1 + index as u64).unwrap());
            take(&mut server, &peer, token, b"selected-b", false);
            assert_eq!(server.engine().observe_interest(id), original);
        }
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            4,
            b"a",
            "other",
            Some(1),
            b"b",
        ));
        without_allocating(|| server.poll(4).unwrap());
        take(&mut server, &peer, b"a", b"selected-b", false);
        assert!(server.engine().lookup_observe(key).is_none());
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 5, b"a", "obs", Some(0), b""));
        server.poll(5).unwrap();
        take(&mut server, &peer, b"a", b"selected-a", true);
    }
}
#[test]
fn fetch_observe_replacement_revokes_pending_deferred_get_handle() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let mut handle = None;
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 1, b"a", "obs", Some(0), b""));
        server
            .poll_with(0, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        let handle = handle.unwrap();
        assert!(server.transport_mut().take().is_none());
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            2,
            b"a",
            "other",
            Some(0),
            b"b",
        ));
        without_allocating(|| server.poll(1).unwrap());
        take(&mut server, &peer, b"a", b"selected-b", false);
        assert_eq!(
            server.complete(handle, Response::content(b"stale"), 2),
            Err(coaptic::Error::DeferredUnknown)
        );
        assert_eq!(server.signal(&["obs"]), 0);
        assert_eq!(server.signal(&["other"]), 0);
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 3, b"a", "obs", Some(0), b""));
        server.poll(3).unwrap();
        take(&mut server, &peer, b"a", b"selected-a", true);
    }
}
#[test]
fn fetch_observe_pinned_get_replacement_refuses_without_detaching_owner() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 1, b"a", "obs", Some(0), b""));
        server.poll(0).unwrap();
        take(&mut server, &peer, b"a", b"selected-a", true);
        assert_eq!(
            server
                .notify_confirmable(
                    1,
                    &["obs"],
                    Response::content(b"selected-a").content_format(ContentFormat::TEXT_PLAIN)
                )
                .unwrap(),
            1
        );
        let f = peer.open(server.transport_mut().take().unwrap());
        let mid = decode(&f.bytes[..f.len]).unwrap().message_id();
        let tx = server.engine().lookup_pending_con(mid, PEER).unwrap();
        let key = coaptic::storage::ObserveKey::new(Token::new(b"a").unwrap(), PEER);
        let id = server.engine().lookup_observe(key).unwrap();
        let original = server.engine().observe_interest(id);
        let pending = server.engine().pending_con(tx);
        core::mem::forget(server.engine_mut().access_tx(tx).unwrap());
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            2,
            b"a",
            "other",
            Some(0),
            b"b",
        ));
        assert_eq!(
            without_allocating(|| server.poll(2)),
            Err(coaptic::Error::Slot(coaptic::storage::SlotError::Pinned))
        );
        assert_eq!(server.engine().observe_interest(id), original);
        assert_eq!(server.engine().pending_con(tx), pending);
        assert!(server.transport_mut().take().is_none());
        assert_eq!(server.engine_mut().rx_occupied(), 0);
        server.transport_mut().incoming = Some(request(
            &mut peer,
            Code::FETCH,
            3,
            b"b",
            "obs",
            Some(0),
            b"b",
        ));
        server.poll(3).unwrap();
        take(&mut server, &peer, b"b", b"selected-b", false);
        assert_eq!(server.engine().observe_interest(id), original);
    }
}
#[cfg(feature = "oscore")]
#[test]
fn fetch_observe_unauthenticated_replacement_cannot_remove_get_owner() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, true);
    server.transport_mut().incoming =
        Some(request(&mut peer, Code::GET, 1, b"a", "obs", Some(0), b""));
    server.poll(0).unwrap();
    take(&mut server, &peer, b"a", b"selected-a", true);
    let key = coaptic::storage::ObserveKey::new(Token::new(b"a").unwrap(), PEER);
    let id = server.engine().lookup_observe(key).unwrap();
    let original = server.engine().observe_interest(id);
    let mut frame = request(&mut peer, Code::FETCH, 2, b"a", "other", Some(0), b"b");
    frame.bytes[frame.len - 1] ^= 1;
    server.transport_mut().incoming = Some(frame);
    without_allocating(|| server.poll(1).unwrap());
    assert_eq!(server.engine().observe_interest(id), original);
    assert!(server.transport_mut().take().is_none());
    server.transport_mut().incoming = Some(request(
        &mut peer,
        Code::FETCH,
        3,
        b"a",
        "other",
        Some(0),
        b"b",
    ));
    server.poll(2).unwrap();
    take(&mut server, &peer, b"a", b"selected-b", false);
    assert!(server.engine().lookup_observe(key).is_none());
}
fn query_request(peer: &mut Peer, mid: u16, observation: Option<u32>, query: &str) -> Frame {
    let value = coaptic::message::encode_uint(observation.unwrap_or(0));
    let mut opts = coaptic::message::OptionsBuilder::<3>::new();
    if observation.is_some() {
        opts.push(Opt::observe(&value)).unwrap();
    }
    opts.push(Opt::uri_path("obs")).unwrap();
    opts.push(Opt::uri_query(query)).unwrap();
    peer.encode(
        &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(mid))
            .with_token(Token::new(b"q").unwrap())
            .with_options(opts.as_slice()),
    )
}
fn query_cycle<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    SOURCE_CALLS.with(|n| n.set(0));
    for (index, query) in ["b", ""].into_iter().enumerate() {
        server.transport_mut().incoming =
            Some(query_request(&mut peer, 1 + index as u16, Some(0), query));
        without_allocating(|| server.poll(index as u64 * 10).unwrap());
        take(
            &mut server,
            &peer,
            b"q",
            if query == "b" {
                b"selected-b"
            } else {
                b"selected-a"
            },
            false,
        );
        assert_eq!(server.signal(&["obs"]), 0);
        without_allocating(|| server.poll(index as u64 * 10 + 5).unwrap());
        assert!(server.transport_mut().take().is_none());
    }
    assert_eq!(SOURCE_CALLS.with(Cell::get), 0);
    server.transport_mut().incoming =
        Some(request(&mut peer, Code::GET, 3, b"q", "obs", Some(0), b""));
    server.poll(20).unwrap();
    take(&mut server, &peer, b"q", b"selected-a", true);
    server.transport_mut().incoming = Some(query_request(&mut peer, 4, Some(0), "b"));
    server.poll(21).unwrap();
    take(&mut server, &peer, b"q", b"selected-b", false);
    assert_eq!(server.signal(&["obs"]), 0);
}
#[test]
fn query_observe_declines_empty_and_selected_query_then_get_recovers() {
    for protected in [false, cfg!(feature = "oscore")] {
        query_cycle(fixed(), protected);
    }
}

#[test]
fn query_observe_deferred_refuses_without_blocking_ordinary_deferred_query() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        for (index, query) in ["b", ""].into_iter().enumerate() {
            server.transport_mut().incoming =
                Some(query_request(&mut peer, 1 + index as u16, Some(0), query));
            without_allocating(|| {
                server
                    .poll_with(index as u64, |_, candidate| {
                        assert!(candidate.is_none());
                        Response::deferred()
                    })
                    .unwrap()
            });
            let f = peer.open(server.transport_mut().take().unwrap());
            let p = decode(&f.bytes[..f.len]).unwrap();
            assert_eq!(p.code(), Code::SERVICE_UNAVAILABLE);
            assert!(p.observe().is_none());
            assert_eq!(server.signal(&["obs"]), 0);
        }
        let mut handle = None;
        server.transport_mut().incoming = Some(query_request(&mut peer, 3, None, "b"));
        server
            .poll_with(2, |req, candidate| {
                assert_eq!(req.uri_query().next(), Some(Ok("b")));
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        assert!(server.transport_mut().take().is_none());
        without_allocating(|| {
            server
                .complete(
                    handle.unwrap(),
                    Response::content(b"selected-b").content_format(ContentFormat::TEXT_PLAIN),
                    3,
                )
                .unwrap()
        });
        take(&mut server, &peer, b"q", b"selected-b", false);
        server.transport_mut().incoming =
            Some(request(&mut peer, Code::GET, 4, b"q", "obs", Some(0), b""));
        server.poll(4).unwrap();
        take(&mut server, &peer, b"q", b"selected-a", true);
    }
}

#[cfg(feature = "alloc")]
#[test]
fn query_observe_allocated_exact_budget_preserves_refusal_and_recovery() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<2>()
            .block_wise::<false>()
            .deferred::<2>()
    };
    let budget = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let server = App::with_resources(resources().allocated(budget))
            .allow_unprotected()
            .deterministic_for_tests()
            .route("obs", coaptic::get(handler).fetch(handler).observe(source))
            .route("other", coaptic::fetch(handler).observe(source))
            .bind(Io::default())
            .unwrap();
        query_cycle(server, protected);
    }
}
