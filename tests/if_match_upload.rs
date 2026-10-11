//! Bounded fragmented If-Match retains wire identity and checks at complete-body dispatch.
use coaptic::app::AppStorage;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, put};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static REPRESENTATION: Cell<Option<u8>> = const { Cell::new(Some(1)) };
    static EFFECTS: Cell<usize> = const { Cell::new(0) };
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static EMPTY_MATCH: Cell<bool> = const { Cell::new(false) };
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
        rx_datagram: (4, 1280), tx_datagram: (4, 1280),
        rx_body: (2, 4096), tx_body: (2, 4096),
        dedup: 8, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const BODY: [u8; 2000] = [0x5a; 2000];
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
fn replace(request: Request<'_>) -> Response<'static> {
    DISPATCHES.with(|v| v.set(v.get() + 1));
    assert_eq!(request.body(), Some(BODY.as_slice()));
    let expected: &[u8] = if EMPTY_MATCH.with(Cell::get) {
        &[]
    } else {
        &[1]
    };
    assert_eq!(request.if_match().next(), Some(expected));
    let version = REPRESENTATION.with(Cell::get);
    let etag = version.map(|v| [v]);
    if !request
        .precondition(version.is_some(), etag.as_ref().map(|v| v.as_slice()))
        .holds()
    {
        return Response::new(Code::PRECONDITION_FAILED);
    }
    EFFECTS.with(|v| v.set(v.get() + 1));
    Response::new(Code::CHANGED)
}
type Client<S> = App<Buffers, Io, 1, true, S>;
type Server = App<Buffers, Io, 1, true>;
fn server() -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .route("upload", put(replace))
    .bind(Io::default())
    .unwrap()
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
fn exercise<S: AppStorage>(
    mut client: Client<S>,
    qblock: bool,
    empty: bool,
    initial: Option<u8>,
    changed: Option<Option<u8>>,
    protected: bool,
) {
    REPRESENTATION.with(|v| v.set(initial));
    EFFECTS.with(|v| v.set(0));
    DISPATCHES.with(|v| v.set(0));
    EMPTY_MATCH.with(|v| v.set(empty));
    let mut server = server();
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(context(&[1], &[2]));
        server.set_oscore(context(&[2], &[1]));
    }
    without_allocating(|| {
        for phase in 0..2 {
            let empty = phase == 0 && empty;
            if phase == 1 {
                REPRESENTATION.with(|v| v.set(Some(1)));
                EMPTY_MATCH.with(|v| v.set(false));
            }
            let prior_effects = EFFECTS.with(Cell::get);
            let expected: &[u8] = if empty { &[] } else { &[1] };
            let mut outgoing = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_match(expected)
                .request_tag(BodyTag::new(b"transfer").unwrap());
            if qblock {
                outgoing = outgoing.q_block1();
            }
            let call = outgoing.send(phase * 100).unwrap();
            let mut blocks = 0;
            let mut completed = false;
            for now in (phase * 100 + 1)..(phase * 100 + 40) {
                if let Some(frame) = client.transport_mut().take() {
                    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                    if protected {
                        assert!(parsed.oscore().is_some());
                        assert!(parsed.if_match().next().is_none());
                    } else {
                        assert_eq!(parsed.if_match().next(), Some(expected));
                    }
                    server.transport_mut().incoming = Some(frame);
                    server.poll(now).unwrap();
                    blocks += 1;
                    if blocks == 1 {
                        assert_eq!(
                            DISPATCHES.with(Cell::get),
                            phase as usize,
                            "partial ACK must not dispatch"
                        );
                        assert_eq!(EFFECTS.with(Cell::get), prior_effects);
                        if phase == 0 {
                            if let Some(value) = changed {
                                REPRESENTATION.with(|v| v.set(value));
                            }
                        }
                    }
                }
                if let Some(frame) = server.transport_mut().take() {
                    client.transport_mut().incoming = Some(frame);
                    client.poll(now).unwrap();
                }
                if let Some(response) = client.take_response(call) {
                    let current = REPRESENTATION.with(Cell::get);
                    let succeeds = if empty {
                        current.is_some()
                    } else {
                        current == Some(1)
                    };
                    assert_eq!(
                        response.unwrap().code(),
                        if succeeds {
                            Code::CHANGED
                        } else {
                            Code::PRECONDITION_FAILED
                        }
                    );
                    assert_eq!(DISPATCHES.with(Cell::get), phase as usize + 1);
                    assert_eq!(
                        EFFECTS.with(Cell::get),
                        prior_effects + usize::from(succeeds)
                    );
                    assert_eq!(blocks, 2);
                    completed = true;
                    break;
                }
            }
            assert!(completed, "fragmented If-Match did not complete");
        }
    });
}
fn fixed(
    qblock: bool,
    empty: bool,
    initial: Option<u8>,
    changed: Option<Option<u8>>,
    protected: bool,
) {
    exercise(
        App::with_resources(
            Resources::<Buffers>::new()
                .routes::<1>()
                .block_wise::<true>(),
        )
        .allow_plaintext()
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap(),
        qblock,
        empty,
        initial,
        changed,
        protected,
    );
}
#[test]
fn if_match_fixed_completion_checks_current_representation() {
    for qblock in [false, true] {
        for (empty, initial, changed) in [
            (false, Some(1), None),
            (false, Some(2), None),
            (false, Some(1), Some(Some(2))),
            (true, Some(2), None),
            (true, None, None),
            (true, Some(1), Some(None)),
        ] {
            fixed(qblock, empty, initial, changed, false);
        }
    }
}
#[cfg(feature = "oscore")]
#[test]
fn if_match_protected_inner_completion_checks_current_representation() {
    for qblock in [false, true] {
        for (empty, initial, changed) in [
            (false, Some(1), None),
            (false, Some(2), None),
            (false, Some(1), Some(Some(2))),
            (true, Some(2), None),
            (true, None, None),
            (true, Some(1), Some(None)),
        ] {
            fixed(qblock, empty, initial, changed, true);
        }
    }
}
#[cfg(feature = "alloc")]
#[test]
fn if_match_allocated_completion_has_identical_bounds() {
    let heap_bytes = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for qblock in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            for changed in [None, Some(Some(2))] {
                exercise(
                    App::with_resources(
                        Resources::<Buffers>::new()
                            .routes::<1>()
                            .block_wise::<true>()
                            .allocated(heap_bytes),
                    )
                    .allow_plaintext()
                    .deterministic_for_tests()
                    .bind(Io::default())
                    .unwrap(),
                    qblock,
                    false,
                    Some(1),
                    changed,
                    protected,
                );
            }
        }
    }
}

