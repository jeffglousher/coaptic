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
impl DatagramIo for Io {
    type Error = core::convert::Infallible;
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
    .allow_plaintext()
    .deterministic_for_tests()
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
                server.set_oscore(make(&[2], &[1]));
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
fn campaign<S: AppStorage>(mut server: Server<S>, protected: bool) {
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
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        campaign(server, protected);
    }
}

fn bounds<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    without_allocating(|| {
        // Header/options/marker occupy17 bytes in the retained512-byte selection.
        // Refusal must leave both deferred rows free for the exact-bound request.
        for mid in 100..112 {
            server.transport_mut().incoming = Some(peer.request(mid, &[b"v1"], None, &[7; 496]));
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
        server.transport_mut().incoming = Some(peer.request(120, &[b"v1"], None, &[7; 495]));
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
        server.transport_mut().incoming = Some(peer.request(122, &[b"v1"], Some(1), &[7; 495]));
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
