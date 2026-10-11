//! NON Q1 success suppression uses finite pacing, never timer acknowledgements.
use coaptic::app::AppStorage;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, put};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static LENGTH: Cell<usize> = const { Cell::new(32768) };
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
        rx_body: (2, 32768), tx_body: (2, 32768),
        dedup: 8, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
static BODY: [u8; 32768] = [0x5a; 32768];
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
    refuse: bool,
    fail_after: Option<usize>,
    entropy: bool,
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
    fn fill_random(&mut self, out: &mut [u8]) -> bool {
        if !self.entropy {
            return false;
        }
        out.fill(0);
        true
    }
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some(frame) = self.incoming.take() else {
            return Ok(None);
        };
        out[..frame.len].copy_from_slice(&frame.bytes[..frame.len]);
        Ok(Some((frame.len, frame.peer)))
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if self.refuse || self.fail_after == Some(self.len) {
            return Err("send refused");
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
    .allow_plaintext()
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

fn protect<C: AppStorage, S: AppStorage>(
    client: &mut Peer<C>,
    server: &mut Peer<S>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    #[cfg(not(feature = "oscore"))]
    let _ = (client, server, protected);
}
fn deliver<C: AppStorage, S: AppStorage>(
    client: &mut Peer<C>,
    server: &mut Peer<S>,
    now: u64,
    protected: bool,
    mask: u8,
    first: usize,
) -> usize {
    let mut n = 0;
    while let Some(frame) = client.transport_mut().take() {
        let p = decode(&frame.bytes[..frame.len]).unwrap();
        assert_eq!(p.ty(), coaptic::message::Type::NonConfirmable);
        if !protected {
            assert_eq!(
                p.no_response(),
                Some(Ok(coaptic::message::NoResponse::new(mask)))
            );
            assert_eq!(p.q_block1().unwrap().unwrap().num(), (first + n) as u32);
            assert_eq!(
                p.payload(),
                &BODY[(first + n) * 1024..((first + n + 1) * 1024).min(LENGTH.with(Cell::get))]
            );
        }
        server.transport_mut().incoming = Some(frame);
        server.poll(now).unwrap();
        n += 1;
    }
    n
}
fn campaign<C: AppStorage, S: AppStorage>(
    mut client: Peer<C>,
    mut server: Peer<S>,
    protected: bool,
) {
    protect(&mut client, &mut server, protected);
    without_allocating(|| {
        let mut start = 0;
        for mask in [2, 18] {
            MASK.with(|v| v.set(mask));
            LENGTH.with(|v| v.set(BODY.len()));
            for status in [
                Code::CHANGED,
                Code::PRECONDITION_FAILED,
                Code::SERVICE_UNAVAILABLE,
                Code::CHANGED,
            ] {
                STATUS.with(|v| v.set(status));
                EFFECTS.with(|v| v.set(0));
                DISPATCHES.with(|v| v.set(0));
                let call = client
                    .put("upload")
                    .to(PEER)
                    .non()
                    .payload(&BODY)
                    .q_block1()
                    .request_tag(BodyTag::new(b"paced").unwrap())
                    .no_response(coaptic::message::NoResponse::new(mask))
                    .deadline(start + 20000)
                    .send(start)
                    .unwrap();
                assert_eq!(
                    deliver(&mut client, &mut server, start, protected, mask, 0),
                    10
                );
                assert!(server.transport_mut().take().is_none());
                client.poll(start + 1999).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                client.poll(start + 2000).unwrap();
                assert_eq!(
                    deliver(&mut client, &mut server, start + 2000, protected, mask, 10),
                    10
                );
                // A large jump gives only one payload set, not accumulated credit.
                client.poll(start + 10000).unwrap();
                assert_eq!(
                    deliver(&mut client, &mut server, start + 10000, protected, mask, 20),
                    10
                );
                client.poll(start + 10000).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                client.poll(start + 11999).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                client.poll(start + 12000).unwrap();
                assert_eq!(
                    deliver(&mut client, &mut server, start + 12000, protected, mask, 30),
                    2
                );
                assert_eq!(DISPATCHES.with(Cell::get), 1);
                assert_eq!(
                    EFFECTS.with(Cell::get),
                    usize::from(status == Code::CHANGED)
                );
                let suppressed =
                    status == Code::CHANGED || (mask == 18 && status == Code::SERVICE_UNAVAILABLE);
                if suppressed {
                    assert!(server.transport_mut().take().is_none());
                    assert!(client.take_response(call).is_none());
                    client.poll(start + 20000).unwrap();
                    assert_eq!(
                        client.take_response(call).unwrap().unwrap_err(),
                        coaptic::CallFailure::DeadlineExceeded
                    );
                } else {
                    client.transport_mut().incoming = server.transport_mut().take();
                    client.poll(start + 12001).unwrap();
                    assert_eq!(client.take_response(call).unwrap().unwrap().code(), status);
                }
                assert_eq!(client.engine_mut().tx_occupied(), 0);
                #[cfg(feature = "oscore")]
                if protected {
                    assert!(client.oscore().unwrap().lookup(call.token()).is_none());
                }
                start += 30000;
            }
        }
    });
}
#[test]
fn non_q_pacing_fixed_multiset_unknown_errors_and_fresh_recovery() {
    campaign(fixed(), fixed(), false)
}
#[cfg(feature = "oscore")]
#[test]
fn non_q_pacing_protected_multiset_unknown_errors_and_fresh_recovery() {
    campaign(fixed(), fixed(), true)
}
#[cfg(feature = "alloc")]
#[test]
fn non_q_pacing_allocated_both_peers_exact_budget() {
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
                .allow_plaintext()
                .deterministic_for_tests()
                .route("upload", put(upload))
                .bind(Io::default())
                .unwrap()
        };
        campaign(make(), make(), protected);
    }
}

