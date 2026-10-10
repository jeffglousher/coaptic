//! Verify the std-only fixed-storage path without counting test-harness allocation.
#![cfg(all(feature = "std", not(feature = "alloc")))]

use coaptic::storage::{DatagramIo, UdpSocketIo};
use coaptic::{App, Code, Response, get};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct TrackingAllocator;
// Test-only instrumentation delegates every allocation to the system allocator.
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
            ALLOCATIONS.with(|count| count.set(None));
        }
    }
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let guard = Guard;
    let result = work();
    let count = ALLOCATIONS.with(|count| count.get().unwrap());
    drop(guard);
    assert_eq!(count, 0, "fixed-storage operation allocated");
    result
}

#[test]
fn std_only_udp_refuses_unbudgeted_scratch_and_recovers_without_allocation() {
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let peer = socket.local_addr().unwrap();
    sender.send_to(&[0x39; 2048], peer).unwrap();
    let mut body = [0xa5; 2048];
    let refused = without_allocating(|| DatagramIo::recv(&mut socket, &mut body));
    assert_eq!(
        refused.unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(body, [0xa5; 2048]);
    let mut io = without_allocating(|| UdpSocketIo::new(socket, [0; 2049]).unwrap());
    // Refused capacity did not consume the datagram. The caller's budget admits it.
    assert_eq!(
        without_allocating(|| io.recv(&mut body))
            .unwrap()
            .unwrap()
            .0,
        2048
    );
    assert_eq!(body, [0x39; 2048]);
    sender.send_to(&[0x44; 2049], peer).unwrap();
    body.fill(0xa5);
    assert!(without_allocating(|| io.recv(&mut body)).is_err());
    assert_eq!(body, [0xa5; 2048]);
    sender.send_to(b"complete", peer).unwrap();
    let n = without_allocating(|| io.recv(&mut body))
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(&body[..n], b"complete");
}

#[test]
fn std_only_apps_deliver_complete_blockwise_body_and_refuse_short_output_without_allocation() {
    const BODY: [u8; 2000] = [0x5a; 2000];
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let peer = socket.local_addr().unwrap().into();
    let client_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    client_socket.set_nonblocking(true).unwrap();
    let (mut server, mut client) = without_allocating(|| {
        let server = App::builder()
            .block_wise::<true>()
            .randomness(|b| {
                b.fill(0x39);
                true
            })
            .route("large", get(|_| Response::content(&BODY)));
        let client = App::builder()
            .block_wise::<true>()
            .full_responses()
            .randomness(|b| {
                b.fill(0x42);
                true
            });
        #[cfg(feature = "oscore")]
        let (server, client) = {
            use coaptic::oscore::{DeriveParams, SecurityContext};
            let params = |sender, recipient| DeriveParams {
                master_secret: &[0x27; 32],
                master_salt: &[],
                sender_id: sender,
                recipient_id: recipient,
                id_context: &[],
            };
            (
                server.oscore(SecurityContext::derive(params(&[2], &[1])).unwrap()),
                client.oscore(SecurityContext::derive(params(&[1], &[2])).unwrap()),
            )
        };
        #[cfg(not(feature = "oscore"))]
        let (server, client) = (server.allow_plaintext(), client.allow_plaintext());
        (
            server
                .bind(UdpSocketIo::new(socket, [0; 1473]).unwrap())
                .unwrap(),
            client
                .bind(UdpSocketIo::new(client_socket, [0; 1473]).unwrap())
                .unwrap(),
        )
    });
    let call = without_allocating(|| client.get("large").to(peer).send(0).unwrap());
    let clock = Instant::now();
    let mut small = [0xa5; 1999];
    loop {
        let now = clock.elapsed().as_millis() as u64;
        without_allocating(|| {
            server.poll(now).unwrap();
            client.poll(now).unwrap();
        });
        match without_allocating(|| client.take_response_into(call, &mut small)) {
            Ok(None) => {}
            Err(_) => break,
            Ok(Some(_)) => panic!("short output was accepted"),
        }
        assert!(clock.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert_eq!(small, [0xa5; 1999]);
    let mut body = [0; 2000];
    let reply = without_allocating(|| client.take_response_into(call, &mut body))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(reply.code(), Code::CONTENT);
    assert_eq!(reply.payload(), BODY);
}
