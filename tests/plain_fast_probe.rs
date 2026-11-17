//! Quick thread-cycle check for one ready unprotected GET.

#![cfg(windows)]

use core::cell::Cell;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::hint::black_box;
use std::time::Instant;

use coaptic::app::App;
use coaptic::message::{Code, Message, MessageId, Opt, Token, Type, encode};
use coaptic::platform::AsyncTimer;
use coaptic::storage::{AsyncDatagramIo, DatagramIo, DatagramTransport, Endpoint};
use coaptic::{Request, Response, get, profiles};

const N: u32 = 8_192;
const WARMUP: u32 = 256;
const ROUNDS: u32 = 4;
const PEER: Endpoint = Endpoint::v4([192, 0, 2, 8], 5683);

unsafe extern "system" {
    fn GetCurrentThread() -> *mut core::ffi::c_void;
    fn QueryThreadCycleTime(thread: *mut core::ffi::c_void, cycles: *mut u64) -> i32;
    fn SetThreadAffinityMask(thread: *mut core::ffi::c_void, mask: usize) -> usize;
}

fn thread_cycles() -> u64 {
    let mut cycles = 0u64;
    assert_eq!(
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) },
        1
    );
    cycles
}

struct CaptureIo<'a> {
    sends: &'a Cell<u64>,
}
impl DatagramTransport for CaptureIo<'_> {
    type Error = ();
}
impl DatagramIo for CaptureIo<'_> {
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        Ok(None)
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        self.sends.set(self.sends.get() + 1);
        Ok(bytes.len())
    }
}
impl AsyncDatagramIo for CaptureIo<'_> {
    async fn recv(&mut self, _: &mut [u8]) -> Result<(usize, Endpoint), ()> {
        core::future::pending().await
    }
    async fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        self.sends.set(self.sends.get() + 1);
        Ok(bytes.len())
    }
}

