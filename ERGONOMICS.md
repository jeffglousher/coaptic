# 0.1 feature inventory and last-mile bind

Code-confirmed against `main` `be8438a` plus this PR’s [`App`](src/app/mod.rs). Status is **Present** / **Partial** / **Absent**. Citation is `path::item`. This file does not restate RFC wire format.

Happy path: `examples/coap_server.rs` (`cargo run --example coap_server --features std`). Advanced: Engine + [`DatagramIo`](src/storage/io.rs).

## Feature confirmation

| Capability | Status | Citation |
| --- | --- | --- |
| Message decode / encode | Present | `src/message/decode.rs::decode`, `src/message/encode.rs::encode` |
| Empty ACK / RST | Present | `src/message/mod.rs::empty_ack`, `empty_rst` |
| RFC 7252 Table 4 option numbers + `Opt` helpers | Present | `src/message/option.rs::OptionNumber::is_rfc7252` (15 numbers); `src/message/value.rs::Opt` (`if_match` … `size1`) |
| Observe option + register/deregister | Present | `src/message/value.rs::Opt::observe`, `ParsedMessage::is_observe_register` |
| Block / Q-Block / Size2 codecs; BERT SZX 7 | Present | `src/message/value.rs::BlockValue`, `BlockValue::SZX_BERT`, `Opt::block1` / `q_block1` |
| Request-Tag / ETag `BodyTag` | Present | `src/message/value.rs::Opt::request_tag`; `src/storage/block.rs::BodyTag` |
| Echo option + time freshness | Present | `src/message/echo.rs::Echo`, `EchoFreshness`; `src/storage/engine.rs::Engine::echo_freshness` |
| Hop-Limit value + decrement | Present | `src/message/hop.rs::HopLimit`; forwarding / 4.00 / 5.08 policy **Absent** |
| No-Response bitmap + `suppresses` | Present | `src/message/no_response.rs::NoResponse::suppresses`; send-skip policy **Absent** |
| If-Match / If-None-Match | Present | `src/message/precondition.rs::Precondition`; 4.12 policy **Absent** |
| FETCH / PATCH / iPATCH | Present (codes only) | `src/message/mod.rs::Code::FETCH`, `PATCH`, `IPATCH`; method policy **Absent** |
| Named 2.31 / 4.08 / 4.09 / 4.22 / 5.08 | Present (codes only) | `src/message/mod.rs::Code::CONTINUE` and siblings |
| **App façade** | Present | `src/app/mod.rs::App`; `App::profile` / `block_wise` / `route` / `bind`; `get` / `put` method routers; `fn(Request<'_>) -> Reply`; `Reply`; `App::poll`; `well_known_core` |
| Engine / Memory / builder | Present | `src/storage/engine.rs::Engine`; `src/storage/memory.rs::Memory`; `src/storage/builder.rs::EngineBuilder` |
| `Endpoint` sidecar | Present | `src/storage/endpoint.rs::Endpoint` |
| **Transport bind** | Present | `src/storage/io.rs::DatagramIo`; `Engine::recv_from`, `Engine::send_tx`; `std::net::UdpSocket` impl under `std` |
| Dedup | Present | `src/storage/table.rs::DedupEntry`; `src/storage/engine.rs::Engine::insert_dedup` |
| Pending CON / RTO | Present | `src/storage/pending.rs::PendingCon`, `PendingRto`; `src/storage/engine.rs::Engine::record_pending_con` |
| Exchange (token match + Echo sidecar) | Present | `src/storage/exchange.rs::ExchangeEntry`; `src/storage/engine.rs::Engine::record_request` |
| ObserveInterest notify | Present | `src/storage/table.rs::ObserveInterest::mark_due`; `src/storage/engine.rs::Engine::signal_observe`; `src/storage/progress.rs::Progress::observe_notify` |
| Observe Max-Age / client-OFF | Present | `src/storage/table.rs::ObserveLifetime`; `src/storage/engine.rs::Engine::refresh_observe_max_age`; `Progress::observe_expired` |
| RFC 7641 §4.5 pacing | Present | `src/storage/table.rs::ObserveInterest::must_confirm`, `record_notify`; `src/storage/engine.rs::Engine::record_observe_notify` |
| Block / Q-Block body transfers | Present | `src/storage/engine.rs::Engine::apply_block1`, `apply_q_block1`, `start_block2`, `next_bert1` |
| First-block Observe on Block2 | Present | `src/storage/engine.rs::Engine::encode_block2_observe_tx` |
| Q-Block recover | Present | `src/storage/block.rs::QBlockRecover`; `src/storage/progress.rs::Progress::qblock_recover` |
| `Engine::progress` | Present | `src/storage/progress.rs::Engine::progress` |
| `Access` / `AccessMut` pin | Present | `src/storage/access.rs::Access` |
| Core sends on the wire | Absent | `CALLER.md`; `src/storage/io.rs::DatagramIo` (caller impl sends) |
| 6LoWPAN / `TD_6LoWPAN_*` | Absent (not planned) | `src/lib.rs`; `tests/plugtest/catalog.rs::skip_reason` |
| DTLS / OSCORE | Absent (deferred) | `tests/plugtest/catalog.rs::DTLS`; `skip_reason` |

