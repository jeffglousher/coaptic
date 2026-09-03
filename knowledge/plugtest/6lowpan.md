---
type: Test Descriptions
title: "CoAP#4 6LoWPAN"
description: "Deferred ETSI CoAP#4 6LoWPAN TDs. Not required until 6LoWPAN / security work."
resource: td-coap4/6lowpan.yml
td_ids: [TD_6LoWPAN_FORMAT_01, TD_6LoWPAN_FORMAT_07, TD_6LoWPAN_FORMAT_06, TD_6LoWPAN_ND_01, TD_6LoWPAN_ND_06, TD_6LoWPAN_HC_01, TD_6LoWPAN_HC_03, TD_6LoWPAN_HC_09, TD_6LoWPAN_HC_05, TD_6LoWPAN_HC_07, TD_6LoWPAN_ND_HC_01, TD_6LoWPAN_ND_HC_03, TD_6LoWPAN_FORMAT_02, TD_6LoWPAN_FORMAT_08, TD_6LoWPAN_FORMAT_05, TD_6LoWPAN_ND_02, TD_6LoWPAN_ND_07, TD_6LoWPAN_HC_02, TD_6LoWPAN_HC_04, TD_6LoWPAN_HC_10, TD_6LoWPAN_HC_06, TD_6LoWPAN_HC_08, TD_6LoWPAN_ND_HC_02, TD_6LoWPAN_ND_HC_04, TD_6LoWPAN_FORMAT_03, TD_6LoWPAN_FORMAT_04, TD_6LoWPAN_ND_03, TD_6LoWPAN_ND_04, TD_6LoWPAN_ND_05]
tags: [plugtest, 6lowpan, deferred]
status: deprecated
scope: deferred
sources:
  - id: 6lowpan-yml
    resource: "https://raw.githubusercontent.com/cabo/td-coap4/master/6lowpan.yml"
    title: cabo/td-coap4 6lowpan.yml
    retrieved: "2026-09-03T00:00:00Z"
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource, td_ids]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# 6LoWPAN (deferred)

Local copy: [td-coap4/6lowpan.yml](td-coap4/6lowpan.yml). Kept as a reference artifact.

Not required for the current engine. The YAML `ref` lines cite RFC 4944, RFC 6775, and RFC 6282 (not copied into this bundle). Do not invent 6LoWPAN behavior to pass these TDs.

| ID | Objective (from YAML `obj`) |
| --- | --- |
| `TD_6LoWPAN_FORMAT_01` | Check that EUTs correctly handle uncompressed 6LoWPAN packets (EUI-64 link-local) |
| `TD_6LoWPAN_FORMAT_02` | Check that EUTs correctly handle uncompressed 6LoWPAN packets (16-bit link-local) |
| `TD_6LoWPAN_FORMAT_03` | Check that EUTs correctly handle uncompressed 6LoWPAN fragmented packets |
| `TD_6LoWPAN_FORMAT_04` | Check that EUTs correctly handle maximum size uncompressed 6LoWPAN fragmented packets |
| `TD_6LoWPAN_FORMAT_05` | Check that EUTs correctly handle uncompressed 6LoWPAN multicast to all-nodes (16-bit link-local) |
| `TD_6LoWPAN_FORMAT_06` | Check that EUTs correctly handle uncompressed 6LoWPAN multicast to all-nodes (EUI-64 link-local) |
| `TD_6LoWPAN_FORMAT_07` | Check that EUTs correctly handle uncompressed 6LoWPAN packets (EUI-64 to 16-bit link-local) |
| `TD_6LoWPAN_FORMAT_08` | Check that EUTs correctly handle uncompressed 6LoWPAN packets (16-bit to EUI-64 link-local) |
| `TD_6LoWPAN_ND_01` | Check that a host is able to register its global IPv6 address (EUI-64) |
| `TD_6LoWPAN_ND_02` | Check that a host is able to register its global IPv6 address (16-bit) |
| `TD_6LoWPAN_ND_03` | Check Host NUD behavior |
| `TD_6LoWPAN_ND_04` | Check 6LR NUD behavior (ICMP version) |
| `TD_6LoWPAN_ND_05` | Check 6LR NUD behavior (UDP version) |
| `TD_6LoWPAN_ND_06` | Check host behavior under multiple prefixes (EUI-64) |
| `TD_6LoWPAN_ND_07` | Check host behavior under multiple prefixes (16-bit) |
| `TD_6LoWPAN_HC_01` | Check that EUTs correctly handle compressed 6LoWPAN packets (EUI-64 link-local, hop limit=64) |
| `TD_6LoWPAN_HC_02` | Check that EUTs correctly handle compressed 6LoWPAN packets (16-bit link-local, hop limit=64) |
| `TD_6LoWPAN_HC_03` | Check that EUTs correctly handle compressed 6LoWPAN packets (EUI-64 link-local, hop limit=63) |
| `TD_6LoWPAN_HC_04` | Check that EUTs correctly handle compressed 6LoWPAN packets (16-bit link-local, hop limit=63) |
| `TD_6LoWPAN_HC_05` | Check that EUTs correctly handle compressed UDP packets (EUI-64, server port 5683) |
| `TD_6LoWPAN_HC_06` | Check that EUTs correctly handle compressed UDP packets (16-bit, server port 5683) |
| `TD_6LoWPAN_HC_07` | Check that EUTs correctly handle compressed UDP packets (EUI-64, server port 61616) |
| `TD_6LoWPAN_HC_08` | Check that EUTs correctly handle compressed UDP packets (16-bit, server port 61616) |
| `TD_6LoWPAN_HC_09` | Check that EUTs correctly handle compressed 6LoWPAN packets (EUI-64 to 16-bit link-local, hop limit=64) |
| `TD_6LoWPAN_HC_10` | Check that EUTs correctly handle compressed 6LoWPAN packets (16-bit to EUI-64 link-local, hop limit=64) |
| `TD_6LoWPAN_ND_HC_01` | Check that EUTs make use of context 0 (EUI-64) |
| `TD_6LoWPAN_ND_HC_02` | Check that EUTs make use of context 0 (16-bit) |
| `TD_6LoWPAN_ND_HC_03` | Check that EUTs make use of context ≠ 0 (EUI-64) |
| `TD_6LoWPAN_ND_HC_04` | Check that EUTs make use of context ≠ 0 (16-bit) |
