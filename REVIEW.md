# External review

Brief for an outside reader of **jeffglousher/coaptic** (private). Start here, then `design.md` and rustdoc. Do not treat this file as the architecture source.

## What it is

A stand-alone `no_std` CoAP **library** (not a daemon, not a socket stack):

- Message domain: RFC 7252 decode/encode, option values, Observe and Block/Q-Block codecs, `Ids` / Token mint.
- Storage domain: six bounded areas, `Engine<S: Storage>`, `Memory` / optional `AllocMemory`.
- Progress domain: `Engine::progress` → `Progress` (CON RTO poll, rotating RX, Observe notify, incoming `QBlockRecover`).

Zero crate dependencies. Default is `no_std` with no allocator. `alloc` and `std` are optional (`std` implies `alloc`). Licensed MIT OR Apache-2.0. Not published to crates.io.

## Six areas

Incoming Datagram Pool, Outgoing Datagram Pool, Incoming Body Pool, Outgoing Body Pool, Dedup Table, Observe Interest Table.

Body pools exist only when `.block_wise(true)`. Datagram slots hold CoAP UDP-payload bytes. `Endpoint` is sidecar metadata (address + port), not a seventh area. `Access` / `AccessMut` temporarily pin occupied bytes. Canonical contract: [`design.md`](design.md). Names: [`knowledge/memory.md`](knowledge/memory.md).

## Done on this tree

Through squash-merges **#7–#28** on `main` plus the in-crate validation harness on this branch:

| Domain | In the crate |
| --- | --- |
| Message | Decode/encode, empty ACK/RST, option values, Observe + Block/Q-Block codecs, `OptionsBuilder`, `Ids`, Token mint |
| Storage | `Engine` / `Memory` / `AllocMemory`, `Endpoint`, Dedup, pending CON + RTO, `ExchangeEntry`, Observe interest, classic Block + Q-Block body paths |
| Progress | `Access` pins, `Engine::progress`, Observe notify, Observe Max-Age / client-OFF lifetime, incoming `QBlockRecover` + outgoing Q-Block reissue |
| Validation | SZX `{16…1024}` × 1..=25 Block/Q-Block sweep (`tests/block_sweep.rs`; `SweepProfile` / `AllocMemory`). In-memory CoAP#4 plugtest (`tests/plugtest/`; 52 RUN / 36 SKIP). |

The core does not send. The caller owns the clock, jitter, and socket.

## Deferred

- DTLS, OSCORE, 6LoWPAN (and those plugtest suites)
- BERT, Request-Tag / ETag body identity
- Public crates.io / public GitHub

Harness filters: [`README.md`](README.md) §Validation harness. Policy: [`knowledge/block-testing.md`](knowledge/block-testing.md). TDs: [`knowledge/plugtest/`](knowledge/plugtest/).

`design.md` is the architecture contract. A few “next implementation work” sentences there predate the progress/Access/Q-Block recover landings; the crate is ahead of those sentences. Do not rewrite `design.md` from a docs pass.

## How to check

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo test --test block_sweep --all-features
cargo test --test plugtest
cargo test --test plugtest inventory -- --nocapture
cargo doc --no-deps --all-features
python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate
```

CI runs those on pull requests to `main` (and `workflow_dispatch`). No push-to-main CI. Squash merge. Agents and humans are equal authors and reviewers ([`AGENTS.md`](AGENTS.md), [`knowledge/review.md`](knowledge/review.md)).

## Reading order

1. This file
2. [`README.md`](README.md) (crate surface)
3. [`design.md`](design.md) (architecture)
4. Crate rustdoc (`src/lib.rs`)
5. [`knowledge/index.md`](knowledge/index.md) → memory, profiles, plugtest, RFCs
