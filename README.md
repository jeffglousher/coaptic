# coaptic

Stand-alone `no_std` CoAP library: RFC 7252 message decode/encode plus a bounded six-area storage engine. Custom slots and tables. Optional `alloc` / `std`. Zero crate dependencies.

## Crate surface

Happy-path types live at the crate root (`Engine`, `Memory`, `Endpoint`, `Progress`, `Access`, `Ids`, keyed table rows, Block/Q-Block types, main errors). Typestate markers, raw `*Table` / `*Pool` types, and backend traits stay under `storage` / `message`.

### `coaptic::message`

- Decode / encode a CoAP datagram (`&[u8]`)
- Empty ACK/RST constructors
- Wrapping Message ID sequence (`Ids`) and Token mint (`TokenSource`; no OS RNG)
- CON/NON request skeletons
- RFC 7252 option value codecs (`message::value`)
- Observe (RFC 7641 option 6) and Block / Q-Block / Size2 (`BlockValue`; SZX 7 is BERT)
- `ObserveTransmission` (24-hour NON-confirm / default NON timeout)
- Request-Tag and Echo (RFC 9175); ETag body identity (`BodyTag` on `BlockKey`)
- Hop-Limit (RFC 8768), No-Response (RFC 7967), If-Match / If-None-Match `Precondition`
- FETCH / PATCH / iPATCH codes (RFC 8132); named 4.09 / 4.22 / 5.08
- `OptionsBuilder` for out-of-order insertion
- No `Engine` required

### `coaptic::storage`

- Bounded `Engine` / `Memory` / pools
- `Endpoint` sidecar (address + port); datagram slots hold CoAP bytes only
- `Access` / `AccessMut` pin occupied slot bytes against `release`
- `Engine::progress` → `Progress`: pending CON retransmit poll, one rotating unpinned RX step, one rotating Observe notify (skips an endpoint at notification NSTART), at most one Observe lifetime expiry, at most one incoming `QBlockRecover`
- Dedup (`DedupEntry`), pending CON + RTO (`PendingCon` / `PendingRto`), token matching (`ExchangeEntry`; Echo sidecar), Observe interest (`ObserveInterest` / `ObserveLifetime` / `ObserveNotifyHold`)
- Classic Block and Q-Block body assembly on body-pool slots when `.block_wise(true)`; BERT multi-block payloads; Request-Tag / ETag on the sidecar; `encode_block2_observe_tx` for the first block of a Block2 notification
- None of those is a seventh area

### `coaptic::profiles`

- `Default` — 1472-byte datagrams; enabled body 4096 (4 × 1024)
- `Constrained` — 1152-byte datagrams

## Docs

| Doc | Role |
| --- | --- |
| [REVIEW.md](REVIEW.md) | External-reviewer brief |
| [design.md](design.md) | Canonical architecture and progress contract |
| [knowledge/](knowledge/) | OKF bundle: memory names, profiles, plugtest, RFCs |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contributor entry |
| [AGENTS.md](AGENTS.md) | Author/review policy and the CI command list |

Protocol behavior stays in [knowledge/rfcs/](knowledge/rfcs/). This README does not restate wire format.

## Checks

From [AGENTS.md](AGENTS.md). Run these when you touch Rust:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo doc --no-deps --all-features
```

When you touch `knowledge/`:

```bash
python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate
```

CI (`fmt`, `clippy`, `test`, `doc`, `okf`) is PR-only (`pull_request` to `main` plus `workflow_dispatch`). No push-to-main CI.

## Validation harness

Vendored ETSI TDs and the coaptic mapping live in [knowledge/plugtest/](knowledge/plugtest/). Combinatorial SZX sweep policy: [knowledge/block-testing.md](knowledge/block-testing.md).

In-repo harness (integration tests; `std` is fine there; no extra Cargo deps; no sockets / DTLS):

```text
cargo test --test block_sweep
cargo test --test block_sweep --all-features
cargo test --test plugtest
cargo test --test plugtest catalog
cargo test --test plugtest td_coap_core
cargo test --test plugtest td_coap_block
cargo test --test plugtest td_coap_obs
cargo test --test plugtest td_coap_link
cargo test --test plugtest inventory -- --nocapture
```

`--all-features` on `block_sweep` includes the `AllocMemory` 25 × 1024 case. Suite filters match the `#[test]` names in `tests/plugtest.rs`. `inventory` prints RUN vs SKIP for every vendored TD id (`dtls` deferred; `6lowpan` not planned). See rustdoc §Validation harness, [knowledge/block-testing.md](knowledge/block-testing.md), and [knowledge/plugtest/](knowledge/plugtest/).

## Architecture (short)

Six core-managed areas: Incoming / Outgoing Datagram Pool, Incoming / Outgoing Body Pool, Dedup Table, Observe Interest Table.

Progress is bounded and rotating. Saturation is a normal operating region. Canonical write-up: [design.md](design.md). Locked crate names: [knowledge/memory.md](knowledge/memory.md). UDP peer sidecar is `Endpoint` (not `Peer`).

Crate: `#![no_std]`; optional `alloc` and `std`. Licensed MIT OR Apache-2.0.
