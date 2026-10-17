//! CON Q-Block1 success suppression retains ACK progress and uncertain outcomes.
use coaptic::app::AppStorage;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, put};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static LENGTH: Cell<usize> = const { Cell::new(11264) };
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
    assert_eq!(request.body(), Some(&BODY[..LENGTH.with(Cell::get)]));
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

fn campaign<C: AppStorage, S: AppStorage>(
    mut client: Peer<C>,
    mut server: Peer<S>,
    protected: bool,
) {
    use coaptic::message::{NoResponse, Type};
    #[cfg(feature = "oscore")]
    if protected {
        if client.oscore().is_none() {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        }
        if server.oscore().is_none() {
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
    }
    without_allocating(|| {
        let mut now = 0;
        for length in [3000, BODY.len()] {
            LENGTH.with(|v| v.set(length));
            for mask in [2, 18] {
                MASK.with(|v| v.set(mask));
                for status in [
                    Code::CHANGED,
                    Code::PRECONDITION_FAILED,
                    Code::SERVICE_UNAVAILABLE,
                    Code::CHANGED,
                ] {
                    STATUS.with(|v| v.set(status));
                    DISPATCHES.with(|v| v.set(0));
                    EFFECTS.with(|v| v.set(0));
                    let call = client
                        .put("upload")
                        .to(PEER)
                        .payload(&BODY[..length])
                        .q_block1()
                        .request_tag(BodyTag::new(b"transfer").unwrap())
                        .no_response(NoResponse::new(mask))
                        .deadline(now + 100)
                        .send(now)
                        .unwrap();
                    let count = length.div_ceil(1024);
                    let suppressed = status == Code::CHANGED
                        || (mask == 18 && status == Code::SERVICE_UNAVAILABLE);
                    for index in 0..count {
                        let frame = client.transport_mut().take().expect("one admitted CON");
                        assert!(client.transport_mut().take().is_none());
                        let parsed = decode(&frame.bytes[..frame.len]).unwrap();
                        assert_eq!(parsed.ty(), Type::Confirmable);
                        if !protected {
                            assert_eq!(parsed.no_response(), Some(Ok(NoResponse::new(mask))));
                            assert_eq!(parsed.q_block1().unwrap().unwrap().num(), index as u32);
                        }
                        server.transport_mut().incoming = Some(frame);
                        server.poll(now + 1 + index as u64).unwrap();
                        let reply = server
                            .transport_mut()
                            .take()
                            .expect("ACK or unsuppressed response");
                        if index + 1 < count || suppressed {
                            let ack = decode(&reply.bytes[..reply.len]).unwrap();
                            assert_eq!(ack.code(), Code::EMPTY);
                            assert_eq!(ack.ty(), Type::Acknowledgement);
                            assert_eq!(ack.message_id(), parsed.message_id());
                        }
                        client.transport_mut().incoming = Some(reply);
                        client.poll(now + 1 + index as u64).unwrap();
                    }
                    assert_eq!(DISPATCHES.with(Cell::get), 1);
                    assert_eq!(
                        EFFECTS.with(Cell::get),
                        usize::from(status == Code::CHANGED)
                    );
                    if suppressed {
                        assert!(client.take_response(call).is_none());
                        client.poll(now + 100).unwrap();
                        assert_eq!(
                            client.take_response(call).unwrap().unwrap_err(),
                            coaptic::CallFailure::DeadlineExceeded
                        );
                    } else {
                        assert_eq!(client.take_response(call).unwrap().unwrap().code(), status);
                    }
                    assert_eq!(client.engine_mut().tx_occupied(), 0);
                    #[cfg(feature = "oscore")]
                    if protected {
                        assert!(client.oscore().unwrap().lookup(call.token()).is_none());
                    }
                    if status == Code::SERVICE_UNAVAILABLE && !suppressed {
                        assert!(matches!(
                            client.put("upload").to(PEER).send(now + 100),
                            Err(coaptic::app::Error::Backoff { .. })
                        ));
                        now += 60_200;
                    } else {
                        now += 200;
                    }
                }
            }
        }
    });
}
#[test]
fn q_con_success_no_response_fixed_complete_silence_errors_and_recovery() {
    campaign(fixed(), fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn q_con_success_no_response_protected_complete_silence_errors_and_recovery() {
    campaign(fixed(), fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn q_con_success_no_response_allocated_exact_budget_both_peers() {
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
                .route("upload", put(upload))
                .bind(Io::default())
                .unwrap()
        };
        campaign(make(), make(), protected);
    }
}

#[test]
fn q_con_success_no_response_unsupported_modes_refuse_before_io_then_recover() {
    use coaptic::message::NoResponse;
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        }
        without_allocating(|| {
            for now in 0..16 {
                for non in [false, true] {
                    for mask in [1, 2, 8, 10, 18, 24, 26, 128, 255] {
                        if matches!(mask, 2 | 18) {
                            continue;
                        }
                        let request = client
                            .put("upload")
                            .to(PEER)
                            .payload(&BODY)
                            .q_block1()
                            .request_tag(BodyTag::new(b"refusal").unwrap())
                            .no_response(NoResponse::new(mask));
                        let request = if non { request.non() } else { request };
                        assert_eq!(
                            request.send(now),
                            Err(coaptic::app::Error::NoResponseUploadUnsupported)
                        );
                    }
                }
                for mask in [2, 18] {
                    assert_eq!(
                        client
                            .put("upload")
                            .to(PEER)
                            .payload(&BODY)
                            .no_response(NoResponse::new(mask))
                            .send(now),
                        Err(coaptic::app::Error::NoResponseUploadUnsupported)
                    );
                }
                assert_eq!(client.transport_mut().len, 0);
                assert_eq!(client.engine_mut().tx_occupied(), 0);
            }
        });
        campaign(client, fixed(), protected);
    }
}
#[test]
fn q_con_success_no_response_lost_acks_wrong_owner_retry_and_fresh_recovery() {
    use coaptic::message::{Message, MessageId, NoResponse, encode};
    for protected in [false, cfg!(feature = "oscore")] {
        for mask in [2, 18] {
            let mut client = fixed();
            let mut server = fixed();
            LENGTH.with(|v| v.set(3000));
            MASK.with(|v| v.set(mask));
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
                    .payload(&BODY[..3000])
                    .q_block1()
                    .request_tag(BodyTag::new(b"loss").unwrap())
                    .no_response(NoResponse::new(mask))
                    .deadline(10000)
                    .send(0)
                    .unwrap();
                let first = client.transport_mut().take().unwrap();
                assert_eq!(
                    decode(&first.bytes[..first.len])
                        .unwrap()
                        .oscore()
                        .is_some(),
                    protected
                );
                server.transport_mut().incoming = Some(first);
                server.poll(1).unwrap();
                let ack = server.transport_mut().take().unwrap();
                let mut wrong = ack;
                wrong.peer = Endpoint::v4([192, 0, 2, 3], 5683);
                client.transport_mut().incoming = Some(wrong);
                client.poll(2).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                let mid = decode(&ack.bytes[..ack.len]).unwrap().message_id();
                let mut wrong = ack;
                wrong.len = encode(
                    &Message::empty_ack(MessageId::new(mid.get().wrapping_add(1))),
                    &mut wrong.bytes,
                )
                .unwrap();
                client.transport_mut().incoming = Some(wrong);
                client.poll(3).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                assert!(client.get("other").to(PEER).send(4).is_err());
                assert_eq!(client.transport_mut().len, 0);
                assert_eq!(client.engine_mut().tx_occupied(), 1);
                client.poll(3000).unwrap();
                let retry = client.transport_mut().take().unwrap();
                assert_eq!(&retry.bytes[..retry.len], &first.bytes[..first.len]);
                server.transport_mut().incoming = Some(retry);
                server.poll(3001).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3002).unwrap();
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(3003).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3003).unwrap();
                let last = client.transport_mut().take().unwrap();
                server.transport_mut().incoming = Some(last);
                server.poll(3004).unwrap();
                let ack = server.transport_mut().take().unwrap();
                assert_eq!(decode(&ack.bytes[..ack.len]).unwrap().code(), Code::EMPTY);
                assert_eq!(EFFECTS.with(Cell::get), 1);
                client.poll(6000).unwrap();
                let retry = client.transport_mut().take().unwrap();
                assert_eq!(&retry.bytes[..retry.len], &last.bytes[..last.len]);
                server.transport_mut().incoming = Some(retry);
                server.poll(6001).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(6002).unwrap();
                assert_eq!(DISPATCHES.with(Cell::get), 1);
                assert_eq!(EFFECTS.with(Cell::get), 1);
                assert!(client.take_response(call).is_none());
                client.poll(10000).unwrap();
                assert_eq!(
                    client.take_response(call).unwrap().unwrap_err(),
                    coaptic::CallFailure::DeadlineExceeded
                );
                MASK.with(|v| v.set(0));
                let fresh = client
                    .put("upload")
                    .to(PEER)
                    .payload(&BODY[..3000])
                    .q_block1()
                    .request_tag(BodyTag::new(b"fresh").unwrap())
                    .no_response(NoResponse::new(0))
                    .send(10001)
                    .unwrap();
                for now in 10002..10005 {
                    server.transport_mut().incoming = client.transport_mut().take();
                    server.poll(now).unwrap();
                    client.transport_mut().incoming = server.transport_mut().take();
                    client.poll(now).unwrap();
                }
                assert_eq!(
                    client.take_response(fresh).unwrap().unwrap().code(),
                    Code::CHANGED
                );
                assert_eq!(EFFECTS.with(Cell::get), 2);
                assert_eq!(DISPATCHES.with(Cell::get), 2);
            });
        }
    }
}

