---
name: bounded-engine
description: Use when changing coaptic memory areas, slots, tables, progress, or pin/app access. Not for generic Rust.
license: MIT OR Apache-2.0
---

# Bounded engine

Read [knowledge/index.md](../../../knowledge/index.md) then `design.md` first. Architecture facts stay in `design.md`; this skill does not restate them.

Six core-managed areas: Incoming Datagram Pool, Outgoing Datagram Pool, Incoming Body Pool, Outgoing Body Pool, Dedup Table, Observe Interest Table.

- Progress is rotating and fair. Saturation is a normal operating region, not a reason to add queues or grow memory.
- Own the slot and table types in this crate. Do not add `heapless` as the architecture.
- Change `design.md` and `coap_constrained_design.drawio` together. If those facts change, update [Architecture](../../../knowledge/architecture.md) and [Memory areas](../../../knowledge/memory-areas.md).
- Do not add a seventh area without a demonstrated protocol lifetime the six cannot represent.
- Q-Block is preferred. Classic Block remains required.
- Do not restate RFCs. Open `knowledge/rfcs/rfcNNNN.txt` (and the matching `rfcNNNN.md`) for protocol behavior.
