//! Classic Block1 FETCH retains the bounded cache validator and complete selection.
use coaptic::app::AppStorage;
use coaptic::message::ContentFormat;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, fetch};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static CURRENT: Cell<u8> = const { Cell::new(1) };
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
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
        rx_datagram: (4, 128), tx_datagram: (4, 128),
        rx_body: (2, 1024), tx_body: (2, 1024),
        dedup: 8, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const SELECTION: [u8; 256] = [0x5a; 256];
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 1280],
    len: usize,
    peer: Endpoint,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 1280],
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

fn fixed() -> App<Buffers, Io, 1, true> {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("selection", fetch(selected))
    .bind(Io::default())
    .unwrap()
}

#[test]
fn fragmented_fetch_etag_first_block_is_admitted() {
    let mut client = fixed();
    without_allocating(|| {
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .send(0)
            .expect("bounded FETCH validator should survive Block1 fragmentation");
        assert!(client.transport_mut().take().is_some());
        assert!(client.cancel(call));
    });
}

#[test]
fn fragmented_fetch_without_validator_first_block_fits_small_tx() {
    let mut client = fixed();
    without_allocating(|| {
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .send(0)
            .expect("initial Block1 must fit caller TX slot");
        assert!(client.transport_mut().take().is_some());
        assert!(client.cancel(call));
    });
}

