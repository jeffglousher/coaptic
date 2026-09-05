---
type: Crate
title: coaptic
description: Stand-alone no_std CoAP engine
resource: ../Cargo.toml
crate_name: coaptic
license: MIT OR Apache-2.0
edition: "2024"
rust-version: "1.85"
features:
  default: []
  alloc: []
  std: [alloc]
tags: [coaptic, crate, no_std]
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
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource, title, crate_name, license, edition, rust-version, description, features]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Crate

| Field | Value |
| --- | --- |
| Name | `coaptic` |
| Default | `#![no_std]`, no allocator |
| Features | optional `alloc`; `std` implies `alloc` |
| License | MIT OR Apache-2.0 |
| Visibility | private until a first public go |

Own slot and table types in this crate. Do not add `heapless` as the architecture. Zero crate dependencies.

Message codec, bounded storage (`Endpoint`, `Access`, `Progress`, `QBlockRecover`), optional `alloc`. Not published to crates.io.

See [Architecture](/architecture.md) and the external-reviewer brief [`REVIEW.md`](../REVIEW.md).
