//! Bounded IPv4 qualification service; application state stays in this adapter.
//!
//! `/identity` returns the build's 32-byte run ID, `/test` returns `coaptic`,
//! `/echo` accepts POST bodies up to 128 bytes, `/upload` returns an eight-byte
//! length and rolling checksum for assembled POST bodies, and `/large`
//! returns 2,000 bytes of `0x5a`, and `/ticks` is an observable big-endian u32.
//! This service has no actuators, credentials, OSCORE context or durable state.
//! Use it on a private qualification network, not as a deployed device driver.
//!
//! Generate and compile with ESPHome 2026.9.1 using
//! `tools/qualification/esphome_network.py --compile --chip esp32s3 --output
//! target/qualification/s3-network/build.json`. The generated `access.json`,
//! YAML and firmware contain a disposable AP credential and stay local. Join
//! that AP, then run `tools/qualification/network_peer.py --host 192.168.4.1
//! --build target/qualification/s3-network/build.json --output
//! target/qualification/s3-network/runtime.json` with aiocoap 0.4.16. This peer
//! validates exact bodies, transfer assembly, Observe cancellation/reuse,
//! malformed-input recovery and repeated requests over a non-loopback address.
//! Both scripts require Python 3.13 in the Windows qualification environment.
//! Record the USB console separately for the owned task's stack high-water;
//! the client report alone does not establish whole-firmware stack coverage.
#![forbid(unsafe_code)]

use coaptic::storage::DatagramIo;
use coaptic::{App, Request, Response, get, post, profiles};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ID: [AtomicU8; 32] = [const { AtomicU8::new(0) }; 32];
static TICKS: AtomicU32 = AtomicU32::new(0);
const LARGE: [u8; 2000] = [0x5a; 2000];

struct Owner;
impl Drop for Owner {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

fn identity(_: Request<'_>) -> Response<'static> {
    let id = core::array::from_fn::<_, 32, _>(|i| ID[i].load(Ordering::Relaxed));
    Response::content_copy(&id)
}

fn test(_: Request<'_>) -> Response<'static> {
    Response::content(b"coaptic")
}

fn echo(request: Request<'_>) -> Response<'static> {
    let body = request.body().unwrap_or(request.payload());
    if body.len() > 128 {
        return Response::new(coaptic::Code::REQUEST_ENTITY_TOO_LARGE);
    }
    Response::content_copy(body)
}

fn upload(request: Request<'_>) -> Response<'static> {
    let body = request.body().unwrap_or(request.payload());
    let checksum = body
        .iter()
        .fold(0u32, |sum, byte| sum.rotate_left(5) ^ u32::from(*byte));
    let mut result = [0; 8];
    result[..4].copy_from_slice(&(body.len() as u32).to_be_bytes());
    result[4..].copy_from_slice(&checksum.to_be_bytes());
    Response::content_copy(&result)
}

fn large(_: Request<'_>) -> Response<'static> {
    Response::content(&LARGE)
}

fn ticks(_: Request<'_>) -> Response<'static> {
    Response::content_copy(&TICKS.load(Ordering::Relaxed).to_be_bytes()).observe(0)
}

/// Runs until the owning platform returns `None` from its yielding clock.
///
/// The caller owns the live transport and supplies a secure entropy function.
/// Only one service may run. A stopped service must not be rebound to the same
/// endpoint during EXCHANGE_LIFETIME; this adapter does not persist MID state.
/// Each iteration yields through `clock`; no unbounded packet-draining loop is
/// added here. Consecutive transport/poll failures refuse after 32 iterations.
pub fn run<T: DatagramIo>(
    io: T,
    random: fn(&mut [u8]) -> bool,
    mut clock: impl FnMut() -> Option<u64>,
    id: &[u8; 32],
) -> Result<(), &'static str> {
    if ACTIVE.swap(true, Ordering::Acquire) {
        return Err("already running");
    }
    let _owner = Owner;
    for (byte, value) in ID.iter().zip(id) {
        byte.store(*value, Ordering::Relaxed);
    }
    TICKS.store(0, Ordering::Relaxed);
    let mut app = App::profile::<profiles::Constrained>()
        .randomness(random)
        .block_wise::<true>()
        .route("identity", get(identity))
        .route("test", get(test))
        .route("echo", post(echo))
        .route("upload", post(upload))
        .route("large", get(large))
        .route("ticks", get(ticks))
        .well_known_core()
        .bind(io)
        .map_err(|_| "bind")?;
    let mut next = 0;
    let mut errors = 0;
    while let Some(now) = clock() {
        if app.poll(now).is_err() {
            errors += 1;
            if errors >= 32 {
                return Err("poll failures");
            }
        } else {
            errors = 0;
        }
        if now >= next {
            let value = TICKS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            let _ = app.notify(
                now,
                &["ticks"],
                Response::content_copy(&value.to_be_bytes()),
            );
            next = now.saturating_add(250);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::Endpoint;

    struct Idle;
    impl DatagramIo for Idle {
        type Error = ();
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            Ok(bytes.len())
        }
    }

    #[test]
    fn stopped_service_releases_ownership_and_entropy_refusal_is_reported() {
        let id = [b'a'; 32];
        assert_eq!(run(Idle, |_| false, || None, &id), Err("bind"));
        for _ in 0..2 {
            let mut time = Some(1000);
            assert_eq!(
                run(
                    Idle,
                    |bytes| {
                        bytes.fill(42);
                        true
                    },
                    || time.take(),
                    &id
                ),
                Ok(())
            );
        }
    }
}