const REPRESENTATION: [u8; 32] = [0x73; 32];
fn selected(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.body(), Some(SELECTION.as_slice()));
    assert_eq!(request.etag().next(), Some(&b"v1"[..]));
    assert_eq!(request.etag().count(), 1);
    DISPATCHES.with(|v| v.set(v.get() + 1));
    if CURRENT.with(Cell::get) == 1 {
        Response::new(Code::VALID).etag(b"v1")
    } else {
        Response::content(&REPRESENTATION).etag(b"v2")
    }
}
#[cfg(feature = "oscore")]
fn context(sender: &[u8], recipient: &[u8]) -> coaptic::oscore::SecurityContext {
    coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
        master_secret: &[0x24; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
type Peer<S> = App<Buffers, Io, 1, true, S>;
fn exercise<C: AppStorage, S: AppStorage>(
    mut client: Peer<C>,
    mut server: Peer<S>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
        if client.oscore().is_none() {
            client.set_oscore(context(&[1], &[2]));
        }
        server.set_oscore(context(&[2], &[1]));
    }
    without_allocating(|| {
        for (case, (version, change)) in [(1, false), (2, false), (1, true), (1, false)]
            .into_iter()
            .enumerate()
        {
            CURRENT.with(|v| v.set(version));
            DISPATCHES.with(|v| v.set(0));
            let now = case as u64 * 100;
            #[cfg(feature = "oscore")]
            let before = client.oscore().map(|ctx| ctx.sender_seq());
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .request_tag(BodyTag::new(b"owner").unwrap())
                .send(now)
                .unwrap();
            #[cfg(feature = "oscore")]
            if let Some(before) = before {
                let ctx = client.oscore().unwrap();
                assert!(
                    ctx.sender_seq() > before + 1,
                    "failed size attempts must burn, never rewind PIVs"
                );
                assert_eq!(
                    ctx.lookup(call.token()).unwrap().piv().seq() + 1,
                    ctx.sender_seq(),
                    "winning ciphertext binding is retained without a second protection"
                );
            }
            let mut blocks = 0;
            let mut complete = false;
            for step in 1..30 {
                if let Some(frame) = client.transport_mut().take() {
                    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                    if protected {
                        assert!(parsed.oscore().is_some());
                    } else {
                        assert_eq!(parsed.etag().next(), Some(&b"v1"[..]));
                        assert_eq!(parsed.request_tag().next(), Some(&b"owner"[..]));
                        let block = parsed.block1().unwrap().unwrap();
                        assert_eq!(block.num(), blocks);
                        assert_eq!(block.szx(), 2);
                        assert_eq!(
                            parsed.payload(),
                            &SELECTION[blocks as usize * 64..(blocks as usize + 1) * 64]
                        );
                    }
                    server.transport_mut().incoming = Some(frame);
                    server.poll(now + step).unwrap();
                    blocks += 1;
                    if blocks == 1 && change {
                        CURRENT.with(|v| v.set(2));
                    }
                    if blocks < 4 {
                        assert_eq!(DISPATCHES.with(Cell::get), 0);
                    }
                }
                if let Some(frame) = server.transport_mut().take() {
                    client.transport_mut().incoming = Some(frame);
                    client.poll(now + step).unwrap();
                }
                if let Some(response) = client.take_response(call) {
                    let response = response.unwrap();
                    let matching = version == 1 && !change;
                    assert_eq!(
                        response.code(),
                        if matching { Code::VALID } else { Code::CONTENT }
                    );
                    assert_eq!(
                        response.body().unwrap_or(response.payload()),
                        if matching {
                            &[][..]
                        } else {
                            &REPRESENTATION[..]
                        }
                    );
                    assert_eq!(
                        response.etag_bytes(),
                        Some(if matching { &b"v1"[..] } else { &b"v2"[..] })
                    );
                    complete = true;
                    break;
                }
            }
            assert!(complete);
            assert_eq!(blocks, 4);
            assert_eq!(DISPATCHES.with(Cell::get), 1);
        }
    });
}
#[test]
fn fetch_etag_complete_selection_validation_and_current_representation() {
    exercise(fixed(), fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn fetch_etag_protected_complete_selection_and_recovery() {
    exercise(fixed(), fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn fetch_etag_allocated_exact_budget_both_peers() {
    use coaptic::storage::{AllocMemory, DatagramSlots};
    let scratch = <AllocMemory as DatagramSlots>::TxScratch::default();
    assert!(
        scratch.as_ref().len() > 128,
        "exercise scratch larger than actual TX"
    );
    let budget = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let make = || {
            App::with_resources(
                Resources::<Buffers>::new()
                    .routes::<1>()
                    .block_wise::<true>()
                    .allocated(budget),
            )
            .allow_plaintext()
            .deterministic_for_tests()
            .route("selection", fetch(selected))
            .bind(Io::default())
            .unwrap()
        };
        exercise(make(), make(), protected);
    }
}

#[test]
fn fetch_etag_changed_omitted_validator_and_wrong_owner_preserve_selection() {
    use coaptic::message::{Message, MessageId, Opt, OptionNumber, OptionsBuilder, encode};
    for mutation in 0..4 {
        let mut client = fixed();
        let mut server = fixed();
        CURRENT.with(|v| v.set(1));
        DISPATCHES.with(|v| v.set(0));
        without_allocating(|| {
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .request_tag(BodyTag::new(b"owner").unwrap())
                .send(0)
                .unwrap();
            server.transport_mut().incoming = client.transport_mut().take();
            server.poll(1).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(1).unwrap();
            let original = client.transport_mut().take().unwrap();
            let parsed = decode(&original.bytes[..original.len]).unwrap();
            let mut options = OptionsBuilder::<16>::new();
            for option in parsed.options() {
                if mutation == 1 && option.number() == OptionNumber::ETAG {
                    continue;
                }
                let option = match (mutation, option.number()) {
                    (0, OptionNumber::ETAG) => Opt::etag(b"v2"),
                    (3, OptionNumber::REQUEST_TAG) => Opt::request_tag(b"other"),
                    _ => option,
                };
                options.push(option).unwrap();
            }
            let poison = [0xee; 64];
            let message = Message::new(parsed.ty(), parsed.code(), MessageId::new(60000))
                .with_token(parsed.token())
                .with_options(options.as_slice())
                .with_payload(&poison);
            let mut altered = Frame::default();
            altered.len = encode(&message, &mut altered.bytes).unwrap();
            if mutation == 2 {
                altered.peer = Endpoint::v4([192, 0, 2, 3], 5683);
            }
            server.transport_mut().incoming = Some(altered);
            server.poll(2).unwrap();
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            let refused = server.transport_mut().take().unwrap();
            assert_eq!(
                decode(&refused.bytes[..refused.len]).unwrap().code(),
                Code::REQUEST_ENTITY_INCOMPLETE
            );
            server.transport_mut().incoming = Some(original);
            server.poll(3).unwrap();
            let mut done = false;
            for now in 3..20 {
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(now).unwrap();
                if let Some(response) = client.take_response(call) {
                    assert_eq!(response.unwrap().code(), Code::VALID);
                    done = true;
                    break;
                }
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(now + 1).unwrap();
            }
            assert!(done);
            assert_eq!(DISPATCHES.with(Cell::get), 1);
        });
    }
}

#[test]
fn fetch_etag_retry_retains_exact_protected_or_plaintext_bytes() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        CURRENT.with(|v| v.set(1));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(context(&[1], &[2]));
            server.set_oscore(context(&[2], &[1]));
        }
        without_allocating(|| {
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .send(0)
                .unwrap();
            let lost = client.transport_mut().take().unwrap();
            client.poll(3000).unwrap();
            let retry = client.transport_mut().take().unwrap();
            assert_eq!(retry.peer, lost.peer);
            assert_eq!(&retry.bytes[..retry.len], &lost.bytes[..lost.len]);
            assert_eq!(
                decode(&retry.bytes[..retry.len])
                    .unwrap()
                    .oscore()
                    .is_some(),
                protected
            );
            server.transport_mut().incoming = Some(retry);
            server.poll(3001).unwrap();
            let mut done = false;
            for now in 3001..3020 {
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(now).unwrap();
                if let Some(response) = client.take_response(call) {
                    assert_eq!(response.unwrap().code(), Code::VALID);
                    done = true;
                    break;
                }
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(now + 1).unwrap();
            }
            assert!(done);
        });
    }
}

#[test]
fn fetch_etag_invalid_unsupported_overflow_and_minimum_size_refuse_then_recover() {
    use coaptic::app::Error;
    let mut client = fixed();
    without_allocating(|| {
        for now in 0..16 {
            for tag in [&[][..], &b"123456789"[..]] {
                assert!(
                    client
                        .fetch("selection")
                        .to(PEER)
                        .content_format(ContentFormat::OCTET_STREAM)
                        .payload(&SELECTION)
                        .etag(tag)
                        .send(now)
                        .is_err()
                );
            }
            assert_eq!(
                client
                    .fetch("selection")
                    .to(PEER)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .payload(&[0; 512])
                    .etag(b"v1")
                    .send(now),
                Err(Error::FetchRequestTooLarge)
            );
            for method in [
                coaptic::Method::Put,
                coaptic::Method::Post,
                coaptic::Method::Patch,
            ] {
                assert_eq!(
                    client
                        .request(method, "selection")
                        .to(PEER)
                        .payload(&SELECTION)
                        .etag(b"v1")
                        .send(now),
                    Err(Error::ConditionalUploadUnsupported)
                );
            }
            let path = core::str::from_utf8(&[b'x'; 120]).unwrap();
            assert!(matches!(
                client
                    .fetch(path)
                    .to(PEER)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .payload(&SELECTION)
                    .etag(b"v1")
                    .send(now),
                Err(Error::Message(coaptic::error::SlotMessageError::Encode(
                    coaptic::error::EncodeError::BufferTooSmall
                )))
            ));
            assert_eq!(client.transport_mut().len, 0);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
        }
    });
    exercise(client, fixed(), false);
}

#[test]
fn fetch_etag_full_body_pool_refuses_without_eviction_then_recovers() {
    let mut client = fixed();
    without_allocating(|| {
        let first = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .non()
            .send(0)
            .unwrap();
        let second = client
            .fetch("selection")
            .to(Endpoint::v4([192, 0, 2, 3], 5683))
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .non()
            .send(0)
            .unwrap();
        for _ in 0..16 {
            assert!(matches!(
                client
                    .fetch("selection")
                    .to(Endpoint::v4([192, 0, 2, 4], 5683))
                    .content_format(ContentFormat::OCTET_STREAM)
                    .payload(&SELECTION)
                    .etag(b"v1")
                    .non()
                    .send(1),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::Saturated
                ))
            ));
            assert_eq!(client.transport_mut().len, 2);
        }
        for call in [first, second] {
            assert!(client.cancel(call));
            assert!(client.take_response(call).unwrap().is_err());
            client.transport_mut().take().unwrap();
        }
    });
    exercise(client, fixed(), false);
}

