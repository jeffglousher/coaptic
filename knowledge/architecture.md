---
type: Architecture
title: Constrained CoAP architecture
description: Six core memory areas and a bounded progress contract. Canonical source is design.md.
resource: ../design.md
tags: [architecture, bounded, memory]
status: stable
sources:
  - id: design
    resource: ../design.md
    title: Constrained CoAP reference architecture
  - id: drawio
    resource: ../coap_constrained_design.drawio
    title: Architecture diagrams
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Model

Six core-managed memory areas. Progress is bounded and rotating. Inventory: [Memory areas](/memory-areas.md).

| Artifact | Role |
| --- | --- |
| `design.md` | Canonical architecture and progress contract |
| `coap_constrained_design.drawio` | Editable diagrams (do not persist generated renders) |

Change those two together when architecture facts change. Update this concept if those facts change.

# Boundary

Protocol behavior stays in [`knowledge/rfcs/`](/rfcs/index.md). This concept does not restate wire format.
