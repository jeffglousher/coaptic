//! UDP receive never introduces an allocation policy through the `alloc` feature.
#![cfg(feature = "std")]

use coaptic::storage::{DatagramIo, UdpSocketIo};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::UdpSocket;
use std::time::Duration;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct TrackingAllocator;

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}

// Instrument only this test thread; socket setup and the test harness stay outside.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn without_allocating<R>(work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.with(|count| count.set(None));
        }
    }
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let reset = Reset;
    let result = work();
    let count = ALLOCATIONS.with(|count| count.get().unwrap());
    drop(reset);
    assert_eq!(count, 0, "UDP receive allocated or grew storage");
    result
}

#[test]
fn raw_socket_refusal_preserves_datagram_for_reusable_scratch() {
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let sender = UdpSocket::bind(address).unwrap();
        let mut receiver = UdpSocket::bind(address).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let destination = receiver.local_addr().unwrap();
        sender.send_to(&[0x39; 2048], destination).unwrap();
        let mut output = [0xa5; 2048];
        let refused = without_allocating(|| DatagramIo::recv(&mut receiver, &mut output));
        assert_eq!(
            refused.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(output, [0xa5; 2048]);

        let mut receiver = without_allocating(|| UdpSocketIo::new(receiver, [0; 2049]).unwrap());
        let got = without_allocating(|| receiver.recv(&mut output)).unwrap();
        assert_eq!(got, Some((2048, sender.local_addr().unwrap().into())));
        assert_eq!(output, [0x39; 2048]);

        sender.send_to(&[0x44; 2049], destination).unwrap();
        output.fill(0xa5);
        assert!(without_allocating(|| receiver.recv(&mut output)).is_err());
        assert_eq!(output, [0xa5; 2048]);
        sender.send_to(b"complete", destination).unwrap();
        let n = without_allocating(|| receiver.recv(&mut output))
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(&output[..n], b"complete");
        receiver.socket().set_nonblocking(true).unwrap();
        assert_eq!(
            without_allocating(|| receiver.recv(&mut output)).unwrap(),
            None
        );
    }
}

#[cfg(feature = "alloc")]
#[test]
fn caller_allocated_scratch_is_reused_without_growth() {
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let destination = socket.local_addr().unwrap();
    let scratch = vec![0; 2049];
    let pointer = scratch.as_ptr();
    let capacity = scratch.capacity();
    let mut receiver = without_allocating(|| UdpSocketIo::new(socket, scratch).unwrap());
    let mut output = [0; 2048];
    for value in [0x39, 0x44, 0x5a] {
        sender.send_to(&[value; 2048], destination).unwrap();
        let got = without_allocating(|| receiver.recv(&mut output)).unwrap();
        assert_eq!(got, Some((2048, sender.local_addr().unwrap().into())));
        assert_eq!(output, [value; 2048]);
    }
    let (_, scratch) = receiver.into_parts();
    assert_eq!(scratch.as_ptr(), pointer);
    assert_eq!(scratch.capacity(), capacity);
    assert_eq!(scratch.len(), 2049);
}
