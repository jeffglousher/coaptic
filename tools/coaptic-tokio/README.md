# Coaptic on a caller-owned Tokio runtime

This unpublished adapter connects Coaptic's native async App API to Tokio UDP
and timers. It accepts an already bound `tokio::net::UdpSocket`; socket creation,
runtime configuration and task scheduling remain application decisions.

```rust
use coaptic_tokio::{Clock, Udp};

async fn adopt(socket: tokio::net::UdpSocket) -> std::io::Result<()> {
    // Constrained RX slots hold 1152 bytes; one extra byte detects truncation.
    let transport = Udp::new(socket, [0; 1153])?;
    let clock = Clock::default();
    let _now = clock.now_ms()?;
    // Bind transport to an App and use wait_async/progress_async/send_async.
    let (_socket, _scratch) = transport.into_parts();
    Ok(())
}
```

Enable the caller's Tokio I/O and time drivers and keep the originating runtime
alive. `Udp::from_std` can register a standard socket inside an entered runtime;
it sets nonblocking mode. A runtime without I/O enabled violates that method's
documented contract. The adapter never creates a runtime, thread or background
task, and never changes App capacities. `Clock` copies share one monotonic epoch
for timestamps and absolute deadlines.

Receive scratch is reusable caller-owned storage, including a borrowed slice.
Its size must exceed the largest receive buffer by one byte. Oversized packets
are consumed and rejected without delivering a truncated prefix. Dropping a
pending receive or timer cancels that operation; later calls remain usable.
Tokio readiness may retry spurious readiness internally. Reactor/executor
allocations are distinct from the App's fixed pools and the adapter's scratch.

The tests exercise current-thread Tokio with real loopback UDP, socket options
and ownership, cancellation, oversize/refusal/recovery, timers, and a complete
unprotected blockwise App exchange. This does not establish performance, device
support, every executor configuration, or testing on operating systems other
than the recorded validation host. Local validation on Windows x86-64 with
Rust 1.99.1 and Tokio 1.53.3 passed all seven tests and strict all-target Clippy.
Existing `coaptic-async-host` remains usable.

Run `cargo test --locked --manifest-path tools/coaptic-tokio/Cargo.toml` and
`cargo clippy --locked --manifest-path tools/coaptic-tokio/Cargo.toml --all-targets -- -D warnings`
from the repository root using the project's registered build lane.

The runnable example creates a current-thread runtime, adopts two sockets,
completes one loopback request, and shuts down within a five-second deadline:

```text
cargo run --locked --manifest-path tools/coaptic-tokio/Cargo.toml --example loopback
```

The example explicitly owns a 2 MiB thread stack for debug construction of its
two Apps; Windows main's default stack was insufficient. This stack reservation
is application/runtime storage, not the adapter's packet memory or a measured
peak or minimum requirement. The library itself creates no threads and sets no
stack size. The example's real loopback run and final all-target Clippy passed
on the same Windows validation host.