#[cfg(feature = "oscore")]
#[test]
fn fetch_etag_protected_invalid_and_exhausted_sizing_refuse_without_io() {
    use coaptic::app::Error;
    let mut client = fixed();
    client.set_oscore(context(&[1], &[2]));
    without_allocating(|| {
        for now in 0..8 {
            for tag in [&[][..], &b"123456789"[..]] {
                let before = client.oscore().unwrap().sender_seq();
                assert!(
                    client
                        .fetch("selection")
                        .to(PEER)
                        .content_format(ContentFormat::OCTET_STREAM)
                        .payload(&SELECTION)
                        .etag(tag)
                        .send(now)
                        .is_err()
                );
                assert_eq!(client.oscore().unwrap().sender_seq(), before);
            }
            let before = client.oscore().unwrap().sender_seq();
            let path = core::str::from_utf8(&[b'x'; 120]).unwrap();
            assert!(matches!(
                client
                    .fetch(path)
                    .to(PEER)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .payload(&SELECTION)
                    .etag(b"v1")
                    .send(now),
                Err(Error::Message(coaptic::error::SlotMessageError::Encode(
                    coaptic::error::EncodeError::BufferTooSmall
                )))
            ));
            assert_eq!(
                client.oscore().unwrap().sender_seq(),
                before + 8,
                "one unfragmented and seven candidate protects"
            );
            assert_eq!(client.transport_mut().len, 0);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
            let a = client.engine_mut().acquire_tx_body().unwrap();
            let b = client.engine_mut().acquire_tx_body().unwrap();
            assert!(client.engine_mut().acquire_tx_body().is_none());
            client.engine_mut().release_tx_body(a).unwrap();
            client.engine_mut().release_tx_body(b).unwrap();
        }
    });
    exercise(client, fixed(), true);
    let mut exhausted = fixed();
    let mut ctx = context(&[3], &[4]);
    ctx.set_sender_seq((1 << 40) - 1).unwrap();
    exhausted.set_oscore(ctx);
    assert!(matches!(
        exhausted
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .send(0),
        Err(Error::Oscore(coaptic::oscore::Error::SequenceExhausted))
    ));
    assert_eq!(exhausted.oscore().unwrap().sender_seq(), 1 << 40);
    assert_eq!(exhausted.transport_mut().len, 0);
    assert_eq!(exhausted.engine_mut().tx_occupied(), 0);
}

