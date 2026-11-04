# Coaptic with Embassy networking

This unpublished `no_std` adapter adopts an already bound `embassy_net::udp::UdpSocket`
and supplies an `AsyncTimer` using the board's Embassy time driver. It creates no
executor, task, thread, heap allocation or extra packet scratch. Embassy's own
socket metadata/buffers, stack resources, driver buffers and tasks remain separate
caller-owned costs. App pool capacities do not change.

```rust
let transport = coaptic_embassy::Udp::new(bound_socket);
let clock = coaptic_embassy::Clock;
// Bind transport to the App; use clock.now_ms() for every App timestamp.
// The board must run embassy_net::Runner concurrently and provide a time driver.
```

`socket()` exposes configuration/inspection; `into_inner()` returns the original
socket. Leave receives to the App while it owns the adapter. A socket belongs to
one stack/interface. IPv6 peers use scope zero within that namespace; nonzero
outbound zone IDs are refused instead of silently discarded. Native send success
means the entire packet entered Embassy's bounded TX queue, not radio delivery.
Pending sends accept no packet. Pending receives retain no application buffer
access; oversized input is consumed and returned as `RecvError::Truncated`.
The caller must disregard receive-buffer contents after an error.

Clock conversion rounds deadlines upward to ticks, never early. Unrepresentable
future deadlines remain pending; unrepresentable current milliseconds return an
error. The target selects its time-driver tick frequency and critical-section
implementation. Host-only test features are not enabled for bare-metal targets.

The [target entry example](examples/target.rs) is a board-independent, unprotected
SmallDevice server function accepting a socket and secure randomness source. It
returns a typed failure cause; the board owns restart/shutdown policy. It is a
crosscompiled integration seam, **not a linked board firmware or physical ESP32
qualification**. A board still supplies HAL, network device, runner, clock driver,
executor/task storage and deployment security configuration.

Validated on Windows x86-64 with Rust 1.99.1, embassy-net 0.9.1 and embassy-time
0.5.1: two tests exercise real smoltcp IP/UDP ingestion, truncation/recovery,
pending receive cancellation, bounded send queue cancellation/recovery, exact
emitted payloads, endpoint conversion and deadline rounding. The test link is
an in-memory IP driver with synthetic packets, not Wi-Fi or OS sockets. Host
Clippy is warning-clean. The library and example compile for Cortex-M4
`thumbv7em-none-eabi`; no full-App exchange or performance measurement is claimed.

```text
cargo test --locked --manifest-path tools/coaptic-embassy/Cargo.toml
cargo clippy --locked --manifest-path tools/coaptic-embassy/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path tools/coaptic-embassy/Cargo.toml --target thumbv7em-none-eabi --lib --example target --features embassy-time/tick-hz-1_000
```

Use the registered build lane described in the project development guidance.
The existing ESP-IDF driver remains unchanged; this adapter does not migrate a
device or select a Wi-Fi stack.
