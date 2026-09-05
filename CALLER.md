# What the caller must own

coaptic is a library, not a daemon or socket stack. The core never sends on the wire and has no OS clock or RNG. Integration owns the items below. Protocol rules stay in [`knowledge/rfcs/`](knowledge/rfcs/); this file does not restate them.

## Two paths

**App (happy path).** [`App`](src/app/mod.rs) is a routing façade: recv / progress / route / handler / reply / send / release. Handlers are `fn(Request<'_>) -> Reply`. You own the socket (passed into [`App::profile`](src/app/mod.rs)`.block_wise(…).route(…).bind(io)`) and the clock (`now_ms` into [`App::poll`](src/app/mod.rs)). You do not touch slots to expose a GET or PUT. `App` does **not** own a global mutable shared bag; domain data that outlives a request (GPIO, sensor firmware, …) stays outside coaptic ([`design.md`](design.md) §Application memory access).

**Engine (advanced / reactor).** Per-slot state machines plus [`progress`](src/storage/progress.rs) ship protocol mechanics (pending CON/RTO, BlockTransfer, ObserveInterest, Dedup, Exchange). Slots, [`Access`](src/storage/access.rs), Observe notify, Block / Q-Block, and custom RST / 4.xx policy stay on [`Engine`](src/storage/engine.rs). Use this when the façade is not enough. [`DatagramIo`](src/storage/io.rs) is the bind for both paths.

## Send

- Own the socket or radio. Bind it with [`DatagramIo`](src/storage/io.rs) (`std::net::UdpSocket` implements it under `std`). The core never sends.
- Recv into an RX slot with [`Engine::recv_from`](src/storage/io.rs). Send an occupied TX slot with [`Engine::send_tx`](src/storage/io.rs) (pins `Access` for the call). [`Engine::progress`](src/storage/progress.rs) only reports work.
- On `Progress::retransmit`: `send_tx` the occupied datagram (`Due`) or release after `GiveUp` (pending is already cleared; the TX slot stays occupied so you can free it).
- On `Progress::observe_notify`: encode a notification (ordinary TX or first-block Block2) and `send_tx`. The Observe table does not queue bodies.
- On `Progress::qblock_recover`: encode a Q-Block2 recover, or own any Q-Block1 4.08.
- Honor `NoResponse` yourself (`NoResponse::suppresses`). The library does not skip sends.

## Clock

- Pass `now_ms` into `Engine::progress`, `record_pending_con`, Observe lifetime (`refresh_observe_max_age` / `record_observe_notify`), and Echo freshness.
- Use one monotonic millisecond domain for those calls.

## Jitter and entropy

- Pass `jitter_ms` when recording a pending CON (`0` is the ACK_TIMEOUT floor; the core clamps to the ACK_RANDOM_FACTOR span).
- Mint Tokens (`Token::mint` / `message::TokenSource`) and time-based Echo pads with caller entropy. The core does not call an OS RNG.

## When to RST or 4.xx

Constructors and named codes exist (`empty_ack` / `empty_rst`, 2.31 / 4.01 / 4.02 / 4.08 / 4.09 / 4.12 / 4.22 / 5.08). The library classifies; it does not send.

- Unrecognized critical / wrong format: `check_rfc7252_options` / `check_rfc7252_formats` — choose RST vs 4.02 vs ignore.
- Echo: `EchoFreshness` / `Engine::echo_freshness` — you own 4.01 / RST.
- Hop-Limit: decrement helper only — you own 4.00 / 5.08 / forwarding.
- Q-Block1 holes / incomplete: you own 4.08. 2.31 Continue is a code only.
- Observe expiry: the row stays occupied; you drop it or stop notifying. No RST / 4.02 from the core.

## Resource and If-Match

- **App:** [`AppBuilder::route`](src/app/mod.rs) binds a [`MethodRouter`](src/app/routing.rs) (`get` / `put` / `post` / `delete` / `fetch`, plus `patch` / `ipatch`) on Uri-Path segments. Handlers are `fn(Request<'_>) -> Reply` (or a type that becomes [`Reply`](src/app/reply.rs) via [`IntoReply`](src/app/reply.rs)). Unknown path is 4.04; path exists but the method is unbound is 4.05. Do not put a shared mutable `Sensors { led_on }` bag in `App`.
- **Engine:** resource selection is still application-owned ([`design.md`](design.md) §Application memory access).
- `ParsedMessage::precondition(exists, etag)` classifies If-Match / If-None-Match. You decide 2.xx vs 4.12 (or RST). `Reply::precondition_failed` is the 4.12 builder.
- FETCH / PATCH / iPATCH are named codes; the router binds them if you register `.fetch` / `.patch` / `.ipatch`.

## Encode + Access loops

**App:** [`App::poll`](src/app/mod.rs) is the loop. Retransmit send / give-up release, request dispatch, piggybacked ACK, and slot release are inside. Observe notify and Q-Block recover are not sent on this path yet (Phase 2); use `App::engine_mut` if you need them now.

**Engine** (advanced):

1. Recv into an RX slot (`Engine::recv_from` + your [`DatagramIo`](src/storage/io.rs)). `write_rx` remains for tests that already have bytes.
2. Call `Engine::progress(now_ms)`.
3. Handle `Progress` (decode `rx_ready`, `send_tx` retransmit, encode notify / recover).
4. Encode outgoing messages with `encode_tx` or `AccessMut` into a TX or body slot.
5. `Engine::send_tx` the filled datagram (Access pin lives only for that call). Then `release` if the slot is no longer pending.
6. Repeat. Progress skips pinned slots and does not drain the pools.

`decode` / `encode` also work on plain buffers with no `Engine`.