fn fresh_client() -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .deterministic_for_tests()
    .bind(Io::default())
    .unwrap()
}

#[test]
fn if_match_changed_condition_refuses_and_distinct_owners_do_not_cross_complete() {
    use coaptic::message::{Message, MessageId, Opt, OptionNumber, OptionsBuilder, encode};
    for qblock in [false, true] {
        for mutation in 0..3 {
            let mut client = fresh_client();
            let mut server = server();
            REPRESENTATION.with(|v| v.set(Some(1)));
            EMPTY_MATCH.with(|v| v.set(false));
            EFFECTS.with(|v| v.set(0));
            DISPATCHES.with(|v| v.set(0));
            without_allocating(|| {
                let mut request = client
                    .put("upload")
                    .to(PEER)
                    .payload(&BODY)
                    .if_match(&[1])
                    .request_tag(BodyTag::new(b"transfer").unwrap());
                if qblock {
                    request = request.q_block1();
                }
                let call = request.send(0).unwrap();
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(1).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(1).unwrap();
                let original = client.transport_mut().take().unwrap();
                let parsed = decode(&original.bytes[..original.len]).unwrap();
                let mut options = OptionsBuilder::<16>::new();
                for option in parsed.options() {
                    let option = match (mutation, option.number()) {
                        (0, OptionNumber::IF_MATCH) => Opt::if_match(&[2]),
                        (1, OptionNumber::REQUEST_TAG) => Opt::request_tag(b"another"),
                        _ => option,
                    };
                    options.push(option).unwrap();
                }
                let poison = [0xee; 976];
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
                assert_eq!(EFFECTS.with(Cell::get), 0);
                if let Some(reply) = server.transport_mut().take() {
                    let parsed = decode(&reply.bytes[..reply.len]).unwrap();
                    if mutation == 0 || !qblock {
                        assert_eq!(parsed.code(), Code::REQUEST_ENTITY_INCOMPLETE);
                    } else {
                        assert!(parsed.is_empty_ack());
                    }
                } else {
                    assert!(qblock && mutation != 0);
                }
                // The original owner still completes the exact body, never the poison bytes.
                server.transport_mut().incoming = Some(original);
                server.poll(3).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3).unwrap();
                assert_eq!(
                    client.take_response(call).unwrap().unwrap().code(),
                    Code::CHANGED
                );
                assert_eq!(DISPATCHES.with(Cell::get), 1);
                assert_eq!(EFFECTS.with(Cell::get), 1);
            });
        }
    }
}

