# coaptic

Free, open-source CoAP for constrained devices and Linux services, written in Rust.

Serve resources, send requests, subscribe to updates with Observe, and transfer
larger bodies with block-wise messages through one client/server API. Coaptic
uses fixed storage and OSCORE message protection by default. Applications supply
the transport, clock, secure credentials and polling loop.

The core is independent of a particular microcontroller, board SDK or operating
system. Constrained devices use it without allocation or the Rust standard
library. ESP32-S3 is our current physical qualification platform; it does not
define the library's supported programming model.

Version **0.0.10** is available from Git while publication is being prepared.
`no_std` is supported; `std` and `alloc` are independent optional features.

## 1. Run a protected exchange

With Git and Rust installed:

```sh
git clone https://github.com/jeffglousher/coaptic.git
cd coaptic
cargo run --locked --example oscore_pair --features std
```

Expected output:

```text
Storage: fixed constrained profile (2 RX/TX slots, 1152 bytes each)
OSCORE: received all 200 bytes
```

The [complete example](examples/oscore_pair.rs) starts a client and server on
localhost, gives them a fresh secret from the OS, requests `/telemetry`, checks
every response byte, and exits. No device, account or manually entered key is
needed for this demonstration. Both peers live in this process; this does not
enroll a device or install persistent credentials.

## 2. Choose where and how it runs

The operating system and storage choice are separate:

