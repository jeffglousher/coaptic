---
type: Test Descriptions
title: "CoAP#4 link format"
description: "In-scope ETSI CoAP#4 link-format TDs (TD_COAP_LINK_*). Canonical source is td-coap4/link.yml."
resource: td-coap4/link.yml
td_ids: [TD_COAP_LINK_01, TD_COAP_LINK_02, TD_COAP_LINK_03, TD_COAP_LINK_04, TD_COAP_LINK_05, TD_COAP_LINK_06, TD_COAP_LINK_07, TD_COAP_LINK_08, TD_COAP_LINK_09]
tags: [plugtest, link, core]
status: stable
scope: in-scope
sources:
  - id: link-yml
    resource: "https://raw.githubusercontent.com/cabo/td-coap4/master/link.yml"
    title: cabo/td-coap4 link.yml
    retrieved: "2026-09-03T00:00:00Z"
  - id: rfc6690
    resource: /rfcs/rfc6690.md
    title: RFC 6690
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource, td_ids]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Link format (in scope)

Local copy: [td-coap4/link.yml](td-coap4/link.yml). Sequences stay there. Map each TD to [RFC 6690](/rfcs/rfc6690.md) (`rfc6690.txt`).

See [requirements](/plugtest/requirements.md).

| ID | Objective (from YAML `obj`) |
| --- | --- |
| `TD_COAP_LINK_01` | Access to well-known interface for resource discovery |
| `TD_COAP_LINK_02` | Use filtered requests for limiting discovery results |
| `TD_COAP_LINK_03` | Handle empty prefix value strings |
| `TD_COAP_LINK_04` | Filter discovery results in presence of multiple rt attributes |
| `TD_COAP_LINK_05` | Filter discovery results using if attribute and prefix value strings |
| `TD_COAP_LINK_06` | Filter discovery results using sz attribute and prefix value strings |
| `TD_COAP_LINK_07` | Filter discovery results using href attribute and complete value strings |
| `TD_COAP_LINK_08` | Filter discovery results using href attribute and prefix value strings |
| `TD_COAP_LINK_09` | Arrange link descriptions hierarchically |
