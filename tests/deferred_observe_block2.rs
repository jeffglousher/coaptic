//! Deferred Observe retains a bounded large initial snapshot without redispatch.
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
    fail_send: bool,
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
        if self.fail_send {
            return Err("ambiguous send");
        }
        Ok(bytes.len())
    }
}

type Server<S = coaptic::storage::Memory<Buffers, coaptic::storage::WithBodies<Buffers>>> =
    App<Buffers, Io, 1, true, S, 2>;
fn fixed() -> Server {
    fixed_lifetime(247_000)
}
fn fixed_lifetime(lifetime: u64) -> Server {
    App::with_resources(
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>(),
    )
    .allow_unprotected()
    .deferred_lifetime(lifetime)
    .deterministic_for_tests()
    .route("obs", coaptic::get(handler).observe(source))
    .bind(Io::default())
    .unwrap()
}

struct Peer {
    #[cfg(feature = "oscore")]
    context: Option<coaptic::oscore::SecurityContext>,
    #[cfg(feature = "oscore")]
    observe: Option<coaptic::oscore::RequestRef>,
}
impl Peer {
    fn new<S: AppStorage<Token = coaptic::message::Token>>(
        server: &mut Server<S>,
        protected: bool,
    ) -> Self {
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
                observe: None,
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
            if message.options().iter().any(|o| {
                o.number() == coaptic::message::OptionNumber::OBSERVE && o.value().is_empty()
            }) {
                self.observe = context.lookup(message.token());
            }
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
            let request = if protected.observe().is_some() {
                self.observe.unwrap()
            } else {
                context.lookup(protected.token()).unwrap()
            };
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

fn handler(_: coaptic::Request<'_>) -> Response<'static> {
    response(b"sync")
}
fn source() -> Response<'static> {
    response(b"updated")
}
fn response(body: &'static [u8]) -> Response<'static> {
    Response::content(body).content_format(ContentFormat::TEXT_PLAIN)
}
fn request(peer: &mut Peer, mid: u16, ty: Type, observe: u32) -> Frame {
    let value = coaptic::message::encode_uint(observe);
    let options = [Opt::observe(&value), Opt::uri_path("obs")];
    peer.encode(
        &Message::new(ty, Code::GET, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(&options),
    )
}
fn control(mid: MessageId, ty: Type) -> Frame {
    let mut frame = Frame::default();
    frame.len = encode(&Message::new(ty, Code::EMPTY, mid), &mut frame.bytes).unwrap();
    frame
}
fn begin<S: AppStorage<Token = coaptic::message::Token>>(
    server: &mut Server<S>,
    peer: &mut Peer,
    ty: Type,
    mid: u16,
    now: u64,
) -> coaptic::app::DeferredReply {
    server.transport_mut().incoming = Some(request(peer, mid, ty, 0));
    let mut handle = None;
    without_allocating(|| {
        server
            .poll_with(now, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap()
    });
    if ty == Type::Confirmable {
        let frame = server.transport_mut().take().unwrap();
        let p = decode(&frame.bytes[..frame.len]).unwrap();
        assert!(p.is_empty_ack());
        assert_eq!(p.message_id(), MessageId::new(mid));
    }
    assert!(server.transport_mut().take().is_none());
    handle.expect("bounded Observe handle")
}
fn content<S: AppStorage<Token = coaptic::message::Token>>(
    server: &mut Server<S>,
    peer: &Peer,
    body: &[u8],
    seq: u32,
    ty: Type,
) -> MessageId {
    let frame = peer.open(server.transport_mut().take().expect("response"));
    let p = decode(&frame.bytes[..frame.len]).unwrap();
    assert_eq!(p.code(), Code::CONTENT);
    assert_eq!(p.ty(), ty);
    assert_eq!(p.token(), Token::new(b"owner").unwrap());
    assert_eq!(p.payload(), body);
    #[cfg(feature = "oscore")]
    let seq = if peer.context.is_some() { 0 } else { seq };
    assert_eq!(p.observe(), Some(Ok(seq)));
    assert_eq!(p.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
    p.message_id()
}

static LARGE: [u8; 700] = [b'x'; 700];
fn large() -> Response<'static> {
    response(&LARGE).etag(b"v1").max_age(17)
}

#[test]
fn q_observe_retains_epoch_snapshot_and_last_block_for_recovery() {
    for protected in [false, cfg!(feature = "oscore")] {
        for deferred in [false, true] {
            let mut s = fixed();
            let mut p = Peer::new(&mut s, protected);
            let encode_request = |p: &mut Peer, mid, num, more, observe| {
                let q = BlockValue::new(num, more, 1).unwrap().encode();
                let mut options = coaptic::message::OptionsBuilder::<3>::new();
                if observe {
                    options.push(Opt::observe_register()).unwrap();
                }
                options.push(Opt::uri_path("obs")).unwrap();
                options.push(Opt::q_block2(&q)).unwrap();
                p.encode(
                    &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(mid))
                        .with_token(key().token())
                        .with_options(options.as_slice()),
                )
            };
            s.transport_mut().incoming = Some(encode_request(&mut p, 200, 0, true, true));
            let mut handle = None;
            without_allocating(|| {
                s.poll_with(0, |_, candidate| {
                    if deferred {
                        handle = candidate;
                        Response::deferred()
                    } else {
                        large()
                    }
                })
                .unwrap()
            });
            if deferred {
                without_allocating(|| s.complete(handle.unwrap(), large(), 1).unwrap());
            }
            let take_set = |s: &mut Server, p: &Peer, first: u32, count: u32, observed: bool| {
                let mut sequence = None;
                for num in first..first + count {
                    let opened = p.open(s.transport_mut().take().expect("Q payload"));
                    let message = decode(&opened.bytes[..opened.len]).unwrap();
                    let block = message.q_block2().next().unwrap().unwrap();
                    assert_eq!(block.num(), num);
                    assert_eq!(block.szx(), 1);
                    assert_eq!(message.size2(), Some(Ok(LARGE.len() as u32)));
                    assert_eq!(message.etag().next(), Some(b"v1".as_slice()));
                    assert_eq!(message.observe().is_some(), observed);
                    if observed {
                        let current = message.observe().unwrap().unwrap();
                        assert!(sequence.is_none_or(|previous| previous == current));
                        sequence = Some(current);
                    }
                    let offset = num as usize * 32;
                    assert_eq!(
                        message.payload(),
                        &LARGE[offset..(offset + 32).min(LARGE.len())]
                    );
                }
                assert!(s.transport_mut().take().is_none());
                sequence
            };
            let epoch = take_set(&mut s, &p, 0, 10, true);
            for (num, count, clock) in [(10, 10, 2), (20, 2, 3)] {
                s.transport_mut().incoming =
                    Some(encode_request(&mut p, 200 + num as u16, num, true, false));
                without_allocating(|| {
                    s.poll_with(clock, |_, _| panic!("snapshot must not redispatch"))
                        .unwrap()
                });
                assert_eq!(take_set(&mut s, &p, num, count, true), epoch);
            }
            // Final transmission is not proof of delivery: recovery still
            // selects the same body, but carries no Observe option.
            s.transport_mut().incoming = Some(encode_request(&mut p, 230, 21, false, false));
            without_allocating(|| {
                s.poll_with(4, |_, _| panic!("recovery must not redispatch"))
                    .unwrap()
            });
            take_set(&mut s, &p, 21, 1, false);
            assert!(body(&mut s).is_some());
            assert!(s.engine_mut().lookup_observe(key()).is_some());
            without_allocating(|| s.notify(5000, &["obs"], large()).unwrap());
            let next = take_set(&mut s, &p, 0, 10, true);
            if !protected {
                assert_ne!(next, epoch);
            }
        }
    }
}

#[test]
fn q_observe_builder_negotiates_constrained_packets_and_delivers_updates() {
    for protected in [false, cfg!(feature = "oscore")] {
        for deferred in [false, true] {
            let mut server = fixed();
            let mut client = fixed();
            #[allow(unused_mut, unused_variables)]
            let mut peer = Peer::new(&mut server, protected);
            #[cfg(feature = "oscore")]
            if protected {
                client.set_oscore(&mut peer.context).unwrap();
            }
            let call = without_allocating(|| {
                client
                    .get("obs")
                    .to(PEER)
                    .observe()
                    .q_block2()
                    .send(0)
                    .unwrap()
            });
            let mut clock = 1;
            let mut delivered = 0;
            for _ in 0..80 {
                while let Some(frame) = client.transport_mut().take() {
                    server.transport_mut().incoming = Some(frame);
                    let mut handle = None;
                    without_allocating(|| {
                        server
                            .poll_with(clock, |_, candidate| {
                                if deferred {
                                    handle = candidate;
                                    Response::deferred()
                                } else {
                                    large()
                                }
                            })
                            .unwrap()
                    });
                    clock += 1;
                    if let Some(handle) = handle {
                        without_allocating(|| server.complete(handle, large(), clock).unwrap());
                        clock += 1;
                    }
                }
                while let Some(frame) = server.transport_mut().take() {
                    client.transport_mut().incoming = Some(frame);
                    without_allocating(|| client.poll(clock).unwrap());
                    clock += 1;
                }
                if let Some(reply) = client.take_response(call) {
                    let reply = reply.unwrap();
                    assert_eq!(reply.body(), Some(LARGE.as_slice()));
                    assert!(reply.observe_seq().is_some());
                    delivered += 1;
                    if delivered == 2 {
                        break;
                    }
                    clock += 5000;
                    without_allocating(|| server.notify(clock, &["obs"], large()).unwrap());
                }
            }
            assert_eq!(delivered, 2, "protected={protected}, deferred={deferred}");
        }
    }
}

#[test]
fn q_observe_initial_non_reset_retires_snapshot_and_relation() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, protected);
        let q = BlockValue::new(0, true, 1).unwrap().encode();
        let options = [
            Opt::observe_register(),
            Opt::uri_path("obs"),
            Opt::q_block2(&q),
        ];
        server.transport_mut().incoming = Some(
            peer.encode(
                &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(400))
                    .with_token(key().token())
                    .with_options(&options),
            ),
        );
        server.poll_with(0, |_, _| large()).unwrap();
        let mut last = None;
        while let Some(frame) = server.transport_mut().take() {
            last = Some(decode(&frame.bytes[..frame.len]).unwrap().message_id());
        }
        let last = last.expect("initial Q notification");
        server.transport_mut().incoming = Some(control(
            MessageId::new(last.get().wrapping_add(100)),
            Type::Reset,
        ));
        server.poll(1).unwrap();
        assert!(server.engine_mut().lookup_observe(key()).is_some());
        server.transport_mut().incoming = Some(control(last, Type::Reset));
        server.poll(2).unwrap();
        assert!(server.engine_mut().lookup_observe(key()).is_none());
        assert!(body(&mut server).is_none());
    }
}

