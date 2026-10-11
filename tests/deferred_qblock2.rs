//! Deferred single Q-Block2 selections retain complete bounded snapshots.
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
    static FAIL_RANDOM: Cell<bool> = const { Cell::new(false) };
}
fn random_for_refusal(bytes: &mut [u8]) -> bool {
    bytes.fill(0x42);
    !FAIL_RANDOM.with(Cell::get)
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
        rx_body: (2, 12288), tx_body: (2, 12288),
        dedup: 16, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const BODY: [u8; 11264] = {
    let mut bytes = [0; 11264];
    let mut index = 0;
    while index < bytes.len() {
        bytes[index] = (index * 31 + index / 512) as u8;
        index += 1;
    }
    bytes
};
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
    fail_at: Option<usize>,
    sends: usize,
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
        self.sends += 1;
        if self.fail_at == Some(self.sends) {
            return Err("injected send failure");
        }
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
    .deferred_lifetime(40)
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
    fn request(&mut self, mid: u16, num: u32, more: bool, szx: u8) -> Frame {
        self.custom(
            mid,
            Code::GET,
            "snapshot",
            &[BlockValue::new(num, more, szx).unwrap()],
            false,
            None,
            b"",
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn custom(
        &mut self,
        mid: u16,
        code: Code,
        path: &str,
        blocks: &[BlockValue],
        observe: bool,
        etag: Option<&[u8]>,
        body: &[u8],
    ) -> Frame {
        let mut options = coaptic::message::OptionsBuilder::<12>::new();
        if let Some(tag) = etag {
            options.push(Opt::etag(tag)).unwrap();
        }
        if observe {
            options
                .push(Opt::new(coaptic::message::OptionNumber::OBSERVE, &[]))
                .unwrap();
        }
        options.push(Opt::uri_path(path)).unwrap();
        let cf = ContentFormat::CBOR.encode();
        if code == Code::FETCH {
            options.push(Opt::content_format(&cf)).unwrap();
        }
        let values = [
            blocks
                .first()
                .copied()
                .unwrap_or(BlockValue::new(0, false, 6).unwrap())
                .encode(),
            blocks
                .get(1)
                .copied()
                .unwrap_or(BlockValue::new(0, false, 6).unwrap())
                .encode(),
        ];
        for value in &values[..blocks.len()] {
            options.push(Opt::q_block2(value)).unwrap();
        }
        let message = Message::new(Type::Confirmable, code, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(options.as_slice())
            .with_payload(body);
        self.encode(&message)
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
    fn invalid_fetch(&mut self, mid: u16, tags: &[&[u8]], q: &[u8]) -> Frame {
        let mut options = coaptic::message::OptionsBuilder::<12>::new();
        for tag in tags {
            options.push(Opt::etag(tag)).unwrap();
        }
        options.push(Opt::uri_path("snapshot")).unwrap();
        let format = ContentFormat::CBOR.encode();
        options.push(Opt::content_format(&format)).unwrap();
        options
            .push(Opt::new(coaptic::message::OptionNumber::Q_BLOCK2, q))
            .unwrap();
        self.encode(
            &Message::new(Type::Confirmable, Code::FETCH, MessageId::new(mid))
                .with_token(Token::new(b"owner").unwrap())
                .with_options(options.as_slice())
                .with_payload(b"query"),
        )
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
fn packets<S: AppStorage>(server: &mut Server<S>, peer: &Peer, start: u32, end: u32, szx: u8) {
    for num in start..end {
        let frame = peer.open(server.transport_mut().take().expect("selected payload"));
        let p = decode(&frame.bytes[..frame.len]).unwrap();
        let q = p.q_block2().next().unwrap().unwrap();
        assert_eq!(p.ty(), Type::NonConfirmable);
        assert_eq!(p.code(), Code::CONTENT);
        assert_eq!(q.num(), num);
        assert_eq!(q.szx(), szx);
        let size = 1usize << (szx + 4);
        let offset = num as usize * size;
        let stop = (offset + size).min(BODY.len());
        assert_eq!(p.payload(), &BODY[offset..stop]);
        assert_eq!(q.more(), stop < BODY.len());
        assert_eq!(p.etag().next(), Some(&b"snapshot"[..]));
        assert_eq!(p.content_format(), Some(Ok(ContentFormat::OCTET_STREAM)));
        assert_eq!(p.size2(), Some(Ok(BODY.len() as u32)));
    }
    assert!(server.transport_mut().take().is_none());
}
fn ack<S: AppStorage>(server: &mut Server<S>, mid: u16) {
    let f = server.transport_mut().take().unwrap();
    let p = decode(&f.bytes[..f.len]).unwrap();
    assert_eq!(p.code(), Code::EMPTY);
    assert_eq!(p.ty(), Type::Acknowledgement);
    assert_eq!(p.message_id(), MessageId::new(mid));
}
fn campaign<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    without_allocating(|| {
        server.transport_mut().incoming = Some(peer.request(1, 0, true, 6));
        let mut handle = None;
        server
            .poll_with(0, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        assert!(
            handle.is_some(),
            "single Q selection must admit deferred work"
        );
        ack(&mut server, 1);
        let mut bytes = BODY;
        server
            .complete(
                handle.unwrap(),
                Response::new(Code::CONTENT)
                    .payload_borrowed(&bytes)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM),
                1,
            )
            .unwrap();
        bytes.fill(0x33);
        packets(&mut server, &peer, 0, 10, 6);
        for (mid, num, more, szx, end) in [
            (2, 10, true, 6, 11),
            (3, 4, false, 6, 5),
            (4, 7, true, 6, 10),
            (5, 20, true, 5, 22),
        ] {
            server.transport_mut().incoming = Some(peer.request(mid, num, more, szx));
            server
                .poll_with(mid as u64 + 1, |_, _| {
                    panic!("snapshot followup cannot redispatch")
                })
                .unwrap();
            ack(&mut server, mid);
            packets(&mut server, &peer, num, end, szx);
        }
    });
}
#[test]
fn deferred_qblock2_single_selection_preserves_full_snapshot() {
    campaign(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_qblock2_protected_single_selection_preserves_full_snapshot() {
    campaign(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_qblock2_allocated_exact_budget_preserves_full_snapshot() {
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
        let app = App::with_resources(resources().allocated(bytes))
            .allow_plaintext()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        campaign(app, protected);
    }
}

#[test]
fn deferred_qblock2_failed_partial_send_retains_snapshot_and_recovers() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, true, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, h| {
                    handle = h;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            let handle = handle.unwrap();
            server.transport_mut().fail_at = Some(server.transport_mut().sends + 2);
            assert!(matches!(
                server.complete(
                    handle,
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1
                ),
                Err(coaptic::app::Error::Io(_))
            ));
            packets(&mut server, &peer, 0, 1, 6);
            for response in [
                Response::content(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::TEXT_PLAIN),
                Response::new(Code::CHANGED)
                    .payload_borrowed(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM),
                Response::content(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM)
                    .location_path("changed"),
            ] {
                assert!(matches!(
                    server.complete(handle, response, 2),
                    Err(coaptic::app::Error::Block(
                        coaptic::error::BlockTransferError::IdentityMismatch
                    ))
                ));
                assert!(server.transport_mut().take().is_none());
            }
            for response in [
                Response::new(Code::VALID).etag(b"snapshot"),
                Response::new(Code::PRECONDITION_FAILED)
                    .payload_borrowed(&BODY)
                    .etag(b"snapshot"),
            ] {
                assert!(matches!(
                    server.complete(handle, response, 2),
                    Err(coaptic::app::Error::Block(
                        coaptic::error::BlockTransferError::IdentityMismatch
                    ))
                ));
                assert!(server.transport_mut().take().is_none());
            }
            assert!(matches!(
                server.complete(handle, Response::content(&BODY).etag(b"changed"), 2),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::IdentityMismatch
                ))
            ));
            assert!(matches!(
                server.complete(
                    handle,
                    Response::content(&[0x33; 11264]).etag(b"snapshot"),
                    2
                ),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::IdentityMismatch
                ))
            ));
            assert!(server.transport_mut().take().is_none());
            server.transport_mut().fail_at = None;
            server
                .complete(
                    handle,
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    3,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 10, 6);
            server.transport_mut().incoming = Some(peer.request(2, 10, true, 6));
            server
                .poll_with(4, |_, _| panic!("no repeated effect"))
                .unwrap();
            ack(&mut server, 2);
            packets(&mut server, &peer, 10, 11, 6);
        });
    }
}
#[test]
fn deferred_qblock2_expiry_and_full_pool_refusal_recover() {
    let mut server = fixed();
    let mut peer = Peer::new(&mut server, false);
    without_allocating(|| {
        server.transport_mut().incoming = Some(peer.request(1, 4, false, 6));
        let mut handle = None;
        server
            .poll_with(0, |_, h| {
                handle = h;
                Response::deferred()
            })
            .unwrap();
        ack(&mut server, 1);
        let handle = handle.unwrap();
        let a = server.engine_mut().acquire_tx_body().unwrap();
        let b = server.engine_mut().acquire_tx_body().unwrap();
        for now in 1..17 {
            assert!(matches!(
                server.complete(
                    handle,
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    now
                ),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::Saturated
                ))
            ));
            assert!(server.transport_mut().take().is_none());
        }
        server.engine_mut().release_tx_body(a).unwrap();
        server.engine_mut().release_tx_body(b).unwrap();
        assert!(matches!(
            server.complete(handle, Response::content(&BODY), 17),
            Err(coaptic::app::Error::Block(
                coaptic::error::BlockTransferError::MissingIdentity
            ))
        ));
        server
            .complete(
                handle,
                Response::content(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM),
                18,
            )
            .unwrap();
        packets(&mut server, &peer, 4, 5, 6);
        server.poll(40).unwrap();
        assert!(matches!(
            server.complete(handle, Response::content(&BODY).etag(b"snapshot"), 41),
            Err(coaptic::app::Error::DeferredUnknown)
        ));
        let a = server.engine_mut().acquire_tx_body().unwrap();
        let b = server.engine_mut().acquire_tx_body().unwrap();
        server.engine_mut().release_tx_body(a).unwrap();
        server.engine_mut().release_tx_body(b).unwrap();
        server.transport_mut().incoming = Some(peer.request(2, 10, true, 6));
        let mut fresh = None;
        server
            .poll_with(42, |_, h| {
                fresh = h;
                Response::deferred()
            })
            .unwrap();
        ack(&mut server, 2);
        server
            .complete(
                fresh.unwrap(),
                Response::content(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM),
                43,
            )
            .unwrap();
        packets(&mut server, &peer, 10, 11, 6);
    });
}

