---
type: Test Descriptions
title: CoAP#4 base
description: In-scope ETSI CoAP#4 base TDs (TD_COAP_CORE_*). Canonical source is td-coap4/base.yml.
resource: td-coap4/base.yml
tags: [plugtest, base, core]
generated: { by: cursor_agent/cursor-grok-4.6, at: 2026-09-03T12:10:15Z }
status: stable
scope: in-scope
sources:
  - id: base-yml
    resource: https://raw.githubusercontent.com/cabo/td-coap4/master/base.yml
    title: cabo/td-coap4 base.yml
    retrieved: 2026-09-03T00:00:00Z
  - id: rfc7252
    resource: /rfcs/rfc7252.md
    title: RFC 7252
---

# Base (in scope)

Local copy: [td-coap4/base.yml](td-coap4/base.yml). Sequences stay there. Map each TD to [RFC 7252](/rfcs/rfc7252.md) (`rfc7252.txt`).

Client and server are both first-class. Loss and separate-response TDs must use the six-area engine; do not invent extra pools. See [requirements](/plugtest/requirements.md).

Identifiers below are the keys in the vendored YAML. Do not invent others.

| ID | Objective (from YAML `obj`) |
| --- | --- |
| `TD_COAP_CORE_01` | Perform GET transaction (CON mode) |
| `TD_COAP_CORE_02` | Perform DELETE transaction (CON mode) |
| `TD_COAP_CORE_03` | Perform PUT transaction (CON mode) |
| `TD_COAP_CORE_04` | Perform POST transaction (CON mode) |
| `TD_COAP_CORE_05` | Perform GET transaction (NON mode) |
| `TD_COAP_CORE_06` | Perform DELETE transaction (NON mode) |
| `TD_COAP_CORE_07` | Perform PUT transaction (NON mode) |
| `TD_COAP_CORE_08` | Perform POST transaction (NON mode) |
| `TD_COAP_CORE_09` | Perform GET transaction with separate response (CON mode, no piggyback) |
| `TD_COAP_CORE_10` | Perform GET transaction containing non-empty Token (CON mode) |
| `TD_COAP_CORE_11` | Perform GET transaction containing non-empty Token with a separate response (CON mode) |
| `TD_COAP_CORE_12` | Perform GET transaction using empty Token (CON mode) |
| `TD_COAP_CORE_13` | Perform GET transaction containing several URI-Path options (CON mode) |
| `TD_COAP_CORE_14` | Perform GET transaction containing several URI-Query options (CON mode) |
| `TD_COAP_CORE_15` | Perform GET transaction (CON mode, piggybacked response) in a lossy context |
| `TD_COAP_CORE_16` | Perform GET transaction (CON mode, delayed response) in a lossy context |
| `TD_COAP_CORE_17` | Perform GET transaction with a separate response (NON mode) |
| `TD_COAP_CORE_18` | Perform POST transaction with responses containing several Location-Path options (CON mode) |
| `TD_COAP_CORE_19` | Perform POST transaction with responses containing several Location-Query options (CON mode) |
| `TD_COAP_CORE_20` | Perform GET transaction containing the Accept option (CON mode) |
| `TD_COAP_CORE_21` | Perform GET transaction containing the ETag option (CON mode) |
| `TD_COAP_CORE_22` | Perform GET transaction with responses containing the ETag option and requests containing the If-Match option (CON mode) |
| `TD_COAP_CORE_23` | Perform PUT transaction containing the If-None-Match option (CON mode) |
| `TD_COAP_CORE_31` | Perform CoAP Ping (CON mode) |