fn response_to(frame: Frame, protected: bool, code: Code, qnum: Option<u32>) -> Frame {
    response_with_q(
        frame,
        protected,
        code,
        qnum.map(|num| coaptic::message::BlockValue::new(num, true, 6).unwrap()),
    )
}
fn response_with_q(
    frame: Frame,
    protected: bool,
    code: Code,
    q: Option<coaptic::message::BlockValue>,
) -> Frame {
    use coaptic::message::{Message, MessageId, Opt, OptionsBuilder, Type, encode};
    let outer = decode(&frame.bytes[..frame.len]).unwrap();
    let mut opts = OptionsBuilder::<1>::new();
    let q = q.map(|value| value.encode());
    if let Some(q) = q.as_ref() {
        opts.push(Opt::q_block1(q)).unwrap();
    }
    let response = Message::new(Type::NonConfirmable, code, MessageId::new(500))
        .with_token(outer.token())
        .with_options(opts.as_slice());
    let mut reply = Frame::default();
    if protected {
        #[cfg(feature = "oscore")]
        {
            let mut ctx = context(&[2], &[1]);
            let mut inner = [0; 1280];
            let (_, binding) = ctx.unprotect_request(&outer, &mut inner).unwrap();
            reply.len = ctx
                .protect_response(&response, binding, &mut reply.bytes)
                .unwrap();
        }
    } else {
        reply.len = encode(&response, &mut reply.bytes).unwrap();
    }
    reply
}
#[test]
fn non_q_pacing_optional_continue_rearms_and_stale_continue_cannot_advance() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        protect(&mut client, &mut server, protected);
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .non()
                .payload(&BODY)
                .q_block1()
                .request_tag(BodyTag::new(b"continue").unwrap())
                .no_response(coaptic::message::NoResponse::new(2))
                .deadline(10000)
                .send(0)
                .unwrap();
            let mut last = Frame::default();
            for _ in 0..10 {
                last = client.transport_mut().take().unwrap();
            }
            let response = response_to(last, protected, Code::CONTINUE, Some(9));
            client.transport_mut().incoming = Some(response);
            client.poll(1000).unwrap();
            assert_eq!(client.transport_mut().len, 10);
            for _ in 0..10 {
                client.transport_mut().take();
            }
            client.transport_mut().incoming = Some(response);
            let _ = client.poll(1001);
            assert_eq!(client.transport_mut().len, 0);
            client.poll(2000).unwrap();
            assert_eq!(client.transport_mut().len, 0);
            client.poll(2999).unwrap();
            assert_eq!(client.transport_mut().len, 0);
            client.poll(3000).unwrap();
            assert_eq!(client.transport_mut().len, 10);
            let mut final_request = Frame::default();
            for _ in 0..10 {
                final_request = client.transport_mut().take().unwrap();
            }
            // No-Response is optional: an actual returned success is observable.
            client.transport_mut().incoming =
                Some(response_to(final_request, protected, Code::CHANGED, None));
            client.poll(3001).unwrap();
            assert_eq!(
                client.take_response(call).unwrap().unwrap().code(),
                Code::CHANGED
            );
            client.poll(6000).unwrap();
            assert_eq!(client.transport_mut().len, 0);
        });
    }
}
#[test]
fn non_q_pacing_missing_report_reissue_delays_next_set_and_recovers() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        protect(&mut client, &mut server, protected);
        MASK.with(|v| v.set(18));
        LENGTH.with(|v| v.set(11264));
        STATUS.with(|v| v.set(Code::CHANGED));
        EFFECTS.with(|v| v.set(0));
        DISPATCHES.with(|v| v.set(0));
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .non()
                .payload(&BODY[..11264])
                .q_block1()
                .request_tag(BodyTag::new(b"missing").unwrap())
                .no_response(coaptic::message::NoResponse::new(18))
                .deadline(20000)
                .send(0)
                .unwrap();
            let lost = client.transport_mut().take().unwrap();
            for now in 1..10 {
                server.transport_mut().incoming = client.transport_mut().take();
                server.poll(now).unwrap();
            }
            server.poll(4009).unwrap();
            let report = server.transport_mut().take().unwrap();
            let mut wrong = report;
            wrong.peer = Endpoint::v4([192, 0, 2, 99], 5683);
            client.transport_mut().incoming = Some(wrong);
            let _ = client.poll(1000);
            assert_eq!(client.transport_mut().len, 0);
            client.transport_mut().incoming = Some(report);
            client.poll(4009).unwrap();
            let reissued = client.transport_mut().take().unwrap();
            assert_eq!(client.transport_mut().len, 0);
            let outer = decode(&reissued.bytes[..reissued.len]).unwrap();
            if !protected {
                let old = decode(&lost.bytes[..lost.len]).unwrap();
                assert_eq!(outer.q_block1(), old.q_block1());
                assert_eq!(outer.size1(), old.size1());
                assert_eq!(outer.payload(), old.payload());
                assert_eq!(outer.no_response(), old.no_response());
            }
            #[cfg(feature = "oscore")]
            if protected {
                let mut ctx = context(&[2], &[1]);
                let mut scratch = [0; 1280];
                let (inner, _) = ctx.unprotect_request(&outer, &mut scratch).unwrap();
                assert_eq!(inner.q_block1().unwrap().unwrap().num(), 0);
                assert_eq!(
                    inner.no_response(),
                    Some(Ok(coaptic::message::NoResponse::new(18)))
                );
                assert_eq!(inner.payload(), &BODY[..1024]);
                assert_eq!(inner.size1(), Some(Ok(11264)));
            }
            server.transport_mut().incoming = Some(reissued);
            server.poll(4010).unwrap();
            assert!(server.transport_mut().take().is_none());
            client.poll(6008).unwrap();
            assert_eq!(client.transport_mut().len, 0);
            client.poll(6009).unwrap();
            assert_eq!(
                deliver(&mut client, &mut server, 6010, protected, 18, 10),
                1
            );
            assert_eq!(EFFECTS.with(Cell::get), 1);
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert!(client.take_response(call).is_none());
            assert!(client.cancel(call));
            assert_eq!(
                client.take_response(call).unwrap().unwrap_err(),
                coaptic::CallFailure::Cancelled
            );
            client.transport_mut().incoming = Some(report);
            let _ = client.poll(7000);
            assert_eq!(client.transport_mut().len, 0);
            let fresh = client
                .put("upload")
                .to(PEER)
                .non()
                .payload(&BODY[..11264])
                .q_block1()
                .request_tag(BodyTag::new(b"fresh").unwrap())
                .no_response(coaptic::message::NoResponse::new(18))
                .send(8000)
                .unwrap();
            assert_eq!(client.transport_mut().len, 10);
            assert!(client.cancel(fresh));
        });
    }
}

