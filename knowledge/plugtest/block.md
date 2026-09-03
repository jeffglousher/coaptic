---
type: Test Descriptions
title: CoAP#4 block and Observe
description: In-scope ETSI CoAP#4 block and Observe TDs. Canonical source is td-coap4/block.yml.
resource: td-coap4/block.yml
tags: [plugtest, block, observe]
generated: { by: cursor_agent/cursor-grok-4.6, at: 2026-09-03T12:10:15Z }
status: stable
scope: in-scope
sources:
  - id: block-yml
    resource: https://raw.githubusercontent.com/cabo/td-coap4/master/block.yml
    title: cabo/td-coap4 block.yml
    retrieved: 2026-09-03T00:00:00Z
  - id: rfc7959
    resource: /rfcs/rfc7959.md
    title: RFC 7959
  - id: rfc7641
    resource: /rfcs/rfc7641.md
    title: RFC 7641
---

# Block and Observe (in scope)

Local copy: [td-coap4/block.yml](td-coap4/block.yml). Sequences stay there.

- `TD_COAP_BLOCK_*` maps to [RFC 7959](/rfcs/rfc7959.md) (classic Block). Required even though [RFC 9177](/rfcs/rfc9177.md) Q-Block is preferred by `design.md`.
- `TD_COAP_OBS_*` maps to [RFC 7641](/rfcs/rfc7641.md).

There is no `TD_COAP_OBS_03` in the vendored YAML. Do not invent it. Q-Block has no ETSI CoAP#4 TDs; derive those checks from `rfc9177.txt`.

See [requirements](/plugtest/requirements.md).

| ID | Objective (from YAML `obj`) |
| --- | --- |
| `TD_COAP_BLOCK_01` | Handle GET blockwise transfer for large resource (early negotiation) |
| `TD_COAP_BLOCK_02` | Handle GET blockwise transfer for large resource (late negotiation) |
| `TD_COAP_BLOCK_03` | Handle PUT blockwise transfer for large resource |
| `TD_COAP_BLOCK_04` | Handle POST blockwise transfer for creating large resource |
| `TD_COAP_BLOCK_05` | Handle POST with two-way blockwise transfer |
| `TD_COAP_BLOCK_06` | Handle GET blockwise transfer for large resource (early negotiation, 16 byte block size) |
| `TD_COAP_OBS_01` | Handle resource observation with CON messages |
| `TD_COAP_OBS_02` | Handle resource observation with NON messages |
| `TD_COAP_OBS_04` | Client detection of deregistration (Max-Age) |
| `TD_COAP_OBS_05` | Server detection of deregistration (client OFF) |
| `TD_COAP_OBS_06` | Server detection of deregistration (explicit RST) |
| `TD_COAP_OBS_07` | Server cleans the observers list on DELETE |
| `TD_COAP_OBS_08` | Server cleans the observers list when observed resource content-format changes |
| `TD_COAP_OBS_09` | Update of the observed resource |
| `TD_COAP_OBS_10` | GET does not cancel resource observation |
| `TD_COAP_OBS_11` | Handle resource observation with CON messages (lossy case) |
| `TD_COAP_OBS_12` | GET with Observe=1 does cancel resource observation |
| `TD_COAP_OBS_13` | Handle observation of large resources (with Block2) |
| `TD_COAP_OBS_14` | Handle observation of variable size large resources (with Block2) |
