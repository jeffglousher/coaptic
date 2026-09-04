# Directory Update Log

## 2026-09-04

* **Update**: Replaced the `Peer` placeholder with [`Endpoint`](../src/storage/endpoint.rs) (IPv4/IPv6 sidecar) and typed Dedup Table rows (`DedupKey` / `DedupEntry`: Message ID + remote Endpoint). No `design.md` change.
* **Update**: Fixed-capacity `OptionsBuilder` for non-decreasing option encode; thin `Engine` ↔ message slot glue (`decode_rx` / `encode_tx` and mirrors). No `design.md` change.
* **Update**: RFC 7252 §3.2 option value codecs on the library façade (`message::value`). Optional format check is separate from wire decode and from unrecognized-critical. No `design.md` change.
* **Update**: Locked block-wise as an explicit builder switch: disabled ⇒ no body pools; enabled body bytes are a multiple of 1024 (default **4096** = 4 × 1024). Added [Block testing](/block-testing.md) combinatorial SZX × 1–25-block sweep (spec only).

## 2026-09-03

* **Update**: Added locked crate memory/API concepts [Memory](/memory.md) and [Profiles](/profiles.md) (1472 datagram default, Storage trait, named `MemoryProfile` constants). Spec only; no engine implementation.
* **Update**: Added the repo-specific `okf-frontmatter` skill (extract → enrich → validate, fail closed, including PR CI job `okf`) and regenerated concept frontmatter from a fresh deterministic extract plus LLM enrich/dedupe.
* **Update**: Moved IETF RFC copies into [RFCs](/rfcs/index.md) (one concept per RFC plus txt/pdf). Added [Plugtest](/plugtest/index.md) and [plugtest requirements](/plugtest/requirements.md). Replaced generic skills with `bounded-engine` and `plugtest`.
* **Creation**: Established the OKF v0.2 bundle and concepts [coaptic](/crate.md), [Architecture](/architecture.md), [Memory areas](/memory-areas.md), [CI](/ci.md), and [Review](/review.md).
