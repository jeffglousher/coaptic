---
type: Architecture
title: Constrained CoAP architecture
description: Six core memory areas and a bounded progress contract. Canonical source is design.md.
resource: ../design.md
tags: [architecture, bounded, memory]
generated: { by: cursor_agent/cursor-grok-4.6-high-fast, at: 2026-09-03T11:50:09Z }
status: stable
sources:
  - id: design
    resource: ../design.md
    title: Constrained CoAP reference architecture
  - id: drawio
    resource: ../coap_constrained_design.drawio
    title: Architecture diagrams
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
