# coaptic

A bounded `no_std` CoAP engine with an approachable `App` face. Slots and tables sit under the hood; there is no global App State.

Start at rustdoc (`cargo doc --open`). Crate-root types are the happy path (`App`, `Request`, `Response`, `Call`, `Outgoing`, method routers, `Endpoint`, `profiles`, `Code`, `ContentFormat`, `Method`, `ProblemDetails`, bind/poll/send errors). Engine slots, tables, and `DatagramIo` live in `storage`; codecs and tokens live in `message`.

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

fn get_temp(_req: Request<'_>) -> Response {
    Response::content(b"21.5")
}

let mut app = App::profile::<profiles::Default>()
    .block_wise(true)
    .route("sensors/temp", get(get_temp))
    .bind(io)?;
app.poll(now_ms)?;

let call = app.get("sensors/temp").to(peer).send(now_ms)?;
app.poll(now_ms)?;
let response = app.take_response(call);
```

`.route` / `app.get` also accept `&["sensors", "temp"]`. Handlers are `fn(Request<'_>) -> Response`. You own the socket (`storage::DatagramIo`), the clock, the destination of a client request, and any domain data that outlives a request. Tokens and Message IDs are App counters (no OS RNG).

Observe subscribe is `app.get(path).observe().to(peer).send(now)`; the initial representation and later notifications use the same `Call` / `take_response`. `deregister()` sends Observe=1. Structured 4.xx bodies use `Response::problem` (RFC 9290). Q-Block1 holes from `poll` use `Response::missing_blocks` (RFC 9177, Content-Format 272).

Engine BERT, OSCORE, and first-party DTLS are future / backlog. DTLS is harness `DatagramIo` only (`coaptic-plugtest --features dtls`). 6LoWPAN is not planned.

## Features

Default is `no_std` with no allocator. Optional `alloc` and `std` (`std` implies `alloc`). Zero crate dependencies.

## Plugtest harness

Classic in-crate loopback: `cargo test --test plugtest`. Multi-implementation UDP + pcap grading (coap-rs peer, golden JSON, optional DTLS) lives in a workspace crate so the library stays zero-dep:

```bash
cargo test -p coaptic-plugtest
cargo test -p coaptic-plugtest --features dtls
```

Tracking: issue [#49](https://github.com/jeffglousher/coaptic/issues/49).

## Example

```bash
cargo run --example coap_server --features std
```

## Docs and planning

API: rustdoc on [docs.rs/coaptic](https://docs.rs/coaptic) (or `cargo doc --open`). Protocol copies: [`knowledge/rfcs/`](knowledge/rfcs/). Architecture planning lives on [GitHub Issues / project](https://github.com/users/jeffglousher/projects/2).

Licensed MIT OR Apache-2.0.