thread_local! {
    static RNG_OK: Cell<bool> = const { Cell::new(true) };
    static RNG_TOKEN: Cell<u64> = const { Cell::new(0) };
}
fn entropy(out: &mut [u8]) -> bool {
    if !RNG_OK.with(Cell::get) {
        return false;
    }
    out.fill(0);
    if out.len() == 4 {
        out.copy_from_slice(&1000u32.to_be_bytes());
    }
    if out.len() == 8 {
        let token = RNG_TOKEN.with(|v| {
            v.set(v.get() + 1);
            v.get()
        });
        out.copy_from_slice(&token.to_be_bytes());
    }
    true
}
fn randomized() -> App<Buffers, Io, 1, true> {
    RNG_OK.with(|v| v.set(true));
    RNG_TOKEN.with(|v| v.set(0));
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>(),
    )
    .allow_plaintext()
    .randomness(entropy)
    .route("upload", put(upload))
    .bind(Io::default())
    .unwrap()
}
fn begin<S: AppStorage>(client: &mut Peer<S>, now: u64) -> coaptic::Call {
    client
        .put("upload")
        .to(PEER)
        .non()
        .payload(&BODY)
        .q_block1()
        .request_tag(BodyTag::new(b"bounds").unwrap())
        .no_response(coaptic::message::NoResponse::new(2))
        .deadline(now + 10000)
        .send(now)
        .unwrap()
}
fn drain<S: AppStorage>(client: &mut Peer<S>) {
    while client.transport_mut().take().is_some() {}
}
#[test]
fn non_q_pacing_sampled_upper_boundary_and_entropy_refusal_recover() {
    let mut client = randomized();
    without_allocating(|| {
        let call = begin(&mut client, 0);
        drain(&mut client);
        client.poll(2000).unwrap();
        client.poll(2999).unwrap();
        assert_eq!(client.transport_mut().len, 0);
        client.poll(3000).unwrap();
        assert_eq!(client.transport_mut().len, 10);
        drain(&mut client);
        RNG_OK.with(|v| v.set(false));
        assert!(matches!(
            client.poll(6000),
            Err(coaptic::app::Error::Identity(
                coaptic::app::IdentityError::RandomnessUnavailable
            ))
        ));
        assert_eq!(client.transport_mut().len, 0);
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            coaptic::CallFailure::ContinuationFailed
        );
        assert_eq!(client.engine_mut().tx_occupied(), 0);
        RNG_OK.with(|v| v.set(true));
        let fresh = begin(&mut client, 7000);
        assert_eq!(client.transport_mut().len, 10);
        assert!(client.cancel(fresh));
    });
}
#[test]
fn non_q_pacing_partial_send_and_tx_refusal_retire_without_leaks() {
    for protected in [false, cfg!(feature = "oscore")] {
        for tx_full in [false, true] {
            let mut client = fixed();
            let mut server = fixed();
            protect(&mut client, &mut server, protected);
            without_allocating(|| {
                let call = begin(&mut client, 0);
                drain(&mut client);
                let mut held = [None; 4];
                if tx_full {
                    for slot in &mut held {
                        *slot = client.engine_mut().acquire_tx();
                        assert!(slot.is_some());
                    }
                } else {
                    client.transport_mut().fail_after = Some(3);
                }
                assert!(client.poll(2000).is_err());
                assert_eq!(client.transport_mut().len, if tx_full { 0 } else { 3 });
                assert_eq!(
                    client.take_response(call).unwrap().unwrap_err(),
                    coaptic::CallFailure::ContinuationFailed
                );
                assert_eq!(
                    client.engine_mut().tx_occupied(),
                    if tx_full { 4 } else { 0 }
                );
                for slot in held.into_iter().flatten() {
                    client.engine_mut().release_tx(slot).unwrap();
                }
                client.transport_mut().fail_after = None;
                drain(&mut client);
                client.poll(4000).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                let fresh = begin(&mut client, 5000);
                assert_eq!(client.transport_mut().len, 10);
                assert!(client.cancel(fresh));
                #[cfg(feature = "oscore")]
                if protected {
                    assert!(client.oscore().unwrap().lookup(call.token()).is_none());
                }
            });
        }
    }
}
#[test]
fn non_q_pacing_capacity_cancel_and_deadline_precede_timed_output() {
    let mut client = fixed();
    without_allocating(|| {
        let first = begin(&mut client, 0);
        drain(&mut client);
        let second = client
            .put("upload")
            .to(Endpoint::v4([192, 0, 2, 3], 5683))
            .non()
            .payload(&BODY)
            .q_block1()
            .request_tag(BodyTag::new(b"second").unwrap())
            .no_response(coaptic::message::NoResponse::new(18))
            .deadline(2000)
            .send(0)
            .unwrap();
        drain(&mut client);
        for _ in 0..8 {
            assert!(matches!(
                client
                    .put("upload")
                    .to(Endpoint::v4([192, 0, 2, 4], 5683))
                    .non()
                    .payload(&BODY)
                    .q_block1()
                    .request_tag(BodyTag::new(b"third").unwrap())
                    .no_response(coaptic::message::NoResponse::new(2))
                    .send(1),
                Err(coaptic::app::Error::Block(
                    coaptic::error::BlockTransferError::Saturated
                ))
            ));
        }
        assert_eq!(client.transport_mut().len, 0);
        assert!(client.cancel(first));
        assert_eq!(
            client.take_response(first).unwrap().unwrap_err(),
            coaptic::CallFailure::Cancelled
        );
        client.poll(2000).unwrap();
        assert_eq!(client.transport_mut().len, 0);
        assert_eq!(
            client.take_response(second).unwrap().unwrap_err(),
            coaptic::CallFailure::DeadlineExceeded
        );
        let fresh = begin(&mut client, 3000);
        assert_eq!(client.transport_mut().len, 10);
        assert!(client.cancel(fresh));
    });
}

