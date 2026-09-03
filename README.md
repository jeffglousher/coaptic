# coaptic

A stand-alone `no_std` CoAP engine. Custom slots and tables, optional alloc, high throughput.

Crate: `coaptic` (`#![no_std]`; optional `alloc` and `std`). Licensed MIT OR Apache-2.0. See [CONTRIBUTING.md](CONTRIBUTING.md). Knowledge: [knowledge/](knowledge/).

Architecture notes below are the current reference. Protocol behavior stays in `knowledge/rfcs/`.

# Constrained CoAP Reference Architecture

A bounded, allocation-independent CoAP architecture for constrained devices.

## Canonical package

- `design.md` — current architecture and reference progress contract.
- `coap_constrained_design.drawio` — editable architecture diagrams.
- `knowledge/rfcs/` — local IETF CoAP/CoRE copies (txt + PDF). See `knowledge/rfcs/index.md`. Protocol behavior stays there.

Generated PNG/SVG/PDF diagram renders are disposable and are not canonical.

## Current posture

Applicable IETF CoAP specifications define protocol behavior. Local artifacts define only the constrained-memory model, ownership/lifetimes, application boundary, capacity posture, and bounded progress contract.

The core model has six bounded memory areas:

- Incoming Datagram Pool
- Outgoing Datagram Pool
- Incoming Body Pool
- Outgoing Body Pool
- Dedup Table
- Observe Interest Table

Ordinary messages use Datagram Slots directly. Block-wise bodies use contiguous Body Slots while every individual CoAP message still uses the shared Datagram Pools. Body Slots do not reserve or own Datagram Slots.

Progress occurs in bounded passes. Iteration is rotating rather than restarting at slot zero, giving recurring fair opportunity without priorities. Each occupied bounded owner is governed by explicit state-machine retention/release conditions, with timing colocated where required.

Datagram Slots remain shared without a fixed reservation scheme or traffic-priority hierarchy. Saturation is handled by the bounded state machines and applicable protocol/platform behavior; tuning determines throughput and loss.

## Maintenance rules

- Keep only the current architecture; history belongs in versioned snapshots/source control.
- Change `design.md` and `.drawio` together when architecture changes.
- Do not persist generated diagram renders.
- Do not restate IETF wire behavior, protocol state machines, constants, response rules, or option semantics.
- Use IETF terminology where it exists; do not create project synonyms without explicit review.
- Add a core memory area only when a demonstrated protocol lifetime requires it.
- Keep capacity exhaustion explicit and bounded.
- Keep application semantics and deferred application work outside protocol-owned state.
- Use flow walking and implementation tests to challenge ownership/lifetime assumptions, then record only the architectural result.