#[test]
fn if_match_qblock_missing_reissue_preserves_condition_and_protected_inner() {
    use coaptic::message::QBlockTransmission;
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fresh_client();
        let mut server = server();
        REPRESENTATION.with(|v| v.set(Some(1)));
        EMPTY_MATCH.with(|v| v.set(false));
        EFFECTS.with(|v| v.set(0));
        DISPATCHES.with(|v| v.set(0));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(context(&[1], &[2]));
            server.set_oscore(context(&[2], &[1]));
        }
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_match(&[1])
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .q_block1()
                .non()
                .send(0)
                .unwrap();
            let missing = client.transport_mut().take().unwrap();
            let final_block = client.transport_mut().take().unwrap();
            server.transport_mut().incoming = Some(final_block);
            server.poll(1).unwrap();
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            assert!(server.transport_mut().take().is_none());
            let due = 1 + u64::from(QBlockTransmission::NON_RECEIVE_TIMEOUT_MS);
            server.poll(due).unwrap();
            let report = server.transport_mut().take().unwrap();
            if protected {
                assert!(
                    decode(&report.bytes[..report.len])
                        .unwrap()
                        .oscore()
                        .is_some()
                );
            }
            client.transport_mut().incoming = Some(report);
            client.poll(due).unwrap();
            let reissued = client.transport_mut().take().unwrap();
            let parsed = decode(&reissued.bytes[..reissued.len]).unwrap();
            if protected {
                assert!(parsed.oscore().is_some());
                assert!(parsed.if_match().next().is_none());
            } else {
                assert_eq!(parsed.if_match().next(), Some(&[1][..]));
                assert_eq!(parsed.q_block1().unwrap().unwrap().num(), 0);
                assert_eq!(
                    parsed.payload(),
                    decode(&missing.bytes[..missing.len]).unwrap().payload()
                );
            }
            server.transport_mut().incoming = Some(reissued);
            server.poll(due + 1).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(due + 1).unwrap();
            assert_eq!(
                client.take_response(call).unwrap().unwrap().code(),
                Code::CHANGED
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), 1);
        });
    }
}

