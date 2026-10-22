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
