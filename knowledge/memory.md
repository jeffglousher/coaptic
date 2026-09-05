---
type: Architecture
title: Locked memory and API
description: Locked datagram/body slot sizes, block-wise builder switch, Storage trait, and crate names.
resource: ../design.md
tags: [architecture, memory, slots, storage]
status: stable
sources:
  - id: design
    resource: ../design.md
    title: Constrained CoAP reference architecture
  - id: rfc7252
    resource: /rfcs/rfc7252.md
    title: "RFC 7252 §4.6 Message Size"
  - id: block-testing
    resource: /block-testing.md
    title: Block and Q-Block testing policy
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-04T11:33:33Z"
  fields: [type, resource]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-04T11:33:33Z"
---

# Locked memory and API

Decisions below are locked (Jeff, 2026-09-03). Do not invent extra architecture. Protocol behavior stays in [`knowledge/rfcs/`](/rfcs/index.md). Inventory of the six areas: [Memory areas](/memory-areas.md). Canonical ownership and progress: [`design.md`](../design.md). Profile numbers: [Profiles](/profiles.md).

# Datagram slot

A datagram slot holds one CoAP message: the UDP payload. That is the CoAP header, token, options, payload marker, and payload.

It does not hold Ethernet, IP, or UDP headers. The engine does not parse Ethernet. A normal socket `recvfrom` already stripped L2/L3/L4 headers.

[`Endpoint`](../src/storage/endpoint.rs) (address and port) is sidecar metadata next to the slot, not bytes inside it.

# Datagram slot bytes

Default datagram slot size is **1472** bytes. That is IPv4 UDP max on Ethernet (`1500 − 20 − 8`), so a standard Ethernet-sized CoAP message fits.

[RFC 7252](/rfcs/rfc7252.md) §4.6 **1152** remains a constrained / unknown-PMTU profile, not the crate default. See [`profiles::Constrained`](/profiles.md).

**1280** is IPv6 IP-packet MTU, not a CoAP body size. **1024** is payload-in-one-datagram, not the slot.

# Body slots

Body slots are a separate complete-body capacity for block-wise transfer. Independent of datagram slot size.

Block-wise is an explicit builder typestate switch (`.block_wise(true)` / `.block_wise(false)`). Locked (Jeff, 2026-09-04).

**Disabled:** Storage has **no body pools**. No body slot arrays. No body capacity knobs. Do not allocate body RAM when block-wise is off.

**Enabled:** Size body capacity in **bytes**. The byte count must be a **multiple of 1024** (max Block/Q-Block SZX). Default enabled capacity is **4096** = 4 × 1024. Datagram default stays **1472**. Default is small; tests go bigger. Sweep policy: [Block testing](/block-testing.md).

Do not use 0 body slots or 0 body bytes as “disabled.” Disabled means the pools are absent.

# Storage

One engine, two backends. Logic (acquire / release / rotate) is written once against the trait.

```text
Engine<S: Storage>
```

- `no_std` default: `Memory<P: MemoryProfile>` owns typed arrays for the areas present in Storage. `MemoryProfile` is a trait with **named** associated constants, not a pile of positional const generics: RX/TX datagram slot counts and bytes, dedup entries, observe entries, and — **only when block-wise is enabled** — RX/TX body slot counts and bytes. Ship `profiles::Default` (1472 dgram; when enabled, 4096 body = 4 × 1024; modest slot counts) and `profiles::Constrained` (1152 dgram). Numbers: [Profiles](/profiles.md).
- `alloc` feature: `AllocMemory` with runtime `Capacities`, one heap allocation at init, then no growth.

Do not carve a byte slab.

RX and TX datagrams are two pools of the same type (`DatagramPool`). Body pools (`BodyPool`) exist only when block-wise is enabled.

# Builder

`EngineBuilder` is consuming, with a nats-style terminal `build()`. Typestate so `build()` exists only when the required areas are specified (via a profile or method-by-method) and the block-wise switch is set. `.block_wise(false)` omits body pools from Storage. `.block_wise(true)` requires body capacity in bytes (multiple of 1024). `build()` moves `Storage` in. `alloc` adds `build_alloc()` from `Capacities`. Size mismatch is a build error. Enabled body bytes that are not a multiple of 1024 are a build error.

# Names

| Name | Role |
| --- | --- |
| `Engine` | Protocol engine, generic over `Storage` |
| `EngineBuilder` | Consuming typestate builder |
| `Memory` | `no_std` backend: typed arrays for areas present in Storage |
| `MemoryProfile` | Named associated constants for those areas |
| `AllocMemory` | `alloc` backend: one heap allocation, then no growth |
| `Capacities` | Runtime sizes for `AllocMemory` / `build_alloc()` |
| `Storage` | Trait both backends implement |
| `DatagramPool` | Pool of datagram slots (RX and TX are two pools) |
| `BodyPool` | Pool of body slots when block-wise is enabled (RX and TX are two pools) |
| `BlockTransfer` / `BlockKey` | Block/Q-Block sidecar on a body slot (Token + remote Endpoint); classic and windowed paths |
| `QBlockRecover` | Incoming Q-Block hole from one `Engine::progress` pass (rotating RX body cursor) |
| `BodySlots` | Typed admit / write / complete / slice access to body pools |
| `DedupTable` | Dedup table |
| `Endpoint` | UDP peer sidecar next to a datagram slot (not `Peer`) |
| `DedupKey` / `DedupEntry` | Dedup identity (Message ID + remote Endpoint) |
| `PendingCon` / `PendingRto` | TX-sidecar pending CON + caller-clock RTO |
| `ExchangeKey` / `ExchangeEntry` | Token + remote Endpoint request/response matching |
| `ObserveTable` | Observe interest table |
| `ObserveKey` / `ObserveInterest` | Observe identity (Token + remote Endpoint); pending notify and 24-bit sequence on the row |
| `SlotId` | Slot identifier |
| `Ids` / `TokenSource` | Wrapping Message ID counter; caller-entropy Token mint (no OS RNG) |
| `Access` / `AccessMut` | Temporary application borrow of occupied datagram or body bytes; pin bit refuses `release` until drop |
| `Progress` | Outcome of one bounded `Engine::progress` pass (retransmit + rotating unpinned RX + rotating Observe notify + at most one `QBlockRecover`) |
| `profiles::Default` | 1472 dgram; enabled body 4096 (4 × 1024); modest slot counts |
| `profiles::Constrained` | 1152 dgram |

# Tests

Named coverage in `src/storage/tests.rs` and `src/message/tests.rs` (not a plugtest harness).

- build mismatch
- acquire-until-full then saturation
- release-and-reuse
- rotating cursor does not restart at zero
- alloc and no-alloc backends pass the same pool tests
- access pins against release
- access drop unpins
- double access is exclusive
- access requires occupied
- progress idle
- progress retransmit due
- progress does not release pinned RX
- progress rotating RX fairness
- progress observe notify due
- progress observe rotating fairness
- progress observe deregister clears
- observe sequence wrap
- progress Q-Block recover gap / fairness / absent when block-wise off
