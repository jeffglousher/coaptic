# coaptic

A bounded `no_std` CoAP engine with an approachable `App` face. Slots and tables sit under the hood; there is no global App State.

## Sketch

```text
RX slot (+ body) --view--> Request
handler(Request) -> Response
Response --encode--> TX slot (+ body)

Outgoing (get/put) --encode--> TX slot
poll matches Token + endpoint
RX --copy--> Reply
```

```rust
use coaptic::{App, Request, Response, get, profiles};

fn get_temp(_req: Request<'_>) -> Response {
    Response::content(b"21.5")
}

let mut app = App::profile::<profiles::Default>()
    .route(&["sensors", "temp"], get(get_temp))
    .bind(io)?;
app.poll(now_ms)?;

let call = app.get(&["sensors", "temp"]).to(peer).send(now_ms)?;
app.poll(now_ms)?;
let reply = app.take_reply(call);
```

Handlers are `fn(Request<'_>) -> Response`. Structured 4.xx bodies use `Response::problem` (RFC 9290 CBOR). `App::poll` recv / progress / route / send / release, and matches outbound exchanges. You own the socket, the clock, the destination of a client request, and any domain data that outlives a request. Tokens and Message IDs are App counters (no OS RNG).

## Features

Default is `no_std` with no allocator. Optional `alloc` and `std` (`std` implies `alloc`). Zero crate dependencies.

## Example

```bash
cargo run --example coap_server --features std
```

## Docs and planning

API: rustdoc on [docs.rs/coaptic](https://docs.rs/coaptic) (or `cargo doc --open`). Protocol copies: [`knowledge/rfcs/`](knowledge/rfcs/). Architecture planning lives on [GitHub Issues / project](https://github.com/users/jeffglousher/projects/2).

Licensed MIT OR Apache-2.0.