fn reply_code<S: AppStorage>(server: &mut Server<S>, peer: &Peer, code: Code) {
    let frame = peer.open(server.transport_mut().take().unwrap());
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(parsed.code(), code, "reply MID {:?}", parsed.message_id());
    assert!(server.transport_mut().take().is_none());
}

#[test]
fn deferred_qblock2_unsupported_admission_and_selection_refuse_then_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            let q = BlockValue::new(0, true, 6).unwrap();
            for (mid, repeated, observe) in [(1, true, false), (2, false, true)] {
                server.transport_mut().incoming = Some(peer.custom(
                    mid,
                    Code::GET,
                    "snapshot",
                    &[
                        BlockValue::new(0, false, 6).unwrap(),
                        BlockValue::new(1, false, 6).unwrap(),
                    ][..if repeated { 2 } else { 1 }],
                    observe,
                    None,
                    b"",
                ));
                server
                    .poll_with(mid as u64, |_, candidate| {
                        assert!(candidate.is_none());
                        Response::deferred()
                    })
                    .unwrap();
                reply_code(&mut server, &peer, Code::SERVICE_UNAVAILABLE);
            }
            server.transport_mut().incoming = Some(peer.request(3, 0, true, 6));
            let mut handle = None;
            server
                .poll_with(3, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 3);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    4,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 10, 6);
            for (mid, path, blocks) in [
                (4, "other", [q, q]),
                (
                    5,
                    "snapshot",
                    [
                        BlockValue::new(0, false, 6).unwrap(),
                        BlockValue::new(11, false, 6).unwrap(),
                    ],
                ),
            ] {
                server.transport_mut().incoming = Some(peer.custom(
                    mid,
                    Code::GET,
                    path,
                    if mid == 4 { &blocks[..1] } else { &blocks },
                    false,
                    None,
                    b"",
                ));
                server
                    .poll_with(mid as u64 + 1, |_, _| {
                        panic!("owned snapshot cannot redispatch")
                    })
                    .unwrap();
                reply_code(&mut server, &peer, Code::BAD_REQUEST);
            }
            server.transport_mut().incoming = Some(peer.request(6, 11, false, 6));
            server
                .poll_with(8, |_, _| panic!("invalid range cannot redispatch"))
                .unwrap();
            reply_code(&mut server, &peer, Code::BAD_REQUEST);
            // GET cache validators do not alter the selected representation.
            server.transport_mut().incoming = Some(peer.custom(
                7,
                Code::GET,
                "snapshot",
                &[BlockValue::new(10, false, 6).unwrap()],
                false,
                Some(b"other"),
                b"",
            ));
            server
                .poll_with(9, |_, _| panic!("GET validator must preserve snapshot"))
                .unwrap();
            ack(&mut server, 7);
            packets(&mut server, &peer, 10, 11, 6);
        });
    }
}

