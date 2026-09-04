---
type: Playbook
title: coaptic plugtest requirements
description: Repo-specific mapping of vendored ETSI CoAP plugtest TDs onto the six-area coaptic engine.
tags: [plugtest, interop, coaptic]
status: stable
sources:
  - id: td-coap4
    resource: "https://github.com/cabo/td-coap4"
    title: "ETSI CoAP#4 Test Descriptions (Carsten Bormann)"
    retrieved: "2026-09-03T00:00:00Z"
  - id: design
    resource: ../design.md
    title: Constrained CoAP reference architecture
  - id: rfcs
    resource: /rfcs/index.md
    title: Local IETF RFC copies
  - id: block-testing
    resource: /block-testing.md
    title: Block and Q-Block testing policy
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-04T11:33:33Z"
  fields: [type]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-04T11:33:33Z"
---

# Scope

This is the coaptic mapping. It does not rewrite ETSI TDs or IETF RFCs.

## In scope

CoAP#4 **base**, **block**, and **link**. Client and server are both first-class.

| Area | Artifact | RFCs |
| --- | --- | --- |
| [base](/plugtest/base.md) | [td-coap4/base.yml](td-coap4/base.yml) | [RFC 7252](/rfcs/rfc7252.md) |
| [block](/plugtest/block.md) | [td-coap4/block.yml](td-coap4/block.yml) | [RFC 7959](/rfcs/rfc7959.md), [RFC 7641](/rfcs/rfc7641.md) |
| [link](/plugtest/link.md) | [td-coap4/link.yml](td-coap4/link.yml) | [RFC 6690](/rfcs/rfc6690.md) |

Implement the TD identifiers that appear in those YAML files. Do not invent TD identifiers.

## Q-Block and Echo / Request-Tag

[RFC 9177](/rfcs/rfc9177.md) Q-Block is preferred by `design.md`. Classic Block is still required; CoAP#4 block TDs are [RFC 7959](/rfcs/rfc7959.md).

The 2014 CoAP#4 TDs do not cover Q-Block or Echo / Request-Tag ([RFC 9175](/rfcs/rfc9175.md)). Extra interop checks for those come from the RFC files themselves, not from made-up TD numbers.

Engine Block/Q-Block tests (beyond CoAP#4 TDs) are a dynamic combinatorial SZX × 1–25-block sweep, not a hard-coded case list. Sweep assumes block-wise **enabled**. `profiles::Default` body 4096 is the enabled default, not the test ceiling, and not a disable knob. Policy: [Block testing](/block-testing.md).

## Deferred (OSCORE / DTLS / 6LoWPAN)

[dtls](/plugtest/dtls.md) and [6lowpan](/plugtest/6lowpan.md) are deferred. Do not implement DTLS or OSCORE to pass those TDs yet.

## Engine constraint

Tests must run on the six-area bounded engine in `design.md`. No hidden queues. No `heapless` as the architecture. Saturation is normal. Plugtest loss and separate-response cases must not invent extra pools.

See [Architecture](/architecture.md) and [Memory areas](/memory-areas.md).

# What ETSI TDs are

ETSI TDs are interoperability tests between two implementations, not conformance packet traces. A pass is two engines talking, not a golden hex dump.

# Optional peer

Eclipse Californium `cf-plugtest-server` at `coap://californium.eclipseprojects.io:5683` is an optional interop peer, not the spec. Do not vendor Java.

# Skip

ETSI TS 103 104 (M2M binding) is not this engine.
