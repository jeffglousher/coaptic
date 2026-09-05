---
type: Playbook
title: Block and Q-Block testing policy
description: Combinatorial Block/Q-Block SZX × 1–25-block sweep; Default 4096 is not the test ceiling.
tags: [plugtest, block, q-block, testing]
status: stable
sources:
  - id: rfc7959
    resource: /rfcs/rfc7959.md
    title: RFC 7959
  - id: rfc9177
    resource: /rfcs/rfc9177.md
    title: RFC 9177
  - id: memory
    resource: /memory.md
    title: Locked memory and API
  - id: requirements
    resource: /plugtest/requirements.md
    title: coaptic plugtest requirements
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-04T11:33:33Z"
  fields: [type]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-04T11:33:33Z"
---

# Block / Q-Block testing policy

Locked (Jeff, 2026-09-04). Policy. An in-crate harness is not on `main` yet (draft PR #27); mapping and TDs: [Plugtest](/plugtest/index.md). Protocol behavior stays in [RFC 7959](/rfcs/rfc7959.md) and [RFC 9177](/rfcs/rfc9177.md). Slot defaults: [Memory](/memory.md). Profile numbers: [Profiles](/profiles.md).

Block/Q-Block tests are a **dynamic combinatorial sweep**, not a hard-coded list of cases. Do not invent ETSI TD identifiers. CoAP#4 block TDs stay in [block](/plugtest/block.md). Extra Q-Block checks come from [RFC 9177](/rfcs/rfc9177.md), not made-up TD numbers.

# Sweep

| Axis | Values |
| --- | --- |
| Block size | All SZX sizes, smallest to largest: 16, 32, 64, 128, 256, 512, 1024 ([RFC 7959](/rfcs/rfc7959.md) / [RFC 9177](/rfcs/rfc9177.md)) |
| Body length in blocks | 1 through 25 inclusive |
| Path | Classic Block and Q-Block, where applicable |

Body length of 25 blocks is required so Q-Block exercises multiple cycles of parallel in-flight blocks when the window is smaller than the body.

# Capacity

This sweep requires block-wise **enabled**. Enabled body capacity is in **bytes**, a **multiple of 1024**. `profiles::Default` body **4096** = 4 × 1024 is a small starting default, not the test ceiling. The harness may use `AllocMemory` or a large test profile.

`.block_wise(false)` means Storage has no body pools. 0 body bytes is not a stand-in for disabled and is not a sweep case.

# Mapping

| Kind | Where |
| --- | --- |
| CoAP#4 `TD_COAP_BLOCK_*` | [block](/plugtest/block.md), [requirements](/plugtest/requirements.md) |
| Classic Block / Q-Block sweep | this playbook |
| Echo / Request-Tag | [RFC 9175](/rfcs/rfc9175.md); no invented TDs |
