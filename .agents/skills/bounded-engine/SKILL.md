---
name: bounded-engine
description: Use when changing coaptic memory areas, slots, tables, progress, or pin/app access. Not for generic Rust.
license: MIT OR Apache-2.0
---

# Bounded engine

Read [knowledge/index.md](../../../knowledge/index.md) then `design.md` first. Architecture facts stay in `design.md`; this skill does not restate them.

Six core-managed areas: Incoming Datagram Pool, Outgoing Datagram Pool, Incoming Body Pool, Outgoing Body Pool, Dedup Table, Observe Interest Table.

- Progress is rotating and fair (`Engine::progress` → `Progress`). Application pin is `Access` / `AccessMut`. Incoming Q-Block holes surface as `QBlockRecover`. UDP peer sidecar is `Endpoint` (not `Peer`). Saturation is a normal operating region, not a reason to add queues or grow memory.
- Own the slot and table types in this crate. Do not add `heapless` as the architecture.
- Default datagram slot is **1472** bytes (IPv4 UDP max on Ethernet). RFC 7252 §4.6 1152 is `profiles::Constrained`, not the crate default. Block-wise is a builder typestate switch. `.block_wise(false)`: no body pools in Storage. Enabled body capacity is bytes, a multiple of **1024**, default **4096** = 4 × 1024. Do not use 0 as disable. Block/Q-Block test sweep: [Block testing](../../../knowledge/block-testing.md).
- One engine, two backends: `Engine<S: Storage>`. Logic (acquire/release/rotate) is written once against the trait. `no_std` default is `Memory<P: MemoryProfile>` with **named** associated constants (not positional const generics). `alloc` adds `AllocMemory` + runtime `Capacities` (one heap allocation, then no growth). Do not carve a byte slab.
- Locked crate names, builder typestate, and named tests: [Memory](../../../knowledge/memory.md). Profile numbers: [Profiles](../../../knowledge/profiles.md).
- Change `design.md` and `coap_constrained_design.drawio` together. If those facts change, update [Architecture](../../../knowledge/architecture.md) and [Memory areas](../../../knowledge/memory-areas.md).
- Do not add a seventh area without a demonstrated protocol lifetime the six cannot represent.
- Q-Block is preferred. Classic Block remains required.
- Do not restate RFCs. Open `knowledge/rfcs/rfcNNNN.txt` (and the matching `rfcNNNN.md`) for protocol behavior.