#[test]
fn q_observe_first_block_preference_registers_and_continues_without_redispatch() {
    for deferred in [false, true] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, false);
        for (mid, num, more, registering, expected_count, observed) in [
            (500, 0, false, true, 1, true),
            (501, 1, true, false, 9, false),
            (502, 10, true, false, 10, true),
            (503, 20, true, false, 2, true),
        ] {
            let q = BlockValue::new(num, more, 1).unwrap().encode();
            let mut options = coaptic::message::OptionsBuilder::<3>::new();
            if registering {
                options.push(Opt::observe_register()).unwrap();
            }
            options.push(Opt::uri_path("obs")).unwrap();
            options.push(Opt::q_block2(&q)).unwrap();
            server.transport_mut().incoming = Some(
                peer.encode(
                    &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(mid))
                        .with_token(key().token())
                        .with_options(options.as_slice()),
                ),
            );
            let mut handle = None;
            server
                .poll_with(u64::from(mid), |_, candidate| {
                    assert!(registering, "retained selection must not redispatch");
                    if deferred {
                        handle = candidate;
                        Response::deferred()
                    } else {
                        large()
                    }
                })
                .unwrap();
            if let Some(handle) = handle {
                server.complete(handle, large(), u64::from(mid)).unwrap();
            }
            let mut count = 0;
            while let Some(frame) = server.transport_mut().take() {
                let message = decode(&frame.bytes[..frame.len]).unwrap();
                assert_eq!(
                    message.q_block2().next().unwrap().unwrap().num(),
                    num + count
                );
                assert_eq!(message.observe().is_some(), observed);
                count += 1;
            }
            assert_eq!(count, expected_count);
            assert!(server.engine_mut().lookup_observe(key()).is_some());
        }
    }
}

