# coaptic

CoAP over UDP for devices and host services, written in Rust, with explicit
memory and work bounds for reliable operation.
Serve resources, send requests, subscribe with Observe and transfer larger
bodies through one client/server API. OSCORE message protection and fixed storage
are the defaults. The core supports `no_std`; `std` and `alloc` are independent.

Coaptic is pre-release and available from Git. Current development prioritizes
completing UDP/OSCORE capabilities, EDHOC, durable provisioning and device/host
integration. APIs may change directly during development.

## Run a protected exchange

```sh
git clone https://github.com/jeffglousher/coaptic.git
cd coaptic
cargo run --locked --example oscore_pair --features std
```

The [example](examples/oscore_pair.rs) creates fresh local credentials, starts a
client and server, and verifies all 200 response bytes:

```text
Storage: fixed constrained profile (2 RX/TX slots, 1152 bytes each)
OSCORE: received all 200 bytes
```

To use bounded heap storage instead:

```sh
cargo run --locked --example oscore_pair --features std,alloc -- --alloc
```

Both runs protect messages with OSCORE. The demonstration uses process-local keys;
real devices need persistent identity and authenticated pairing.

## Integrate

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic" }
```

Add `features = ["std"]` for OS sockets or `["std", "alloc"]` for host networking
with optional heap-backed pools. Enabling a feature does not select heap storage
or disable security.

Applications supply a `DatagramIo` transport, secure entropy, a provisioned OSCORE
context and a monotonic millisecond clock. They own the polling loop:

```rust
use coaptic::{App, Request, Response, get, resources::SmallDevice};

fn telemetry(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

// io, context and secure_random come from the platform/provisioning layer.
let mut service = App::with_resources(SmallDevice::new())
    .oscore(context)
    .route("telemetry", get(telemetry))
    .bind_with_entropy(io, secure_random)?;
service.poll(now_ms)?;
```

The [shared service](examples/support/fixed_service.rs) provides telemetry and echo
handlers. The same service can use a microcontroller adapter or a nonblocking
host UDP socket. `platform::CheckedClock` checks fallible timer reads and refuses
backward time. Use one clock epoch for all timed App operations.

Start device integration with `SmallDevice`: constrained packet pools, four
routes and one client Call. Customize capacities before binding; choose `Device`,
`Gateway` or `Service` when the workload needs their larger starting budgets.
Use `.allocated(limit)` to allocate bounded heap pools once at startup and reuse
them for requests. Capacities stay fixed after binding; enabling `alloc` never
enables automatic pool expansion.
The heap limit covers requested pool allocations, not whole-process memory.
Collect complete replies with `take_response_into` and a caller-owned buffer.
See rustdoc for resource capacities, response lifetimes, Observe and block-wise
contracts rather than inferring them from a preset's name.

The companion [native async host check](https://github.com/jeffglousher/coaptic-validation#native-async-host-check)
uses the same startup allocation policy with Tokio UDP in unprotected and OSCORE modes.

These are configurations of the same App. Blockwise transfers split larger
messages across packets and reserve bounded body storage when enabled. A report
should say, for example, "SmallDevice, OSCORE, blockwise enabled" and name any
capacity overrides. "Constrained" means predictable resource use, not missing
capabilities. Pool capacity and App size are only parts of the deployment budget;
account for stack, scratch and platform transport storage as well.

Our physical target is the Waveshare ESP32-S3, using the companion
[ESPHome integration](https://github.com/jeffglousher/coaptic-validation#custom-esphome-qualification).
It supplies board startup, Wi-Fi and deployment. Other targets need their own
platform adapters and qualification. The companion also provides a
[Linux container recipe](https://github.com/jeffglousher/coaptic-validation#protected-linux-container).

## Provision and recover

Use the guided Rust [commissioning CLI](tools/coaptic-enroll/README.md) on Windows,
Linux or macOS for inspection, identity setup, pairing and recovery. Its default
PIV provider supports touch-required and explicit touchless development credentials.
The tool documents native smart-card and credential-store prerequisites.
Choose read-only, echo-only or durable-telemetry access. Inspection reports
pending-work custody; drain telemetry before changing its grant so the exact
outstanding effect can finish.

Provision a unique device identity and authenticate the peer/owner binding.
The optional `edhoc` feature establishes fresh OSCORE sessions from installed
trust. Normal reconnects restore durable identity and permissions, then establish
fresh session keys; they do not need another operator approval.

If retaining an OSCORE key across restart instead, recover durable sender-sequence
reservations and replay checkpoints before using it. Durable application effects
also need an application receipt; a CoAP acknowledgement alone is not a commit.
See [SECURITY.md](SECURITY.md) for the boundary and the
[qualification plan](architecture/requirements/VALIDATION-PLAN.md) for remaining
independent and installed-device proof.

## Unprotected mode

Unprotected CoAP carries binary or textual payloads without message encryption,
authentication or replay protection. Select `.allow_unprotected()` explicitly for
compatibility or tests.
An attached OSCORE context still enforces protection; authentication failure never
selects unprotected mode. Disabling Cargo defaults does not select it either.

Run the optional localhost examples in separate terminals:

```sh
cargo run --locked --example server --features std
cargo run --locked --example client --features std
```

## Documentation and development

```sh
cargo doc --all-features --open
```

- [Development and validation](DEVELOPMENT.md)
- [Accepted protocol scope](architecture/IETF-RFC-PLAN.md)
- [Delivery and remaining qualification](https://github.com/jeffglousher/coaptic/issues/130)
- [Physical qualification and source-bound evidence](https://github.com/jeffglousher/coaptic/issues/333)
- [Independent peers and benchmarks](https://github.com/jeffglousher/coaptic-validation)

Current hardware reports cover finite named workloads, not production readiness.
DTLS, TCP/TLS/WebSockets and BERT endpoints are contribution opportunities outside
active MVP development.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
