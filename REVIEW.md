# External review

Brief for an outside reader of **jeffglousher/coaptic** (private). Start here, then `design.md` and rustdoc. Do not treat this file as the architecture source.

## What it is

A stand-alone `no_std` CoAP **library** (not a daemon, not a socket stack):

- Message domain: RFC 7252 decode/encode, option values, Observe and Block/Q-Block codecs, `Ids` / Token mint.
- Storage domain: six bounded areas, `Engine<S: Storage>`, `Memory` / optional `AllocMemory`.
- Progress domain: `Engine::progress` → `Progress` (CON RTO poll, rotating RX, Observe notify, Observe lifetime, incoming `QBlockRecover`).

Zero crate dependencies. Default is `no_std` with no allocator. `alloc` and `std` are optional (`std` implies `alloc`). Licensed MIT OR Apache-2.0. Not published to crates.io.

## Six areas

Incoming Datagram Pool, Outgoing Datagram Pool, Incoming Body Pool, Outgoing Body Pool, Dedup Table, Observe Interest Table.

Body pools exist only when `.block_wise(true)`. Datagram slots hold CoAP UDP-payload bytes. `Endpoint` is sidecar metadata (address + port), not a seventh area. `Access` / `AccessMut` temporarily pin occupied bytes. Canonical contract: [`design.md`](design.md). Names: [`knowledge/memory.md`](knowledge/memory.md).

## Done on this tree

Through squash-merges **#7–#34** on `main`:

| Domain | In the crate |
| --- | --- |
| Message | Decode/encode, empty ACK/RST, option values, Observe + Block/Q-Block codecs (SZX 7 is BERT), Request-Tag, Echo, Hop-Limit, No-Response, If-Match / If-None-Match [`Precondition`](src/message/precondition.rs), FETCH / PATCH / iPATCH codes, named 2.31 / 4.08 / 4.09 / 4.22 / 5.08, `ObserveTransmission`, `OptionsBuilder`, `Ids`, Token mint |
| Storage | `Engine` / `Memory` / `AllocMemory`, `Endpoint`, Dedup, pending CON + RTO, `ExchangeEntry` (Echo sidecar), Observe interest, classic Block + Q-Block body paths, [`BodyTag`](src/storage/block.rs) on [`BlockKey`](src/storage/block.rs), BERT multi-block payloads |
| Progress | `Access` pins, `Engine::progress`, Observe notify, Observe Max-Age / client-OFF lifetime, RFC 7641 §4.5 24-hour NON-confirm + notification NSTART on [`ObserveInterest`](src/storage/table.rs), incoming `QBlockRecover` + outgoing Q-Block reissue, first-block Observe on Block2 (`encode_block2_observe_tx`) |
| Validation | SZX `{16…1024}` × 1..=25 Block/Q-Block sweep (`tests/block_sweep.rs`; `SweepProfile` / `AllocMemory`) plus BERT / Request-Tag identity cases. In-memory CoAP#4 plugtest (`tests/plugtest/`; 52 RUN / 36 SKIP). |

The core does not send. The caller owns the clock, jitter, and socket. Integration checklist: [`CALLER.md`](CALLER.md).

## Core CoAP bar

The standing goal is a library that is functional and complete for **core CoAP** on the six areas, plus in-scope CoAP#4 TD passage (`base` / `block` / `link`). That bar is the RFC 7252 message/option surface (including FETCH / PATCH / iPATCH codes, No-Response, Hop-Limit, If-Match / If-None-Match classification), RFC 7641 Observe, RFC 7959 Block, RFC 9177 Q-Block, and RFC 9175 Echo / Request-Tag.

This is not a transport-stack completeness claim. The caller owns send, clock, jitter, RST / 4.xx policy, resource / If-Match decisions, and Encode+Access loops ([`CALLER.md`](CALLER.md)). UDP / DTLS / OSCORE / 6LoWPAN are not part of that bar.

## Not planned

- **6LoWPAN** and `TD_6LoWPAN_*`. Adaptation-layer. Not a completeness gap. Skip reason is **not planned**, not deferred.

## Deferred (out of the standing goal)

- DTLS, OSCORE (and the `TD_COAP_DTLS_*` suite)
- RST / 4.02 / 2.31 / 4.08 / 4.09 / 4.22 / 5.08 **policy** (caller-owned; codes and classifiers exist)
- Public crates.io / public GitHub

Harness filters: [`README.md`](README.md) §Validation harness. Policy: [`knowledge/block-testing.md`](knowledge/block-testing.md). TDs: [`knowledge/plugtest/`](knowledge/plugtest/).

`design.md` is the architecture contract. Its Validation and tuning section matches this crate (`Access` pins, `Engine::progress`, Observe notify and Max-Age / client-OFF lifetime, Q-Block recover, BERT and `BodyTag` identity, Echo on `ExchangeEntry`, in-crate harness). Ownership, six areas, datagram = CoAP payload, `Endpoint` sidecar, and block-wise builder rules are unchanged.

## What the caller must own

See [`CALLER.md`](CALLER.md). Short form: send, clock (`now_ms`), jitter / entropy, when to RST or 4.xx (and 2.31), resource / If-Match decisions, Encode+Access loops. Codes and classifiers exist; the library does not invent policy.

## 0.1 API surface freeze

This is the 0.1 public surface. Do not balloon crate-root re-exports. Do not rename unless a name is actively wrong (not taste). Do not start an optimization pass in the same change as a rename.

**Crate root (happy path):** `Engine`, `Memory` / `AllocMemory`, `EngineBuilder`, `Endpoint`, `Progress`, `Access` / `AccessMut`, `Ids`, `decode` / `encode`, `Message` / `ParsedMessage`, `Opt` / `OptionsBuilder`, keyed table rows (`DedupEntry`, `ExchangeEntry`, `ObserveInterest`, and siblings), Block/Q-Block types (`BlockValue`, `BlockTransfer`, `QBlockRecover`, `BodyTag`, and siblings), named option helpers (`Echo`, `HopLimit`, `NoResponse`, `Precondition`), `Transmission` / `ObserveTransmission`, main errors.

**Stay nested:**

- `message::` — `TokenSource`, `message::value` iterators and extra codecs.
- `storage::` — typestate `Missing` / `Present`, raw `*Table` / `*Pool` types, `MemoryProfile` / `NoBodies` / `WithBodies`, backend traits (`Storage`, `SlotPool`, `DatagramSlots`, and siblings).
- `profiles` — capacity numbers (`Default`, `Constrained`).

New types default to their module until a second happy-path caller needs them at the root.

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
2. [`CALLER.md`](CALLER.md) (what the caller must own)
3. [`README.md`](README.md) (crate surface)
4. [`design.md`](design.md) (architecture)
5. Crate rustdoc (`src/lib.rs`)
6. [`knowledge/index.md`](knowledge/index.md) → memory, profiles, plugtest, RFCs