#[test]
fn q_observe_failed_deferred_send_keeps_cancel_and_reset_authority() {
    for reset in [false, true] {
        let mut server = fixed();
        let mut peer = Peer::new(&mut server, false);
        let q = BlockValue::new(0, true, 1).unwrap().encode();
        let options = [
            Opt::observe_register(),
            Opt::uri_path("obs"),
            Opt::q_block2(&q),
        ];
        server.transport_mut().incoming = Some(
            peer.encode(
                &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(600))
                    .with_token(key().token())
                    .with_options(&options),
            ),
        );
        let mut handle = None;
        server
            .poll_with(0, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
        let handle = handle.unwrap();
        server.transport_mut().fail_send = true;
        assert!(server.complete(handle, large(), 1).is_err());
        server.transport_mut().fail_send = false;
        let attempted = server.transport_mut().take().unwrap();
        let mid = decode(&attempted.bytes[..attempted.len])
            .unwrap()
            .message_id();
        assert!(body(&mut server).is_some());
        if reset {
            server.transport_mut().incoming = Some(control(mid, Type::Reset));
            server.poll(2).unwrap();
        } else {
            assert!(server.cancel_deferred(handle));
        }
        assert!(server.engine_mut().lookup_observe(key()).is_none());
        assert!(body(&mut server).is_none());
    }
}

