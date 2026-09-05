---
name: bounded-engine
description: Use when changing coaptic memory areas, slots, tables, progress, or pin/app access. Not for generic Rust.
license: MIT OR Apache-2.0
---

# Bounded engine

Architecture planning: <https://github.com/jeffglousher/coaptic/issues/45>. Protocol copies: `knowledge/rfcs/`.

- Own the slot and table types in this crate. Do not add `heapless` as the architecture.
- Do not restate RFCs. Open `knowledge/rfcs/rfcNNNN.txt` (and the matching `rfcNNNN.md`) for protocol behavior.