#[test]
fn deferred_qblock2_fetch_validator_and_payload_bind_snapshot() {
    fetch_selection(false);
}
#[test]
fn deferred_qblock2_repeated_fetch_validator_and_payload_bind_snapshot() {
    fetch_selection(true);
}
fn fetch_selection(repeated: bool) {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            let q = BlockValue::new(0, false, 6).unwrap();
            let selections = [
                BlockValue::new(9, false, 6).unwrap(),
                BlockValue::new(10, false, 6).unwrap(),
            ];
            let selections = if repeated {
                &selections[..]
            } else {
                &selections[1..]
            };
            server.transport_mut().incoming = Some(peer.custom(
                1,
                Code::FETCH,
                "snapshot",
                &[q],
                false,
                Some(b"selected"),
                b"query",
            ));
            let mut handle = None;
            server
                .poll_with(0, |request, candidate| {
                    assert_eq!(request.payload(), b"query");
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 1, 6);
            for (mid, tag, body) in [
                (2, Some(&b"changed"[..]), &b"query"[..]),
                (3, None, &b"query"[..]),
                (4, Some(&b"selected"[..]), &b"other"[..]),
            ] {
                server.transport_mut().incoming =
                    Some(peer.custom(mid, Code::FETCH, "snapshot", selections, false, tag, body));
                server
                    .poll_with(mid as u64, |_, _| {
                        panic!("mismatched FETCH cannot redispatch")
                    })
                    .unwrap();
                reply_code(&mut server, &peer, Code::BAD_REQUEST);
            }
            server.transport_mut().incoming = Some(peer.custom(
                5,
                Code::FETCH,
                "snapshot",
                selections,
                false,
                Some(b"selected"),
                b"query",
            ));
            server
                .poll_with(5, |_, _| panic!("matching FETCH cannot redispatch"))
                .unwrap();
            ack(&mut server, 5);
            packets(&mut server, &peer, if repeated { 9 } else { 10 }, 11, 6);
            for (mid, tags, q, expected) in [
                (
                    6,
                    &[&b"selected"[..], &b"selected"[..]][..],
                    &[0xa6][..],
                    Code::BAD_REQUEST,
                ),
                (7, &[&b""[..]][..], &[0xa6][..], Code::BAD_REQUEST),
                (8, &[&b"123456789"[..]][..], &[0xa6][..], Code::BAD_REQUEST),
                (9, &[&b"selected"[..]][..], &[0xa7][..], Code::BAD_OPTION),
            ] {
                server.transport_mut().incoming = Some(peer.invalid_fetch(mid, tags, q));
                server
                    .poll_with(u64::from(mid), |_, _| {
                        panic!("invalid selection cannot redispatch")
                    })
                    .unwrap();
                reply_code(&mut server, &peer, expected);
            }
            server.transport_mut().incoming = Some(peer.custom(
                10,
                Code::FETCH,
                "snapshot",
                selections,
                false,
                Some(b"selected"),
                b"query",
            ));
            server
                .poll_with(10, |_, _| panic!("recovered selection cannot redispatch"))
                .unwrap();
            ack(&mut server, 10);
            packets(&mut server, &peer, if repeated { 9 } else { 10 }, 11, 6);
        });
    }
}

