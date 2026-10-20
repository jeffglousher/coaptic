//! Service-sized heap pools are constructed once and reused over actual UDP.
//!
//! The allocator counter covers App/adapter operations on this test thread only.
//! It excludes construction, OS socket buffers, allocator bookkeeping and other
//! threads. Four concurrent NON Calls test memory admission independently of
//! the endpoint's CON NSTART limit. This is a finite integration check, not a
//! cloud throughput or whole-process memory measurement.
#![cfg(all(feature = "std", feature = "alloc"))]

use coaptic::resources::{Allocated, Resources};
use coaptic::storage::{AllocMemory, DatagramIo, DatagramTransport, UdpSocketIo};
use coaptic::{App, Call, CallFailure, Code, Endpoint, Error, Response, get, profiles};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct Allocations {
    alloc: usize,
    realloc: usize,
}

thread_local! {
    static ALLOCATIONS: Cell<Option<Allocations>> = const { Cell::new(None) };
}

struct TrackingAllocator;

fn record_allocation(reallocation: bool) {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(mut value) = count.get() {
            if reallocation {
                value.realloc += 1;
            } else {
                value.alloc += 1;
            }
            count.set(Some(value));
        }
    });
}

// Instrumentation uses const thread-local state and delegates every request to
// System. No worker thread is started while the counter is armed.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(false);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(false);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation(true);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn without_allocating<R>(work: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ALLOCATIONS.with(|count| count.set(None));
        }
    }

    ALLOCATIONS.with(|count| {
        assert!(count.get().is_none(), "nested allocation measurement");
        count.set(Some(Allocations::default()));
    });
    let guard = Guard;
    let result = work();
    let count = ALLOCATIONS.with(|count| count.get().unwrap());
    drop(guard);
    assert_eq!(count.alloc, 0, "steady-state App/adapter allocated");
    assert_eq!(count.realloc, 0, "steady-state App/adapter reallocated");
    result
}

const SCRATCH_BYTES: usize = 1473;
const BODY: [u8; 2000] = [0x5a; 2000];
const TINY: &[u8] = b"complete";

struct Io {
    udp: UdpSocketIo<[u8; SCRATCH_BYTES]>,
    send_attempts: usize,
}

impl DatagramTransport for Io {
    type Error = std::io::Error;
}

impl DatagramIo for Io {
    fn recv(&mut self, bytes: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        self.udp.recv(bytes)
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.send_attempts += 1;
        self.udp.send(dest, bytes)
    }
}

type Configuration = Resources<profiles::Service, 2, true, 0, 4, Allocated, 4>;
type Service = App<profiles::Service, Io, 2, true, AllocMemory, 0, 4, 4>;

fn configuration(limit: usize) -> Configuration {
    coaptic::resources::Service::new()
        .routes::<2>()
        .deferred::<0>()
        .client_calls::<4>()
        .security_requests::<4>()
        .allocated(limit)
}

fn bind(
    socket: UdpSocket,
    required_heap: usize,
    protected: bool,
    secret: &[u8; 32],
    server: bool,
) -> Service {
    let builder = App::with_resources(configuration(required_heap))
        .deterministic_for_tests()
        .route("body", get(|_| Response::content(&BODY)))
        .route("tiny", get(|_| Response::content(TINY)));
    #[cfg(feature = "oscore")]
    let builder = if protected {
        let context = coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
            master_secret: secret,
            master_salt: &[],
            sender_id: if server { &[2] } else { &[1] },
            recipient_id: if server { &[1] } else { &[2] },
            id_context: &[],
        })
        .unwrap();
        builder.oscore(context)
    } else {
        builder.allow_unprotected()
    };
    #[cfg(not(feature = "oscore"))]
    let builder = {
        assert!(!protected);
        let _ = (secret, server);
        builder.allow_unprotected()
    };
    builder
        .bind(Io {
            udp: UdpSocketIo::new(socket, [0; SCRATCH_BYTES]).unwrap(),
            send_attempts: 0,
        })
        .unwrap()
}

fn now(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_millis()).unwrap()
}

