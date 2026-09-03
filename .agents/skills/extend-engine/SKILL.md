---
name: extend-engine
description: Add slots, tables, or engine surfaces in this crate. Use when changing the bounded-memory engine, slot/table types, or architecture surfaces.
license: MIT OR Apache-2.0
---

# Extend the engine

- Own the types in this crate. Do not add `heapless` as the architecture.
- Change `design.md` and `coap_constrained_design.drawio` together.
- If architecture facts change, update [knowledge/architecture.md](../../../knowledge/architecture.md) and [knowledge/memory-areas.md](../../../knowledge/memory-areas.md).
- Keep the PR small. One concern.
- Do not implement or restate CoAP wire format. Point at `rfcs/` and `design.md`.

Read [knowledge/index.md](../../../knowledge/index.md) then `design.md` first.