#[test]
fn deferred_qblock2_small_snapshot_and_noncontent_codes_preserve_semantics() {
    for protected in [false, cfg!(feature = "oscore")] {
        for code in [Code::CONTENT, Code::VALID, Code::PRECONDITION_FAILED] {
            let mut server = fixed();
            let mut peer = Peer::new(&mut server, protected);
            without_allocating(|| {
                server.transport_mut().incoming = Some(peer.request(1, 0, true, 6));
                let mut handle = None;
                server
                    .poll_with(0, |_, candidate| {
                        handle = candidate;
                        Response::deferred()
                    })
                    .unwrap();
                ack(&mut server, 1);
                if code == Code::CONTENT {
                    assert!(matches!(
                        server.complete(handle.unwrap(), Response::content(b"small"), 1),
                        Err(coaptic::app::Error::Block(
                            coaptic::error::BlockTransferError::MissingIdentity
                        ))
                    ));
                    assert!(server.transport_mut().take().is_none());
                }
                let response = Response::new(code).etag(b"snapshot");
                let response = if code == Code::CONTENT {
                    response.payload_borrowed(b"small")
                } else {
                    response
                };
                server.complete(handle.unwrap(), response, 1).unwrap();
                let frame = peer.open(server.transport_mut().take().unwrap());
                let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                assert_eq!(parsed.code(), code, "reply MID {:?}", parsed.message_id());
                if code == Code::CONTENT {
                    assert_eq!(parsed.payload(), b"small");
                    assert_eq!(
                        parsed.q_block2().next(),
                        Some(Ok(BlockValue::new(0, false, 6).unwrap()))
                    );
                    assert_eq!(parsed.size2(), Some(Ok(5)));
                } else {
                    assert!(parsed.q_block2().next().is_none());
                    assert!(parsed.payload().is_empty());
                }
                assert!(server.transport_mut().take().is_none());
            });
        }
    }
}

