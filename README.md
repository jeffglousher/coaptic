# coaptic

A bounded `no_std` CoAP engine with an approachable `App` face. Slots and tables sit under the hood; there is no global App State.

## Sketch

```text
RX slot (+ body) --view--> Request
handler(Request) -> Response
Response --encode--> TX slot (+ body)
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
```

Handlers are `fn(Request<'_>) -> Response`. `App::poll` recv / progress / route / send / release. You own the socket, the clock, and any domain data that outlives a request.

## Features

Default is `no_std` with no allocator. Optional `alloc` and `std` (`std` implies `alloc`). Zero crate dependencies.

## Example

```bash
cargo run --example coap_server --features std
```

## Docs and planning

API: rustdoc on [docs.rs/coaptic](https://docs.rs/coaptic) (or `cargo doc --open`). Protocol copies: [`knowledge/rfcs/`](knowledge/rfcs/). Architecture planning lives on [GitHub Issues / project](https://github.com/users/jeffglousher/projects/2).

Licensed MIT OR Apache-2.0.
