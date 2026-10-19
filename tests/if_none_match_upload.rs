//! Classic Block1 retains NUM0 If-None-Match without repeating it on later blocks.
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
fn replace(request: Request<'_>) -> Response<'static> {
    DISPATCHES.with(|v| v.set(v.get() + 1));
    assert_eq!(request.body(), Some(BODY.as_slice()));
    assert!(
        request.if_none_match(),
        "effective NUM0 condition survives assembly"
    );
    assert!(
        !request.message().if_none_match(),
        "raw completing datagram omits NUM0 option"
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
    .allow_unprotected()
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
    .allow_unprotected()
    .deterministic_for_tests()
    .bind(Io::default())
    .unwrap()
}
fn reset(exists: bool) {
    REPRESENTATION.with(|v| v.set(exists.then_some(1)));
    EFFECTS.with(|v| v.set(0));
    DISPATCHES.with(|v| v.set(0));
}
fn complete<
    S: AppStorage<Token = coaptic::message::Token>,
    R: AppStorage<Token = coaptic::message::Token>,
>(
    client: &mut Client<S>,
    server: &mut Client<R>,
    now: u64,
    create_between: bool,
    protected: bool,
) -> Code {
    let call = client
        .put("upload")
        .to(PEER)
        .payload(&BODY)
        .if_none_match()
        .request_tag(BodyTag::new(b"transfer").unwrap())
        .send(now)
        .unwrap();
    let initial = client.transport_mut().take().unwrap();
    if !protected {
        assert!(
            decode(&initial.bytes[..initial.len])
                .unwrap()
                .if_none_match()
        );
    }
    server.transport_mut().incoming = Some(initial);
    server.poll(now + 1).unwrap();
    if create_between {
        REPRESENTATION.with(|v| v.set(Some(2)));
    }
    client.transport_mut().incoming = server.transport_mut().take();
    client.poll(now + 1).unwrap();
    let later = client.transport_mut().take().unwrap();
    let parsed = decode(&later.bytes[..later.len]).unwrap();
    if protected {
        assert!(parsed.oscore().is_some());
    }
    assert!(!parsed.if_none_match());
    server.transport_mut().incoming = Some(later);
    server.poll(now + 2).unwrap();
    client.transport_mut().incoming = server.transport_mut().take();
    client.poll(now + 2).unwrap();
    client.take_response(call).unwrap().unwrap().code()
}
fn exercise<
    S: AppStorage<Token = coaptic::message::Token>,
    R: AppStorage<Token = coaptic::message::Token>,
>(
    mut client: Client<S>,
    mut server: Client<R>,
    protected: bool,
) {
    #[cfg(feature = "oscore")]
    if protected {
        client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
        server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
    }
    without_allocating(|| {
        for (i, (exists, create_between)) in
            [(false, false), (true, false), (false, true), (false, false)]
                .into_iter()
                .enumerate()
        {
            reset(exists);
            let code = complete(
                &mut client,
                &mut server,
                i as u64 * 100,
                create_between,
                protected,
            );
            let success = !exists && !create_between;
            assert_eq!(
                code,
                if success {
                    Code::CREATED
                } else {
                    Code::PRECONDITION_FAILED
                }
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), usize::from(success));
        }
    });
}
#[test]
fn if_none_match_classic_complete_current_existence_and_recovery() {
    exercise(client(), server(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn if_none_match_protected_classic_inner_condition_and_recovery() {
    exercise(client(), server(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn if_none_match_classic_bounded_heap_has_same_behavior() {
    let bytes = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let resources = || {
            Resources::<Buffers>::new()
                .routes::<1>()
                .block_wise::<true>()
                .allocated(bytes)
        };
        let client = App::with_resources(resources())
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        let server = App::with_resources(resources())
            .allow_unprotected()
            .deterministic_for_tests()
            .route("upload", put(replace))
            .bind(Io::default())
            .unwrap();
        exercise(client, server, protected);
    }
}

fn alter(frame: Frame, mid: u16, condition: bool, peer: Endpoint, poison: bool) -> Frame {
    use coaptic::message::{Message, MessageId, Opt, OptionNumber, OptionsBuilder, encode};
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    let mut options = OptionsBuilder::<16>::new();
    for option in parsed.options() {
        if option.number() != OptionNumber::IF_NONE_MATCH {
            options.push(option).unwrap();
        }
    }
    if condition {
        options.push(Opt::if_none_match()).unwrap();
    }
    let bad = [0xee; 1024];
    let message = Message::new(parsed.ty(), parsed.code(), MessageId::new(mid))
        .with_token(parsed.token())
        .with_options(options.as_slice())
        .with_payload(if poison {
            &bad[..parsed.payload().len()]
        } else {
            parsed.payload()
        });
    let mut changed = Frame {
        peer,
        ..Frame::default()
    };
    changed.len = encode(&message, &mut changed.bytes).unwrap();
    changed
}
#[test]
fn if_none_match_illegal_later_and_changed_num0_refuse_before_body_mutation() {
    for mutation in 0..3 {
        let mut client = client();
        let mut server = server();
        reset(false);
        without_allocating(|| {
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_none_match()
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(0)
                .unwrap();
            let initial = client.transport_mut().take().unwrap();
            server.transport_mut().incoming = Some(initial);
            server.poll(1).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(1).unwrap();
            let final_block = client.transport_mut().take().unwrap();
            let changed = match mutation {
                0 => alter(final_block, 60000, true, PEER, true),
                1 => alter(initial, 60000, false, PEER, true),
                _ => alter(
                    final_block,
                    60000,
                    false,
                    Endpoint::v4([192, 0, 2, 3], 5683),
                    true,
                ),
            };
            server.transport_mut().incoming = Some(changed);
            server.poll(2).unwrap();
            let refusal = server.transport_mut().take().unwrap();
            assert_eq!(
                decode(&refusal.bytes[..refusal.len]).unwrap().code(),
                Code::REQUEST_ENTITY_INCOMPLETE
            );
            assert_eq!(DISPATCHES.with(Cell::get), 0);
            assert_eq!(EFFECTS.with(Cell::get), 0);
            server.transport_mut().incoming = Some(final_block);
            server.poll(3).unwrap();
            client.transport_mut().incoming = server.transport_mut().take();
            client.poll(3).unwrap();
            assert_eq!(
                client.take_response(call).unwrap().unwrap().code(),
                Code::CREATED
            );
            assert_eq!(DISPATCHES.with(Cell::get), 1);
            assert_eq!(EFFECTS.with(Cell::get), 1);
        });
    }
}
#[test]
fn if_none_match_two_complete_owners_cannot_both_create() {
    let mut first = client();
    let mut second = client();
    let mut server = server();
    reset(false);
    let other = Endpoint::v4([192, 0, 2, 3], 5683);
    without_allocating(|| {
        let a = first
            .put("upload")
            .to(PEER)
            .payload(&BODY)
            .if_none_match()
            .request_tag(BodyTag::new(b"first").unwrap())
            .send(0)
            .unwrap();
        let b = second
            .put("upload")
            .to(other)
            .payload(&BODY)
            .if_none_match()
            .request_tag(BodyTag::new(b"second").unwrap())
            .send(0)
            .unwrap();
        server.transport_mut().incoming = first.transport_mut().take();
        server.poll(1).unwrap();
        first.transport_mut().incoming = server.transport_mut().take();
        first.poll(1).unwrap();
        server.transport_mut().incoming = second.transport_mut().take();
        server.poll(2).unwrap();
        second.transport_mut().incoming = server.transport_mut().take();
        second.poll(2).unwrap();
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
fn if_none_match_num0_retry_is_exact_and_completes_after_loss() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut client = client();
        let mut server = server();
        REPRESENTATION.with(|v| v.set(None));
        EFFECTS.with(|v| v.set(0));
        DISPATCHES.with(|v| v.set(0));
        #[cfg(feature = "oscore")]
        if protected {
            client.set_oscore(&mut Some(context(&[1], &[2]))).unwrap();
            server.set_oscore(&mut Some(context(&[2], &[1]))).unwrap();
        }
        without_allocating(|| {
            let request = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_none_match()
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

#[test]
fn fragmented_etag_refusal_cancel_and_deadline_reclaim_before_recovery() {
    use coaptic::app::{CallFailure, Error};
    let mut client = client();
    without_allocating(|| {
        for cycle in 0..16 {
            let now = cycle * 100;
            assert_eq!(
                client
                    .put("upload")
                    .to(PEER)
                    .payload(&BODY)
                    .etag(b"version")
                    .request_tag(BodyTag::new(b"transfer").unwrap())
                    .q_block1()
                    .send(now),
                Err(Error::ConditionalUploadUnsupported)
            );
            assert_eq!(client.transport_mut().len, 0);
            let call = client
                .put("upload")
                .to(PEER)
                .payload(&BODY)
                .if_none_match()
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .deadline(now + 10)
                .send(now)
                .unwrap();
            assert!(client.transport_mut().take().is_some());
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
            assert_eq!(client.transport_mut().len, 0);
        }
        let mut server = server();
        reset(false);
        assert_eq!(
            complete(&mut client, &mut server, 2000, false, false),
            Code::CREATED
        );
        assert_eq!(EFFECTS.with(Cell::get), 1);
    });
}
#[test]
fn if_none_match_added_option_capacity_shrinks_without_stale_ownership() {
    let path = core::str::from_utf8(&[b'x'; 255]).unwrap();
    let mut client = client();
    without_allocating(|| {
        for now in 0..16 {
            let call = client
                .put(path)
                .to(PEER)
                .payload(&BODY)
                .if_none_match()
                .request_tag(BodyTag::new(b"transfer").unwrap())
                .send(now)
                .unwrap();
            let frame = client.transport_mut().take().unwrap();
            let parsed = decode(&frame.bytes[..frame.len]).unwrap();
            assert_eq!(parsed.block1().unwrap().unwrap().szx(), 5);
            assert!(parsed.if_none_match());
            assert_eq!(parsed.payload(), &BODY[..512]);
            assert!(client.cancel(call));
            assert!(client.take_response(call).unwrap().is_err());
            assert_eq!(client.transport_mut().len, 0);
        }
    });
    exercise(client, server(), false);
}