fn await_response(
    server: &mut Service,
    client: &mut Service,
    call: Call,
    epoch: Instant,
    expected: &[u8],
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = [0; BODY.len()];
    loop {
        let tick = now(epoch);
        server.poll(tick).unwrap();
        client.poll(tick).unwrap();
        if let Some(response) = client.take_response_into(call, &mut output).unwrap() {
            let response = response.unwrap();
            assert_eq!(response.code(), Code::CONTENT);
            assert_eq!(response.payload(), expected);
            return;
        }
        assert!(Instant::now() < deadline, "loopback response deadline");
        std::thread::yield_now();
    }
}

fn exercise(protected: bool) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let peer = Endpoint::from(socket.local_addr().unwrap());
    let client_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    client_socket.set_nonblocking(true).unwrap();
    let mut secret = [0; 32];
    if protected {
        // These contexts exist only for this process; they are not restarted or
        // claimed as a durable sender/replay-state integration.
        getrandom::fill(&mut secret).unwrap();
    }
    let required_heap = configuration(usize::MAX).required_heap_bytes().unwrap();
    let mut server = bind(socket, required_heap, protected, &secret, true);
    let mut client = bind(client_socket, required_heap, protected, &secret, false);
    secret.fill(0);
    let capacities = configuration(required_heap).pool_capacities();
    assert_eq!(client.engine().capacities(), capacities);
    assert_eq!(server.engine().capacities(), capacities);
    assert_eq!(client.client_capacity(), 4);
    assert_eq!(client.security_request_capacity(), 4);
    let epoch = Instant::now();

    without_allocating(|| {
        for _ in 0..3 {
            let calls: [Call; 4] = core::array::from_fn(|_| {
                client.get("body").non().to(peer).send(now(epoch)).unwrap()
            });
            let sent = client.transport().send_attempts;
            let refused = client.get("body").non().to(peer).send(now(epoch));
            #[cfg(feature = "oscore")]
            if protected {
                assert!(matches!(
                    refused,
                    Err(Error::Saturated | Error::Oscore(coaptic::oscore::Error::Saturated))
                ));
            } else {
                assert!(matches!(refused, Err(Error::Saturated)));
            }
            #[cfg(not(feature = "oscore"))]
            assert!(matches!(refused, Err(Error::Saturated)));
            assert_eq!(client.transport().send_attempts, sent);

            // Consuming one complete body releases credit while the other three
            // Calls still own active or retained responses.
            await_response(&mut server, &mut client, calls[0], epoch, &BODY);
            let recovered = client.get("body").non().to(peer).send(now(epoch)).unwrap();
            for call in &calls[1..] {
                await_response(&mut server, &mut client, *call, epoch, &BODY);
            }
            await_response(&mut server, &mut client, recovered, epoch, &BODY);
            assert_eq!(client.storage_usage().rx_body.unwrap().occupied, 0);
            assert_eq!(client.storage_usage().tx_body.unwrap().occupied, 0);
            assert_eq!(client.engine().capacities(), capacities);
            assert_eq!(server.engine().capacities(), capacities);
        }

        let cancelled = client.get("tiny").non().to(peer).send(now(epoch)).unwrap();
        assert!(client.cancel(cancelled));
        assert!(matches!(
            client.take_response(cancelled),
            Some(Err(CallFailure::Cancelled))
        ));
        // The canceled NON may still reach the peer. Its late reply must not
        // complete this replacement or prevent the next full response.
        let recovered = client.get("body").non().to(peer).send(now(epoch)).unwrap();
        await_response(&mut server, &mut client, recovered, epoch, &BODY);
        assert!(client.take_response(cancelled).is_none());
        assert_eq!(client.storage_usage().rx_body.unwrap().occupied, 0);
        assert_eq!(client.engine().capacities(), capacities);
        assert_eq!(server.engine().capacities(), capacities);
    });

    println!(
        "allocated Service UDP: protected={protected}, heap layout bytes/App={required_heap}, inline App bytes={} (includes {}-byte transport), 16 complete 2000-byte bodies, 3 local refusals, cancellation/recovery; steady-state alloc=0 realloc=0",
        core::mem::size_of::<Service>(),
        core::mem::size_of::<Io>(),
    );
}

#[test]
fn allocated_service_udp_reuses_budget_after_refusal_and_cancellation() {
    exercise(false);
}

#[cfg(feature = "oscore")]
#[test]
fn protected_allocated_service_udp_reuses_budget_after_refusal_and_cancellation() {
    exercise(true);
}
