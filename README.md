# Coaptic

Coaptic is a Rust library for CoAP: small messages over UDP between devices and
services. Use it to serve resources, send requests, watch values with Observe,
and transfer bodies that do not fit in one packet.

It runs on microcontrollers and on servers. You choose the memory budget before
starting: fixed arrays on a device, or pools allocated once on a host. Requests
reuse that storage. When capacity runs out, the library reports the limit and
reserves packet capacity for protocol control and recovery.

Messages are protected with OSCORE by default. Add the EDHOC feature to establish
authenticated session keys. The core supports `no_std` without a heap, with both
synchronous and native async interfaces.

**Pre-release:** use the Git dependency below. APIs may change.

## Try it

Run a protected client and server on your computer:

```sh
git clone https://github.com/jeffglousher/coaptic.git
cd coaptic
cargo run --locked --example oscore_pair --features std
```

The example starts both peers, creates fresh keys and checks a complete
200-byte response. It uses fixed storage. To try the same exchange with pools
allocated at startup:

```sh
cargo run --locked --example oscore_pair --features std,alloc -- --alloc
```

These keys last for the demonstration. For a real device, use the
[commissioning guide](tools/coaptic-enroll/README.md).

## Use it in an application

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic" }
```

Add the `std` feature for host sockets and `alloc` if you want heap-backed pools.
Neither feature changes the security policy or chooses the storage backend.

A resource handler returns a response. The builder connects it to a path:

```rust
use coaptic::{App, Request, Response, get, resources::SmallDevice};

fn temperature(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

// Supply your transport, OSCORE context and secure random source.
let mut app = App::with_resources(SmallDevice::new())
    .oscore(context)
    .route("temperature", get(temperature))
    .bind_with_entropy(io, secure_random)?;

app.poll(now_ms)?; // Call regularly with a monotonic millisecond clock.
```

The complete [shared service example](examples/support/fixed_service.rs) shows
the device-independent application code. Start with `SmallDevice` and adjust its
capacities for your workload. For startup allocation, select
`SmallDevice::new().allocated(heap_limit)` before binding. Socket, runtime and
stack memory also belong in your application's budget.

For async host integration, see the
[UDP adapter and runnable example](tools/coaptic-async-host/README.md).
The companion repository supplies an
[ESP32-S3 integration](https://github.com/jeffglousher/coaptic-validation#custom-esphome-qualification)
and a [Linux container example](https://github.com/jeffglousher/coaptic-validation#protected-linux-container).

## Supply a socket and choose how to wait

An App accepts a transport you supply. Its builder's `.bind(transport)` attaches
that transport; it does not bind an OS port or select a runtime. On a host, bind
and configure a standard UDP socket once, then move it into the adapter:

```rust
use coaptic::storage::{MemoryProfile, UdpSocketIo, profiles::Constrained};
use std::net::UdpSocket;

let socket = UdpSocket::bind("127.0.0.1:5683")?;
socket.set_nonblocking(true)?;
let io = UdpSocketIo::new(socket, [0; Constrained::RX_DATAGRAM_BYTES + 1])?;
// Pass io to App::with_resources(SmallDevice::new()).bind_with_entropy(...).
```

This keeps the same socket, local port and configured options on Windows, Linux
and macOS. Size scratch for the selected App's RX datagram capacity plus one byte
to detect truncation. `UdpSocketIo::new` retains the socket's blocking policy;
nonblocking mode lets `App::poll` return when there is no packet. Avoid concurrent
receives through cloned handles: they compete with the App for its packets.

Synchronous and asynchronous Apps use the same protocol engine. The difference
is how the application waits for work:

- **Synchronous polling:** call `App::poll(now_ms)` with one monotonic clock.
  A tight loop is busy polling. A readiness loop such as Mio can sleep until
  socket activity or `App::next_timeout()`; it must also wake for application
  work, notification signals and checkpoint retries. A fixed sleep between polls
  is simple but adds latency; it is not equivalent to an OS readiness loop.
- **Native async:** pass the same kind of preconfigured socket to
  `coaptic_async_host::Udp::new(socket, scratch)`. It enables nonblocking mode and
  registers host readiness without rebinding. Drive `wait_async` and
  `progress_async` with the adapter's `Clock`, as in the runnable example. This
  adapter uses `async-io`; the core itself requires no executor or runtime.
- **An existing Tokio application:** pass an already registered socket to
  `coaptic_tokio::Udp::new(socket, scratch)`, or register a standard socket with
  `Udp::from_std` inside the caller's I/O-enabled runtime. Use its `Clock` for App
  timestamps and timers. See the [Tokio adapter](tools/coaptic-tokio/README.md).
- **Embedded or an existing event loop:** implement `DatagramIo` or
  `AsyncDatagramIo` for the platform's socket/driver and pass it to the same App
  builder. Readiness, task creation and timer waiting remain platform choices.

Inspect an attached transport with `app.transport()` or `app.transport_mut()`.
Host adapters expose `socket()`; keep an async socket nonblocking. Standalone
adapters can return their socket and scratch with `into_parts()`; the async-io form
is fallible because it deregisters OS readiness. The Tokio form returns the
registered Tokio socket unchanged. Dropping the owning App closes
its transport. Socket ownership does not imply a task or a new thread per App.

See [runtime choices and integration status](architecture/ASYNC-RUNTIMES.md).
Tokio is the preferred adapter for applications already using that runtime;
the standalone async-io adapter remains an alternative. Embassy is a planned
embedded integration, not yet a tested adapter.
Existing ESPHome firmware uses a separate ESP-IDF/FreeRTOS driver.

## Pair a device

The Rust commissioning tool guides you through inspection, identity setup,
pairing, permissions and recovery on Windows, Linux and macOS:

```sh
cargo run --manifest-path tools/coaptic-enroll/Cargo.toml
```

It supports touch-required YubiKey PIV credentials and explicit touchless
development credentials. See its [setup requirements](tools/coaptic-enroll/README.md)
before connecting hardware. Normal reconnects restore the installed identity
and permissions and establish fresh sessions; they do not require another touch.

For interoperability or local tests, select `.allow_unprotected()` explicitly.
An authentication failure never switches a protected connection to unprotected
operation. See [SECURITY.md](SECURITY.md) for persistent keys, recovery and durable
application effects.

## Find your way around

- [Examples](examples) and `cargo doc --all-features --open` for the API.
- [Project plan](PLAN.md) for priorities and remaining work.
- [Protocol scope](architecture/IETF-RFC-PLAN.md) for supported CoAP features.
- [Development](DEVELOPMENT.md) for contributing and checking changes.
- [Benchmarks and independent peers](https://github.com/jeffglousher/coaptic-validation)
  for measured results and their configurations.

DTLS, TCP/TLS/WebSockets, proxies, Resource Directory, multicast and ACE are
outside the current delivery scope. Contributions need an agreed use case and
implementation plan.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