#[test]
fn q_con_success_no_response_full_body_refusal_cancel_and_recovery() {
    use coaptic::message::NoResponse;
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        }
        without_allocating(|| {
            let first = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .q_block1()
                .request_tag(BodyTag::new(b"first").unwrap())
                .no_response(NoResponse::new(2))
                .send(0)
                .unwrap();
            let second = client
                .put("upload")
                .to(Endpoint::v4([192, 0, 2, 3], 5683))
                .payload(&BODY)
                .q_block1()
                .request_tag(BodyTag::new(b"second").unwrap())
                .no_response(NoResponse::new(18))
                .send(0)
                .unwrap();
            for _ in 0..16 {
                assert_eq!(
                    client
                        .put("upload")
                        .to(Endpoint::v4([192, 0, 2, 4], 5683))
                        .payload(&BODY)
                        .q_block1()
                        .request_tag(BodyTag::new(b"third").unwrap())
                        .no_response(NoResponse::new(2))
                        .send(1),
                    Err(coaptic::app::Error::Block(
                        coaptic::error::BlockTransferError::Saturated
                    ))
                );
                assert_eq!(client.transport_mut().len, 2);
                let first = client.transport_mut().output[0];
                assert_eq!(
                    decode(&first.bytes[..first.len])
                        .unwrap()
                        .oscore()
                        .is_some(),
                    protected
                );
            }
            for call in [first, second] {
                assert!(client.cancel(call));
                assert_eq!(
                    client.take_response(call).unwrap().unwrap_err(),
                    coaptic::CallFailure::Cancelled
                );
                client.transport_mut().take().unwrap();
            }
            assert_eq!(client.engine_mut().tx_occupied(), 0);
        });
        // Keep the consumed security sequence; fresh protected work must use later PIVs.
        let mut server = fixed();
        #[cfg(feature = "oscore")]
        if protected {
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
        MASK.with(|v| v.set(0));
        LENGTH.with(|v| v.set(3000));
        STATUS.with(|v| v.set(Code::CHANGED));
        let call = client
            .put("upload")
            .to(PEER)
            .payload(&BODY[..3000])
            .q_block1()
            .request_tag(BodyTag::new(b"fresh").unwrap())
            .no_response(NoResponse::new(0))
            .send(10)
            .unwrap();
        for now in 11..14 {
            server.transport_mut().incoming = client.transport_mut().take();
            server.poll(now).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(now).unwrap();
        }
        assert_eq!(
            client.take_response(call).unwrap().unwrap().code(),
            Code::CHANGED
        );
    }
}
