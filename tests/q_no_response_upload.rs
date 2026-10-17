//! Q-Block1 server-error suppression retains progress and explicit uncertain outcomes.
use coaptic::app::AppStorage;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, put};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static STATUS: Cell<Code> = const { Cell::new(Code::CHANGED) };
    static MASK: Cell<u8> = const { Cell::new(8) };
    static EFFECTS: Cell<usize> = const { Cell::new(0) };
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
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
        rx_body: (2, 12288), tx_body: (2, 12288),
        dedup: 8, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const BODY: [u8; 11264] = [0x5a; 11264];
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

fn upload(request: Request<'_>) -> Response<'static> {
    DISPATCHES.with(|v| v.set(v.get() + 1));
    assert_eq!(request.body(), Some(BODY.as_slice()));
    assert_eq!(
        request.no_response(),
        Some(Ok(coaptic::message::NoResponse::new(MASK.with(Cell::get))))
    );
    let code = STATUS.with(Cell::get);
    if code == Code::CHANGED {
        EFFECTS.with(|v| v.set(v.get() + 1));
    }
    Response::new(code)
}
type Peer<S> = App<Buffers, Io, 1, true, S>;
fn fixed() -> App<Buffers, Io, 1, true> {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_unprotected()
    .deterministic_for_tests()
    .route("upload", put(upload))
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
fn exercise<C: AppStorage, S: AppStorage>(
    mut client: Peer<C>,
    mut server: Peer<S>,
    protected: bool,
) {
    use coaptic::app::CallFailure;
    use coaptic::message::NoResponse;
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    EFFECTS.with(|v| v.set(0));
    DISPATCHES.with(|v| v.set(0));
    without_allocating(|| {
        refuse_and_reclaim(&mut client);
        let mut now = 100;
        for mask in [16] {
            for non in [false, true] {
                MASK.with(|v| v.set(mask));
                // Repeated refused outcomes exceed Call/body capacity; a successful operation
                // after each proves reclamation on the same client and server.
                for outcome in 0..4 {
                    let status = match outcome {
                        0 => Code::PRECONDITION_FAILED,
                        1 => Code::SERVICE_UNAVAILABLE,
                        _ => Code::CHANGED,
                    };
                    STATUS.with(|v| v.set(status));
                    let effects = EFFECTS.with(Cell::get);
                    let mut request = client
                        .put("upload")
                        .to(PEER)
                        .payload(&BODY)
                        .q_block1()
                        .request_tag(BodyTag::new(b"transfer").unwrap())
                        .no_response(NoResponse::new(mask))
                        .deadline(now + 100);
                    if non {
                        request = request.non();
                    }
                    let call = request.send(now).unwrap();
                    let mut blocks = 0;
                    for step in 1..60 {
                        if let Some(frame) = client.transport_mut().take() {
                            let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                            if protected {
                                assert!(parsed.oscore().is_some());
                            } else {
                                assert_eq!(parsed.no_response(), Some(Ok(NoResponse::new(mask))));
                            }
                            server.transport_mut().incoming = Some(frame);
                            server.poll(now + step).unwrap();
                            blocks += 1;
                        }
                        if let Some(frame) = server.transport_mut().take() {
                            client.transport_mut().incoming = Some(frame);
                            client.poll(now + step).unwrap();
                        }
                    }
                    assert_eq!(blocks, 11, "Continue must advance the entire upload");
                    let suppressed = (status == Code::PRECONDITION_FAILED && mask & 8 != 0)
                        || (status == Code::SERVICE_UNAVAILABLE && mask & 16 != 0);
                    if suppressed {
                        assert!(
                            client.take_response(call).is_none(),
                            "silence/empty ACK is not success"
                        );
                        if non {
                            assert!(client.cancel(call));
                            assert_eq!(
                                client.take_response(call).unwrap().unwrap_err(),
                                CallFailure::Cancelled
                            );
                        } else {
                            client.poll(now + 100).unwrap();
                            assert_eq!(
                                client.take_response(call).unwrap().unwrap_err(),
                                CallFailure::DeadlineExceeded
                            );
                        }
                    } else {
                        assert_eq!(client.take_response(call).unwrap().unwrap().code(), status);
                    }
                    assert_eq!(
                        EFFECTS.with(Cell::get),
                        effects + usize::from(status == Code::CHANGED)
                    );
                    now += 200;
                }
            }
        }
        assert_eq!(DISPATCHES.with(Cell::get), 8);
    });
}
#[test]
fn q_server_error_no_response_fixed_success_silence_and_recovery() {
    exercise(fixed(), fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn q_server_error_no_response_protected_success_silence_and_recovery() {
    exercise(fixed(), fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn q_server_error_no_response_allocated_both_peers_exact_budget() {
    let bytes = Resources::<Buffers>::new()
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
                    .allocated(bytes),
            )
            .allow_unprotected()
            .deterministic_for_tests()
            .route("upload", put(upload))
            .bind(Io::default())
            .unwrap()
        };
        exercise(make(), make(), protected);
    }
}

#[test]
fn q_server_error_no_response_missing_report_and_selective_reissue_recover() {
    use coaptic::message::{NoResponse, QBlockTransmission};
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        MASK.with(|v| v.set(16));
        STATUS.with(|v| v.set(Code::CHANGED));
        DISPATCHES.with(|v| v.set(0));
        EFFECTS.with(|v| v.set(0));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .q_block1()
                .non()
                .no_response(NoResponse::new(16))
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(0)
                .unwrap();
            let missing = client.transport_mut().take().unwrap();
            for now in 1..10 {
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(now).unwrap();
                assert!(server.transport_mut().take().is_none());
            }
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            let due = 9 + u64::from(QBlockTransmission::NON_RECEIVE_TIMEOUT_MS);
            server.poll(due).unwrap();
            let report = server.transport_mut().take().unwrap();
            if !protected {
                assert_eq!(
                    decode(&report.bytes[..report.len]).unwrap().code(),
                    Code::REQUEST_ENTITY_INCOMPLETE
                );
            }
            client.transport_mut().incoming = Some(report);
            client.poll(due).unwrap();
            let reissued = client.transport_mut().take().unwrap();
            assert_eq!(
                client.transport_mut().len,
                0,
                "only the missing block is reissued"
            );
            if !protected {
                let parsed = decode(&reissued.bytes[..reissued.len]).unwrap();
                assert_eq!(parsed.no_response(), Some(Ok(NoResponse::new(16))));
                assert_eq!(parsed.q_block1().unwrap().unwrap().num(), 0);
                assert_eq!(
                    parsed.payload(),
                    decode(&missing.bytes[..missing.len]).unwrap().payload()
                );
            }
            server.transport_mut().incoming = Some(reissued);
            server.poll(due + 1).unwrap();
            let progress = server.transport_mut().take().unwrap();
            if !protected {
                assert_eq!(
                    decode(&progress.bytes[..progress.len]).unwrap().code(),
                    Code::CONTINUE
                );
            }
            client.transport_mut().incoming = Some(progress);
            client.poll(due + 1).unwrap();
            let last = client.transport_mut().take().unwrap();
            assert_eq!(client.transport_mut().len, 0);
            server.transport_mut().incoming = Some(last);
            server.poll(due + 2).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(due + 2).unwrap();
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
fn q_server_error_no_response_optional_returned_5xx_is_observable() {
    use coaptic::message::{Message, NoResponse, Type, encode};
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        #[cfg(feature = "oscore")]
        let mut peer_context = context(&[2], &[1]);
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        }
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .q_block1()
                .no_response(NoResponse::new(16))
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(0)
                .unwrap();
            let frame = client.transport_mut().take().unwrap();
            let parsed = decode(&frame.bytes[..frame.len]).unwrap();
            let mut reply = Frame::default();
            if protected {
                #[cfg(feature = "oscore")]
                {
                    let mut inner = [0; 1280];
                    let (request, binding) =
                        peer_context.unprotect_request(&parsed, &mut inner).unwrap();
                    assert_eq!(request.no_response(), Some(Ok(NoResponse::new(16))));
                    let response = Message::new(
                        Type::Acknowledgement,
                        Code::SERVICE_UNAVAILABLE,
                        request.message_id(),
                    )
                    .with_token(request.token());
                    reply.len = peer_context
                        .protect_response(&response, binding, &mut reply.bytes)
                        .unwrap();
                }
            } else {
                let response = Message::new(
                    Type::Acknowledgement,
                    Code::SERVICE_UNAVAILABLE,
                    parsed.message_id(),
                )
                .with_token(parsed.token());
                reply.len = encode(&response, &mut reply.bytes).unwrap();
            }
            client.transport_mut().incoming = Some(reply);
            client.poll(1).unwrap();
            assert_eq!(
                client.take_response(call).unwrap().unwrap().code(),
                Code::SERVICE_UNAVAILABLE
            );
            assert_eq!(client.transport_mut().len, 0);
            assert_eq!(client.engine_mut().tx_occupied(), 0);
        });
    }
}
fn refuse_and_reclaim<S: AppStorage>(client: &mut Peer<S>) {
    use coaptic::app::{CallFailure, Error};
    use coaptic::message::NoResponse;
    for now in 0..16 {
        for mask in [1, 8, 10, 24, 26, 128, 255] {
            let request = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .q_block1()
                .no_response(NoResponse::new(mask));
            assert_eq!(request.send(now), Err(Error::NoResponseUploadUnsupported));
        }
        assert_eq!(
            client
                .get("upload")
                .to(PEER)
                .observe()
                .no_response(NoResponse::new(16))
                .send(now),
            Err(Error::NoResponseObserveUnsupported)
        );
        assert_eq!(client.transport_mut().len, 0);
    }
    let first = client
        .put("upload")
        .to(PEER)
        .payload(&BODY)
        .q_block1()
        .non()
        .request_tag(BodyTag::new(b"first").unwrap())
        .no_response(NoResponse::new(16))
        .send(0)
        .unwrap();
    for _ in 0..10 {
        client.transport_mut().take().unwrap();
    }
    let second = client
        .put("upload")
        .to(Endpoint::v4([192, 0, 2, 3], 5683))
        .payload(&BODY)
        .q_block1()
        .non()
        .request_tag(BodyTag::new(b"second").unwrap())
        .no_response(NoResponse::new(16))
        .send(0)
        .unwrap();
    for _ in 0..16 {
        assert_eq!(
            client
                .put("upload")
                .to(Endpoint::v4([192, 0, 2, 4], 5683))
                .payload(&BODY)
                .q_block1()
                .non()
                .request_tag(BodyTag::new(b"third").unwrap())
                .no_response(NoResponse::new(16))
                .send(1),
            Err(Error::Block(coaptic::error::BlockTransferError::Saturated))
        );
        assert_eq!(client.transport_mut().len, 10);
    }
    for _ in 0..10 {
        client.transport_mut().take().unwrap();
    }
    for call in [first, second] {
        assert!(client.cancel(call));
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            CallFailure::Cancelled
        );
    }
}

