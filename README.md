# coaptic

Free, open-source CoAP for microcontrollers and cloud services.

- Rust. `no_std`. No allocation by default; optional `alloc` and `std`.
- OSCORE message protection by default; optional authenticated EDHOC provisioning.
- Retransmission, Observe subscriptions, and block-wise transfers.
- One client/server API. No background runtime.

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
<summary>Complete server (examples/server.rs)</summary>

```rust
use std::{net::UdpSocket, thread, time::{Duration, Instant}};
use coaptic::{App, Code, Request, Response, get, post};

fn temperature(_: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

fn echo(request: Request<'_>) -> Response<'static> {
    Response::try_content_copy(request.payload())
        .unwrap_or_else(|_| Response::new(Code::REQUEST_ENTITY_TOO_LARGE))
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

Run `cargo run --example server --features std`.

### Client

```rust
let call = client.get("sensors/temp").to(peer).send(now_ms)?;
```

Drive exchanges with `poll`; collect complete replies into your buffer.

<details>
<summary>Complete client (examples/client.rs)</summary>

```rust
use std::{net::UdpSocket, thread, time::{Duration, Instant}};
use coaptic::{App, Endpoint};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_plaintext()
        .bind(socket)?;

    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let clock = Instant::now();
    let call = client.get("sensors/temp").to(peer).send(0)?;
    let mut body = [0; 1472];
    loop {
        client.poll(clock.elapsed().as_millis() as u64)?;
        if let Some(reply) = client.take_response_into(call, &mut body)? {
            let reply = reply?;
            if !reply.code().is_success() {
                return Err(format!("peer replied {}", reply.code()).into());
            }
            println!("{}", String::from_utf8_lossy(reply.payload()));
            return Ok(());
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err("request timed out".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
```

</details>

Run `cargo run --example client --features std` in another terminal. It prints `21.5`.

### Messages

Replace the GET with a POST to echo a payload of up to 128 bytes:

```rust
let call = client.post("echo").payload(b"hello").to(peer).send(now_ms)?;
```

## Security

Use `.oscore(context)` on each peer. EDHOC can provision authenticated peers.
Applications own credentials, secure entropy, and durable replay state.
[Security boundary](SECURITY.md).

API reference and larger-payload examples: `cargo doc --open`.

## Demonstrations

Run the resource and echo examples above, or a protected UDP exchange:

```sh
cargo run --example oscore_pair --features std
```

ESP32-S3 tests cover plaintext Wi-Fi UDP and separate OSCORE loopback.
Network OSCORE and flash power-loss recovery remain unqualified.
[Setup, results, and limitations](https://github.com/jeffglousher/coaptic/issues/333) |
[Test driver](https://github.com/jeffglousher/coaptic/blob/f618540597f4062655761e122c8c24d062f13a86/tools/qualification/network_peer.py) |
[Benchmark method](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/benchmark/README.md).

Measurements remain preliminary. Production readiness has not been established.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Vendored EDHOC code: [BSD-3-Clause](src/provisioning/lakers/LICENSE-BSD).