#[test]
fn if_match_encode_refusal_and_cancel_release_bounded_ownership() {
    // Classic Block1 now selects a smaller initial fragment when the condition
    // pushes the 1024-byte candidate beyond TX capacity; Q retains explicit refusal.
    let long_path = core::str::from_utf8(&[b'x'; 255]).unwrap();
    let mut client = fresh_client();
    without_allocating(|| {
        for now in 0..16 {
            let call = client
                .put(long_path)
                .to(PEER)
                .payload(&BODY)
                .if_match(b"12345678")
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(now)
                .unwrap();
            let frame = client.transport_mut().take().unwrap();
            let parsed = decode(&frame.bytes[..frame.len]).unwrap();
            assert_eq!(parsed.block1().unwrap().unwrap().szx(), 5);
            assert_eq!(parsed.if_match().next(), Some(&b"12345678"[..]));
            assert_eq!(parsed.payload(), &BODY[..512]);
            assert!(client.cancel(call));
            assert!(client.take_response(call).unwrap().is_err());
            assert_eq!(client.transport_mut().len, 0);
        }
    });
    exercise(client, false, false, Some(1), None, false);
    {
        static PATH_BYTES: [u8; 255] = [b'x'; 255];
        let mut found = false;
        for length in 200..=255 {
            let path = core::str::from_utf8(&PATH_BYTES[..length]).unwrap();
            let mut client = fresh_client();
            let plain = client
                .put(path)
                .to(PEER)
                .payload(&BODY)
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .q_block1();
            let Ok(call) = plain.send(0) else {
                continue;
            };
            assert!(client.cancel(call));
            assert!(client.take_response(call).unwrap().is_err());
            client.transport_mut().take().unwrap();
            let conditioned = client
                .put(path)
                .to(PEER)
                .payload(&BODY)
                .if_match(b"12345678")
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .q_block1();
            let result = conditioned.send(1);
            if let Ok(call) = result {
                client.cancel(call);
                client.take_response(call);
                continue;
            }
            assert!(matches!(
                result,
                Err(coaptic::app::Error::Message(
                    coaptic::error::SlotMessageError::Encode(
                        coaptic::error::EncodeError::BufferTooSmall
                    )
                ))
            ));
            found = true;
            assert_eq!(
                client.transport_mut().len,
                0,
                "added option must not escape truncated"
            );
            without_allocating(|| {
                // More than the two body slots and default Call capacity: every failed encode
                // and cancellation must return its ownership before the next admission.
                for now in 2..18 {
                    let request = client
                        .put(path)
                        .to(PEER)
                        .payload(&BODY)
                        .if_match(b"12345678")
                        .request_tag(BodyTag::new(b"transfer").unwrap())
                        .q_block1();
                    assert!(matches!(
                        request.send(now),
                        Err(coaptic::app::Error::Message(
                            coaptic::error::SlotMessageError::Encode(
                                coaptic::error::EncodeError::BufferTooSmall
                            )
                        ))
                    ));
                    assert_eq!(client.transport_mut().len, 0);
                    let request = client
                        .put("upload")
                        .to(PEER)
                        .payload(&BODY)
                        .if_match(&[1])
                        .request_tag(BodyTag::new(b"transfer").unwrap())
                        .q_block1();
                    let call = request.send(now).unwrap();
                    assert!(client.cancel(call));
                    assert_eq!(
                        client.take_response(call).unwrap().unwrap_err(),
                        coaptic::app::CallFailure::Cancelled
                    );
                    assert!(client.transport_mut().take().is_some());
                    assert_eq!(client.transport_mut().len, 0);
                }
            });
            exercise(client, true, false, Some(1), None, false);
            break;
        }
        assert!(
            found,
            "fixture must exercise an added-If-Match datagram capacity boundary"
        );
    }
}

#[test]
fn if_match_con_retry_is_exact_and_completes_after_loss() {
    for qblock in [false, true] {
        for protected in [false, cfg!(feature = "oscore")] {
            let mut client = fresh_client();
            let mut server = server();
            REPRESENTATION.with(|v| v.set(Some(1)));
            EMPTY_MATCH.with(|v| v.set(false));
            EFFECTS.with(|v| v.set(0));
            DISPATCHES.with(|v| v.set(0));
            #[cfg(feature = "oscore")]
            if protected {
                client.set_oscore(context(&[1], &[2]));
                server.set_oscore(context(&[2], &[1]));
            }
            without_allocating(|| {
                let mut request = client
                    .put("upload")
                    .to(PEER)
                    .payload(&BODY)
                    .if_match(&[1])
                    .request_tag(BodyTag::new(b"transfer").unwrap());
                if qblock {
                    request = request.q_block1();
                }
                let call = request.send(0).unwrap();
                let lost = client.transport_mut().take().unwrap();
                client.poll(3000).unwrap();
                let retry = client.transport_mut().take().unwrap();
                assert_eq!(retry.peer, lost.peer);
                assert_eq!(&retry.bytes[..retry.len], &lost.bytes[..lost.len]);
                if !protected {
                    assert_eq!(
                        decode(&retry.bytes[..retry.len]).unwrap().if_match().next(),
                        Some(&[1][..])
                    );
                }
                server.transport_mut().incoming = Some(retry);
                server.poll(3001).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3001).unwrap();
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(3002).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3002).unwrap();
                assert_eq!(
                    client.take_response(call).unwrap().unwrap().code(),
                    Code::CHANGED
                );
                assert_eq!(DISPATCHES.with(Cell::get), 1);
                assert_eq!(EFFECTS.with(Cell::get), 1);
            });
        }
    }
}