fn key() -> coaptic::storage::ObserveKey {
    coaptic::storage::ObserveKey::new(Token::new(b"owner").unwrap(), PEER)
}
fn body<S: AppStorage<Token = coaptic::message::Token>>(
    s: &mut Server<S>,
) -> Option<coaptic::storage::SlotId> {
    s.engine_mut().lookup_tx_body(
        coaptic::storage::BlockKey::new(key().token(), PEER)
            .with_identity(coaptic::storage::BodyTag::new(b"v1").unwrap()),
    )
}
fn follow(peer: &mut Peer, mid: u16, block: BlockValue, path: &str) -> Frame {
    let value = block.encode();
    peer.encode(
        &Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
            .with_token(Token::new(b"owner").unwrap())
            .with_options(&[Opt::uri_path(path), Opt::block2(&value)]),
    )
}
fn first<S: AppStorage<Token = coaptic::message::Token>>(
    s: &mut Server<S>,
    p: &Peer,
) -> (MessageId, BlockValue) {
    let f = p.open(s.transport_mut().take().expect("first block"));
    let m = decode(&f.bytes[..f.len]).unwrap();
    assert_eq!(m.code(), Code::CONTENT);
    assert!(m.observe().is_some());
    let block = m.block2().unwrap().unwrap();
    assert_eq!(block.num(), 0);
    assert!(block.more());
    assert_eq!(m.payload(), &LARGE[..usize::from(block.size())]);
    assert_eq!(m.etag().next(), Some(&b"v1"[..]));
    assert_eq!(m.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
    assert_eq!(m.max_age(), Some(Ok(17)));
    (m.message_id(), block)
}
fn finish<S: AppStorage<Token = coaptic::message::Token>>(
    s: &mut Server<S>,
    p: &mut Peer,
    szx: u8,
    start: u64,
) {
    let size = 1usize << (szx + 4);
    let count = LARGE.len().div_ceil(size);
    let observing = s.engine_mut().lookup_observe(key()).is_some();
    for num in 1..count {
        s.transport_mut().incoming = Some(follow(
            p,
            100 + num as u16,
            BlockValue::new(num as u32, false, szx).unwrap(),
            "obs",
        ));
        without_allocating(|| {
            s.poll_with(start + num as u64, |_, _| {
                panic!("retained snapshot must not redispatch")
            })
        })
        .unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        assert_eq!(m.code(), Code::CONTENT);
        assert!(m.observe().is_none());
        let b = m.block2().unwrap().unwrap();
        assert_eq!(b.num(), num as u32);
        assert_eq!(b.szx(), szx);
        let end = ((num + 1) * size).min(LARGE.len());
        assert_eq!(m.payload(), &LARGE[num * size..end]);
        assert_eq!(b.more(), end < LARGE.len());
        assert_eq!(m.etag().next(), Some(&b"v1"[..]));
        assert_eq!(m.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
        assert_eq!(m.max_age(), Some(Ok(17)));
        assert_eq!(
            s.engine_mut().lookup_observe(key()).is_some(),
            observing,
            "ordinary follow-up preserves observation"
        );
    }
    assert!(body(s).is_none());
}
fn cycle<S: AppStorage<Token = coaptic::message::Token>>(mut s: Server<S>, protected: bool) {
    let mut p = Peer::new(&mut s, protected);
    let h = begin(&mut s, &mut p, Type::Confirmable, 1, 0);
    without_allocating(|| s.complete(h, large(), 1)).unwrap();
    let (mid, b) = first(&mut s, &p);
    assert!(s.complete(h, large(), 2).is_err());
    s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
    s.poll(2).unwrap();
    s.signal(&["obs"]);
    s.poll(3).unwrap();
    assert!(s.transport_mut().take().is_none());
    finish(&mut s, &mut p, b.szx(), 10);
    s.poll(5000).unwrap();
    content(&mut s, &p, b"updated", 1, Type::NonConfirmable);
}
#[test]
fn deferred_observe_block2_fixed_complete_snapshot_then_notification() {
    cycle(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_observe_block2_protected_complete_snapshot_then_notification() {
    cycle(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_observe_block2_allocated_exact_budget_matches_fixed() {
    let resources = || {
        Resources::<Buffers>::new()
            .routes::<1>()
            .block_wise::<true>()
            .deferred::<2>()
    };
    let budget = resources()
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    for protected in [false, cfg!(feature = "oscore")] {
        let s = App::with_resources(resources().allocated(budget))
            .allow_unprotected()
            .deterministic_for_tests()
            .route("obs", coaptic::get(handler).observe(source))
            .bind(Io::default())
            .unwrap();
        cycle(s, protected);
    }
}
#[test]
fn deferred_observe_block2_missing_etag_and_full_pool_refuse_then_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::Confirmable, 2, 0);
        assert!(without_allocating(|| s.complete(h, response(&LARGE), 1)).is_err());
        assert!(s.transport_mut().take().is_none());
        assert!(body(&mut s).is_none());
        assert!(s.engine_mut().lookup_observe(key()).is_none());
        let a = s.engine_mut().acquire_tx_body().unwrap();
        let b = s.engine_mut().acquire_tx_body().unwrap();
        assert!(without_allocating(|| s.complete(h, large(), 2)).is_err());
        assert!(s.transport_mut().take().is_none());
        assert_eq!(s.engine_mut().tx_occupied(), 0);
        s.engine_mut().release_tx_body(a).unwrap();
        s.engine_mut().release_tx_body(b).unwrap();
        s.complete(h, large(), 3).unwrap();
        first(&mut s, &p);
    }
}
#[test]
fn deferred_observe_block2_failed_send_preserves_snapshot_and_latest_reset_owner() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::Confirmable, 3, 0);
        s.transport_mut().fail_send = true;
        assert!(s.complete(h, large(), 1).is_err());
        let (old, _) = first(&mut s, &p);
        s.transport_mut().fail_send = false;
        let id = body(&mut s).unwrap();
        for (index, r) in [
            response(b"changed").etag(b"v1"),
            response(&LARGE).etag(b"v2"),
            large().max_age(99),
            large().content_format(ContentFormat::CBOR),
            Response::bad_request(),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(without_allocating(|| s.complete(h, r, index as u64 + 2)).is_err());
            assert!(s.transport_mut().take().is_none());
            assert_eq!(s.engine_mut().tx_body_payload(id), Some(&LARGE[..]));
        }
        #[cfg(feature = "oscore")]
        let before = s.oscore().map(|c| c.sender_seq());
        s.complete(h, large(), 8).unwrap();
        let (current, b) = first(&mut s, &p);
        assert_ne!(old, current);
        #[cfg(feature = "oscore")]
        if protected {
            assert!(s.oscore().unwrap().sender_seq() > before.unwrap());
        }
        s.transport_mut().incoming = Some(control(old, Type::Reset));
        s.poll(9).unwrap();
        assert!(s.engine_mut().lookup_observe(key()).is_some());
        assert_eq!(body(&mut s), Some(id));
        s.transport_mut().incoming = Some(control(current, Type::Acknowledgement));
        s.poll(10).unwrap();
        finish(&mut s, &mut p, b.szx(), 20);
    }
}
#[test]
fn deferred_observe_block2_wrong_selection_and_range_preserve_snapshot() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::Confirmable, 4, 0);
        s.complete(h, large(), 1).unwrap();
        let (mid, b) = first(&mut s, &p);
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(2).unwrap();
        let id = body(&mut s).unwrap();
        for (i, (path, num)) in [("other", 1), ("obs", 999)].into_iter().enumerate() {
            s.transport_mut().incoming = Some(follow(
                &mut p,
                20 + i as u16,
                BlockValue::new(num, false, b.szx()).unwrap(),
                path,
            ));
            s.poll_with(3 + i as u64, |_, _| panic!("must refuse without dispatch"))
                .unwrap();
            let f = p.open(s.transport_mut().take().unwrap());
            assert_eq!(decode(&f.bytes[..f.len]).unwrap().code(), Code::BAD_REQUEST);
            assert_eq!(body(&mut s), Some(id));
            assert_eq!(s.engine_mut().tx_body_payload(id), Some(&LARGE[..]));
            assert!(s.engine_mut().lookup_observe(key()).is_some());
        }
        let block = BlockValue::new(1, false, b.szx()).unwrap().encode();
        let options = [
            Opt::if_match(b"changed"),
            Opt::uri_path("obs"),
            Opt::block2(&block),
        ];
        s.transport_mut().incoming = Some(
            p.encode(
                &Message::new(Type::Confirmable, Code::GET, MessageId::new(22))
                    .with_token(key().token())
                    .with_options(&options),
            ),
        );
        s.poll_with(5, |_, _| {
            panic!("changed conditional selection must not redispatch")
        })
        .unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        assert_eq!(decode(&f.bytes[..f.len]).unwrap().code(), Code::BAD_REQUEST);
        static FOREIGN: [u8; 700] = [b'z'; 700];
        for (i, (token, endpoint)) in [
            (key().token(), Endpoint::v4([192, 0, 2, 3], 5683)),
            (Token::new(b"foreign").unwrap(), PEER),
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = p.encode(
                &Message::new(Type::Confirmable, Code::GET, MessageId::new(23 + i as u16))
                    .with_token(token)
                    .with_options(&[Opt::uri_path("obs"), Opt::block2(&block)]),
            );
            request.peer = endpoint;
            s.transport_mut().incoming = Some(request);
            let mut dispatched = 0;
            without_allocating(|| {
                s.poll_with(6 + i as u64, |_, _| {
                    dispatched += 1;
                    response(&FOREIGN).etag(b"foreign")
                })
            })
            .unwrap();
            assert_eq!(dispatched, 1, "foreign owner performs its own request");
            let f = p.open(s.transport_mut().take().unwrap());
            let parsed = decode(&f.bytes[..f.len]).unwrap();
            assert_eq!(parsed.code(), Code::CONTENT);
            assert!(parsed.payload().iter().all(|byte| *byte == b'z'));
            assert!(!parsed.payload().is_empty());
            assert_eq!(body(&mut s), Some(id));
            assert_eq!(s.engine_mut().tx_body_payload(id), Some(&LARGE[..]));
        }
        finish(&mut s, &mut p, b.szx(), 10);
    }
}
#[test]
fn deferred_observe_block2_completed_expiry_unblocks_live_relation() {
    let mut s = fixed_lifetime(100);
    let mut p = Peer::new(&mut s, false);
    let h = begin(&mut s, &mut p, Type::Confirmable, 5, 0);
    s.complete(h, large(), 1).unwrap();
    let (mid, _) = first(&mut s, &p);
    s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
    s.poll(2).unwrap();
    s.signal(&["obs"]);
    s.poll(99).unwrap();
    assert!(s.transport_mut().take().is_none());
    assert!(body(&mut s).is_some());
    s.poll(100).unwrap();
    assert!(body(&mut s).is_none());
    assert!(s.engine_mut().lookup_observe(key()).is_some());
    content(&mut s, &p, b"updated", 1, Type::NonConfirmable);
}
#[test]
fn deferred_observe_block2_replacement_owns_reused_snapshot_and_deadline() {
    let mut s = fixed_lifetime(100);
    let mut p = Peer::new(&mut s, false);
    let h = begin(&mut s, &mut p, Type::Confirmable, 6, 0);
    s.complete(h, large(), 1).unwrap();
    let (mid, _) = first(&mut s, &p);
    s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
    s.poll(2).unwrap();
    let oldbody = body(&mut s).unwrap();
    let held = s.engine_mut().acquire_tx_body().unwrap();
    let next = begin(&mut s, &mut p, Type::Confirmable, 7, 50);
    s.complete(next, large(), 51).unwrap();
    let (newmid, b) = first(&mut s, &p);
    assert_eq!(body(&mut s), Some(oldbody));
    s.transport_mut().incoming = Some(control(newmid, Type::Acknowledgement));
    s.poll(52).unwrap();
    s.poll(100).unwrap();
    assert!(s.transport_mut().take().is_none());
    assert!(body(&mut s).is_some());
    assert!(s.engine_mut().lookup_observe(key()).is_some());
    finish(&mut s, &mut p, b.szx(), 110);
    s.engine_mut().release_tx_body(held).unwrap();
}

