---
type: Crate
title: coaptic
description: Stand-alone no_std CoAP engine crate. Locked name coaptic.
resource: ../Cargo.toml
tags: [coaptic, crate, no_std]
generated: { by: cursor_agent/cursor-grok-4.6-high-fast, at: 2026-09-03T11:50:09Z }
status: stable
sources:
  - id: cargo
    resource: ../Cargo.toml
    title: Crate manifest
  - id: license-mit
    resource: ../LICENSE-MIT
    title: MIT license
  - id: license-apache
    resource: ../LICENSE-APACHE
    title: Apache-2.0 license
---

# Crate

| Field | Value |
| --- | --- |
| Name | `coaptic` |
| Default | `#![no_std]`, no allocator |
| Features | optional `alloc`; `std` implies `alloc` |
| License | MIT OR Apache-2.0 |
| Visibility | private until a first public go |

Own slot and table types in this crate. Do not add `heapless` as the architecture.

See [Architecture](/architecture.md).
