# coaptic

CoAP from microcontrollers to cloud services. Protected by default. No heap required.

- Rust. `no_std`. No allocation by default; optional `alloc` and `std`.
- OSCORE message protection; optional authenticated EDHOC provisioning.
- Retransmission, Observe subscriptions, and block-wise transfers.
- One client/server API. No background runtime.

An alternative to HTTP for telemetry and frequent resource exchanges.

## Get started

Until publication, install from Git:

```toml
[dependencies]
coaptic = { git = "https://github.com/jeffglousher/coaptic", features = ["std"] }
getrandom = "0.3"
```

These host examples explicitly allow plaintext on localhost.

### Server

```rust
let mut server = App::builder()
    .randomness(|bytes| getrandom::fill(bytes).is_ok())
    .route("sensors/temp", get(temperature))
    .route("echo", post(echo))
    .allow_plaintext()
    .bind(socket)?;
```

<details>
<summary>Complete server ? save as examples/server.rs</summary>

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

</details>

Run `cargo run --example server`.

### Client

```rust
let call = client.get("sensors/temp").to(peer).send(now_ms)?;
```

Drive exchanges with `poll`; collect replies with `take_response`.

<details>
<summary>Complete client ? save as examples/client.rs</summary>

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

</details>

Run `cargo run --example client` in another terminal. It prints `21.5`.

### Messages

Replace the GET with a POST to echo a short payload:

```rust
let call = client.post("echo").payload(b"hello").to(peer).send(now_ms)?;
```

## Security

Use `.oscore(context)` on each peer. EDHOC can provision authenticated peers.
Applications own credentials, secure entropy, and durable replay state.
[Security boundary](SECURITY.md).

API reference and larger-payload examples: `cargo doc --open`.

## Preliminary evidence

ESP32-S3 tests cover plaintext Wi-Fi UDP and separate OSCORE loopback.
Network OSCORE and flash power-loss recovery remain unqualified.
[Setup, results, and limitations](https://github.com/jeffglousher/coaptic/issues/333) ?
[Test driver](https://github.com/jeffglousher/coaptic/blob/f618540597f4062655761e122c8c24d062f13a86/tools/qualification/network_peer.py) ?
[Benchmark method](https://github.com/jeffglousher/coaptic/blob/main/tools/benchmark/README.md).

Production readiness has not been established. Early users and contributors welcome.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
