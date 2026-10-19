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


