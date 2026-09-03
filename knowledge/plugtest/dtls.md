---
type: Test Descriptions
title: CoAP#4 DTLS
description: Deferred ETSI CoAP#4 DTLS TDs. Not required until security work.
resource: td-coap4/dtls.yml
tags: [plugtest, dtls, deferred]
generated: { by: cursor_agent/cursor-grok-4.6, at: 2026-09-03T12:10:15Z }
status: deprecated
scope: deferred
sources:
  - id: dtls-yml
    resource: https://raw.githubusercontent.com/cabo/td-coap4/master/dtls.yml
    title: cabo/td-coap4 dtls.yml
    retrieved: 2026-09-03T00:00:00Z
---

# DTLS (deferred)

Local copy: [td-coap4/dtls.yml](td-coap4/dtls.yml). Kept as a reference artifact.

Do not implement DTLS or OSCORE to pass these TDs yet. `design.md` defers security integration. Not required for the current engine.

| ID | Objective (from YAML `obj`) |
| --- | --- |
| `TD_COAP_DTLS_01` | Basic DTLS PSK (success case) |
| `TD_COAP_DTLS_02` | Basic DTLS PSK (failure case — wrong PSK) |
| `TD_COAP_DTLS_03` | Lossy DTLS PSK (success case) |
| `TD_COAP_DTLS_04` | Basic DTLS RPK (success case) |
| `TD_COAP_DTLS_05` | Basic DTLS RPK (client failure case) |
| `TD_COAP_DTLS_06` | Basic DTLS RPK (server failure case) |
| `TD_COAP_DTLS_07` | Lossy DTLS RPK (success case) |