#[test]
fn deferred_qblock2_wrong_peer_and_followup_io_failure_preserve_owner() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, false, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 1, 6);
            // Another authenticated endpoint is a new request, never a selector
            // for the existing owner's retained representation.
            let mut other = peer.request(2, 10, false, 6);
            other.peer = Endpoint::v4([192, 0, 2, 3], 5683);
            server.transport_mut().incoming = Some(other);
            let mut calls = 0;
            server
                .poll_with(2, |_, _| {
                    calls += 1;
                    Response::new(Code::SERVICE_UNAVAILABLE)
                })
                .unwrap();
            assert_eq!(calls, 1);
            reply_code(&mut server, &peer, Code::SERVICE_UNAVAILABLE);
            server.transport_mut().incoming = Some(peer.request(3, 10, false, 6));
            server.transport_mut().fail_at = Some(server.transport_mut().sends + 2);
            assert!(matches!(
                server.poll_with(3, |_, _| panic!("owned followup cannot redispatch")),
                Err(coaptic::app::Error::Io(_))
            ));
            ack(&mut server, 3);
            assert!(server.transport_mut().take().is_none());
            server.transport_mut().fail_at = None;
            server.transport_mut().incoming = Some(peer.request(4, 10, false, 6));
            server
                .poll_with(4, |_, _| panic!("recovery cannot redispatch"))
                .unwrap();
            ack(&mut server, 4);
            packets(&mut server, &peer, 10, 11, 6);
            let one = server.engine_mut().acquire_tx_body().unwrap();
            assert!(server.engine_mut().acquire_tx_body().is_none());
            server.engine_mut().release_tx_body(one).unwrap();
        });
    }
}

