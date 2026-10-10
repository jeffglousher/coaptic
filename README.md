# coaptic

Free, open-source CoAP for microcontrollers and cloud services.

- Rust. `no_std`. Fixed storage by default; independent optional `std` and `alloc`.
- OSCORE message protection by default; optional authenticated EDHOC provisioning.
- Retransmission, Observe subscriptions, and block-wise transfers.
- One client/server API. Caller-owned transport, storage and polling.

## Get started: one service, two environments

Until publication, install from Git. A constrained device uses the defaults:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic" }
```

A Linux service adds OS socket support while keeping Coaptic's allocator disabled:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic", features = ["std"] }
getrandom = "0.3"
```

Both configurations keep OSCORE enabled and use the same fixed-storage `App`.
A host application's own allocator does not require Coaptic's `alloc` feature.
Cargo features are additive: another dependency enabling `coaptic/alloc` enables
it for that build. Even then, `App::builder().bind(...)` still uses fixed storage;
allocator-backed storage requires an explicit choice such as `bind_alloc`.
Fixed storage can live in caller-owned stack or static memory. OS, SDK and
application allocations are separate from Coaptic's storage.

### Shared Rust service

The [complete service](examples/support/fixed_service.rs) is compiled by the
[`no_std` library example](examples/fixed_service.rs) and reused by the host demo:

```rust
use coaptic::{App, Request, Response, get};

const TELEMETRY: [u8; 200] = [b'x'; 200];

fn telemetry(_: Request<'_>) -> Response<'static> {
    Response::content(&TELEMETRY)
}

// The caller supplies the transport, secure entropy and provisioned context.
let mut service = App::builder()
    .randomness(secure_random)
    .oscore(context)
    .route("telemetry", get(telemetry))
    .bind(io)?;
service.poll(now_ms)?;
```

Default storage has explicit resource bounds and no body pools. This service
returns a borrowed 200-byte representation and copies echo replies up to 128
bytes; larger echo requests receive 4.13. Use fixed body pools for block-wise
messages and `take_response_into` to collect complete replies into your buffer.

### ESP32 device

Use the shared service with your SDK's bounded `DatagramIo` adapter, secure
entropy function and monotonic clock. Your task owns the `App` and calls `poll`;
no Coaptic background runtime is installed. The shared example compiles without
`std` or `alloc`:

```sh
cargo check --example fixed_service
cargo test --example fixed_service
```

Board startup, Wi-Fi, ESP-IDF/ESPHome bindings and firmware qualification live in
[coaptic-validation](https://github.com/jeffglousher/coaptic-validation#custom-esphome-qualification).
Its custom ESPHome package preserves the consuming configuration's Wi-Fi and
API/OTA settings and requires explicit test-mode selection. The shared Rust
service above is application code, not a complete ESP32 firmware image.

The reserved Waveshare ESP32-S3 has passed finite protected LAN checks, complete
2000-byte replies, refusal cases and a software-restart replay check with a
fixed-storage Rust host. Credentials are privately staged by the operator.
Flash power-loss recovery, hostile rollback and production key custody remain
unqualified; [device evidence and limitations](https://github.com/jeffglousher/coaptic/issues/333).

### Linux or container service

Use the same shared service with a nonblocking UDP socket and fixed receive
scratch. The extra byte detects oversized datagrams instead of accepting a
truncated prefix:

```rust
use std::net::UdpSocket;
use coaptic::storage::UdpSocketIo;

let socket = UdpSocket::bind("127.0.0.1:0")?;
socket.set_nonblocking(true)?;
let io = UdpSocketIo::new(socket, [0; 1473])?;
let mut service = fixed_service::server(io, context, secure_random)?;
service.poll(now_ms)?;
```

Run the [complete protected pair](examples/oscore_pair.rs) on a host:

```sh
cargo run --example oscore_pair --features std
```

It supplies fresh OS-generated credentials to two localhost peers, uses fixed
socket scratch and response arrays, and verifies all 200 response bytes. It
prints `OSCORE: received all 200 bytes`. It demonstrates protected message
delivery, not authenticated enrollment or persistent credentials.

To run the same source in a disposable Linux container, from the repository root
with Docker installed:

```sh
docker run --rm --mount type=bind,src="$PWD",dst=/source,readonly -e CARGO_TARGET_DIR=/tmp/coaptic-target rust:1.99 cargo run --manifest-path /source/Cargo.toml --locked --example oscore_pair --features std
```

Both peers run inside the container; no UDP ports or credentials are published.
This recipe needs a Linux container runtime, network access to fetch build
inputs, and enough build storage for compilation. Container execution is a
separate platform check; a host run does not establish it.

### Plaintext localhost comparison

For an explicit unprotected compatibility demonstration, run
[`server`](examples/server.rs) and [`client`](examples/client.rs) in separate
terminals:

```sh
cargo run --example server --features std
cargo run --example client --features std
```

The client prints `21.5`. Both examples select `.allow_plaintext()` explicitly,
keep fixed socket scratch and collect responses into caller storage. The same
client API sends messages:

```rust
let call = client.post("echo").payload(b"hello").to(peer).send(now_ms)?;
```

## Security

Applications own authenticated credentials, secure entropy and durable security
state. Retained OSCORE credentials require sender-sequence reservations and
replay checkpoints before traffic resumes after restart. Fresh demo contexts
must not be reused after restarting. EDHOC can provision authenticated peers.
[Security boundary](SECURITY.md).

API reference and larger-payload examples: `cargo doc --open`.
[Qualification tooling](https://github.com/jeffglousher/coaptic-validation) |
[Benchmark method](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/benchmark/README.md).
Measurements remain preliminary. Production readiness has not been established.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
