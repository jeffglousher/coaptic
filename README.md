# coaptic

CoAP in Rust for constrained devices and network services.

Expose resources and exchange messages through one client/server API. You supply
the transport and clock; Coaptic handles protocol exchanges with bounded storage.

- `no_std` and no heap allocation by default; optional `alloc` and `std`.
- Configurable memory limits and explicit backpressure.
- Confirmable requests, retransmission, Observe subscriptions, and configurable block-wise transfers.
- OSCORE protection required by default; optional EDHOC for authenticated provisioning.

Explore CoAP as an alternative to HTTP for telemetry and services where
throughput and predictable resource use matter. Coaptic gives you explicit
control over memory, scheduling, and message protection. Call `poll` to drive
communication; no background runtime is required.

Early users and contributors are welcome.

## Get started

Until the first crates.io release, add Coaptic from Git:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic", features = ["std"] }
getrandom = "0.3"
```

The examples below use `std` for UDP sockets and time, and explicitly allow
plaintext on localhost. Embedded applications can use the default `no_std`
build with their own transport, monotonic clock, and secure random source.

### Serve a resource

Save this as `examples/server.rs` in your project:

```rust
use std::{net::UdpSocket, thread, time::{Duration, Instant}};
use coaptic::{App, Request, Response, get, post};

fn temperature(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

fn echo(request: Request<'_>) -> Response<'static> {
    Response::content_copy(request.payload())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("127.0.0.1:5683")?;
    socket.set_nonblocking(true)?;
    let mut server = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .route("sensors/temp", get(temperature))
        .route("echo", post(echo))
        .allow_plaintext()
        .bind(socket)?;

    let clock = Instant::now();
    loop {
        server.poll(clock.elapsed().as_millis() as u64)?;
        thread::sleep(Duration::from_millis(1));
    }
}
```

Run `cargo run --example server`. The server now exposes `GET /sensors/temp`
and `POST /echo` for short payloads.
Add more resources with `.route(...)`; handlers receive a `Request` and return
a `Response`. Chain methods such as `get(handler).put(other_handler)` to expose
GET and PUT on the same resource.

### Send a request

Save this as `examples/client.rs` and run `cargo run --example client` in a
second terminal:

```rust
use std::{net::UdpSocket, thread, time::{Duration, Instant}};
use coaptic::{App, Endpoint};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .allow_plaintext()
        .bind(socket)?;

    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let clock = Instant::now();
    let call = client.get("sensors/temp").to(peer).send(0)?;
    loop {
        client.poll(clock.elapsed().as_millis() as u64)?;
        if let Some(reply) = client.take_response(call) {
            let reply = reply?;
            println!("{}", String::from_utf8_lossy(reply.payload()));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
```

To exchange a short payload with the echo resource, use the same polling and
response workflow:

```rust
let call = client.post("echo").payload(b"hello").to(peer).send(now_ms)?;
```

For larger representations, enable body pools with `.block_wise::<true>()`;
read assembled replies through `Response::body()`. The shipped profiles provide
4096 bytes per body slot. Custom storage lets you choose other capacities.
With `alloc`, `.bind_alloc(...)` allocates bounded
pools once, with no pool growth during `poll`.

## Protected communication

Ordinary App construction requires a provisioned OSCORE context. Replace
`.allow_plaintext()` with `.oscore(provisioned_context)` on each peer for
protected communication. Peer provisioning and safe key reuse across restarts
are part of your application's security setup; hardware security qualification
is still in progress.

The API reference is rustdoc: run `cargo doc --open` for transports, resource
handlers, security contexts, and further examples. See [SECURITY.md](SECURITY.md)
for the supported security boundary and vulnerability reporting.

## Status and preliminary evidence

Production readiness has not yet been established. Security qualification
continues; longer-running device validation is still planned.

Preliminary ESP32-S3 hardware testing includes a dedicated Waveshare
ESP32-S3-ETH running a Wi-Fi IPv4 UDP service against an independent aiocoap
0.4.16 peer. The 2026-10-04 report records nine passing functional cases,
including GET, echo, Block1/Block2, Observe, and malformed-input recovery.
See the [firmware identity, toolchains, results, and limitations](https://github.com/jeffglousher/coaptic/issues/333)
and the [test driver at the tested revision](https://github.com/jeffglousher/coaptic/blob/f618540597f4062655761e122c8c24d062f13a86/tools/qualification/network_peer.py).
Results apply to that recorded firmware revision.
These network tests used explicit plaintext and sequential requests on one
board; they do not measure saturation throughput. Protected hardware tests were
separate local OSCORE loopback probes. Network OSCORE, durable flash replay
state, power-loss recovery, and broader device qualification remain open.
An earlier soak timeout remains unresolved.

For reproducible performance evaluation, the
[network campaign method](https://github.com/jeffglousher/coaptic/blob/main/tools/benchmark/README.md)
documents commands, pinned peers, complete-response checks, payload sizes,
concurrency, repetitions, and failure accounting. Its loss-free CoAP workload
does not exercise retransmission; plaintext CoAP and TLS-protected HTTP/3 have
different security costs. Smoke samples do not establish a performance advantage
or reliability under network faults.

## License

Coaptic is available under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option. Vendored EDHOC code retains its
[BSD-3-Clause notice](src/provisioning/lakers/LICENSE-BSD).