#[cfg(feature = "oscore")]
coaptic::memory_profile! { ExactBuffers {
    rx_datagram: (4, 128), tx_datagram: (4, 109),
    rx_body: (2, 1024), tx_body: (2, 1024), dedup: 8, observe: 1,
}}
#[cfg(feature = "oscore")]
#[test]
fn fetch_etag_later_piv_growth_refuses_and_releases_then_recovers() {
    let mut client = App::with_resources(
        Resources::<ExactBuffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("s", fetch(selected))
    .bind(Io::default())
    .unwrap();
    let mut server = App::with_resources(
        Resources::<Buffers>::new()
            .routes::<2>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("selection", fetch(selected))
    .route("s", fetch(selected))
    .bind(Io::default())
    .unwrap();
    let mut ctx = context(&[1], &[2]);
    ctx.set_sender_seq(250).unwrap();
    client.set_oscore(ctx);
    server.set_oscore(context(&[2], &[1]));
    without_allocating(|| {
        CURRENT.with(|c| c.set(1));
        DISPATCHES.with(|c| c.set(0));
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .send(0)
            .unwrap();
        let first = client.transport_mut().take().unwrap();
        assert_eq!(first.len, 109);
        assert_eq!(
            client
                .oscore()
                .unwrap()
                .lookup(call.token())
                .unwrap()
                .piv()
                .seq(),
            255
        );
        server.transport_mut().incoming = Some(first);
        server.poll(1).unwrap();
        client.transport_mut().incoming = server.transport_mut().take();
        assert!(matches!(
            client.poll(2),
            Err(coaptic::app::Error::Message(
                coaptic::error::SlotMessageError::Encode(
                    coaptic::error::EncodeError::BufferTooSmall
                )
            ))
        ));
        assert!(matches!(
            client.take_response(call),
            Some(Err(coaptic::CallFailure::ContinuationFailed))
        ));
        assert_eq!(client.oscore().unwrap().sender_seq(), 257);
        assert!(client.oscore().unwrap().lookup(call.token()).is_none());
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        assert_eq!(client.transport_mut().len, 0);
        assert_eq!(DISPATCHES.with(Cell::get), 0);
        let a = client.engine_mut().acquire_tx_body().unwrap();
        let b = client.engine_mut().acquire_tx_body().unwrap();
        client.engine_mut().release_tx_body(a).unwrap();
        client.engine_mut().release_tx_body(b).unwrap();
        let fresh = client
            .fetch("s")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .send(10)
            .unwrap();
        for now in 11..20 {
            if let Some(frame) = client.transport_mut().take() {
                server.transport_mut().incoming = Some(frame);
                server.poll(now).unwrap();
            }
            if let Some(frame) = server.transport_mut().take() {
                client.transport_mut().incoming = Some(frame);
                client.poll(now).unwrap();
            }
            if let Some(response) = client.take_response(fresh) {
                let response = response.unwrap();
                assert_eq!(response.code(), Code::VALID);
                assert_eq!(response.payload(), &[]);
                assert_eq!(DISPATCHES.with(Cell::get), 1);
                return;
            }
        }
        panic!("fresh smaller-header selection did not recover");
    });
}

coaptic::memory_profile! { DownloadBuffers {
    rx_datagram: (4, 1280), tx_datagram: (4, 128),
    rx_body: (2, 4096), tx_body: (2, 4096), dedup: 8, observe: 1,
}}
coaptic::memory_profile! { WideBuffers {
    rx_datagram: (4, 1280), tx_datagram: (4, 1280),
    rx_body: (2, 4096), tx_body: (2, 4096), dedup: 8, observe: 1,
}}
fn large_selected(request: Request<'_>) -> Response<'static> {
    let response = selected(request);
    if CURRENT.with(Cell::get) == 2 {
        Response::content(&[0x74; 2000]).etag(b"v2")
    } else {
        response
    }
}
#[test]
fn fetch_etag_block2_full_selection_followup_limit_refuses_and_recovers() {
    let mut client = App::with_resources(
        Resources::<DownloadBuffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("selection", fetch(large_selected))
    .bind(Io::default())
    .unwrap();
    let mut server = App::with_resources(
        Resources::<WideBuffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("selection", fetch(large_selected))
    .bind(Io::default())
    .unwrap();
    without_allocating(|| {
        for (case, version) in [2, 1].into_iter().enumerate() {
            CURRENT.with(|c| c.set(version));
            DISPATCHES.with(|c| c.set(0));
            let start = case as u64 * 100;
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .send(start)
                .unwrap();
            let mut complete = false;
            let mut blocks = 0;
            for now in start + 1..start + 20 {
                if let Some(frame) = client.transport_mut().take() {
                    let msg = decode(&frame.bytes[..frame.len]).unwrap();
                    assert_eq!(msg.block1().unwrap().unwrap().szx(), 2);
                    blocks += 1;
                    server.transport_mut().incoming = Some(frame);
                    server.poll(now).unwrap();
                }
                if let Some(frame) = server.transport_mut().take() {
                    let msg = decode(&frame.bytes[..frame.len]).unwrap();
                    let large = msg.code() == Code::CONTENT;
                    if large {
                        assert_eq!(msg.payload(), &[0x74; 1024]);
                        assert!(msg.block2().unwrap().unwrap().more());
                    }
                    client.transport_mut().incoming = Some(frame);
                    let result = client.poll(now);
                    if large {
                        assert!(matches!(
                            result,
                            Err(coaptic::app::Error::Message(
                                coaptic::error::SlotMessageError::Encode(
                                    coaptic::error::EncodeError::BufferTooSmall
                                )
                            ))
                        ));
                    } else {
                        result.unwrap();
                    }
                }
                if let Some(response) = client.take_response(call) {
                    if version == 2 {
                        assert!(matches!(
                            response,
                            Err(coaptic::CallFailure::ContinuationFailed)
                        ));
                    } else {
                        let response = response.unwrap();
                        assert_eq!(response.code(), Code::VALID);
                        assert_eq!(response.payload(), &[]);
                    }
                    complete = true;
                    break;
                }
            }
            assert!(complete);
            assert_eq!(blocks, 4);
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(client.transport_mut().len, 0);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
            let a = client.engine_mut().acquire_tx_body().unwrap();
            let b = client.engine_mut().acquire_tx_body().unwrap();
            client.engine_mut().release_tx_body(a).unwrap();
            client.engine_mut().release_tx_body(b).unwrap();
        }
    });
}