#[test]
fn q_server_error_no_response_con_retry_is_exact_and_recovers() {
    use coaptic::message::NoResponse;
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        MASK.with(|v| v.set(16));
        STATUS.with(|v| v.set(Code::CHANGED));
        DISPATCHES.with(|v| v.set(0));
        EFFECTS.with(|v| v.set(0));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .q_block1()
                .no_response(NoResponse::new(16))
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(0)
                .unwrap();
            let lost = client.transport_mut().take().unwrap();
            client.poll(3000).unwrap();
            let retry = client.transport_mut().take().unwrap();
            let parsed = decode(&retry.bytes[..retry.len]).unwrap();
            if protected {
                assert!(parsed.oscore().is_some());
            } else {
                assert_eq!(parsed.no_response(), Some(Ok(NoResponse::new(16))));
            }
            assert_eq!(retry.peer, lost.peer);
            assert_eq!(&retry.bytes[..retry.len], &lost.bytes[..lost.len]);
            server.transport_mut().incoming = Some(retry);
            server.poll(3001).unwrap();
            for now in 3001..3050 {
                if let Some(frame) = server.transport_mut().take() {
                    client.transport_mut().incoming = Some(frame);
                    client.poll(now).unwrap();
                }
                if let Some(response) = client.take_response(call) {
                    assert_eq!(response.unwrap().code(), Code::CHANGED);
                    assert_eq!(EFFECTS.with(Cell::get), 1);
                    return;
                }
                if let Some(frame) = client.transport_mut().take() {
                    server.transport_mut().incoming = Some(frame);
                    server.poll(now + 1).unwrap();
                }
            }
            panic!("lost initial CON did not recover");
        });
    }
}