#[test]
fn deferred_observe_block2_initial_reset_timeout_and_pending_expiry_reclaim() {
    for protected in [false, cfg!(feature = "oscore")] {
        for mode in 0..3 {
            let mut s = fixed_lifetime(if mode == 2 { 100 } else { 247000 });
            let mut p = Peer::new(&mut s, protected);
            let h = begin(&mut s, &mut p, Type::Confirmable, 30, 0);
            s.transport_mut().fail_send = mode == 2;
            let sent = s.complete(h, large(), 1);
            assert_eq!(sent.is_err(), mode == 2);
            let (mid, _) = first(&mut s, &p);
            s.transport_mut().fail_send = false;
            if mode == 0 {
                let mut foreign = control(mid, Type::Reset);
                foreign.peer = Endpoint::v4([192, 0, 2, 3], 5683);
                s.transport_mut().incoming = Some(foreign);
                s.poll(2).unwrap();
                assert!(body(&mut s).is_some());
                s.transport_mut().incoming = Some(control(mid, Type::Reset));
                s.poll(3).unwrap();
            } else if mode == 1 {
                for now in [3001, 9001, 21001, 45001, 93001] {
                    s.poll(now).unwrap();
                    while s.transport_mut().take().is_some() {}
                }
            } else {
                s.poll(100).unwrap();
            }
            assert!(body(&mut s).is_none());
            assert!(s.engine_mut().lookup_observe(key()).is_none());
            assert!(s.complete(h, large(), 100001).is_err());
            let fresh = begin(&mut s, &mut p, Type::Confirmable, 31, 100002);
            s.complete(fresh, large(), 100003).unwrap();
            first(&mut s, &p);
        }
    }
}
#[test]
fn deferred_observe_block2_pending_cancel_and_pinned_retry_preserve_ownership() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::Confirmable, 40, 0);
        s.transport_mut().fail_send = true;
        assert!(s.complete(h, large(), 1).is_err());
        first(&mut s, &p);
        s.transport_mut().fail_send = false;
        assert!(s.cancel_deferred(h));
        assert!(body(&mut s).is_none());
        assert!(s.engine_mut().lookup_observe(key()).is_none());
        let next = begin(&mut s, &mut p, Type::Confirmable, 41, 2);
        s.transport_mut().fail_send = true;
        assert!(s.complete(next, large(), 3).is_err());
        first(&mut s, &p);
        s.transport_mut().fail_send = false;
        let id = body(&mut s).unwrap();
        let access = s.engine_mut().access_tx_body_mut(id).unwrap();
        core::mem::forget(access);
        assert!(s.complete(next, large(), 4).is_err());
        assert!(!s.cancel_deferred(next));
        assert!(s.transport_mut().take().is_none());
        assert_eq!(body(&mut s), Some(id));
        assert!(s.engine_mut().lookup_observe(key()).is_some());
        assert_eq!(s.engine_mut().tx_occupied(), 0);
    }
}

#[test]
fn deferred_observe_block2_non_and_full_observe_table_have_bounded_completion() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::NonConfirmable, 50, 0);
        s.complete(h, large(), 1).unwrap();
        let (_, b) = first(&mut s, &p);
        finish(&mut s, &mut p, b.szx(), 10);
        s.signal(&["obs"]);
        s.poll(5000).unwrap();
        content(&mut s, &p, b"updated", 1, Type::NonConfirmable);
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let other = coaptic::storage::ObserveKey::new(Token::new(b"other").unwrap(), PEER);
        s.engine_mut()
            .insert_observe(coaptic::storage::ObserveInterest::new(other.token(), PEER))
            .unwrap();
        let h = begin(&mut s, &mut p, Type::Confirmable, 51, 0);
        s.complete(h, large(), 1).unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        assert_eq!(m.code(), Code::CONTENT);
        assert!(m.observe().is_none());
        let b = m.block2().unwrap().unwrap();
        assert_eq!(m.payload(), &LARGE[..usize::from(b.size())]);
        s.transport_mut().incoming = Some(control(m.message_id(), Type::Acknowledgement));
        s.poll(2).unwrap();
        finish(&mut s, &mut p, b.szx(), 10);
        assert!(s.engine_mut().lookup_observe(other).is_some());
        assert!(s.engine_mut().lookup_observe(key()).is_none());
    }
}
#[test]
fn deferred_observe_block2_exact_con_retry_and_smaller_followup_preserve_body() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = begin(&mut s, &mut p, Type::Confirmable, 60, 0);
        s.complete(h, large(), 1).unwrap();
        let wire = s.transport_mut().take().unwrap();
        let opened = p.open(wire);
        let parsed = decode(&opened.bytes[..opened.len]).unwrap();
        let mid = parsed.message_id();
        let block = parsed.block2().unwrap().unwrap();
        s.signal(&["obs"]);
        s.poll(3001).unwrap();
        let retry = s.transport_mut().take().unwrap();
        assert_eq!(&retry.bytes[..retry.len], &wire.bytes[..wire.len]);
        assert!(s.transport_mut().take().is_none());
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(3002).unwrap();
        finish(&mut s, &mut p, block.szx() - 1, 3010);
        s.poll(5000).unwrap();
        content(&mut s, &p, b"updated", 1, Type::NonConfirmable);
    }
}
#[test]
fn deferred_observe_block2_nonzero_and_q_requests_stay_refused_then_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        for (i, number) in [
            coaptic::message::OptionNumber::BLOCK2,
            coaptic::message::OptionNumber::Q_BLOCK2,
        ]
        .into_iter()
        .enumerate()
        {
            let options = [
                Opt::new(coaptic::message::OptionNumber::OBSERVE, &[]),
                Opt::uri_path("obs"),
                Opt::new(number, &[0x10]),
            ];
            s.transport_mut().incoming = Some(
                p.encode(
                    &Message::new(Type::Confirmable, Code::GET, MessageId::new(70 + i as u16))
                        .with_token(Token::new(b"owner").unwrap())
                        .with_options(&options),
                ),
            );
            s.poll_with(i as u64, |_, h| {
                assert!(h.is_none());
                Response::deferred()
            })
            .unwrap();
            let f = p.open(s.transport_mut().take().unwrap());
            let m = decode(&f.bytes[..f.len]).unwrap();
            assert_eq!(m.code(), Code::SERVICE_UNAVAILABLE);
            assert!(m.observe().is_none());
            assert!(body(&mut s).is_none());
        }
        let h = begin(&mut s, &mut p, Type::Confirmable, 72, 2);
        s.complete(h, large(), 3).unwrap();
        first(&mut s, &p);
    }
}

