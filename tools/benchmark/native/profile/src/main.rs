//! Diagnostic decomposition of the built-in App request path.
//!
//! `bench-profile BYTES OPERATIONS` validates sequential complete GET bodies
//! through a preallocated in-process transport. It separately records App bind
//! allocations, allocations and time strictly inside `poll`, work counters,
//! and idle standard UDP receive allocations at two receive capacities.
//! This excludes kernel I/O and driver encoding from the core measurement and
//! is not a throughput benchmark. Use the independent external socket driver
//! with diagnostics disabled for performance comparisons.
//! The process is single-threaded; allocator counting is enabled only around
//! the named phase. Payload generation, validation, and reporting are excluded.

use coaptic::message::{BlockValue, Message, MessageId, Opt, Token, Type, decode};
use coaptic::storage::{Capacities, DatagramIo, WorkMetrics};
use coaptic::{App, Code, Endpoint, Request, Response, get};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::net::UdpSocket;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BODY: OnceLock<&'static [u8]> = OnceLock::new();

struct CountingAllocator;

/// Forwards every allocation unchanged to System, counting successful calls.
///
/// Layouts and pointers are supplied by Rust's allocator interface. The
/// forwarding operations preserve its ownership and alignment requirements;
/// accounting uses nonallocating atomics and cannot recurse into the allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() && ENABLED.load(Ordering::Relaxed) {
            REALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(size as u64, Ordering::Relaxed);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Default)]
struct Counts {
    allocations: u64,
    reallocations: u64,
    bytes: u64,
}

fn begin() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    REALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Relaxed);
}

fn end() -> Counts {
    ENABLED.store(false, Ordering::Relaxed);
    Counts {
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        reallocations: REALLOCATIONS.load(Ordering::Relaxed),
        bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
    }
}

struct Packets {
    input: [u8; 1472],
    input_len: Option<usize>,
    output: [u8; 1472],
    output_len: Option<usize>,
}

struct ProbeIo(Rc<RefCell<Packets>>);

impl DatagramIo for ProbeIo {
    type Error = &'static str;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let mut packets = self.0.borrow_mut();
        let Some(n) = packets.input_len.take() else {
            return Ok(None);
        };
        if n > buf.len() {
            return Err("input capacity");
        }
        buf[..n].copy_from_slice(&packets.input[..n]);
        Ok(Some((n, Endpoint::v4([127, 0, 0, 1], 30000))))
    }

    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        let mut packets = self.0.borrow_mut();
        if packets.output_len.is_some() || bytes.len() > packets.output.len() {
            return Err("unexpected or oversized response");
        }
        packets.output[..bytes.len()].copy_from_slice(bytes);
        packets.output_len = Some(bytes.len());
        Ok(bytes.len())
    }
}