struct ProbeTimer<'a> {
    now_ms: &'a Cell<u64>,
    deadline: Option<u64>,
}
impl AsyncTimer for ProbeTimer<'_> {
    fn armed(&self) -> Option<u64> {
        self.deadline
    }
    fn schedule(&mut self, deadline: Option<u64>) {
        self.deadline = deadline;
    }
    fn poll_wait(&mut self, _: &mut Context<'_>) -> Poll<()> {
        if self
            .deadline
            .is_some_and(|deadline| self.now_ms.get() >= deadline)
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

fn handler(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

fn prepare(seq: u32, wire: &mut [u8]) -> usize {
    let path = [Opt::uri_path("value")];
    let token = Token::new(&[1, 2, 3, 4]).unwrap();
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(seq as u16))
        .with_token(token)
        .with_options(&path);
    encode(&request, wire).unwrap()
}

fn at(seq: u32) -> u64 {
    10 + u64::from(seq) * 300_000
}

fn app(
    sends: &Cell<u64>,
) -> App<
    profiles::Constrained,
    CaptureIo<'_>,
    8,
    true,
    coaptic::app::AppStore<profiles::Constrained, true>,
    0,
    4,
    4,
    1,
> {
    App::profile::<profiles::Constrained>()
        .deterministic_for_tests()
        .block_wise::<true>()
        .allow_unprotected()
        .route("value", get(handler))
        .bind(CaptureIo { sends })
        .unwrap()
}

fn run_sync(seq: &mut u32) -> u64 {
    let sends = Cell::new(0u64);
    let mut sync = app(&sends);
    let mut wire = [0u8; 64];
    for _ in 0..WARMUP {
        let n = prepare(*seq, &mut wire);
        let rx = sync.engine_mut().acquire_rx().unwrap();
        sync.engine_mut().write_rx(rx, &wire[..n], PEER).unwrap();
        sync.poll(at(*seq)).unwrap();
        *seq += 1;
    }
    let before = sends.get();
    let cycles_before = thread_cycles();
    for _ in 0..N {
        let n = prepare(*seq, &mut wire);
        let rx = sync.engine_mut().acquire_rx().unwrap();
        sync.engine_mut().write_rx(rx, &wire[..n], PEER).unwrap();
        sync.poll(at(*seq)).unwrap();
        *seq += 1;
    }
    assert_eq!(sends.get() - before, u64::from(N));
    (thread_cycles() - cycles_before) / u64::from(N)
}

fn run_async(seq: &mut u32) -> (u64, usize) {
    let sends = Cell::new(0u64);
    let now_ms = Cell::new(0u64);
    let mut async_app = app(&sends);
    let mut timer = ProbeTimer {
        now_ms: &now_ms,
        deadline: None,
    };
    let mut clock = || Ok::<u64, ()>(now_ms.get());
    let mut wire = [0u8; 64];
    for _ in 0..WARMUP {
        let n = prepare(*seq, &mut wire);
        let rx = async_app.engine_mut().acquire_rx().unwrap();
        async_app
            .engine_mut()
            .write_rx(rx, &wire[..n], PEER)
            .unwrap();
        let now = at(*seq);
        now_ms.set(now);
        let mut progress = pin!(async_app.progress_async(now, &mut clock, &mut timer, now + 1_000));
        assert!(matches!(
            progress
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
        *seq += 1;
    }
    let future = async_app.progress_async(at(*seq), &mut clock, &mut timer, at(*seq) + 1_000);
    let future_bytes = core::mem::size_of_val(&future);
    drop(future);
    let before = sends.get();
    let start = Instant::now();
    let mut prep_ticks = 0u64;
    let mut build_ticks = 0u64;
    let mut poll_ticks = 0u64;
    let cycles_before = thread_cycles();
    for _ in 0..N {
        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        let n = prepare(*seq, &mut wire);
        let rx = async_app.engine_mut().acquire_rx().unwrap();
        async_app
            .engine_mut()
            .write_rx(rx, &wire[..n], PEER)
            .unwrap();
        let now = at(*seq);
        now_ms.set(now);
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };
        let mut progress = pin!(async_app.progress_async(now, &mut clock, &mut timer, now + 1_000));
        let t2 = unsafe { core::arch::x86_64::_rdtsc() };
        match progress
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => {
                black_box(result).unwrap();
            }
            Poll::Pending => panic!("pending"),
        }
        let t3 = unsafe { core::arch::x86_64::_rdtsc() };
        prep_ticks += t1.wrapping_sub(t0);
        build_ticks += t2.wrapping_sub(t1);
        poll_ticks += t3.wrapping_sub(t2);
        *seq += 1;
    }
    let ns = (start.elapsed().as_secs_f64() * 1e9 / f64::from(N)).round() as u64;
    assert_eq!(sends.get() - before, u64::from(N));
    println!(
        "ns={ns} prep_ticks={} build_ticks={} poll_ticks={}",
        prep_ticks / u64::from(N),
        build_ticks / u64::from(N),
        poll_ticks / u64::from(N)
    );
    (
        (thread_cycles() - cycles_before) / u64::from(N),
        future_bytes,
    )
}

fn run_floor(seq: &mut u32) -> u64 {
    let mut wire = [0u8; 64];
    let mut out = [0u8; 64];
    for _ in 0..WARMUP {
        let n = prepare(*seq, &mut wire);
        let parsed = coaptic::message::decode(&wire[..n]).unwrap();
        let ack = Message::new(Type::Acknowledgement, Code::CONTENT, parsed.message_id())
            .with_token(parsed.token())
            .with_payload(b"21.5");
        black_box(encode(&ack, &mut out).unwrap());
        *seq += 1;
    }
    let cycles_before = thread_cycles();
    for _ in 0..N {
        let n = prepare(*seq, &mut wire);
        let parsed = coaptic::message::decode(&wire[..n]).unwrap();
        let ack = Message::new(Type::Acknowledgement, Code::CONTENT, parsed.message_id())
            .with_token(parsed.token())
            .with_payload(b"21.5");
        black_box(encode(&ack, &mut out).unwrap());
        *seq += 1;
    }
    (thread_cycles() - cycles_before) / u64::from(N)
}

#[test]
#[ignore = "quick ready-GET thread-cycle check"]
fn plain_fast_screen() {
    assert_ne!(unsafe { SetThreadAffinityMask(GetCurrentThread(), 1) }, 0);
    let mut seq = 1u32;
    for round in 0..ROUNDS {
        let floor_cycles = run_floor(&mut seq);
        let sync_cycles = run_sync(&mut seq);
        let (async_cycles, future_bytes) = run_async(&mut seq);
        println!(
            "FAST round={round} floor_cycles={floor_cycles} sync_cycles={sync_cycles} async_cycles={async_cycles} progress_future={future_bytes}"
        );
    }
}
