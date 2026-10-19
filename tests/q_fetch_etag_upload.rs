//! Q-Block1 FETCH fits bounded datagrams and retains the complete validator selection.
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

fn fixed() -> App<Buffers, Io, 1, true> {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("selection", fetch(selected))
    .bind(Io::default())
    .unwrap()
}

#[test]
fn q_fetch_etag_first_block_is_admitted() {
    let mut client = fixed();
    without_allocating(|| {
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .q_block1()
            .request_tag(BodyTag::new(b"select").unwrap())
            .etag(b"v1")
            .send(0)
            .expect("bounded FETCH validator should survive Block1 fragmentation");
        assert!(client.transport_mut().take().is_some());
        assert!(client.cancel(call));
    });
}

#[test]
fn q_fetch_without_validator_first_block_fits_small_tx() {
    let mut client = fixed();
    without_allocating(|| {
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .q_block1()
            .request_tag(BodyTag::new(b"select").unwrap())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Schedule {
    Con,
    MissingFirst,
    Reverse,
}
fn transfer<
    C: AppStorage<Token = coaptic::message::Token>,
    S: AppStorage<Token = coaptic::message::Token>,
>(
    client: &mut Peer<C>,
    server: &mut Peer<S>,
    now: u64,
    schedule: Schedule,
    current: u8,
    protected: bool,
) {
    DISPATCHES.with(|v| v.set(0));
    CURRENT.with(|v| v.set(1));
    let mut request = client
        .fetch("selection")
        .to(PEER)
        .payload(&SELECTION)
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"v1")
        .q_block1()
        .request_tag(BodyTag::new(b"select").unwrap())
        .deadline(now + 60000);
    if schedule != Schedule::Con {
        request = request.non();
    }
    let call = request.send(now).unwrap();
    let mut queue = [Frame::default(); 16];
    let mut n = 0;
    while let Some(frame) = client.transport_mut().take() {
        queue[n] = frame;
        n += 1;
    }
    assert!(n > 0);
    if schedule == Schedule::Reverse {
        queue[..n].reverse();
    }
    if schedule == Schedule::MissingFirst {
        assert!(n > 1);
        queue.copy_within(1..n, 0);
        n -= 1;
    }
    let mut clock = now + 1;
    for round in 0..64 {
        for frame in &queue[..n] {
            if !protected && decode(&frame.bytes[..frame.len]).unwrap().code() != Code::EMPTY {
                let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                assert_eq!(
                    parsed.etag().next(),
                    Some(&b"v1"[..]),
                    "round={round} schedule={schedule:?} code={:?}",
                    parsed.code()
                );
                assert!(parsed.block1().is_none());
                let block = parsed.q_block1().unwrap().unwrap();
                let offset = block.num() as usize * usize::from(block.size());
                assert_eq!(
                    parsed.payload(),
                    &SELECTION[offset..offset + parsed.payload().len()]
                );
            }
            assert!(frame.len <= 128);
            server.transport_mut().incoming = Some(*frame);
            server.poll(clock).unwrap();
            clock += 1;
            // Change current representation after admission but before completion.
            CURRENT.with(|v| v.set(current));
            while let Some(reply) = server.transport_mut().take() {
                client.transport_mut().incoming = Some(reply);
                client.poll(clock).unwrap();
                clock += 1;
            }
        }
        if let Some(reply) = client.take_response(call) {
            let reply = reply.unwrap();
            assert_eq!(
                reply.code(),
                if current == 1 {
                    Code::VALID
                } else {
                    Code::CONTENT
                }
            );
            assert_eq!(
                reply.payload(),
                if current == 1 {
                    &[][..]
                } else {
                    &REPRESENTATION[..]
                }
            );
            assert_eq!(
                reply.etag_bytes(),
                Some(if current == 1 { &b"v1"[..] } else { &b"v2"[..] })
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            return;
        }
        n = 0;
        while let Some(frame) = client.transport_mut().take() {
            queue[n] = frame;
            n += 1;
        }
        if n == 0 {
            assert_eq!(
                schedule,
                Schedule::MissingFirst,
                "no progress round={round}"
            );
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            clock += u64::from(coaptic::message::QBlockTransmission::NON_RECEIVE_TIMEOUT_MS);
            server.poll(clock).unwrap();
            let report = server
                .transport_mut()
                .take()
                .expect("bounded missing report");
            if !protected {
                assert_eq!(
                    decode(&report.bytes[..report.len]).unwrap().code(),
                    Code::REQUEST_ENTITY_INCOMPLETE
                );
            }
            client.transport_mut().incoming = Some(report);
            client.poll(clock).unwrap();
            clock += 1;
            while let Some(frame) = client.transport_mut().take() {
                queue[n] = frame;
                n += 1;
            }
            assert_eq!(n, 1, "only NUM0 was lost");
        }
    }
    panic!("finite transfer did not finish");
}
fn exercise<
    C: AppStorage<Token = coaptic::message::Token>,
    S: AppStorage<Token = coaptic::message::Token>,
>(
    mut client: Peer<C>,
    mut server: Peer<S>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    without_allocating(|| {
        transfer(&mut client, &mut server, 0, Schedule::Con, 1, protected);
        transfer(
            &mut client,
            &mut server,
            100000,
            Schedule::Con,
            2,
            protected,
        );
        transfer(
            &mut client,
            &mut server,
            200000,
            Schedule::MissingFirst,
            1,
            protected,
        );
        if !protected {
            transfer(
                &mut client,
                &mut server,
                300000,
                Schedule::Reverse,
                2,
                false,
            );
        }
        transfer(
            &mut client,
            &mut server,
            400000,
            Schedule::Con,
            1,
            protected,
        );
    });
}
#[test]
fn q_fetch_etag_complete_current_selection_missing_reorder_and_recovery() {
    exercise(fixed(), fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn q_fetch_etag_protected_complete_selection_and_missing_reissue() {
    exercise(fixed(), fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn q_fetch_etag_allocated_exact_budget_both_peers() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
    };
    let bytes = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let make = || {
            App::with_resources(resources().allocated(bytes))
                .allow_unprotected()
                .deterministic_for_tests()
                .route("selection", fetch(selected))
                .bind(Io::default())
                .unwrap()
        };
        exercise(make(), make(), protected);
    }
}

#[test]
fn q_fetch_etag_changed_omitted_validator_and_wrong_owner_preserve_selection() {
    use coaptic::message::{Message, MessageId, Opt, OptionNumber, OptionsBuilder, encode};
    for mutation in 0..7 {
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
                .q_block1()
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
                    (4, OptionNumber::ETAG) => Opt::etag(b""),
                    (5, OptionNumber::ETAG) => Opt::etag(b"123456789"),
                    (3, OptionNumber::REQUEST_TAG) => Opt::request_tag(b"other"),
                    _ => option,
                };
                options.push(option).unwrap();
                if mutation == 6 && option.number() == OptionNumber::ETAG {
                    options.push(Opt::etag(b"v1")).unwrap();
                }
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
            // A different peer/tag owns a separate incomplete Q transfer; its
            // empty ACK must not complete or contaminate the original selection.
            let refused = server.transport_mut().take().unwrap();
            assert_eq!(
                decode(&refused.bytes[..refused.len]).unwrap().code(),
                if mutation == 2 || mutation == 3 {
                    Code::EMPTY
                } else {
                    Code::REQUEST_ENTITY_INCOMPLETE
                },
                "mutation={mutation}"
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
            transfer(&mut client, &mut server, 100000, Schedule::Con, 2, false);
        });
    }
}

#[test]
fn q_fetch_etag_retry_retains_exact_protected_or_plaintext_bytes() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        CURRENT.with(|v| v.set(1));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
        without_allocating(|| {
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .q_block1()
                .request_tag(BodyTag::new(b"owner").unwrap())
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
fn q_fetch_etag_invalid_unsupported_overflow_and_minimum_size_refuse_then_recover() {
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
                        .q_block1()
                        .request_tag(BodyTag::new(b"select").unwrap())
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
                    .q_block1()
                    .request_tag(BodyTag::new(b"select").unwrap())
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
                        .q_block1()
                        .request_tag(BodyTag::new(b"select").unwrap())
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
                    .q_block1()
                    .request_tag(BodyTag::new(b"select").unwrap())
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
fn q_fetch_etag_full_body_pool_refuses_without_eviction_then_recovers() {
    let mut client = fixed();
    without_allocating(|| {
        let first = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .q_block1()
            .request_tag(BodyTag::new(b"select").unwrap())
            .send(0)
            .unwrap();
        let second = client
            .fetch("selection")
            .to(Endpoint::v4([192, 0, 2, 3], 5683))
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .q_block1()
            .request_tag(BodyTag::new(b"select").unwrap())
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
                    .q_block1()
                    .request_tag(BodyTag::new(b"select").unwrap())
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
fn q_fetch_etag_protected_invalid_and_exhausted_sizing_refuse_without_io() {
    use coaptic::app::Error;
    let mut client = fixed();
    client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
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
                        .q_block1()
                        .request_tag(BodyTag::new(b"select").unwrap())
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
                    .q_block1()
                    .request_tag(BodyTag::new(b"select").unwrap())
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
    let mut server = fixed();
    server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    let before = client.oscore().unwrap().sender_seq();
    transfer(&mut client, &mut server, 100000, Schedule::Con, 1, true);
    assert!(client.oscore().unwrap().sender_seq() > before);
    let mut exhausted = fixed();
    let mut ctx = context(&[3], &[4]);
    ctx.set_sender_seq((1 << 40) - 1).unwrap();
    exhausted.set_oscore(&mut Some(ctx)).unwrap();
    assert!(matches!(
        exhausted
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .etag(b"v1")
            .q_block1()
            .request_tag(BodyTag::new(b"select").unwrap())
            .send(0),
        Err(Error::Oscore(coaptic::oscore::Error::SequenceExhausted))
    ));
    assert_eq!(exhausted.oscore().unwrap().sender_seq(), 1 << 40);
    assert_eq!(exhausted.transport_mut().len, 0);
    assert_eq!(exhausted.engine_mut().tx_occupied(), 0);
}

coaptic::memory_profile! { WindowBuffers {
    rx_datagram: (4, 1280), tx_datagram: (4, 1280),
    rx_body: (2, 12288), tx_body: (2, 12288), dedup: 8, observe: 1,
}}
#[test]
fn q_upload_first_preencoded_non_counts_once_in_ten_block_set() {
    let mut client = App::with_resources(Resources::<WindowBuffers>::new().block_wise::<true>())
        .allow_unprotected()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap();
    let body = [0x61; 11000];
    without_allocating(|| {
        let call = client
            .put("upload")
            .to(PEER)
            .payload(&body)
            .non()
            .q_block1()
            .request_tag(BodyTag::new(b"window").unwrap())
            .send(0)
            .unwrap();
        assert_eq!(client.transport_mut().len, 10);
        let mut previous_mid = None;
        for num in 0..10 {
            let frame = client.transport_mut().take().unwrap();
            let message = decode(&frame.bytes[..frame.len]).unwrap();
            let block = message.q_block1().unwrap().unwrap();
            assert_eq!(block.num(), num);
            assert_eq!(block.size(), 1024);
            assert!(block.more());
            assert_eq!(
                message.payload(),
                &body[num as usize * 1024..(num as usize + 1) * 1024]
            );
            assert_ne!(Some(message.message_id()), previous_mid);
            previous_mid = Some(message.message_id());
        }
        client.poll(1).unwrap();
        assert_eq!(
            client.transport_mut().len,
            0,
            "next set must wait for control"
        );
        assert!(client.cancel(call));
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        let fresh = client.get("fresh").to(PEER).send(2).unwrap();
        assert_eq!(client.transport_mut().len, 1);
        assert!(client.cancel(fresh));
    });
}

#[cfg(feature = "oscore")]
#[test]
fn q_fetch_protected_body_admission_refusal_reclaims_single_request_binding() {
    let mut client = App::with_resources(
        Resources::<Buffers>::new()
            .block_wise::<true>()
            .security_requests::<1>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .bind(Io::default())
    .unwrap();
    client
        .set_oscore(&mut Some(
            coaptic::oscore::BoundedSecurityContext::<1>::derive(coaptic::oscore::DeriveParams {
                master_secret: &[0x24; 16],
                master_salt: &[],
                sender_id: &[1],
                recipient_id: &[2],
                id_context: &[],
            })
            .unwrap(),
        ))
        .unwrap();
    without_allocating(|| {
        let a = client.engine_mut().acquire_tx_body().unwrap();
        let b = client.engine_mut().acquire_tx_body().unwrap();
        for now in 0..16 {
            let before = client.oscore().unwrap().sender_seq();
            assert!(matches!(
                client
                    .fetch("selection")
                    .to(PEER)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .payload(&SELECTION)
                    .etag(b"v1")
                    .q_block1()
                    .request_tag(BodyTag::new(b"select").unwrap())
                    .send(now),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::Saturated
                ))
            ));
            assert!(client.oscore().unwrap().sender_seq() > before);
            assert_eq!(client.transport_mut().len, 0);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
        }
        client.engine_mut().release_tx_body(a).unwrap();
        client.engine_mut().release_tx_body(b).unwrap();
        for now in 20..24 {
            let call = client
                .fetch("selection")
                .to(PEER)
                .content_format(ContentFormat::OCTET_STREAM)
                .payload(&SELECTION)
                .etag(b"v1")
                .q_block1()
                .request_tag(BodyTag::new(b"select").unwrap())
                .send(now)
                .unwrap();
            assert!(client.oscore().unwrap().lookup(call.token()).is_some());
            assert!(client.transport_mut().take().is_some());
            assert!(client.cancel(call));
            assert!(client.oscore().unwrap().lookup(call.token()).is_none());
            assert!(client.take_response(call).unwrap().is_err());
        }
    });
}

const LARGE_SELECTED: [u8; 384] = [0x44; 384];
fn fragmented_selected(request: Request<'_>) -> Response<'static> {
    assert_eq!(request.body().unwrap_or(request.payload()), &SELECTION);
    Response::content(&LARGE_SELECTED).etag(b"snapshot")
}
fn fragmented_observation(protected: bool, deferred: bool) {
    let mut client = fixed();
    let mut server = App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route(
        "selection",
        fetch(fragmented_selected).observe_selected(fragmented_selected),
    )
    .bind(Io::default())
    .unwrap();
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    without_allocating(|| {
        let call = client
            .fetch("selection")
            .to(PEER)
            .content_format(ContentFormat::OCTET_STREAM)
            .payload(&SELECTION)
            .observe()
            .block2(coaptic::message::BlockValue::new(0, false, 1).unwrap())
            .send(0)
            .unwrap();
        let mut clock = 1;
        let mut completed = 0;
        for round in 0..100 {
            while let Some(frame) = client.transport_mut().take() {
                server.transport_mut().incoming = Some(frame);
                if deferred {
                    let mut handle = None;
                    server
                        .poll_with(clock, |request, pending| {
                            assert_eq!(request.body().unwrap_or(request.payload()), &SELECTION);
                            if request.is_observe_register() {
                                handle = pending;
                                {
                                    assert!(pending.is_some(), "fragmented selection admitted");
                                    Response::deferred()
                                }
                            } else {
                                fragmented_selected(request)
                            }
                        })
                        .unwrap();
                    if let Some(handle) = handle {
                        server
                            .complete(
                                handle,
                                Response::content(&LARGE_SELECTED).etag(b"snapshot"),
                                clock + 1,
                            )
                            .unwrap();
                    }
                } else {
                    server.poll(clock).unwrap();
                }
                clock += 2;
            }
            while let Some(frame) = server.transport_mut().take() {
                client.transport_mut().incoming = Some(frame);
                client.poll(clock).unwrap();
                clock += 1;
            }
            if let Some(reply) = client.take_response(call) {
                let reply = reply.unwrap();
                assert_eq!(reply.body(), Some(LARGE_SELECTED.as_slice()));
                assert!(reply.observe_seq().is_some());
                completed += 1;
                if completed == 2 {
                    return;
                }
                assert_eq!(server.signal(&["selection"]), 1);
                server.poll(clock).unwrap();
                clock += 1;
                continue;
            }
            assert!(
                client.transport().len != 0,
                "fragmented follow-up stopped at round {round}, protected={protected}"
            );
        }
        panic!("fragmented selected observation did not complete");
    });
}
#[test]
fn fragmented_observe_selection_and_block2_followups_complete() {
    for deferred in [false, true] {
        fragmented_observation(false, deferred);
        #[cfg(feature = "oscore")]
        fragmented_observation(true, deferred);
    }
}