### Validation harness RUN / SKIP

From `tests/plugtest.rs::inventory` and `tests/plugtest/catalog.rs`:

| Suite | Count | Status |
| --- | ---: | --- |
| CORE (`catalog::CORE`) | 24 | RUN |
| BLOCK (`catalog::BLOCK`) | 6 | RUN |
| OBS (`catalog::OBS`) | 13 | RUN |
| LINK (`catalog::LINK`) | 9 | RUN |
| DTLS (`catalog::DTLS`) | 7 | SKIP (`deferred: DTLS/OSCORE not implemented`) |
| 6LoWPAN (`6lowpan.yml` via `extract_td_ids`) | 29 | SKIP (`not planned: 6LoWPAN`) |
| **Total** | **88** | **52 RUN / 36 SKIP** |

`inventory` asserts `ran >= 24 + 6 + 13 + 9`.

## Last mile: `DatagramIo`

The unfinished last mile was “own a socket and copy bytes.” That is closed with one trait:

```text
DatagramIo::recv  →  Engine::recv_from  →  RX slot + Endpoint
occupied TX slot  →  Engine::send_tx    →  DatagramIo::send
```

- `no_std`: implement `DatagramIo` on the radio / driver. Same Engine methods.
- `std`: `UdpSocket` is a `DatagramIo`. The example does not call `recv_from` / `send_to` on the socket except through the trait.
- `write_rx` stays for tests that already have bytes (plugtest loopback). It is not the integrator bind.
- The core still does not send. Saturation (`DatagramIoError::Saturated`) leaves the transport unread. Idle poll is `Ok(None)`.

## Taste test

**Intuitive**

- `App::profile().block_wise(true).route(path, get(h).put(p)).bind(io)` then `app.poll(now_ms)`.
- Handlers are `fn(Request<'_>) -> Reply`. `Reply::content` / `changed` / `not_found`. No global mutable shared state in `App`.
- `decode` / `encode` work with no Engine.
- Named `Opt` helpers and `EngineBuilder` typestate (`block_wise` is required).
- `Endpoint` ↔ `SocketAddr` under `std`.
- `recv_from` / `send_tx` are the two Engine bind calls. `send_tx` holds `Access` only for the send.

**Clunky**

- Route table is fixed (`DEFAULT_ROUTES` = 8). Raise with `.routes::<M>()` before `.route`.
- `profiles::Default` is not `Default::default()`.
- `Memory::<P>::with_block_wise()` must match `EngineBuilder` `.block_wise(true)` or `BuildError::SizeMismatch`. `App::bind` constructs the matching `Memory`.
- `Progress` is five independent `Option` fields, not one enum. Easy to ignore `observe_expired` / `qblock_recover`.
- `EncodedUint` must outlive `Opt` (`Opt::content_format(&cf)`).
- Forget `release_rx` on the Engine path and Default’s 4 RX slots saturate (`recv_from` → `Saturated`).
- `check_rfc7252_options` is Table 4 only: Observe / Block look “unrecognized critical.”
- Dedup is MID+endpoint history, not a response cache.
- `Token::mint` / `Ids` / jitter / `now_ms` stay caller-owned (not on `DatagramIo`).
- Observe notify and Q-Block recover are not inside `App::poll` yet (Phase 2).

**What a 0.1 integrator trips on**

1. Using Engine slots to expose a GET instead of `App` + `route` + a `fn(Request<'_>) -> Reply` handler.
2. Hand-rolling `recv` + `write_rx` + `access_tx` + `send_to` instead of `recv_from` / `send_tx` (Engine path).
3. Holding `Access` across `progress` (that slot is skipped).
4. Releasing a pending-CON TX before the empty ACK.
5. Expecting Engine to bind a port or skip sends for `NoResponse` (`App` honors `NoResponse`).
6. Mixing clocks across `progress`, RTO, Observe, and Echo.
