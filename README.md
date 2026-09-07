# coaptic

[![crates.io](https://img.shields.io/crates/v/coaptic.svg)](https://crates.io/crates/coaptic)
[![docs.rs](https://docs.rs/coaptic/badge.svg)](https://docs.rs/coaptic)
[![CI](https://github.com/jeffglousher/coaptic/actions/workflows/ci.yml/badge.svg)](https://github.com/jeffglousher/coaptic/actions/workflows/ci.yml)

A bounded `no_std` CoAP engine with an approachable `App` face. Slots and tables sit under the hood; there is no global App State.

```toml
[dependencies]
coaptic = "0.1"
```

Start at rustdoc ([docs.rs](https://docs.rs/coaptic), or `cargo doc --open`). Crate-root types are the happy path (`App`, `Request`, `Response`, `Call`, `Outgoing`, method routers, `Endpoint`, `profiles`, `Code`, `ContentFormat`, `Method`, `ProblemDetails`, bind/poll/send errors). Engine slots, tables, and `DatagramIo` live in `storage`; codecs and tokens live in `message`.

## Sketch

```text
RX slot (+ body) --view--> Request
handler(Request) -> Response
Response --encode--> TX slot (+ body)

Outgoing (get/put) --encode--> TX slot
poll matches Token + endpoint
RX --copy--> Response
```

```rust
use coaptic::{App, Request, Response, get, profiles};

fn get_temp(_req: Request<'_>) -> Response<'static> {
    Response::content(b"21.5")
}

let mut app = App::profile::<profiles::Default>()
    .block_wise::<true>()
    .route("sensors/temp", get(get_temp))
    .bind(io)?;
app.poll(now_ms)?;

let call = app.get("sensors/temp").to(peer).send(now_ms)?;
app.poll(now_ms)?;
let response = app.take_response(call);
```

`.route` / `app.get` also accept `&["sensors", "temp"]`. Handlers are `fn(Request<'_>) -> Response`. You own the socket (`storage::DatagramIo`), the clock, the destination of a client request, and any domain data that outlives a request. Tokens and Message IDs are App counters (no OS RNG).

Observe subscribe is `app.get(path).observe().to(peer).send(now)`; the initial representation and later notifications use the same `Call` / `take_response`. `deregister()` sends Observe=1. Structured 4.xx bodies use `Response::problem` (RFC 9290). Q-Block1 holes from `poll` use `Response::missing_blocks` (RFC 9177, Content-Format 272).

Engine BERT, OSCORE, first-party DTLS, and alternative networks (6LoWPAN, LoRaWAN, …) are future / backlog — contributor opportunities ([#46](https://github.com/jeffglousher/coaptic/issues/46)). DTLS is harness `DatagramIo` only (`coaptic-plugtest --features dtls`).

## Features

Default is `no_std` with no allocator. Optional `alloc` and `std` (`std` implies `alloc`). Zero crate dependencies.

## Plugtest harness

In-crate `cargo test --test plugtest` is an Engine↔Engine byte exchange (no sockets; not an App SUT). App as SUT — UDP server and client, coap-rs peer, golden JSON, optional DTLS — lives in `coaptic-plugtest` so the library stays zero-dep:

```bash
cargo test -p coaptic-plugtest
cargo test -p coaptic-plugtest --features dtls
cargo run -p coaptic-plugtest --bin dogfood
```

The `dogfood` bin is timed coaptic ↔ coap-rs over loopback UDP (both directions, GET/PUT/POST, Observe register, block-wise). It prints timing plus `Engine::metrics`. CI smokes `--iterations 2`. Tracking: issue [#49](https://github.com/jeffglousher/coaptic/issues/49) (plugtest) / [#125](https://github.com/jeffglousher/coaptic/issues/125) (dogfood + metrics).

## Example

```bash
cargo run --example coap_server --features std
```

## Docs and planning

API: [docs.rs/coaptic](https://docs.rs/coaptic) (`cargo doc --open`; version tags also publish rustdoc to GitHub Pages). Protocol copies: [`knowledge/rfcs/`](knowledge/rfcs/). Architecture planning lives on [GitHub Issues / project](https://github.com/users/jeffglousher/projects/2).

The crate is public. Pull requests to `main` must pass `fmt`, `clippy`, `test`, `doc`, and `package`. Merges are squash-only. See [CONTRIBUTING.md](CONTRIBUTING.md).

Licensed MIT OR Apache-2.0.
