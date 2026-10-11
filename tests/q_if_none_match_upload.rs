//! Q-Block1 retains validated NUM0 conditions across out-of-order reception.
use coaptic::app::AppStorage;
use coaptic::message::decode;
use coaptic::resources::Resources;
use coaptic::storage::{BodyTag, DatagramIo};
use coaptic::{App, Code, Endpoint, Request, Response, put};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static CONDITIONAL: Cell<bool> = const { Cell::new(true) };
    static RAW_CONDITION: Cell<bool> = const { Cell::new(true) };
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static REPRESENTATION: Cell<Option<u8>> = const { Cell::new(Some(1)) };
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
    assert_eq!(request.if_none_match(), CONDITIONAL.with(Cell::get));
    assert_eq!(
        request.message().if_none_match(),
        RAW_CONDITION.with(Cell::get)
    );
    let version = REPRESENTATION.with(Cell::get);
    let etag = version.map(|v| [v]);
    if !request
        .precondition(version.is_some(), etag.as_ref().map(|v| v.as_slice()))
        .holds()
    {
        return Response::new(Code::PRECONDITION_FAILED);
    }
    EFFECTS.with(|v| v.set(v.get() + 1));
    REPRESENTATION.with(|v| v.set(Some(1)));
    Response::new(Code::CREATED)
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

