//! Known-size NON Q1 recovery retains bounded receiver state and authenticated ownership.
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
        rx_datagram: (4, 256), tx_datagram: (4, 256),
        rx_body: (2, 1024), tx_body: (2, 1024),
        dedup: 16, observe: 1,
    }
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const BODY: [u8; 512] = {
    let mut bytes = [0; 512];
    let mut index = 0;
    while index < bytes.len() {
        bytes[index] = (index * 31 + index / 512) as u8;
        index += 1;
    }
    bytes
};
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 256],
    len: usize,
    peer: Endpoint,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 256],
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
    type Error = &'static str;
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

type Server<S = coaptic::storage::Memory<Buffers, coaptic::storage::WithBodies<Buffers>>> =
    App<Buffers, Io, 1, true, S, 2>;
fn fixed() -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_unprotected()
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

#[allow(clippy::too_many_arguments)]
fn fragment(
    peer: &mut Peer,
    mid: u16,
    num: u32,
    size: u32,
    path: &str,
    tag: &[u8],
    more: bool,
    payload: &[u8],
) -> Frame {
    fragment_typed(
        peer,
        mid,
        num,
        size,
        path,
        tag,
        more,
        payload,
        Type::NonConfirmable,
    )
}
#[allow(clippy::too_many_arguments)]
fn fragment_typed(
    peer: &mut Peer,
    mid: u16,
    num: u32,
    size: u32,
    path: &str,
    tag: &[u8],
    more: bool,
    payload: &[u8],
    ty: Type,
) -> Frame {
    let block = BlockValue::new(num, more, 0).unwrap().encode();
    let size = coaptic::message::encode_uint(size);
    let options = [
        Opt::uri_path(path),
        Opt::q_block1(&block),
        Opt::size1(&size),
        Opt::request_tag(tag),
    ];
    peer.encode(
        &Message::new(ty, Code::PUT, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(&options)
            .with_payload(payload),
    )
}
fn valid(peer: &mut Peer, mid: u16, num: u32) -> Frame {
    fragment(
        peer,
        mid,
        num,
        BODY.len() as u32,
        "upload",
        b"body",
        num != 31,
        &BODY[num as usize * 16..num as usize * 16 + 16],
    )
}
fn poll<S: AppStorage>(server: &mut Server<S>, now: u64, effects: &Cell<usize>) {
    without_allocating(|| {
        server
            .poll_with(now, |request, _| {
                assert_eq!(request.body(), Some(BODY.as_slice()));
                effects.set(effects.get() + 1);
                Response::new(Code::CHANGED)
            })
            .unwrap()
    });
}
fn drain<S: AppStorage>(server: &mut Server<S>, peer: &Peer) {
    while let Some(frame) = server.transport_mut().take() {
        let frame = peer.open(frame);
        let parsed = decode(&frame.bytes[..frame.len]).unwrap();
        assert!(
            parsed.code() == Code::CONTINUE || parsed.code() == Code::CHANGED,
            "unexpected response {:?}",
            parsed.code()
        );
    }
}
fn report<S: AppStorage>(
    server: &mut Server<S>,
    peer: &Peer,
    now: u64,
    expected: &[u32],
    effects: &Cell<usize>,
) {
    poll(server, now, effects);
    let frame = peer.open(
        server
            .transport_mut()
            .take()
            .expect("bounded missing report"),
    );
    let parsed = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(parsed.code(), Code::REQUEST_ENTITY_INCOMPLETE);
    assert_eq!(
        parsed.content_format().unwrap().unwrap(),
        ContentFormat::MISSING_BLOCKS
    );
    let mut nums = [0; 10];
    let count = coaptic::message::MissingBlocks::decode(parsed.payload(), &mut nums).unwrap();
    assert_eq!(&nums[..count], expected);
    assert!(server.transport_mut().take().is_none());
}
fn campaign<S: AppStorage>(mut server: Server<S>, protected: bool) {
    let effects = Cell::new(0);
    let mut peer = Peer::new(&mut server, protected);
    for num in 1..32 {
        server.transport_mut().incoming = Some(valid(&mut peer, num as u16, num));
        poll(&mut server, 0, &effects);
        assert!(
            server.transport_mut().take().is_none(),
            "future payload must not cause fatal generic 4.08"
        );
    }
    assert_eq!(effects.get(), 0);
    report(&mut server, &peer, 4_000, &[0], &effects);
    let groups: [&[u32]; 4] = [
        &[0],
        &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
        &[20, 21, 22, 23, 24, 25, 26, 27, 28, 29],
        &[30, 31],
    ];
    for (index, nums) in groups.iter().enumerate() {
        let now = 4_001 + index as u64 * 4_001;
        for num in *nums {
            server.transport_mut().incoming = Some(valid(&mut peer, 100 + *num as u16, *num));
            poll(&mut server, now, &effects);
            drain(&mut server, &peer);
        }
        if index + 1 < groups.len() {
            assert_eq!(effects.get(), 0);
            report(&mut server, &peer, now + 4_000, groups[index + 1], &effects);
        }
    }
    assert_eq!(effects.get(), 1);
}
#[test]
fn q1_empty_windows_repair_full_body_without_allocation() {
    campaign(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn q1_empty_windows_use_latest_validated_protected_request() {
    campaign(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn q1_empty_windows_allocated_exact_budget_repair_without_growth() {
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
            .allow_unprotected()
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        campaign(app, protected);
    }
}

fn slot<S: AppStorage>(server: &Server<S>) -> Option<coaptic::storage::SlotId> {
    server.engine().lookup_rx_body(
        coaptic::storage::BlockKey::new(Token::new(b"owner").unwrap(), PEER)
            .with_identity(coaptic::storage::BodyTag::new(b"body").unwrap()),
    )
}
fn transfer<S: AppStorage>(server: &Server<S>) -> coaptic::storage::BlockTransfer {
    server
        .engine()
        .rx_body_transfer(slot(server).unwrap())
        .expect("owned partial")
}
#[test]
fn q1_future_refusal_preserves_body_metadata_owner_and_fresh_recovery() {
    for protected in [false, cfg!(feature = "oscore")] {
        for mutation in 0..10 {
            let mut server = fixed();
            let effects = Cell::new(0);
            let mut peer = Peer::new(&mut server, protected);
            for num in 1..10 {
                server.transport_mut().incoming = Some(valid(&mut peer, num as u16, num));
                poll(&mut server, 0, &effects);
                assert!(server.transport_mut().take().is_none());
            }
            let before = transfer(&server);
            let mut bytes = [0; 1024];
            let original = server
                .engine()
                .rx_body_payload(slot(&server).unwrap())
                .unwrap();
            bytes[..original.len()].copy_from_slice(original);
            let len = original.len();
            let (num, size, path, tag, more, payload): (_, _, _, &[u8], _, &[u8]) = match mutation {
                0 => (10, 512, "other", b"body", true, &BODY[160..176]),
                1 => (10, 511, "upload", b"body", true, &BODY[160..176]),
                2 => (10, 512, "upload", b"other", true, &BODY[160..176]),
                3 => (10, 512, "upload", b"body", true, &BODY[160..175]),
                4 => (64, 512, "upload", b"body", true, &BODY[..16]),
                5 => (31, 512, "upload", b"body", true, &BODY[496..]),
                6 => (10, 512, "upload", b"body", false, &BODY[160..176]),
                9 => (32, 512, "upload", b"body", false, &[]),
                _ => (10, 512, "upload", b"body", true, &BODY[160..176]),
            };
            let mut frame = fragment(&mut peer, 50, num, size, path, tag, more, payload);
            if mutation == 8 {
                let q = BlockValue::new(10, true, 1).unwrap().encode();
                let size = coaptic::message::encode_uint(512);
                let options = [
                    Opt::uri_path("upload"),
                    Opt::q_block1(&q),
                    Opt::size1(&size),
                    Opt::request_tag(b"body"),
                ];
                frame = peer.encode(
                    &Message::new(Type::NonConfirmable, Code::PUT, MessageId::new(51))
                        .with_token(Token::new(b"owner").unwrap())
                        .with_options(&options)
                        .with_payload(&BODY[320..352]),
                );
            }
            if mutation == 7 {
                frame.peer = Endpoint::v4([192, 0, 2, 99], 5683);
            }
            server.transport_mut().incoming = Some(frame);
            poll(&mut server, 1, &effects);
            assert_eq!(
                transfer(&server),
                before,
                "refused mutation {mutation} must preserve exact metadata, PIV owner and timer"
            );
            assert_eq!(
                server
                    .engine()
                    .rx_body_payload(slot(&server).unwrap())
                    .unwrap(),
                &bytes[..len]
            );
            while let Some(frame) = server.transport_mut().take() {
                let frame = peer.open(frame);
                assert!(
                    decode(&frame.bytes[..frame.len])
                        .unwrap()
                        .code()
                        .is_client_error()
                );
            }
            // A fresh authenticated future packet refreshes only the report binding.
            server.transport_mut().incoming = Some(valid(&mut peer, 60, 31));
            poll(&mut server, 2, &effects);
            assert!(server.transport_mut().take().is_none());
            assert_eq!(transfer(&server).q_receive(), before.q_receive());
            report(&mut server, &peer, 4000, &[0], &effects);
            for num in core::iter::once(0).chain(10..32) {
                server.transport_mut().incoming = Some(valid(&mut peer, 100 + num as u16, num));
                poll(&mut server, 4001, &effects);
                drain(&mut server, &peer);
            }
            assert_eq!(effects.get(), 1);
        }
    }
}
#[test]
fn q1_admitted_con_timer_and_mixed_type_retries_preserve_mode() {
    for first in [Type::Confirmable, Type::NonConfirmable] {
        let mut server = fixed();
        let effects = Cell::new(0);
        let mut peer = Peer::new(&mut server, false);
        server.transport_mut().incoming = Some(fragment_typed(
            &mut peer,
            1,
            0,
            512,
            "upload",
            b"body",
            true,
            &BODY[..16],
            first,
        ));
        poll(&mut server, 0, &effects);
        while server.transport_mut().take().is_some() {}
        let non = first == Type::NonConfirmable;
        assert_eq!(transfer(&server).q_holes().is_some(), non);
        let later = if non {
            Type::Confirmable
        } else {
            Type::NonConfirmable
        };
        // Both duplicate and actual next-block acceptance retain the initial mode.
        for num in [0, 1] {
            server.transport_mut().incoming = Some(fragment_typed(
                &mut peer,
                10 + num as u16,
                num,
                512,
                "upload",
                b"body",
                true,
                &BODY[num as usize * 16..num as usize * 16 + 16],
                later,
            ));
            poll(&mut server, 1, &effects);
            while server.transport_mut().take().is_some() {}
            assert_eq!(transfer(&server).q_holes().is_some(), non);
        }
        if !non {
            assert!(transfer(&server).q_receive().is_none());
            poll(&mut server, 20_000, &effects);
            assert!(
                server.transport_mut().take().is_none(),
                "CON contiguous prefix retains original no-hole timing"
            );
        }
    }
}
#[test]
fn q1_future_and_duplicate_traffic_cannot_reset_retry_budget_then_storage_recovers() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let effects = Cell::new(0);
        let mut peer = Peer::new(&mut server, protected);
        server.transport_mut().incoming = Some(valid(&mut peer, 1, 1));
        poll(&mut server, 0, &effects);
        for attempt in 0..5 {
            let wait = transfer(&server).q_receive().unwrap();
            assert_eq!(usize::from(wait.attempts()), attempt);
            let due = wait.next_timeout_ms();
            for num in [1, 31] {
                server.transport_mut().incoming =
                    Some(valid(&mut peer, 100 + attempt as u16 * 2 + num as u16, num));
                poll(&mut server, due - 1, &effects);
                assert_eq!(transfer(&server).q_receive(), Some(wait));
                assert!(server.transport_mut().take().is_none());
            }
            if attempt < 4 {
                report(
                    &mut server,
                    &peer,
                    due,
                    &[0, 2, 3, 4, 5, 6, 7, 8, 9],
                    &effects,
                );
            } else {
                poll(&mut server, due, &effects);
                assert!(server.transport_mut().take().is_none());
            }
        }
        assert!(slot(&server).is_none());
        assert_eq!(effects.get(), 0);
        for num in 0..32 {
            server.transport_mut().incoming = Some(valid(&mut peer, 300 + num as u16, num));
            poll(&mut server, 200_000, &effects);
            drain(&mut server, &peer);
        }
        assert_eq!(effects.get(), 1);
    }
}
#[cfg(feature = "oscore")]
#[test]
fn q1_unauthenticated_and_replayed_future_preserve_owner_then_valid_recovery() {
    let mut server = fixed();
    let effects = Cell::new(0);
    let mut peer = Peer::new(&mut server, true);
    for num in 1..10 {
        server.transport_mut().incoming = Some(valid(&mut peer, num as u16, num));
        poll(&mut server, 0, &effects);
    }
    let frame = valid(&mut peer, 40, 31);
    server.transport_mut().incoming = Some(frame);
    poll(&mut server, 1, &effects);
    let before = transfer(&server);
    let mut tampered = valid(&mut peer, 41, 31);
    tampered.bytes[tampered.len - 1] ^= 1;
    let mut wrong = Peer {
        context: Some(
            coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
                master_secret: &[0x99; 16],
                master_salt: &[],
                sender_id: &[1],
                recipient_id: &[2],
                id_context: &[],
            })
            .unwrap(),
        ),
    };
    for invalid in [frame, tampered, valid(&mut wrong, 42, 31)] {
        server.transport_mut().incoming = Some(invalid);
        poll(&mut server, 2, &effects);
        assert_eq!(transfer(&server), before);
        assert!(server.transport_mut().take().is_none());
        assert_eq!(effects.get(), 0);
    }
    server.transport_mut().incoming = Some(valid(&mut peer, 43, 31));
    poll(&mut server, 3, &effects);
    report(&mut server, &peer, 4000, &[0], &effects);
    for num in core::iter::once(0).chain(10..32) {
        server.transport_mut().incoming = Some(valid(&mut peer, 100 + num as u16, num));
        poll(&mut server, 4001, &effects);
        drain(&mut server, &peer);
    }
    assert_eq!(effects.get(), 1);
}