#[test]
fn deferred_observe_block2_unregistered_staged_body_cancel_replace_and_pin() {
    for protected in [false, cfg!(feature = "oscore")] {
        for mode in 0..3 {
            let mut s = fixed();
            let mut p = Peer::new(&mut s, protected);
            let other = coaptic::storage::ObserveKey::new(Token::new(b"other").unwrap(), PEER);
            s.engine_mut()
                .insert_observe(coaptic::storage::ObserveInterest::new(other.token(), PEER))
                .unwrap();
            let h = begin(&mut s, &mut p, Type::Confirmable, 80, 0);
            s.transport_mut().fail_send = true;
            assert!(s.complete(h, large(), 1).is_err());
            s.transport_mut().fail_send = false;
            let f = p.open(s.transport_mut().take().unwrap());
            assert!(decode(&f.bytes[..f.len]).unwrap().observe().is_none());
            let id = body(&mut s).unwrap();
            if mode == 0 {
                assert!(s.cancel_deferred(h));
                assert!(body(&mut s).is_none());
            } else if mode == 1 {
                let fresh = begin(&mut s, &mut p, Type::Confirmable, 81, 2);
                assert!(body(&mut s).is_none());
                s.complete(fresh, large(), 3).unwrap();
                assert!(body(&mut s).is_some());
            } else {
                let access = s.engine_mut().access_tx_body_mut(id).unwrap();
                core::mem::forget(access);
                assert!(!s.cancel_deferred(h));
                s.transport_mut().incoming = Some(request(&mut p, 82, Type::Confirmable, 0));
                assert!(
                    s.poll_with(2, |_, _| panic!("pinned ownership refuses before dispatch"))
                        .is_err()
                );
                assert!(s.transport_mut().take().is_none());
                assert_eq!(body(&mut s), Some(id));
                assert!(s.complete(h, large(), 3).is_err());
            }
            assert!(s.engine_mut().lookup_observe(other).is_some());
            assert!(s.engine_mut().lookup_observe(key()).is_none());
        }
    }
}

fn preferred_request(p: &mut Peer, mid: u16, szx: u8) -> Frame {
    let block = BlockValue::new(0, false, szx).unwrap().encode();
    p.encode(
        &Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
            .with_token(key().token())
            .with_options(&[
                Opt::new(coaptic::message::OptionNumber::OBSERVE, &[]),
                Opt::uri_path("obs"),
                Opt::block2(&block),
            ]),
    )
}

fn preferred_begin<S: AppStorage<Token = coaptic::message::Token>>(
    s: &mut Server<S>,
    p: &mut Peer,
    mid: u16,
    szx: u8,
    now: u64,
) -> coaptic::app::DeferredReply {
    s.transport_mut().incoming = Some(preferred_request(p, mid, szx));
    let mut handle = None;
    without_allocating(|| {
        s.poll_with(now, |_, h| {
            handle = h;
            Response::deferred()
        })
    })
    .unwrap();
    let f = s.transport_mut().take().unwrap();
    assert!(decode(&f.bytes[..f.len]).unwrap().is_empty_ack());
    handle.expect("NUM0 Block2 Observe admits bounded work")
}
fn preferred_cycle<S: AppStorage<Token = coaptic::message::Token>>(
    mut s: Server<S>,
    protected: bool,
) {
    let mut p = Peer::new(&mut s, protected);
    let h = preferred_begin(&mut s, &mut p, 90, 1, 0);
    without_allocating(|| s.complete(h, large(), 1)).unwrap();
    let (mid, b) = first(&mut s, &p);
    assert_eq!(b.szx(), 1);
    s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
    s.poll(2).unwrap();
    finish(&mut s, &mut p, 1, 10);
    without_allocating(|| s.notify(100, &["obs"], large())).unwrap();
    let (_, b) = first(&mut s, &p);
    assert_eq!(b.szx(), 1);
    // Later blocks retain the complete original representation.
    finish_notification(&mut s, &mut p, 1, 110);
}
#[test]
fn deferred_observe_requested_block2_preserves_preference() {
    preferred_cycle(fixed(), false);
}
#[cfg(feature = "oscore")]
#[test]
fn deferred_observe_requested_block2_protected_preserves_preference() {
    preferred_cycle(fixed(), true);
}
#[cfg(feature = "alloc")]
#[test]
fn deferred_observe_requested_block2_allocated_preserves_preference() {
    let resources = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .deferred::<2>();
    let bytes = resources
        .allocated(usize::MAX)
        .required_heap_bytes()
        .unwrap();
    let resources = Resources::<Buffers>::new()
        .routes::<1>()
        .block_wise::<true>()
        .deferred::<2>();
    let s = App::with_resources(resources.allocated(bytes))
        .allow_unprotected()
        .deterministic_for_tests()
        .route("obs", coaptic::get(handler).observe(source))
        .bind(Io::default())
        .unwrap();
    preferred_cycle(s, false);
}
#[test]
fn observe_requested_block2_tiny_initial_retains_later_limit() {
    for protected in [false, cfg!(feature = "oscore")] {
        for deferred in [false, true] {
            let mut s = fixed();
            let mut p = Peer::new(&mut s, protected);
            let mid = if deferred {
                let h = preferred_begin(&mut s, &mut p, 10, 0, 0);
                s.complete(h, response(b"tiny"), 1).unwrap();
                let f = p.open(s.transport_mut().take().unwrap());
                let m = decode(&f.bytes[..f.len]).unwrap();
                assert_eq!(m.payload(), b"tiny");
                assert_eq!(m.block2().unwrap().unwrap().szx(), 0);
                m.message_id()
            } else {
                s.transport_mut().incoming = Some(preferred_request(&mut p, 10, 0));
                s.poll(0).unwrap();
                let f = p.open(s.transport_mut().take().unwrap());
                let m = decode(&f.bytes[..f.len]).unwrap();
                assert_eq!(m.payload(), b"sync");
                assert_eq!(m.block2().unwrap().unwrap().szx(), 0);
                m.message_id()
            };
            if deferred {
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(2).unwrap();
            }
            s.notify(3, &["obs"], large()).unwrap();
            let (_, b) = first(&mut s, &p);
            assert_eq!(b.szx(), 0);
            finish_notification(&mut s, &mut p, 0, 10);
        }
    }
}
#[test]
fn observe_requested_block2_separate_initial_respects_limit() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        s.transport_mut().incoming = Some(preferred_request(&mut p, 10, 0));
        s.poll_with(0, |_, _| response(&LARGE[..100]).etag(b"v1").separate())
            .unwrap();
        let ack = s.transport_mut().take().unwrap();
        assert!(decode(&ack.bytes[..ack.len]).unwrap().is_empty_ack());
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        assert_eq!(m.ty(), Type::Confirmable);
        assert!(m.observe().is_some());
        assert_eq!(m.payload(), &LARGE[..16]);
        assert_eq!(m.block2().unwrap().unwrap().szx(), 0);
    }
}