#[test]
fn deferred_qblock2_cancel_reclaims_attempted_snapshot_and_fresh_work() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, true, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            let handle = handle.unwrap();
            server.transport_mut().fail_at = Some(server.transport_mut().sends + 2);
            assert!(matches!(
                server.complete(
                    handle,
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1
                ),
                Err(coaptic::app::Error::Io(_))
            ));
            packets(&mut server, &peer, 0, 1, 6);
            assert!(server.cancel_deferred(handle));
            assert!(!server.cancel_deferred(handle));
            let a = server.engine_mut().acquire_tx_body().unwrap();
            let b = server.engine_mut().acquire_tx_body().unwrap();
            server.engine_mut().release_tx_body(a).unwrap();
            server.engine_mut().release_tx_body(b).unwrap();
            assert!(matches!(
                server.complete(handle, Response::content(&BODY).etag(b"snapshot"), 2),
                Err(coaptic::app::Error::DeferredUnknown)
            ));
            server.transport_mut().fail_at = None;
            server.transport_mut().incoming = Some(peer.request(2, 10, false, 6));
            let mut fresh = None;
            server
                .poll_with(3, |_, candidate| {
                    fresh = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 2);
            server
                .complete(
                    fresh.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    4,
                )
                .unwrap();
            packets(&mut server, &peer, 10, 11, 6);
        });
    }
}

#[test]
fn deferred_qblock2_non_identity_refusal_releases_rx_then_recovers() {
    non_identity_refusal(false);
}
#[test]
fn deferred_qblock2_repeated_non_identity_refusal_releases_rx_then_recovers() {
    non_identity_refusal(true);
}
fn non_identity_refusal(repeated: bool) {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = App::with_resources(
            Resources::<Buffers>::new()
                .routes::<1>()
                .block_wise::<true>()
                .deferred::<2>(),
        )
        .allow_plaintext()
        .randomness(random_for_refusal)
        .bind(Io::default())
        .unwrap();
        let mut peer = Peer::new(&mut server, protected);
        let selections = [
            BlockValue::new(9, false, 6).unwrap(),
            BlockValue::new(10, false, 6).unwrap(),
        ];
        let selections = if repeated {
            &selections[..]
        } else {
            &selections[1..]
        };
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, false, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 1, 6);
            for mid in 2..18 {
                let mut frame =
                    peer.custom(mid, Code::GET, "snapshot", selections, false, None, b"");
                frame.bytes[0] = (frame.bytes[0] & !0x30) | 0x10; // NON; unchanged authenticated inner request.
                server.transport_mut().incoming = Some(frame);
                FAIL_RANDOM.with(|flag| flag.set(true));
                let result = server.poll_with(u64::from(mid), |_, _| {
                    panic!("identity refusal cannot redispatch")
                });
                FAIL_RANDOM.with(|flag| flag.set(false));
                assert!(matches!(
                    result,
                    Err(coaptic::app::Error::Identity(
                        coaptic::app::IdentityError::RandomnessUnavailable
                    ))
                ));
                assert!(server.transport_mut().take().is_none());
                let a = server.engine_mut().acquire_rx().unwrap();
                let b = server.engine_mut().acquire_rx().unwrap();
                let c = server.engine_mut().acquire_rx().unwrap();
                let d = server.engine_mut().acquire_rx().unwrap();
                assert!(server.engine_mut().acquire_rx().is_none());
                for id in [a, b, c, d] {
                    server.engine_mut().release_rx(id).unwrap();
                }
            }
            let mut frame = peer.custom(18, Code::GET, "snapshot", selections, false, None, b"");
            frame.bytes[0] = (frame.bytes[0] & !0x30) | 0x10;
            server.transport_mut().incoming = Some(frame);
            server
                .poll_with(18, |_, _| panic!("fresh NON followup cannot redispatch"))
                .unwrap();
            packets(&mut server, &peer, if repeated { 9 } else { 10 }, 11, 6);
        });
    }
}