fn representation(_: Request<'_>) -> Response<'static> {
    Response::content(BODY.get().expect("initialized body")).etag(b"fixture")
}

fn capacities(bytes: usize) -> Capacities {
    Capacities {
        rx_datagram_slots: 4,
        rx_datagram_bytes: 1472,
        tx_datagram_slots: 4,
        tx_datagram_bytes: 1472,
        dedup_entries: 8,
        observe_entries: 4,
        rx_body_slots: Some(1),
        rx_body_bytes: Some(bytes.div_ceil(1024) * 1024),
        tx_body_slots: Some(2),
        tx_body_bytes: Some(bytes.div_ceil(1024) * 1024),
    }
}

fn print_work(work: WorkMetrics) {
    println!(
        "{{\"kind\":\"work\",\"polls\":{},\"recv_attempts\":{},\"recv_idle\":{},\"rx_wire_bytes\":{},\"tx_wire_bytes\":{},\"app_rx_staged_bytes\":{},\"block_rx_staged_bytes\":{},\"block_tx_staged_bytes\":{},\"tx_payload_encoded_bytes\":{},\"body_snapshot_bytes\":{},\"raw_rx_copied_bytes\":{},\"raw_tx_copied_bytes\":{},\"body_compare_calls\":{},\"body_compare_candidate_bytes\":{}}}",
        work.polls,
        work.recv_attempts,
        work.recv_idle,
        work.rx_wire_bytes,
        work.tx_wire_bytes,
        work.app_rx_staged_bytes,
        work.block_rx_staged_bytes,
        work.block_tx_staged_bytes,
        work.tx_payload_encoded_bytes,
        work.body_snapshot_bytes,
        work.raw_rx_copied_bytes,
        work.raw_tx_copied_bytes,
        work.body_compare_calls,
        work.body_compare_candidate_bytes,
    );
}

fn core_probe(bytes: usize, operations: usize) -> Result<(), Box<dyn std::error::Error>> {
    let packets = Rc::new(RefCell::new(Packets {
        input: [0; 1472],
        input_len: None,
        output: [0; 1472],
        output_len: None,
    }));
    begin();
    let bind_result = App::builder()
        .routes::<1>()
        .block_wise::<true>()
        .randomness(|buffer| getrandom::fill(buffer).is_ok())
        .route("bench", get(representation))
        .bind_alloc(ProbeIo(Rc::clone(&packets)), capacities(bytes));
    let setup = end();
    let mut app = bind_result?;
    let mut hot = Counts::default();
    let mut elapsed_ns = 0u128;
    let mut datagrams = 0usize;
    for operation in 0..operations {
        let token = Token::new(&(operation as u64).to_be_bytes()).ok_or("token length")?;
        let mut requested = None;
        let mut offset = 0;
        loop {
            let mid = MessageId::new(datagrams as u16);
            let encoded = requested.map(BlockValue::encode);
            let mut options = [Opt::uri_path("bench"), Opt::uri_path("bench")];
            let nopts = if let Some(ref block) = encoded {
                options[1] = Opt::block2(block);
                2
            } else {
                1
            };
            let request = Message::con(Code::GET, mid, token).with_options(&options[..nopts]);
            {
                let mut state = packets.borrow_mut();
                state.input_len = Some(request.encode(&mut state.input)?);
            }
            begin();
            let started = Instant::now();
            let outcome = app.poll(datagrams as u64);
            elapsed_ns += started.elapsed().as_nanos();
            let count = end();
            hot.allocations += count.allocations;
            hot.reallocations += count.reallocations;
            hot.bytes += count.bytes;
            outcome.map_err(|_| "poll failure")?;
            datagrams += 1;
            let mut state = packets.borrow_mut();
            let n = state.output_len.take().ok_or("missing response")?;
            let reply = decode(&state.output[..n])?;
            if reply.code() != Code::CONTENT
                || reply.message_id() != mid
                || reply.token() != token
                || reply.ty() != Type::Acknowledgement
                || reply
                    .payload()
                    .iter()
                    .enumerate()
                    .any(|(i, byte)| *byte != ((offset + i) % 251) as u8)
            {
                return Err("response mismatch".into());
            }
            let block = reply.block2().transpose()?;
            if let Some(block) = block {
                if block.num() as usize * usize::from(block.size()) != offset {
                    return Err("block offset".into());
                }
                if block.more() && reply.payload().len() != usize::from(block.size()) {
                    return Err("short intermediate block".into());
                }
                offset += reply.payload().len();
                if block.more() {
                    requested = Some(BlockValue::new(block.num() + 1, false, block.szx())?);
                    continue;
                }
            } else {
                offset += reply.payload().len();
            }
            if offset != bytes {
                return Err("incomplete body".into());
            }
            break;
        }
    }
    println!(
        "{{\"kind\":\"core\",\"body_bytes\":{bytes},\"operations\":{operations},\"datagrams\":{datagrams},\"poll_ns\":{elapsed_ns},\"setup_allocations\":{},\"setup_allocated_bytes\":{},\"poll_allocations\":{},\"poll_reallocations\":{},\"poll_allocated_bytes\":{}}}",
        setup.allocations, setup.bytes, hot.allocations, hot.reallocations, hot.bytes
    );
    print_work(app.work_metrics());
    Ok(())
}

fn udp_idle_probe(capacity: usize, polls: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut buffer = vec![0; capacity];
    for _ in 0..20 {
        DatagramIo::recv(&mut socket, &mut buffer)?;
    }
    begin();
    let started = Instant::now();
    for _ in 0..polls {
        if DatagramIo::recv(&mut socket, &mut buffer)?.is_some() {
            return Err("unexpected UDP datagram".into());
        }
    }
    let elapsed = started.elapsed().as_nanos();
    let count = end();
    println!(
        "{{\"kind\":\"udp_idle\",\"capacity\":{capacity},\"polls\":{polls},\"elapsed_ns\":{elapsed},\"allocations\":{},\"reallocations\":{},\"allocated_bytes\":{}}}",
        count.allocations, count.reallocations, count.bytes
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: bench-profile BYTES OPERATIONS".into());
    }
    let bytes: usize = args[1].parse()?;
    let operations: usize = args[2].parse()?;
    if !(1..=1_048_576).contains(&bytes)
        || operations == 0
        || operations
            .checked_mul(bytes.div_ceil(1024))
            .is_none_or(|n| n > 60_000)
    {
        return Err("body or operation limit".into());
    }
    let body: &'static [u8] = Box::leak(
        (0..bytes)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    BODY.set(body).map_err(|_| "body initialization")?;
    core_probe(bytes, operations)?;
    udp_idle_probe(1472, 1000)?;
    udp_idle_probe(2048, 1000)
}