fn finish_notification<S: AppStorage<Token = coaptic::message::Token>>(
    s: &mut Server<S>,
    p: &mut Peer,
    szx: u8,
    start: u64,
) {
    let size = 1usize << (szx + 4);
    let count = LARGE.len().div_ceil(size);
    let observing = s.engine_mut().lookup_observe(key()).is_some();
    for num in 1..count {
        s.transport_mut().incoming = Some(follow(
            p,
            (start + 200 + num as u64) as u16,
            BlockValue::new(num as u32, false, szx).unwrap(),
            "obs",
        ));
        without_allocating(|| s.poll_with(start + num as u64, |_, _| large())).unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        assert_eq!(m.code(), Code::CONTENT);
        assert!(m.observe().is_none());
        let b = m.block2().unwrap().unwrap();
        assert_eq!(b.num(), num as u32);
        assert_eq!(b.szx(), szx);
        let end = ((num + 1) * size).min(LARGE.len());
        assert_eq!(m.payload(), &LARGE[num * size..end]);
        assert_eq!(b.more(), end < LARGE.len());
        assert_eq!(m.etag().next(), Some(&b"v1"[..]));
        assert_eq!(m.content_format(), Some(Ok(ContentFormat::TEXT_PLAIN)));
        assert_eq!(m.max_age(), Some(Ok(17)));
        assert_eq!(
            s.engine_mut().lookup_observe(key()).is_some(),
            observing,
            "ordinary follow-up preserves observation"
        );
    }
    assert!(body(s).is_none());
}

#[test]
fn observe_requested_block2_actual_encoding_shrinks_without_losing_limit() {
    for protected in [false, cfg!(feature = "oscore")] {
        for separate in [false, true] {
            let mut s = fixed();
            let mut p = Peer::new(&mut s, protected);
            s.transport_mut().incoming = Some(preferred_request(&mut p, 30, 6));
            s.poll_with(0, |_, _| {
                if separate {
                    large().separate()
                } else {
                    large()
                }
            })
            .unwrap();
            if separate {
                s.transport_mut().take().unwrap();
            }
            let (mid, b) = first(&mut s, &p);
            assert!(b.szx() < 6);
            if separate {
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(1).unwrap();
            }
            finish_notification(&mut s, &mut p, b.szx(), 10);
            s.notify(100, &["obs"], large()).unwrap();
            let (_, b) = first(&mut s, &p);
            assert!(b.szx() < 6);
            finish_notification(&mut s, &mut p, b.szx(), 110);
        }
    }
}
#[test]
fn deferred_observe_requested_block2_refusal_retry_keeps_immutable_snapshot() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        let h = preferred_begin(&mut s, &mut p, 30, 0, 0);
        assert!(s.complete(h, response(&LARGE), 1).is_err());
        assert!(s.transport_mut().take().is_none());
        assert!(s.engine_mut().lookup_observe(key()).is_none());
        let a = s.engine_mut().acquire_tx_body().unwrap();
        let b = s.engine_mut().acquire_tx_body().unwrap();
        assert!(s.complete(h, large(), 2).is_err());
        assert!(s.transport_mut().take().is_none());
        s.engine_mut().release_tx_body(a).unwrap();
        s.engine_mut().release_tx_body(b).unwrap();
        s.transport_mut().fail_send = true;
        assert!(s.complete(h, large(), 3).is_err());
        let (old, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 0);
        s.transport_mut().fail_send = false;
        for (i, response) in [
            large().etag(b"wrong"),
            large().max_age(99),
            response(b"changed").etag(b"v1"),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(s.complete(h, response, 4 + i as u64).is_err());
            assert!(s.transport_mut().take().is_none());
        }
        s.complete(h, large(), 8).unwrap();
        let (mid, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 0);
        assert_ne!(mid, old);
        s.transport_mut().incoming = Some(control(old, Type::Reset));
        s.poll(9).unwrap();
        assert!(s.engine_mut().lookup_observe(key()).is_some());
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(10).unwrap();
        finish(&mut s, &mut p, 0, 20);
        s.notify(100, &["obs"], large()).unwrap();
        let (_, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 0);
    }
}
#[test]
fn observe_requested_block2_source_updates_and_exact_retry_preserve_cap() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = App::with_resources(
            Resources::<Buffers>::new()
                .routes::<1>()
                .block_wise::<true>()
                .deferred::<2>(),
        )
        .allow_unprotected()
        .deterministic_for_tests()
        .route("obs", coaptic::get(handler).observe(large))
        .bind(Io::default())
        .unwrap();
        let mut p = Peer::new(&mut s, protected);
        let h = preferred_begin(&mut s, &mut p, 30, 0, 0);
        s.complete(h, response(b"tiny"), 1).unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        s.transport_mut().incoming = Some(control(m.message_id(), Type::Acknowledgement));
        s.poll(2).unwrap();
        s.notify_confirmable(3, &["obs"], response(b"prior"))
            .unwrap();
        let wire = s.transport_mut().take().unwrap();
        let opened = p.open(wire);
        let mid = decode(&opened.bytes[..opened.len]).unwrap().message_id();
        s.signal(&["obs"]);
        s.poll(3004).unwrap();
        let (replacement_mid, block) = first(&mut s, &p);
        assert_ne!(replacement_mid, mid);
        assert_eq!(block.szx(), 0);
        assert!(s.transport_mut().take().is_none());
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(3005).unwrap();
        assert!(
            s.engine()
                .lookup_pending_con(replacement_mid, PEER)
                .is_some()
        );
        s.transport_mut().incoming = Some(control(replacement_mid, Type::Acknowledgement));
        s.poll(3006).unwrap();
        finish_notification(&mut s, &mut p, 0, 3010);
    }
}
#[test]
fn observe_requested_block2_replacement_updates_only_current_owner_limit() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed_lifetime(100);
        let mut p = Peer::new(&mut s, protected);
        let h = preferred_begin(&mut s, &mut p, 20, 0, 0);
        s.complete(h, large(), 1).unwrap();
        let (mid, _) = first(&mut s, &p);
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(2).unwrap();
        let fresh = preferred_begin(&mut s, &mut p, 21, 2, 50);
        assert!(s.complete(h, large(), 51).is_err());
        s.complete(fresh, large(), 52).unwrap();
        let (mid, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 2);
        s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
        s.poll(53).unwrap();
        s.poll(101).unwrap();
        assert!(body(&mut s).is_some());
        finish(&mut s, &mut p, 2, 110);
        s.notify(130, &["obs"], large()).unwrap();
        let (_, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 2);
        finish_notification(&mut s, &mut p, 2, 131);
        let h = begin(&mut s, &mut p, Type::Confirmable, 80, 5000);
        s.complete(h, response(b"tiny"), 5001).unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        s.transport_mut().incoming = Some(control(m.message_id(), Type::Acknowledgement));
        s.poll(5002).unwrap();
        s.notify(5003, &["obs"], response(&LARGE[..100])).unwrap();
        let f = p.open(s.transport_mut().take().unwrap());
        let m = decode(&f.bytes[..f.len]).unwrap();
        assert_eq!(m.payload(), &LARGE[..100]);
        assert!(
            m.block2().is_none(),
            "renewal without preference clears prior maximum"
        );
    }
}