fn repeated_campaign<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let mut peer = Peer::new(&mut server, protected);
    without_allocating(|| {
        server.transport_mut().incoming = Some(peer.request(1, 0, false, 6));
        let mut handle = None;
        server
            .poll_with(0, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        ack(&mut server, 1);
        server
            .complete(
                handle.unwrap(),
                Response::content(&BODY)
                    .etag(b"snapshot")
                    .content_format(ContentFormat::OCTET_STREAM),
                1,
            )
            .unwrap();
        packets(&mut server, &peer, 0, 1, 6);
        for (mid, first, last, szx, start, end) in [
            (2, (1, true), (4, false), 6, 1, 10),
            (3, (8, true), (10, false), 6, 8, 11),
            (4, (18, true), (20, false), 5, 18, 21),
        ] {
            server.transport_mut().incoming = Some(peer.custom(
                mid,
                Code::GET,
                "snapshot",
                &[
                    BlockValue::new(first.0, first.1, szx).unwrap(),
                    BlockValue::new(last.0, last.1, szx).unwrap(),
                ],
                false,
                None,
                b"",
            ));
            server
                .poll_with(u64::from(mid), |_, _| {
                    panic!("recovery must not redispatch")
                })
                .unwrap();
            ack(&mut server, mid);
            packets(&mut server, &peer, start, end, szx);
        }
    });
}
#[test]
fn deferred_qblock2_repeated_fixed_overlap_and_downsize() {
    repeated_campaign(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_qblock2_repeated_protected_overlap_and_downsize() {
    repeated_campaign(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_qblock2_repeated_allocated_exact_budget() {
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
        repeated_campaign(server, protected);
    }
}

#[test]
fn deferred_qblock2_repeated_preflight_refuses_whole_union_then_recovers() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, false, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 1, 6);
            for (index, values) in [
                [(4, false, 6), (1, false, 6)],   // unsorted
                [(1, false, 6), (1, false, 6)],   // duplicate
                [(1, false, 6), (2, false, 5)],   // mixed SZX
                [(10, false, 6), (11, false, 6)], // later invalid after body end
                [(1, true, 5), (11, true, 5)],    // eighteen unique payloads
            ]
            .into_iter()
            .enumerate()
            {
                let mid = index as u16 + 2;
                let blocks =
                    values.map(|(num, more, szx)| BlockValue::new(num, more, szx).unwrap());
                server.transport_mut().incoming =
                    Some(peer.custom(mid, Code::GET, "snapshot", &blocks, false, None, b""));
                let before = server.transport_mut().sends;
                server
                    .poll_with(u64::from(mid), |_, _| {
                        panic!("invalid union cannot redispatch")
                    })
                    .unwrap();
                // Exactly one error, no empty ACK or partial selected payload.
                assert_eq!(server.transport_mut().sends, before + 1);
                reply_code(&mut server, &peer, Code::BAD_REQUEST);
                let spare = server.engine_mut().acquire_tx_body().unwrap();
                assert!(server.engine_mut().acquire_tx_body().is_none());
                server.engine_mut().release_tx_body(spare).unwrap();
            }
            let good = BlockValue::new(1, false, 6).unwrap().encode();
            for (mid, bad) in [(8, &[0, 0, 0, 0][..]), (9, &[0x27][..])] {
                let opts = [
                    Opt::uri_path("snapshot"),
                    Opt::q_block2(&good),
                    Opt::new(coaptic::message::OptionNumber::Q_BLOCK2, bad),
                ];
                server.transport_mut().incoming = Some(
                    peer.encode(
                        &Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
                            .with_token(Token::new(b"owner").unwrap())
                            .with_options(&opts),
                    ),
                );
                server
                    .poll_with(u64::from(mid), |_, _| {
                        panic!("malformed union cannot redispatch")
                    })
                    .unwrap();
                reply_code(&mut server, &peer, Code::BAD_OPTION);
            }
            // Exactly ten distinct responses are allowed; the original body remains complete.
            server.transport_mut().incoming = Some(peer.custom(
                10,
                Code::GET,
                "snapshot",
                &[
                    BlockValue::new(1, true, 6).unwrap(),
                    BlockValue::new(10, false, 6).unwrap(),
                ],
                false,
                None,
                b"",
            ));
            server
                .poll_with(10, |_, _| panic!("valid union cannot redispatch"))
                .unwrap();
            ack(&mut server, 10);
            packets(&mut server, &peer, 1, 11, 6);
        });
    }
}