- **[Constrained device](#constrained-device):** `no_std`, fixed Coaptic storage,
  and a platform-supplied transport. The example is independent of a board;
  our physical qualification uses a Waveshare ESP32-S3.
- **[Constrained Linux](#constrained-linux):** OS sockets with the same small,
  fixed Coaptic storage. Useful when a Linux process also needs a predictable
  memory budget.
- **[General-purpose Linux](#general-purpose-linux):** OS sockets and heap
  storage sized at startup. Useful when capacities come from configuration or
  larger pools are appropriate. Limits remain explicit.

All three use OSCORE. Choosing heap storage does not change the security mode.
The service handlers are ordinary Rust functions shared by the examples:

```rust
use coaptic::{Request, Response};

const TELEMETRY: [u8; 200] = [b'x'; 200];

fn telemetry(_: Request<'_>) -> Response<'static> {
    Response::content(&TELEMETRY)
}
```

The [shared service](examples/support/fixed_service.rs) also provides `/echo`:
up to 128 bytes succeed; larger requests receive `4.13 Request Entity Too Large`.
These small examples omit body pools. Larger block-wise messages need explicit
body storage; use `take_response_into` to collect complete replies into a
caller-owned buffer.

### Constrained device

In the device's Rust crate, keep the default features:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic" }
```

Coaptic needs neither `std` nor an allocator in this configuration. The board's
SDK, network stack and application have their own memory requirements.

Your platform integration supplies a bounded `DatagramIo` adapter, secure entropy, a
provisioned OSCORE context and a monotonic millisecond clock. The integration
looks like this; `io`, `context`, `secure_random` and `now_ms` come from those
platform services:

```rust
use coaptic::{App, get, profiles};

let mut service = App::profile::<profiles::Constrained>()
    .block_wise::<false>()
    .randomness(secure_random)
    .oscore(context)
    .route("telemetry", get(telemetry))
    .bind(io)?;
service.poll(now_ms)?;
```

The profile has two incoming and two outgoing datagram slots of 1152 bytes,
four deduplication entries and two Observe entries. These are storage capacities,
not a whole-device RAM or task-stack ceiling. The firmware owns the `App` and
calls `poll`; Coaptic supplies no background runtime.

Check the platform-independent service from this repository:

```sh
cargo check --locked --example fixed_service
cargo test --locked --example fixed_service
```

To use another microcontroller or SDK, provide those platform services and
choose capacities for its memory budget. The service handlers and Coaptic API
stay the same. A working integration still needs target-specific validation;
portable code and cross-compilation alone do not establish physical qualification.

#### Current hardware qualification: ESP32-S3

Our constrained-device test case runs on a Waveshare ESP32-S3. Its firmware uses the
[custom ESPHome integration](https://github.com/jeffglousher/coaptic-validation#custom-esphome-qualification)
in the companion repository. It supplies board startup, ESP-IDF/ESPHome bindings,
Wi-Fi and deployment tooling. Our reserved Waveshare test setup preserves
ESPHome API/OTA access and uses privately staged pairwise credentials. The Rust
library example alone is not a flashable firmware image.

### Constrained Linux

Enable `std` for OS networking while leaving Coaptic's `alloc` feature off:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic", features = ["std"] }
getrandom = "0.3"
```

Use a nonblocking UDP socket in place of the device platform's adapter. The
`fixed_service` module below is the [shared example module](examples/support/fixed_service.rs):

```rust
use std::net::UdpSocket;
use coaptic::storage::UdpSocketIo;

let socket = UdpSocket::bind("127.0.0.1:0")?;
socket.set_nonblocking(true)?;
let io = UdpSocketIo::new(socket, [0; 1153])?;
let mut service = fixed_service::server(io, context, secure_random)?;
service.poll(now_ms)?;
```

The extra scratch byte detects oversized UDP datagrams so a truncated prefix is
not accepted as a complete message. The example uses the same constrained pools
as the constrained-device service and a fixed 200-byte response buffer.

Run the complete client/server pair with the command from step 1:

```sh
cargo run --locked --example oscore_pair --features std
```

The Linux application and OS may allocate memory even though Coaptic uses fixed
storage. Enabling `std` does not enable `alloc`.

### General-purpose Linux

Enable both OS networking and Coaptic's optional allocator support:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic", features = ["std", "alloc"] }
getrandom = "0.3"
```

Choose capacities and call `bind_alloc` instead of `bind`. For example, this
startup configuration doubles the constrained example's datagram slots and
uses 1472-byte datagrams:

```rust
use std::net::UdpSocket;
use coaptic::{App, get, profiles, storage::Capacities};
use coaptic::storage::UdpSocketIo;

let socket = UdpSocket::bind("127.0.0.1:0")?;
socket.set_nonblocking(true)?;
let io = UdpSocketIo::new(socket, [0; 1473])?;
let capacities = Capacities::from_profile::<profiles::Default>();
let mut service = App::builder()
    .randomness(secure_random)
    .oscore(context)
    .route("telemetry", get(telemetry))
    .bind_alloc(io, capacities)?;
service.poll(now_ms)?;
```

The [complete heap-storage example](examples/oscore_pair.rs) spells out its
capacities: four RX and four TX slots of 1472 bytes, eight deduplication entries,
four Observe entries and no body pools. It keeps socket scratch and the complete
response buffer in fixed arrays. Run it explicitly:

```sh
cargo run --locked --example oscore_pair --features std,alloc -- --alloc
```

Expected output:

```text
Storage: bounded heap pools (4 RX/TX slots, 1472 bytes each)
OSCORE: received all 200 bytes
```

Pools are allocated once at binding and do not grow during polling. Larger pools
do not automatically raise the App's call, route or security-association limits;
independent capacity configuration is tracked in
[#340](https://github.com/jeffglousher/coaptic/issues/340).

Cargo features are additive. Another dependency can enable `coaptic/alloc`, but
`bind` still selects fixed storage. This example also requires `--alloc` to choose
heap storage; requesting it without the Cargo feature is an error.

## 3. Carry the example into a real deployment

The localhost example creates fresh keys on every run. A persistent device or
service needs more setup:

1. Provision unique credentials through a trusted process. Optional EDHOC support
   provides an authenticated key exchange; applications still own enrollment,
   ownership, permissions and key custody.
2. Recover durable sender-sequence reservations and replay checkpoints **before**
   reusing OSCORE keys after a restart. Do not reset a retained key's counters.
3. Supply the real transport and monotonic clock, choose storage/work limits,
   and drive `poll` from your task or event loop.
4. If a request changes durable application state, commit the effect and its
   application receipt at the declared storage boundary. A CoAP acknowledgement
   alone does not mean that the application effect was committed.

See [SECURITY.md](SECURITY.md) for the security boundary and
[#365](https://github.com/jeffglousher/coaptic/issues/365) for the remaining
enrollment and durable device-to-host workflow.

## What does “plaintext” mean?

Plaintext is **unprotected CoAP**, not a text-only payload format. It can carry
binary data, but does not provide OSCORE encryption, authenticated messages or
replay protection. A party able to observe or alter the network can read the
payload or forge traffic. OSCORE protects application messages; some routing
metadata remains visible even with OSCORE.

The plaintext option exists for explicit compatibility and unprotected test
setups. Ordinary App construction requires an OSCORE context. Neither `std`,
`alloc`, nor disabling Cargo defaults silently opts into plaintext.

To try the optional unprotected localhost examples, use two terminals:

```sh
# Terminal 1: leave the server running.
cargo run --locked --example server --features std
```

```sh
# Terminal 2: the client prints 21.5.
cargo run --locked --example client --features std
```

Both select `.allow_plaintext()` in source. The protected `oscore_pair` example
does not accept `--plaintext`, and a configured protected App does not fall back
to plaintext when authentication fails.

## Documentation and project status

Build the API reference and larger-payload examples:

```sh
cargo doc --all-features --open
```

The reserved Waveshare ESP32-S3 has passed finite protected LAN checks, complete
2000-byte replies, refusal cases and a replay-first software-restart check. This
evidence belongs to its recorded firmware and source revisions:
[device methods and limitations](https://github.com/jeffglousher/coaptic/issues/333#issuecomment-6092855382).
Flash power-loss recovery, hostile rollback, production key custody and the full
end-user flow remain open. Production readiness has not been established.

- [Ordered roadmap and to-do list](https://github.com/jeffglousher/coaptic/issues/130)
- [Project board](https://github.com/users/jeffglousher/projects/2)
- [Qualification tooling](https://github.com/jeffglousher/coaptic-validation)
- [Benchmark methods and preliminary limits](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/benchmark/README.md)

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