fn client() -> Server {
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
fn reset(exists: bool) {
    REPRESENTATION.with(|v| v.set(exists.then_some(1)));
    EFFECTS.with(|v| v.set(0));
    DISPATCHES.with(|v| v.set(0));
}

fn complete<C: AppStorage, S: AppStorage>(
    client: &mut Client<C>,
    server: &mut Client<S>,
    now: u64,
    condition: bool,
    create_between: bool,
    protected: bool,
    missing: bool,
) -> Code {
    CONDITIONAL.with(|v| v.set(condition));
    RAW_CONDITION.with(|v| v.set(condition));
    let mut request = client
        .put("upload")
        .to(PEER)
        .payload(&BODY)
        .q_block1()
        .non()
        .request_tag(BodyTag::new(b"transfer").unwrap());
    if condition {
        request = request.if_none_match();
    }
    let call = request.send(now).unwrap();
    let initial = client.transport_mut().take().unwrap();
    let later = client.transport_mut().take().unwrap();
    if !protected {
        assert_eq!(
            decode(&initial.bytes[..initial.len])
                .unwrap()
                .if_none_match(),
            condition
        );
        assert!(!decode(&later.bytes[..later.len]).unwrap().if_none_match());
    }
    server.transport_mut().incoming = Some(later);
    server.poll(now + 1).unwrap();
    assert!(server.transport_mut().take().is_none());
    assert_eq!(
        DISPATCHES.with(Cell::get),
        0,
        "missing NUM0 must never dispatch"
    );
    if create_between {
        REPRESENTATION.with(|v| v.set(Some(2)));
    }
    let (initial, done) = if protected || missing {
        // The existing same-Token security contract accepts only the latest
        // request binding. A fresh missing-NUM0 reissue provides that binding.
        let due = now + 1 + u64::from(coaptic::message::QBlockTransmission::NON_RECEIVE_TIMEOUT_MS);
        server.poll(due).unwrap();
        client.transport_mut().incoming = server.transport_mut().take();
        client.poll(due).unwrap();
        (client.transport_mut().take().unwrap(), due + 1)
    } else {
        (initial, now + 2)
    };
    server.transport_mut().incoming = Some(initial);
    server.poll(done).unwrap();
    client.transport_mut().incoming = server.transport_mut().take();
    client.poll(done).unwrap();
    client.take_response(call).unwrap().unwrap().code()
}
fn exercise<C: AppStorage, S: AppStorage>(
    mut client: Client<C>,
    mut server: Client<S>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(context(&[1], &[2]));
        server.set_oscore(context(&[2], &[1]));
    }
    without_allocating(|| {
        for (i, (condition, exists, create_between)) in [
            (true, false, false),
            (true, true, false),
            (true, false, true),
            (false, true, false),
            (true, false, false),
        ]
        .into_iter()
        .enumerate()
        {
            reset(exists);
            let code = complete(
                &mut client,
                &mut server,
                i as u64 * 10000,
                condition,
                create_between,
                protected,
                false,
            );
            let succeeds = !condition || (!exists && !create_between);
            assert_eq!(
                code,
                if succeeds {
                    Code::CREATED
                } else {
                    Code::PRECONDITION_FAILED
                }
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), usize::from(succeeds));
        }
        reclaim(&mut client, &mut server, protected);
    });
}
#[test]
fn q_if_none_match_num0_last_current_existence_and_recovery() {
    exercise(client(), server(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn q_if_none_match_protected_num0_last_and_recovery() {
    exercise(client(), server(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn q_if_none_match_allocated_both_peers_exact_budget() {
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
            .allow_plaintext()
            .deterministic_for_tests()
            .route("upload", put(replace))
            .bind(Io::default())
            .unwrap()
        };
        exercise(make(), make(), protected);
    }
}

#[test]
fn q_if_none_match_two_complete_owners_cannot_both_create() {
    let mut first = client();
    let mut second = client();
    let mut server = server();
    reset(false);
    CONDITIONAL.with(|v| v.set(true));
    RAW_CONDITION.with(|v| v.set(false));
    let other = Endpoint::v4([192, 0, 2, 3], 5683);
    without_allocating(|| {
        let a = first
            .put("upload")
            .to(PEER)
            .payload(&BODY)
            .if_none_match()
            .q_block1()
            .non()
            .request_tag(BodyTag::new(b"first").unwrap())
            .send(0)
            .unwrap();
        let b = second
            .put("upload")
            .to(other)
            .payload(&BODY)
            .if_none_match()
            .q_block1()
            .non()
            .request_tag(BodyTag::new(b"second").unwrap())
            .send(0)
            .unwrap();
        server.transport_mut().incoming = first.transport_mut().take();
        server.poll(1).unwrap();
        assert!(server.transport_mut().take().is_none());
        server.transport_mut().incoming = second.transport_mut().take();
        server.poll(2).unwrap();
        assert!(server.transport_mut().take().is_none());
        assert_eq!(DISPATCHES.with(Cell::get), 0);
        server.transport_mut().incoming = first.transport_mut().take();
        server.poll(3).unwrap();
        first.transport_mut().incoming = server.transport_mut().take();
        first.poll(3).unwrap();
        assert_eq!(
            first.take_response(a).unwrap().unwrap().code(),
            Code::CREATED
        );
        server.transport_mut().incoming = second.transport_mut().take();
        server.poll(4).unwrap();
        second.transport_mut().incoming = server.transport_mut().take();
        second.poll(4).unwrap();
        assert_eq!(
            second.take_response(b).unwrap().unwrap().code(),
            Code::PRECONDITION_FAILED
        );
        assert_eq!(DISPATCHES.with(Cell::get), 2);
        assert_eq!(EFFECTS.with(Cell::get), 1);
    });
}
#[test]
fn q_if_none_match_num0_retry_is_exact_and_completes_after_loss() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = client();
        let mut server = server();
        CONDITIONAL.with(|v| v.set(true));
        RAW_CONDITION.with(|v| v.set(false));
        REPRESENTATION.with(|v| v.set(None));
        EFFECTS.with(|v| v.set(0));
        DISPATCHES.with(|v| v.set(0));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(context(&[1], &[2]));
            server.set_oscore(context(&[2], &[1]));
        }
        without_allocating(|| {
            let request = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_none_match()
                .q_block1()
                .request_tag(BodyTag::new(b"transfer").unwrap());
            let call = request.send(0).unwrap();
            let lost = client.transport_mut().take().unwrap();
            server.transport_mut().incoming = Some(lost);
            server.poll(1).unwrap();
            assert!(server.transport_mut().take().is_some()); // Lose the NUM0 acknowledgement.
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            client.poll(3000).unwrap();
            let retry = client.transport_mut().take().unwrap();
            assert_eq!(retry.peer, lost.peer);
            assert_eq!(&retry.bytes[..retry.len], &lost.bytes[..lost.len]);
            if !protected {
                assert!(decode(&retry.bytes[..retry.len]).unwrap().if_none_match());
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
                Code::CREATED
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), 1);
        });
    }
}

