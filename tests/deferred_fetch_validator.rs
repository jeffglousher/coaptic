//! Deferred FETCH snapshots retain the exact bounded request validator selection.
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
        rx_datagram: (4, 1472), tx_datagram: (4, 1472),
        rx_body: (2, 4096), tx_body: (2, 4096),
        dedup: 16, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const BODY: [u8; 2000] = [0x5a; 2000];
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
            peer: PEER,
        }
    }
}
#[derive(Default)]
struct Io {
    incoming: Option<Frame>,
    output: [Frame; 16],
    len: usize,
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
    type Error = core::convert::Infallible;
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
        Ok(bytes.len())
    }
}

type Server<S = coaptic::storage::Memory<Buffers, coaptic::storage::WithBodies<Buffers>>> =
    App<Buffers, Io, 1, true, S, 2>;
fn fixed() -> Server {
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

struct Peer {
    #[cfg(feature = "oscore")]
    context: Option<coaptic::oscore::SecurityContext>,
}
impl Peer {
    fn new<S: AppStorage<Token = coaptic::message::Token>>(
        server: &mut Server<S>,
        protected: bool,
    ) -> Self {
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
    fn request(&mut self, mid: u16, tags: &[&[u8]], block: Option<u32>, selection: &[u8]) -> Frame {
        let mut opts = coaptic::message::OptionsBuilder::<8>::new();
        for tag in tags {
            opts.push(Opt::etag(tag)).unwrap();
        }
        opts.push(Opt::uri_path("select")).unwrap();
        let cf = ContentFormat::CBOR.encode();
        opts.push(Opt::content_format(&cf)).unwrap();
        let b = block.map(|num| BlockValue::from_size(num, false, 1024).unwrap().encode());
        if let Some(ref b) = b {
            opts.push(Opt::block2(b)).unwrap();
        }
        let message = Message::new(Type::NonConfirmable, Code::FETCH, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(opts.as_slice())
            .with_payload(selection);
        let mut frame = Frame::default();
        #[cfg(feature = "oscore")]
        if let Some(context) = self.context.as_mut() {
            frame.len = context.protect_request(&message, &mut frame.bytes).unwrap();
            return frame;
        }
        frame.len = encode(&message, &mut frame.bytes).unwrap();
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
fn campaign<S: AppStorage<Token = coaptic::message::Token>>(
    mut server: Server<S>,
    protected: bool,
) {
    let mut peer = Peer::new(&mut server, protected);
    without_allocating(|| {
        // Reuse completed rows; a valid repeated ETag list remains a bounded selection.
        for (round, original) in [&[&b"v1"[..]][..], &[&b"v1"[..], &b"v2"[..]][..], &[][..]]
            .into_iter()
            .enumerate()
        {
            let now = round as u64 * 100;
            server.transport_mut().incoming =
                Some(peer.request(1 + round as u16 * 20, original, None, b"selection"));
            let mut handle = None;
            let mut dispatches = 0;
            server
                .poll_with(now, |request, candidate| {
                    dispatches += 1;
                    assert_eq!(request.payload(), b"selection");
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            assert_eq!(dispatches, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY).etag(b"body"),
                    now + 1,
                )
                .unwrap();
            let first = peer.open(server.transport_mut().take().unwrap());
            let first = decode(&first.bytes[..first.len]).unwrap();
            assert_eq!(first.code(), Code::CONTENT);
            assert_eq!(first.token(), Token::new(b"owner").unwrap());
            assert_eq!(first.etag().next(), Some(&b"body"[..]));
            assert_eq!(first.payload(), &BODY[..1024]);
            assert_eq!(first.block2().unwrap().unwrap().num(), 0);
            assert!(server.transport_mut().take().is_none());
            for (i, tags) in [
                &[&b"changed"[..]][..],
                &[][..],
                &[&b"v1"[..], &b"v1"[..]][..],
                &[&b""[..]][..],
                &[&b"123456789"[..]][..],
            ]
            .into_iter()
            .enumerate()
            {
                if tags == original {
                    continue;
                }
                server.transport_mut().incoming = Some(peer.request(
                    2 + i as u16 + round as u16 * 20,
                    tags,
                    Some(1),
                    b"selection",
                ));
                server
                    .poll_with(now + 2 + i as u64, |_, _| {
                        panic!("snapshot follow-up must not redispatch")
                    })
                    .unwrap();
                let bad = peer.open(server.transport_mut().take().unwrap());
                let bad = decode(&bad.bytes[..bad.len]).unwrap();
                assert_eq!(bad.code(), Code::BAD_REQUEST);
                assert!(bad.payload().is_empty());
            }
            server.transport_mut().incoming =
                Some(peer.request(10 + round as u16 * 20, original, Some(1), b"selection"));
            server
                .poll_with(now + 10, |_, _| {
                    panic!("snapshot follow-up must not redispatch")
                })
                .unwrap();
            let last = peer.open(server.transport_mut().take().unwrap());
            let last = decode(&last.bytes[..last.len]).unwrap();
            assert_eq!(last.code(), Code::CONTENT);
            assert_eq!(last.token(), Token::new(b"owner").unwrap());
            assert_eq!(last.payload(), &BODY[1024..]);
            assert_eq!(last.block2().unwrap().unwrap().num(), 1);
            assert!(!last.block2().unwrap().unwrap().more());
            assert_eq!(first.etag().next(), last.etag().next());
        }
    });
}
#[test]
fn deferred_fetch_changed_validator_refuses_then_snapshot_recovers() {
    campaign(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_fetch_protected_validator_refusal_and_complete_recovery() {
    campaign(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_fetch_allocated_exact_budget_retains_selection() {
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
    for protected in [false, cfg!(feature = "oscore")] {
        let server = App::with_resources(resources().allocated(bytes))
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        campaign(server, protected);
    }
}

fn bounds<S: AppStorage<Token = coaptic::message::Token>>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    without_allocating(|| {
        // Header/token/options/marker occupy 22 bytes in the retained 512-byte selection.
        // Refusal must leave both deferred rows free for the exact-bound request.
        for mid in 100..112 {
            server.transport_mut().incoming = Some(peer.request(mid, &[b"v1"], None, &[7; 491]));
            server
                .poll_with(u64::from(mid), |_, candidate| {
                    assert!(candidate.is_none());
                    Response::deferred()
                })
                .unwrap();
            let refusal = peer.open(server.transport_mut().take().unwrap());
            assert_eq!(
                decode(&refusal.bytes[..refusal.len]).unwrap().code(),
                Code::SERVICE_UNAVAILABLE
            );
        }
        server.transport_mut().incoming = Some(peer.request(120, &[b"v1"], None, &[7; 490]));
        let mut handle = None;
        server
            .poll_with(120, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        server
            .complete(handle.unwrap(), Response::content(&BODY).etag(b"body"), 121)
            .unwrap();
        let first = peer.open(server.transport_mut().take().unwrap());
        assert_eq!(
            decode(&first.bytes[..first.len]).unwrap().payload(),
            &BODY[..1024]
        );
        server.transport_mut().incoming = Some(peer.request(122, &[b"v1"], Some(1), &[7; 490]));
        server
            .poll_with(122, |_, _| {
                panic!("exact-bound selection must reuse snapshot")
            })
            .unwrap();
        let last = peer.open(server.transport_mut().take().unwrap());
        assert_eq!(
            decode(&last.bytes[..last.len]).unwrap().payload(),
            &BODY[1024..]
        );
    });
}
#[test]
fn deferred_fetch_selection_byte_bound_refuses_without_capacity_loss() {
    bounds(fixed(), false);
    #[cfg(feature = "oscore")]
    bounds(fixed(), true);
}

fn concurrent_fixed() -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("select", coaptic::get(concurrent_representation))
    .bind(Io::default())
    .unwrap()
}

fn concurrent_representation(request: coaptic::Request<'_>) -> Response<'static> {
    let body = if request.uri_query().next() == Some(Ok("right")) {
        &[0x6b; 2048]
    } else {
        &[0x5a; 2048]
    };
    Response::content(body)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"shared")
}

fn concurrent_downloads<S: AppStorage<Token = Token>>(
    mut client: Server<S>,
    mut server: Server<S>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
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
        client.set_oscore(&mut Some(make(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(make(&[2], &[1]))).unwrap();
    }
    #[cfg(not(feature = "oscore"))]
    assert!(!protected);
    without_allocating(|| {
        let mut now = 0;
        for q_block in [false, true] {
            for different_selection in [false, true] {
                let calls = core::array::from_fn::<_, 2, _>(|index| {
                    let request = client
                        .get("select")
                        .query(if index == 1 && different_selection {
                            "right"
                        } else {
                            "left"
                        })
                        .to(PEER)
                        .non();
                    let request = if q_block { request.q_block2() } else { request };
                    request.send(now).unwrap()
                });
                assert_ne!(calls[0].token(), calls[1].token());
                let mut completed = [false; 2];
                let mut bodies = [None; 2];
                for round in 0..2 {
                    let mut replies = [[Frame::default(); 2]; 2];
                    let mut counts = [0; 2];
                    while let Some(frame) = client.transport_mut().take() {
                        server.transport_mut().incoming = Some(frame);
                        now += 1;
                        server.poll(now).unwrap();
                        while let Some(reply) = server.transport_mut().take() {
                            let token = decode(&reply.bytes[..reply.len]).unwrap().token();
                            let index =
                                calls.iter().position(|call| call.token() == token).unwrap();
                            assert!(counts[index] < 2);
                            replies[counts[index]][index] = reply;
                            counts[index] += 1;
                        }
                    }
                    // Deliver A0, B0 before either Call receives its final block.
                    for (block, by_call) in replies.into_iter().enumerate() {
                        for (index, reply) in by_call.into_iter().enumerate() {
                            if block < counts[index] {
                                client.transport_mut().incoming = Some(reply);
                                now += 1;
                                client.poll(now).unwrap();
                            }
                        }
                        if round == 0 && block == 0 {
                            let key = |call: coaptic::Call| {
                                coaptic::storage::BlockKey::new(call.token(), PEER).with_identity(
                                    coaptic::storage::BodyTag::new(b"shared").unwrap(),
                                )
                            };
                            let a = client.engine().lookup_rx_body(key(calls[0])).unwrap();
                            let b = client.engine().lookup_rx_body(key(calls[1]));
                            assert!(b.is_some(), "second Call must own its own partial body");
                            assert_ne!(Some(a), b);
                            bodies = [Some(a), b];
                            if !q_block {
                                assert_eq!(
                                    client.transport().len,
                                    2,
                                    "both Calls must request block1"
                                );
                            }
                        }
                    }
                    let mut output = [0; 2048];
                    for index in 0..2 {
                        if let Some(reply) = client
                            .take_response_into(calls[index], &mut output)
                            .unwrap()
                        {
                            let reply = reply.unwrap();
                            assert_eq!(reply.code(), Code::CONTENT);
                            assert_eq!(reply.etag_bytes(), Some(b"shared".as_slice()));
                            assert_eq!(
                                reply.body(),
                                Some(if index == 1 && different_selection {
                                    &[0x6b; 2048][..]
                                } else {
                                    &[0x5a; 2048][..]
                                })
                            );
                            completed[index] = true;
                        }
                    }
                    if completed == [true; 2] {
                        break;
                    }
                }
                assert_eq!(completed, [true; 2]);
                assert_eq!(client.engine_mut().rx_occupied(), 0);
                for body in bodies.into_iter().flatten() {
                    assert!(client.engine().rx_body_transfer(body).is_none());
                }
            }
        }
    });
}

#[test]
fn concurrent_same_etag_downloads_keep_call_ownership() {
    concurrent_downloads(concurrent_fixed(), concurrent_fixed(), false);
    #[cfg(feature = "oscore")]
    concurrent_downloads(concurrent_fixed(), concurrent_fixed(), true);
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_concurrent_same_etag_downloads_keep_call_ownership() {
    let make = || {
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
        App::with_resources(resources().allocated(bytes))
            .allow_unprotected()
            .deterministic_for_tests()
            .route("select", coaptic::get(concurrent_representation))
            .bind(Io::default())
            .unwrap()
    };
    for protected in [false, cfg!(feature = "oscore")] {
        concurrent_downloads(make(), make(), protected);
    }
}
