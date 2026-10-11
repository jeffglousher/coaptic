//! Explicit inline security bindings have the same bounds with fixed or allocated pools.
#![cfg(feature = "oscore")]

use coaptic::app::{AppStorage, AppStore};
use coaptic::message::{Message, MessageId, OptionNumber, Type, decode, encode};
use coaptic::oscore::{BoundedSecurityContext, DeriveParams, SecurityContext};
use coaptic::resources::Resources;
use coaptic::storage::DatagramIo;
use coaptic::{App, CallFailure, Code, Endpoint, Error, Response, get};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static EFFECTS: Cell<usize> = const { Cell::new(0) };
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
    Eight {
        rx_datagram: (8, 512), tx_datagram: (8, 512),
        rx_body: (1, 1024), tx_body: (1, 1024),
        dedup: 8, observe: 8,
    }
}
#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; 512],
    len: usize,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }
}
#[derive(Default)]
struct Io {
    incoming: Option<Frame>,
    last: Frame,
    sends: usize,
}
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
impl DatagramIo for Io {
    type Error = core::convert::Infallible;
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let Some(frame) = self.incoming.take() else {
            return Ok(None);
        };
        out[..frame.len].copy_from_slice(&frame.bytes[..frame.len]);
        Ok(Some((frame.len, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.last.bytes[..bytes.len()].copy_from_slice(bytes);
        self.last.len = bytes.len();
        self.sends += 1;
        Ok(bytes.len())
    }
}
fn context<const R: usize>(sender: &[u8], recipient: &[u8]) -> BoundedSecurityContext<R> {
    BoundedSecurityContext::derive(DeriveParams {
        master_secret: &[0x24; 16],
        master_salt: &[],
        sender_id: sender,
        recipient_id: recipient,
        id_context: &[],
    })
    .unwrap()
}
type Client<S, const R: usize> = App<Eight, Io, 8, false, S, 0, 8, R>;
type Server = App<Eight, Io, 8, false, AppStore<Eight, false>, 0, 8, 0>;
fn server() -> Server {
    EFFECTS.with(|count| count.set(0));
    App::with_resources(
        Resources::<Eight>::new()
            .client_calls::<8>()
            .security_requests::<0>(),
    )
    .oscore(context::<0>(&[2], &[1]))
    .deterministic_for_tests()
    .route(
        "value",
        get(|_| {
            EFFECTS.with(|count| count.set(count.get() + 1));
            Response::content(b"authenticated")
        }),
    )
    .route("observe", get(|_| Response::content(b"initial").observe(0)))
    .bind(Io::default())
    .unwrap()
}
fn deliver<S: AppStorage, const R: usize>(
    client: &mut Client<S, R>,
    server: &mut Server,
    request: Frame,
    now: u64,
) -> Frame {
    server.transport_mut().incoming = Some(request);
    server.poll(now).unwrap();
    let reply = server.transport().last;
    assert!(
        decode(&reply.bytes[..reply.len])
            .unwrap()
            .oscore()
            .is_some()
    );
    client.transport_mut().incoming = Some(reply);
    client.poll(now + 1).unwrap();
    reply
}
fn capacity<S: AppStorage>(mut client: Client<S, 6>) {
    let mut server = server();
    without_allocating(|| {
        let mut calls = [None; 6];
        let mut requests = [Frame::default(); 6];
        for (call, request) in calls.iter_mut().zip(&mut requests) {
            *call = Some(
                client
                    .get("value")
                    .non()
                    .deadline(100)
                    .to(PEER)
                    .send(0)
                    .unwrap(),
            );
            *request = client.transport().last;
        }
        assert_eq!(client.security_request_capacity(), 6);
        assert_eq!(client.oscore().unwrap().request_capacity(), 6);
        let bindings = calls.map(|call| {
            client
                .oscore()
                .unwrap()
                .lookup(call.unwrap().token())
                .unwrap()
        });
        for _ in 0..12 {
            assert_eq!(
                client.get("value").non().to(PEER).send(1),
                Err(Error::Oscore(coaptic::oscore::Error::Saturated))
            );
        }
        assert_eq!(client.transport().sends, 6);
        for (call, binding) in calls.into_iter().zip(bindings) {
            assert_eq!(
                client.oscore().unwrap().lookup(call.unwrap().token()),
                Some(binding)
            );
        }
        let first = calls[0].unwrap();
        deliver(&mut client, &mut server, requests[0], 2);
        assert_eq!(
            client.take_response(first).unwrap().unwrap().payload(),
            b"authenticated"
        );
        assert!(client.oscore().unwrap().lookup(first.token()).is_none());
        let recovered = client.get("value").non().to(PEER).send(4).unwrap();
        let second = calls[1].unwrap();
        assert!(client.cancel(second));
        assert_eq!(
            client.take_response(second).unwrap().unwrap_err(),
            CallFailure::Cancelled
        );
        let next = client.get("value").non().to(PEER).send(5).unwrap();
        assert_eq!(client.transport().sends, 8);
        // Cancellation does not erase unrelated request authentication bindings.
        for index in 2..6 {
            let call = calls[index].unwrap();
            deliver(
                &mut client,
                &mut server,
                requests[index],
                6 + index as u64 * 2,
            );
            assert_eq!(
                client.take_response(call).unwrap().unwrap().payload(),
                b"authenticated"
            );
        }
        assert!(client.cancel(recovered));
        assert!(client.cancel(next));
        client.take_response(recovered).unwrap().unwrap_err();
        client.take_response(next).unwrap().unwrap_err();
        for _ in 0..6 {
            client
                .get("value")
                .non()
                .deadline(30)
                .to(PEER)
                .send(20)
                .unwrap();
        }
        client.poll(30).unwrap();
        assert!(client.oscore().unwrap().sender_seq() > 6);
    });
}
#[test]
fn security_capacity_fixed_success_refusal_recovery_without_allocation() {
    capacity(
        App::with_resources(
            Resources::<Eight>::new()
                .client_calls::<8>()
                .security_requests::<6>(),
        )
        .oscore(context::<6>(&[1], &[2]))
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap(),
    );
}
#[cfg(feature = "alloc")]
#[test]
fn security_capacity_allocated_has_identical_bounded_behavior() {
    let resources = Resources::<Eight>::new()
        .client_calls::<8>()
        .security_requests::<6>()
        .allocated(usize::MAX);
    let bytes = resources.required_heap_bytes().unwrap();
    let resources = Resources::<Eight>::new()
        .client_calls::<8>()
        .allocated(bytes)
        .security_requests::<6>();
    assert_eq!(resources.required_heap_bytes().unwrap(), bytes);
    capacity(
        App::with_resources(resources)
            .oscore(context::<6>(&[1], &[2]))
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap(),
    );
}
fn observe<S: AppStorage>(mut client: Client<S, 1>) {
    let mut server = server();
    without_allocating(|| {
        let call = client.get("observe").observe().to(PEER).send(0).unwrap();
        let request = client.transport().last;
        let stale = deliver(&mut client, &mut server, request, 1);
        assert_eq!(
            client.take_response(call).unwrap().unwrap().payload(),
            b"initial"
        );
        assert!(client.oscore().unwrap().lookup(call.token()).is_some());
        assert_eq!(
            client.get("value").non().to(PEER).send(3),
            Err(Error::Oscore(coaptic::oscore::Error::Saturated))
        );
        assert_eq!(client.transport().sends, 1);
        assert!(client.cancel(call));
        assert_eq!(
            client.take_response(call).unwrap().unwrap_err(),
            CallFailure::Cancelled
        );
        let fresh = client.get("value").non().to(PEER).send(4).unwrap();
        let fresh_request = client.transport().last;
        assert_ne!(fresh.token(), call.token());
        client.transport_mut().incoming = Some(stale);
        let _ = client.poll(5); // A late old relation cannot complete the replacement Call.
        assert!(client.take_response(fresh).is_none());
        deliver(&mut client, &mut server, fresh_request, 6);
        assert_eq!(
            client.take_response(fresh).unwrap().unwrap().payload(),
            b"authenticated"
        );
    });
}
#[test]
fn security_capacity_observe_retains_binding_until_cancel_and_rejects_stale_reply() {
    observe(
        App::with_resources(
            Resources::<Eight>::new()
                .client_calls::<8>()
                .security_requests::<1>(),
        )
        .oscore(context::<1>(&[1], &[2]))
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap(),
    );
    #[cfg(feature = "alloc")]
    observe(
        App::with_resources(
            Resources::<Eight>::new()
                .client_calls::<8>()
                .security_requests::<1>()
                .allocated(usize::MAX),
        )
        .oscore(context::<1>(&[1], &[2]))
        .deterministic_for_tests()
        .bind(Io::default())
        .unwrap(),
    );
}
#[test]
fn security_capacity_zero_refuses_outbound_but_authenticates_server_requests() {
    let mut server = server();
    without_allocating(|| {
        for _ in 0..10 {
            assert_eq!(
                server.get("value").non().to(PEER).send(0),
                Err(Error::Oscore(coaptic::oscore::Error::Saturated))
            );
        }
        assert_eq!(server.transport().sends, 0);
        let mut client = App::with_resources(Resources::<Eight>::new().client_calls::<8>())
            .oscore(context::<4>(&[1], &[2]))
            .deterministic_for_tests()
            .bind(Io::default())
            .unwrap();
        let call = client.get("value").non().to(PEER).send(1).unwrap();
        let request = client.transport().last;
        deliver(&mut client, &mut server, request, 2);
        assert_eq!(
            client.take_response(call).unwrap().unwrap().payload(),
            b"authenticated"
        );
        let sends = server.transport().sends;
        let plain_options = [coaptic::message::Opt::new(OptionNumber::URI_PATH, b"value")];
        let mut plain = Frame::default();
        plain.len = encode(
            &Message::new(Type::NonConfirmable, Code::GET, MessageId::new(120))
                .with_options(&plain_options),
            &mut plain.bytes,
        )
        .unwrap();
        server.transport_mut().incoming = Some(plain);
        let _ = server.poll(4);
        assert_eq!(EFFECTS.with(Cell::get), 1);
        assert_eq!(server.transport().sends, sends + 1);
        let refusal = server.transport().last;
        assert_eq!(
            decode(&refusal.bytes[..refusal.len]).unwrap().code(),
            Code::UNAUTHORIZED
        );
        server.transport_mut().incoming = Some(request);
        let _ = server.poll(5);
        assert_eq!(
            EFFECTS.with(Cell::get),
            1,
            "replay must not repeat the application effect"
        );
        let refusal = server.transport().last;
        assert_eq!(
            decode(&refusal.bytes[..refusal.len]).unwrap().code(),
            Code::UNAUTHORIZED
        );
        let next = client.get("value").non().to(PEER).send(6).unwrap();
        let request = client.transport().last;
        deliver(&mut client, &mut server, request, 7);
        assert_eq!(
            client.take_response(next).unwrap().unwrap().payload(),
            b"authenticated"
        );
    });
}
#[test]
fn security_capacity_metadata_and_compatibility_are_explicit() {
    let default: SecurityContext = context::<4>(&[1], &[2]);
    assert_eq!(default.request_capacity(), 4);
    let large = context::<6>(&[1], &[2]);
    let small = context::<0>(&[1], &[2]);
    assert_eq!(small.request_metadata_bytes(), 0);
    assert_eq!(
        large.request_metadata_bytes() * 2,
        default.request_metadata_bytes() * 3
    );
    assert_eq!(
        core::mem::size_of_val(&large) - core::mem::size_of_val(&small),
        large.request_metadata_bytes()
    );
    assert_eq!(
        core::mem::size_of::<SecurityContext>(),
        core::mem::size_of::<BoundedSecurityContext<4>>()
    );
    let field_growth = core::mem::size_of::<Option<BoundedSecurityContext<6>>>()
        - core::mem::size_of::<Option<BoundedSecurityContext<0>>>();
    assert_eq!(
        core::mem::size_of::<Client<AppStore<Eight, false>, 6>>()
            - core::mem::size_of::<Client<AppStore<Eight, false>, 0>>(),
        field_growth
    );
    #[cfg(feature = "alloc")]
    assert_eq!(
        core::mem::size_of::<Client<coaptic::storage::AllocMemory, 6>>()
            - core::mem::size_of::<Client<coaptic::storage::AllocMemory, 0>>(),
        field_growth
    );
    let mut rng_state = 0u8;
    let mut app = App::with_resources(Resources::<Eight>::new().security_requests::<6>())
        .oscore(context::<6>(&[1], &[2]))
        .bind_with_entropy(Io::default(), |bytes| {
            rng_state = rng_state.wrapping_add(1);
            bytes.fill(rng_state);
            true
        })
        .unwrap();
    without_allocating(|| {
        app.get("value").non().to(PEER).send(0).unwrap();
        app.poll(1).unwrap();
    });
    assert_eq!(app.security_request_capacity(), 6);
    assert_eq!(
        app.security_metadata_bytes(),
        core::mem::size_of::<Option<BoundedSecurityContext<6>>>()
    );
    assert!(
        decode(&app.transport().transport().last.bytes[..app.transport().transport().last.len])
            .unwrap()
            .options()
            .any(|o| o.number() == OptionNumber::OSCORE)
    );
}
