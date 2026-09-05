# What the caller must own

coaptic is a library, not a daemon or socket stack. The core never sends on the wire and has no OS clock or RNG. Integration owns the items below. Protocol rules stay in [`knowledge/rfcs/`](knowledge/rfcs/); this file does not restate them.

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

- Resource selection, semantics, and payloads are application-owned ([`design.md`](design.md) §Application memory access).
- `ParsedMessage::precondition(exists, etag)` classifies If-Match / If-None-Match. You decide 2.xx vs 4.12 (or RST).
- FETCH / PATCH / iPATCH are named codes only — no method policy.

## Encode + Access loops

Typical cycle:

1. Recv into an RX slot (`Engine::recv_from` + your [`DatagramIo`](src/storage/io.rs)). `write_rx` remains for tests that already have bytes.
2. Call `Engine::progress(now_ms)`.
3. Handle `Progress` (decode `rx_ready`, `send_tx` retransmit, encode notify / recover).
4. Encode outgoing messages with `encode_tx` or `AccessMut` into a TX or body slot.
5. `Engine::send_tx` the filled datagram (Access pin lives only for that call). Then `release` if the slot is no longer pending.
6. Repeat. Progress skips pinned slots and does not drain the pools.

`decode` / `encode` also work on plain buffers with no `Engine`.