#[test]
fn deferred_observe_requested_block2_invalid_selectors_refuse_then_recover() {
    for protected in [false, cfg!(feature = "oscore")] {
        let mut s = fixed();
        let mut p = Peer::new(&mut s, protected);
        for (i, value) in [&[0x10][..], &[0x08][..], &[0x07][..], &[0, 0, 0, 0][..]]
            .into_iter()
            .enumerate()
        {
            s.transport_mut().incoming = Some(
                p.encode(
                    &Message::new(Type::Confirmable, Code::GET, MessageId::new(200 + i as u16))
                        .with_token(key().token())
                        .with_options(&[
                            Opt::new(coaptic::message::OptionNumber::OBSERVE, &[]),
                            Opt::uri_path("obs"),
                            Opt::new(coaptic::message::OptionNumber::BLOCK2, value),
                        ]),
                ),
            );
            s.poll_with(i as u64, |_, h| {
                assert!(h.is_none(), "invalid selector index {i}");
                Response::deferred()
            })
            .unwrap();
            let f = p.open(s.transport_mut().take().unwrap());
            let m = decode(&f.bytes[..f.len]).unwrap();
            assert!(!m.code().is_success());
            assert!(m.observe().is_none());
            assert!(s.engine_mut().lookup_observe(key()).is_none());
            assert!(body(&mut s).is_none());
        }
        let h = preferred_begin(&mut s, &mut p, 220, 0, 10);
        s.complete(h, large(), 11).unwrap();
        let (_, b) = first(&mut s, &p);
        assert_eq!(b.szx(), 0);
    }
}

#[test]
fn ordinary_separate_block2_without_body_storage_preserves_small_response() {
    let mut s = App::with_resources(Resources::<Buffers>::new().routes::<1>())
        .allow_unprotected()
        .deterministic_for_tests()
        .route("obs", coaptic::get(handler))
        .bind(Io::default())
        .unwrap();
    let block = BlockValue::new(0, false, 0).unwrap().encode();
    let mut incoming = Frame::default();
    incoming.len = encode(
        &Message::new(Type::Confirmable, Code::GET, MessageId::new(10))
            .with_token(key().token())
            .with_options(&[Opt::uri_path("obs"), Opt::block2(&block)]),
        &mut incoming.bytes,
    )
    .unwrap();
    s.transport_mut().incoming = Some(incoming);
    without_allocating(|| s.poll_with(0, |_, _| response(b"ordinary").separate())).unwrap();
    let ack = s.transport_mut().take().unwrap();
    assert!(decode(&ack.bytes[..ack.len]).unwrap().is_empty_ack());
    let f = s.transport_mut().take().unwrap();
    let m = decode(&f.bytes[..f.len]).unwrap();
    assert_eq!(m.ty(), Type::Confirmable);
    assert_eq!(m.code(), Code::CONTENT);
    assert_eq!(m.payload(), b"ordinary");
    assert!(m.observe().is_none());
    assert!(m.block2().is_none());
    assert!(s.transport_mut().take().is_none());
}

#[test]
fn deferred_error_only_no_response_keeps_observe_and_suppresses_terminal() {
    for protected in [false, cfg!(feature = "oscore")] {
        for initial_error in [false, true] {
            let mut s = fixed();
            let mut p = Peer::new(&mut s, protected);
            let observe = coaptic::message::encode_uint(0);
            let mask = coaptic::message::NoResponse::new(24).encode();
            let options = [
                Opt::observe(&observe),
                Opt::uri_path("obs"),
                Opt::no_response(&mask),
            ];
            s.transport_mut().incoming = Some(
                p.encode(
                    &Message::new(Type::Confirmable, Code::GET, MessageId::new(1))
                        .with_token(Token::new(b"owner").unwrap())
                        .with_options(&options),
                ),
            );
            let mut handle = None;
            s.poll_with(0, |_, candidate| {
                handle = candidate;
                Response::deferred()
            })
            .unwrap();
            assert!(s.transport_mut().take().is_some());
            let h = handle.expect("deferred Observe registration");
            if initial_error {
                s.complete(h, Response::new(Code::INTERNAL_SERVER_ERROR), 1)
                    .unwrap();
                assert!(s.transport_mut().take().is_none());
            } else {
                s.complete(h, response(b"initial"), 1).unwrap();
                let mid = content(&mut s, &p, b"initial", 0, Type::Confirmable);
                s.transport_mut().incoming = Some(control(mid, Type::Acknowledgement));
                s.poll(2).unwrap();
                assert_eq!(
                    s.notify_confirmable(3, &["obs"], Response::new(Code::INTERNAL_SERVER_ERROR))
                        .unwrap(),
                    0
                );
                assert!(s.transport_mut().take().is_none());
            }
            assert!(s.engine().lookup_observe(key()).is_none());
            assert_eq!(s.engine_mut().tx_occupied(), 0);
        }
    }
}

#[path = "support/q_observe_con.rs"]
mod q_observe_con;