fn altered(frame: Frame, condition: u8) -> Frame {
    use coaptic::message::{Message, MessageId, Opt, OptionNumber, OptionsBuilder, encode};
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    let mut options = OptionsBuilder::<16>::new();
    for option in parsed.options() {
        if option.number() != OptionNumber::IF_NONE_MATCH {
            options.push(option).unwrap();
        }
    }
    if condition != 0 {
        options
            .push(Opt::new(
                OptionNumber::IF_NONE_MATCH,
                if condition == 2 { b"bad" } else { b"" },
            ))
            .unwrap();
    }
    if condition == 3 {
        options.push(Opt::if_none_match()).unwrap();
    }
    let poison = [0xee; 1024];
    let message = Message::new(parsed.ty(), parsed.code(), MessageId::new(60000))
        .with_token(parsed.token())
        .with_options(options.as_slice())
        .with_payload(
            &poison[..if condition == 4 {
                1
            } else {
                parsed.payload().len()
            }],
        );
    let mut changed = Frame::default();
    changed.len = encode(&message, &mut changed.bytes).unwrap();
    changed
}
#[test]
fn q_if_none_match_changed_num0_illegal_later_and_malformed_refuse_without_corruption() {
    for condition in [false, true] {
        for mutation in 0..5 {
            let mut client = client();
            let mut server = server();
            reset(false);
            CONDITIONAL.with(|v| v.set(condition));
            RAW_CONDITION.with(|v| v.set(false));
            without_allocating(|| {
                let mut request = client
                    .put("upload")
                    .to(PEER)
                    .payload(&BODY)
                    .q_block1()
                    .non()
                    .request_tag(BodyTag::new(b"transfer").unwrap());
                if condition {
                    request = request.if_none_match();
                }
                let call = request.send(0).unwrap();
                let initial = client.transport_mut().take().unwrap();
                let later = client.transport_mut().take().unwrap();
                server.transport_mut().incoming = Some(initial);
                server.poll(1).unwrap();
                assert!(server.transport_mut().take().is_none());
                let bad = match mutation {
                    0 => altered(initial, u8::from(!condition)),
                    1 => altered(later, 1),
                    2 => altered(initial, 2),
                    3 => altered(initial, 3),
                    _ => altered(later, 2),
                };
                server.transport_mut().incoming = Some(bad);
                server.poll(2).unwrap();
                let response = server.transport_mut().take().unwrap();
                assert!(
                    decode(&response.bytes[..response.len])
                        .unwrap()
                        .code()
                        .is_client_error()
                );
                assert_eq!(DISPATCHES.with(Cell::get), 0);
                assert_eq!(EFFECTS.with(Cell::get), 0);
                server.transport_mut().incoming = Some(later);
                server.poll(3).unwrap();
                client.transport_mut().incoming = server.transport_mut().take();
                client.poll(3).unwrap();
                assert_eq!(
                    client.take_response(call).unwrap().unwrap().code(),
                    Code::CREATED
                );
                assert_eq!(EFFECTS.with(Cell::get), 1);
            });
        }
    }
}
#[cfg(feature = "oscore")]
#[test]
fn q_if_none_match_older_protected_completion_refuses_then_fresh_retry_checks_existence() {
    use coaptic::app::CallFailure;
    let mut client = client();
    let mut server = server();
    reset(false);
    client.set_oscore(context(&[1], &[2]));
    server.set_oscore(context(&[2], &[1]));
    CONDITIONAL.with(|v| v.set(true));
    RAW_CONDITION.with(|v| v.set(true));
    without_allocating(|| {
        let call = client
            .put("upload")
            .to(PEER)
            .payload(&BODY)
            .if_none_match()
            .q_block1()
            .non()
            .request_tag(BodyTag::new(b"transfer").unwrap())
            .deadline(100)
            .send(0)
            .unwrap();
        let initial = client.transport_mut().take().unwrap();
        let later = client.transport_mut().take().unwrap();
        server.transport_mut().incoming = Some(later);
        server.poll(1).unwrap();
        assert_eq!(DISPATCHES.with(Cell::get), 0);
        server.transport_mut().incoming = Some(initial);
        server.poll(2).unwrap();
        assert_eq!(DISPATCHES.with(Cell::get), 1);
        assert_eq!(EFFECTS.with(Cell::get), 1);
        client.transport_mut().incoming = server.transport_mut().take();
        assert!(client.transport_mut().incoming.is_some());
        client.poll(2).unwrap();
        assert!(client.take_response(call).is_none());
        client.poll(100).unwrap();
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            CallFailure::DeadlineExceeded
        );
        DISPATCHES.with(|v| v.set(0));
        assert_eq!(
            complete(&mut client, &mut server, 10000, true, false, true, false),
            Code::PRECONDITION_FAILED
        );
        assert_eq!(
            EFFECTS.with(Cell::get),
            1,
            "uncertain first receipt must not duplicate creation"
        );
        reset(false);
        assert_eq!(
            complete(&mut client, &mut server, 20000, true, false, true, false),
            Code::CREATED
        );
    });
}