#[test]
fn non_q_pacing_old_window_hole_recovers_after_later_sets_without_ack() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = fixed();
        let mut server = fixed();
        protect(&mut client, &mut server, protected);
        MASK.with(|v| v.set(2));
        LENGTH.with(|v| v.set(BODY.len()));
        STATUS.with(|v| v.set(Code::CHANGED));
        EFFECTS.with(|v| v.set(0));
        DISPATCHES.with(|v| v.set(0));
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .non()
                .payload(&BODY)
                .q_block1()
                .request_tag(BodyTag::new(b"old-hole").unwrap())
                .no_response(coaptic::message::NoResponse::new(2))
                .deadline(60000)
                .send(0)
                .unwrap();
            let _lost = client.transport_mut().take().unwrap();
            for now in [0, 2000, 4000, 6000] {
                if now != 0 {
                    client.poll(now).unwrap();
                }
                while let Some(frame) = client.transport_mut().take() {
                    server.transport_mut().incoming = Some(frame);
                    server.poll(now).unwrap();
                    // Lose every early report as later sets continue without receipt credit.
                    while server.transport_mut().take().is_some() {}
                }
            }
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            for (now, first, count) in [
                (20000, 0, 1),
                (24001, 10, 10),
                (28002, 20, 10),
                (32003, 30, 2),
            ] {
                server.poll(now).unwrap();
                let report = server.transport_mut().take().expect("retained missing set");
                client.transport_mut().incoming = Some(report);
                client.poll(now).unwrap();
                assert_eq!(client.transport_mut().len, count);
                assert_eq!(
                    deliver(&mut client, &mut server, now + 1, protected, 2, first),
                    count
                );
                assert!(server.transport_mut().take().is_none());
                if first != 30 {
                    assert_eq!(DISPATCHES.with(Cell::get), 0);
                }
                assert!(client.take_response(call).is_none());
            }
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), 1);
            assert!(client.take_response(call).is_none());
            assert!(client.cancel(call));
        });
    }
}