#[test]
fn deferred_qblock2_repeated_failed_send_and_expiry_preserve_bounds() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        without_allocating(|| {
            server.transport_mut().incoming = Some(peer.request(1, 0, false, 6));
            let mut handle = None;
            server
                .poll_with(0, |_, candidate| {
                    handle = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 1);
            server
                .complete(
                    handle.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    1,
                )
                .unwrap();
            packets(&mut server, &peer, 0, 1, 6);
            let selections = [
                BlockValue::new(8, true, 6).unwrap(),
                BlockValue::new(10, false, 6).unwrap(),
            ];
            let mut other = peer.custom(2, Code::GET, "snapshot", &selections, false, None, b"");
            other.peer = Endpoint::v4([192, 0, 2, 3], 5683);
            server.transport_mut().incoming = Some(other);
            let mut calls = 0;
            server
                .poll_with(2, |_, _| {
                    calls += 1;
                    Response::new(Code::SERVICE_UNAVAILABLE)
                })
                .unwrap();
            assert_eq!(calls, 1);
            reply_code(&mut server, &peer, Code::SERVICE_UNAVAILABLE);
            server.transport_mut().incoming =
                Some(peer.custom(3, Code::GET, "snapshot", &selections, false, None, b""));
            server.transport_mut().fail_at = Some(server.transport_mut().sends + 3);
            assert!(matches!(
                server.poll_with(3, |_, _| panic!("owned union cannot redispatch")),
                Err(coaptic::app::Error::Io(_))
            ));
            ack(&mut server, 3);
            packets(&mut server, &peer, 8, 9, 6);
            server.transport_mut().fail_at = None;
            server.transport_mut().incoming =
                Some(peer.custom(4, Code::GET, "snapshot", &selections, false, None, b""));
            server
                .poll_with(39, |_, _| panic!("fresh union cannot redispatch"))
                .unwrap();
            ack(&mut server, 4);
            packets(&mut server, &peer, 8, 11, 6);
            // Selection at deadline-1 must not renew the original lifetime.
            server.poll(40).unwrap();
            let a = server.engine_mut().acquire_tx_body().unwrap();
            let b = server.engine_mut().acquire_tx_body().unwrap();
            server.engine_mut().release_tx_body(a).unwrap();
            server.engine_mut().release_tx_body(b).unwrap();
            server.transport_mut().incoming = Some(peer.request(5, 10, false, 6));
            let mut fresh = None;
            server
                .poll_with(41, |_, candidate| {
                    fresh = candidate;
                    Response::deferred()
                })
                .unwrap();
            ack(&mut server, 5);
            server
                .complete(
                    fresh.unwrap(),
                    Response::content(&BODY)
                        .etag(b"snapshot")
                        .content_format(ContentFormat::OCTET_STREAM),
                    42,
                )
                .unwrap();
            packets(&mut server, &peer, 10, 11, 6);
        });
    }
}
