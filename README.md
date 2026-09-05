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
- Observe (RFC 7641 option 6) and Block / Q-Block / Size2 (`BlockValue`)
- `OptionsBuilder` for out-of-order insertion
- No `Engine` required

### `coaptic::storage`

- Bounded `Engine` / `Memory` / pools
- `Endpoint` sidecar (address + port); datagram slots hold CoAP bytes only
- `Access` / `AccessMut` pin occupied slot bytes against `release`
- `Engine::progress` → `Progress`: pending CON retransmit poll, one rotating unpinned RX step, one rotating Observe notify, at most one incoming `QBlockRecover`
- Dedup (`DedupEntry`), pending CON + RTO (`PendingCon` / `PendingRto`), token matching (`ExchangeEntry`), Observe interest (`ObserveInterest`)
- Classic Block and Q-Block body assembly on body-pool slots when `.block_wise(true)`
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

## Plugtest

Vendored ETSI TDs and the coaptic mapping live in [knowledge/plugtest/](knowledge/plugtest/). Combinatorial SZX sweep policy: [knowledge/block-testing.md](knowledge/block-testing.md). An in-crate validation harness is not on this tree.

## Architecture (short)

Six core-managed areas: Incoming / Outgoing Datagram Pool, Incoming / Outgoing Body Pool, Dedup Table, Observe Interest Table.

Progress is bounded and rotating. Saturation is a normal operating region. Canonical write-up: [design.md](design.md). Locked crate names: [knowledge/memory.md](knowledge/memory.md). UDP peer sidecar is `Endpoint` (not `Peer`).

Crate: `#![no_std]`; optional `alloc` and `std`. Licensed MIT OR Apache-2.0.