#[test]
fn non_q_pacing_two_due_calls_get_one_set_each_without_starvation() {
    let mut client = fixed();
    without_allocating(|| {
        let first = begin(&mut client, 0);
        drain(&mut client);
        let second = client
            .put("upload")
            .to(Endpoint::v4([192, 0, 2, 3], 5683))
            .non()
            .payload(&BODY)
            .q_block1()
            .request_tag(BodyTag::new(b"second").unwrap())
            .no_response(coaptic::message::NoResponse::new(18))
            .deadline(20000)
            .send(0)
            .unwrap();
        drain(&mut client);
        client.poll(2000).unwrap();
        assert_eq!(client.transport_mut().len, 10);
        let frame = client.transport_mut().take().unwrap();
        let token = decode(&frame.bytes[..frame.len]).unwrap().token();
        assert_eq!(token, first.token());
        drain(&mut client);
        client.poll(2000).unwrap();
        assert_eq!(client.transport_mut().len, 10);
        let frame = client.transport_mut().take().unwrap();
        assert_eq!(
            decode(&frame.bytes[..frame.len]).unwrap().token(),
            second.token()
        );
        drain(&mut client);
        client.poll(2000).unwrap();
        assert_eq!(client.transport_mut().len, 0);
        assert!(client.cancel(first));
        assert!(client.cancel(second));
    });
}
#[test]
fn non_q_pacing_clock_saturation_never_grants_same_tick_send_credit() {
    let mut client = fixed();
    without_allocating(|| {
        let call = client
            .put("upload")
            .to(PEER)
            .non()
            .payload(&BODY)
            .q_block1()
            .request_tag(BodyTag::new(b"late").unwrap())
            .no_response(coaptic::message::NoResponse::new(2))
            .send(u64::MAX - 1000)
            .unwrap();
        drain(&mut client);
        client.poll(u64::MAX - 1).unwrap();
        assert_eq!(client.transport_mut().len, 0);
        client.poll(u64::MAX).unwrap();
        assert_eq!(client.transport_mut().len, 0);
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            coaptic::CallFailure::TimedOut
        );
        for _ in 0..4 {
            client.poll(u64::MAX).unwrap();
            assert_eq!(client.transport_mut().len, 0);
        }
    });
}