#[test]
fn q_if_none_match_plain_missing_num0_reissue_retains_condition() {
    let mut client = client();
    let mut server = server();
    reset(false);
    without_allocating(|| {
        assert_eq!(
            complete(&mut client, &mut server, 0, true, false, false, true),
            Code::CREATED
        );
        assert_eq!(EFFECTS.with(Cell::get), 1);
    });
}
fn reclaim<C: AppStorage, S: AppStorage>(
    client: &mut Client<C>,
    server: &mut Client<S>,
    protected: bool,
) {
    use coaptic::app::CallFailure;
    for cycle in 0..16 {
        let now = 100000 + cycle * 1000;
        let call = client
            .put("upload")
            .to(PEER)
            .payload(&BODY)
            .if_none_match()
            .q_block1()
            .request_tag(BodyTag::new(b"transfer").unwrap())
            .deadline(now + 10)
            .send(now)
            .unwrap();
        assert!(client.transport_mut().take().is_some());
        assert_eq!(client.transport_mut().len, 0);
        if cycle % 2 == 0 {
            assert!(client.cancel(call));
            assert_eq!(
                client.take_response(call).unwrap().unwrap_err(),
                CallFailure::Cancelled
            );
        } else {
            client.poll(now + 10).unwrap();
            assert_eq!(
                client.take_response(call).unwrap().unwrap_err(),
                CallFailure::DeadlineExceeded
            );
        }
    }
    let first = client
        .put("upload")
        .to(PEER)
        .payload(&BODY)
        .if_none_match()
        .q_block1()
        .non()
        .request_tag(BodyTag::new(b"first").unwrap())
        .send(120000)
        .unwrap();
    let second = client
        .put("upload")
        .to(Endpoint::v4([192, 0, 2, 3], 5683))
        .payload(&BODY)
        .if_none_match()
        .q_block1()
        .non()
        .request_tag(BodyTag::new(b"second").unwrap())
        .send(120000)
        .unwrap();
    assert_eq!(client.transport_mut().len, 4);
    for _ in 0..16 {
        assert_eq!(
            client
                .put("upload")
                .to(Endpoint::v4([192, 0, 2, 4], 5683))
                .payload(&BODY)
                .if_none_match()
                .q_block1()
                .non()
                .request_tag(BodyTag::new(b"third").unwrap())
                .send(120001),
            Err(coaptic::app::Error::Block(
                coaptic::error::BlockTransferError::Saturated
            ))
        );
        assert_eq!(client.transport_mut().len, 4);
    }
    for call in [first, second] {
        assert!(client.cancel(call));
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            CallFailure::Cancelled
        );
    }
    for _ in 0..4 {
        client.transport_mut().take().unwrap();
    }
    reset(false);
    assert_eq!(
        complete(client, server, 200000, true, false, protected, true),
        Code::CREATED
    );
}

#[test]
fn q_if_none_match_failed_num0_does_not_commit_unknown_condition() {
    let mut client = client();
    let mut server = server();
    reset(false);
    CONDITIONAL.with(|v| v.set(false));
    RAW_CONDITION.with(|v| v.set(false));
    without_allocating(|| {
        let call = client
            .put("upload")
            .to(PEER)
            .payload(&BODY)
            .q_block1()
            .non()
            .request_tag(BodyTag::new(b"transfer").unwrap())
            .send(0)
            .unwrap();
        let initial = client.transport_mut().take().unwrap();
        let later = client.transport_mut().take().unwrap();
        server.transport_mut().incoming = Some(later);
        server.poll(1).unwrap();
        assert_eq!(DISPATCHES.with(Cell::get), 0);
        // Candidate condition is valid, but the non-final NUM0 payload is short.
        // Refusing its body must not promote unknown metadata to Some(true).
        server.transport_mut().incoming = Some(altered(initial, 4));
        server.poll(2).unwrap();
        let response = server.transport_mut().take().unwrap();
        assert!(
            decode(&response.bytes[..response.len])
                .unwrap()
                .code()
                .is_client_error()
        );
        assert_eq!(DISPATCHES.with(Cell::get), 0);
        server.transport_mut().incoming = Some(initial);
        server.poll(3).unwrap();
        client.transport_mut().incoming = server.transport_mut().take();
        client.poll(3).unwrap();
        assert_eq!(
            client.take_response(call).unwrap().unwrap().code(),
            Code::CREATED
        );
        assert_eq!(EFFECTS.with(Cell::get), 1);
    });
}
