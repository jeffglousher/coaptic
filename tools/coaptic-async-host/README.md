# Native async App on host UDP

Run the complete App client/server example:

```sh
cargo run --release --manifest-path tools/coaptic-async-host/Cargo.toml --example app_loopback
cargo test --manifest-path tools/coaptic-async-host/Cargo.toml
```

The example uses `SmallDevice` / `Constrained`, four route slots, one Call,
zero deferred replies, four security request bindings, one association, unprotected localhost UDP and disabled blockwise storage.
It prints complete App and future/task sizes for the compiled target. OS reactor
allocations and stack peaks are not included in those measurements.
On Windows, the unoptimized example exceeds the default main-thread stack; use
the optimized command above. The tests pass on the test runner's larger stack.
No stack-size override or boxed future is used by the example.

## Bring an existing socket

Create and configure a `std::net::UdpSocket` using the host APIs you already use,
then move it into `Udp::new(socket, scratch)`. No raw descriptor conversion or
second bind is needed. The adapter preserves the port and socket options while
enabling nonblocking mode and registering the socket with `async-io`:

```rust
use coaptic_async_host::Udp;
use std::net::UdpSocket;

let socket = UdpSocket::bind("127.0.0.1:0")?;
socket.set_ttl(32)?;
let transport = Udp::new(socket, [0; 1153])?;
let endpoint = transport.local_endpoint()?;
assert_eq!(transport.socket().ttl()?, 32);
// Move transport into the usual SmallDevice App builder's .bind(transport).
```

The 1153-byte scratch here matches SmallDevice's 1152-byte RX datagrams; choose
the matching capacity for another profile. The constructor consumes the socket
even on error. Do not receive through a cloned handle or change its blocking
mode while this adapter is active. `socket()` permits option inspection and
configuration without rebinding. A standalone adapter's `into_parts()`
deregisters readiness and returns its socket and scratch; the socket remains
nonblocking. A deregistration failure returns an error and drops both owners.

This host adapter works on Windows, Linux and macOS. `async_io::block_on` in the
example is the executor, and `async-io` supplies OS readiness and timers. It does
not start a Tokio runtime or require one thread per App. Other executors and
embedded drivers can implement the core async transport/timer contracts without
changing application handlers. The ESP integration is a separate platform driver;
this host reactor is not part of its firmware.

## Drive App progress

`Udp` implements native `AsyncDatagramIo`; no synchronous socket adapter or
queued-success bridge is involved. Bind it through the normal App builder. Use
`Outgoing::send_async` to install a Call after transport acceptance, then alternate
`App::wait_async` and `App::progress_async`. Retrieve responses with the same
`take_response` API as synchronous Apps. `Clock` supplies one monotonic origin for
request timestamps, completion clocks and `AsyncTimer` deadlines. Each progress
pass takes an absolute send deadline. `send_async(now, clock, timer)` enforces
the request's configured `.deadline(...)` while transport is pending. It yields
with `ProgressRequired` when existing App work needs progress sooner. Cancellation
releases local request ownership; earlier accepted datagrams remain uncertain.
`send_until_async(now, clock, timer, limit)` adds a driving limit without shortening
the installed Call's response lifetime.

The caller owns 1153 receive-scratch bytes per Constrained UDP adapter (1152-byte
slot capacity plus one byte for truncation detection). Received bytes copy once
into an existing RX slot. There are no hidden packet queues. The host readiness
reactor may allocate; Coaptic's async core requires neither allocator nor executor.

Tests exercise real OS readiness, cancelled receive recovery, oversized datagram
refusal, classic and Q-Block1/Q-Block2 transfers with 2000-byte bodies, deferred
Observe completion, CON notifications and local-work wake priority. A protected loopback checks OSCORE on every application datagram, a complete Block2 download, Observe registration, and replay-checkpoint callback delivery (the fixture acknowledges checkpoints in memory, not durable disk). This is
library-to-itself integration, not independent interoperability qualification.