#[test]
fn non_q_pacing_invalid_continue_cannot_retire_or_credit_current_set() {
    use coaptic::message::BlockValue;
    for protected in [false, cfg!(feature = "oscore")] {
        for q in [
            Some(BlockValue::new(19, true, 6).unwrap()),
            Some(BlockValue::new(8, true, 6).unwrap()),
            None,
            Some(BlockValue::new(9, true, 5).unwrap()),
            Some(BlockValue::new(9, false, 6).unwrap()),
        ] {
            let mut client = fixed();
            let mut server = fixed();
            protect(&mut client, &mut server, protected);
            without_allocating(|| {
                let call = begin(&mut client, 0);
                let mut last = Frame::default();
                for _ in 0..10 {
                    last = client.transport_mut().take().unwrap();
                }
                client.transport_mut().incoming =
                    Some(response_with_q(last, protected, Code::CONTINUE, q));
                client.poll(1000).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                assert!(client.take_response(call).is_none());
                client.poll(1999).unwrap();
                assert_eq!(client.transport_mut().len, 0);
                client.poll(2000).unwrap();
                assert_eq!(client.transport_mut().len, 10);
                for _ in 0..10 {
                    last = client.transport_mut().take().unwrap();
                }
                client.transport_mut().incoming =
                    Some(response_to(last, protected, Code::CONTINUE, Some(19)));
                client.poll(2001).unwrap();
                assert_eq!(client.transport_mut().len, 10);
                assert!(client.take_response(call).is_none());
                assert!(client.cancel(call));
            });
        }
    }
}
