# coaptic

CoAP for devices and services that need predictable resource use and protected communication.

- Written in Rust, with no standard library or heap allocation required.
- `no_std` by default; optional `alloc` and `std` support when you need them.
- Bounded memory with configurable capacities and explicit backpressure.
- Confirmable requests with retransmission, Observe subscriptions, and block-wise transfers.
- OSCORE protection required by default; optional EDHOC for authenticated provisioning.

One API lets you serve resources, send requests, and exchange messages, from an
ESP32 to a host service. You supply the transport and clock, register resources,
and call `poll`. No background runtime is required.

Explore CoAP as an alternative to HTTP for telemetry and frequent resource
exchanges. Coaptic's focus is high throughput, consistent behavior under load,
and security boundaries you can inspect and test, with progress assessed through
repeatable measurements.

Development includes testing on ESP32-S3 hardware. Production readiness has
not yet been established; longer-running device deployments and security
qualification are still ahead. Early users and contributors are welcome,
especially those bringing real workloads, interoperability checks, and
reproducible performance and security results.

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

## Protected communication

Ordinary App construction requires a provisioned OSCORE context. Replace
`.allow_plaintext()` with `.oscore(provisioned_context)` on each peer for
protected communication. Peer provisioning and safe key reuse across restarts
are part of your application's security setup; hardware security qualification
is still in progress.

The API reference is rustdoc: run `cargo doc --open` for transports, resource
handlers, security contexts, and further examples. See [SECURITY.md](SECURITY.md)
for the supported security boundary and vulnerability reporting.

## License

Coaptic is available under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option. Vendored EDHOC code retains its
[BSD-3-Clause notice](src/provisioning/lakers/LICENSE-BSD).
