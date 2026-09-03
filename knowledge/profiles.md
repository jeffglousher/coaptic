---
type: Architecture
title: Capacity profiles
description: Default (1472 dgram, 4096 body) versus Constrained (1152 dgram) MemoryProfile numbers.
resource: /memory.md
tags: [architecture, memory, profiles]
status: stable
sources:
  - id: memory
    resource: /memory.md
    title: Locked memory and API
  - id: design
    resource: ../design.md
    title: Constrained CoAP reference architecture
  - id: rfc7252
    resource: /rfcs/rfc7252.md
    title: "RFC 7252 §4.6 Message Size"
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T21:11:50Z"
  fields: [type, resource]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T21:11:50Z"
---

# Capacity profiles

Locked starting numbers for `MemoryProfile` implementations. API and slot meaning: [Memory](/memory.md). [RFC 7252](/rfcs/rfc7252.md) §4.6 is the source for **1152**, not the crate default.

`MemoryProfile` uses **named** associated constants (not positional const generics):

| Named constant | What it sizes |
| --- | --- |
| RX datagram slot count | Incoming Datagram Pool occupancy |
| RX datagram slot bytes | Bytes in each RX datagram slot |
| TX datagram slot count | Outgoing Datagram Pool occupancy |
| TX datagram slot bytes | Bytes in each TX datagram slot |
| RX body slot count | Incoming Body Pool occupancy |
| RX body slot bytes | Complete-body bytes in each RX body slot |
| TX body slot count | Outgoing Body Pool occupancy |
| TX body slot bytes | Complete-body bytes in each TX body slot |
| Dedup entries | Dedup table occupancy |
| Observe entries | Observe interest table occupancy |

# Default vs Constrained

| | `profiles::Default` | `profiles::Constrained` |
| --- | --- | --- |
| Datagram slot bytes (RX and TX) | **1472** | **1152** |
| Body slot bytes (RX and TX) | **4096** | not locked here |
| Slot and table counts | modest | not locked here |

1472 is IPv4 UDP max on Ethernet (`1500 − 20 − 8`). 1152 is the RFC 7252 §4.6 constrained / unknown-PMTU message size. 1280 is IPv6 IP-packet MTU, not a CoAP body size. 1024 is payload-in-one-datagram, not a slot size.

Body capacity is independent of datagram slot size. Constrained locks only the datagram slot bytes in this table.

# Backends

- `Memory<P: MemoryProfile>` uses these constants at compile time.
- `AllocMemory` takes the same dimensions as runtime `Capacities`.
