# coaptic

A stand-alone `no_std` CoAP library: RFC 7252 message decode/encode plus a bounded storage engine. Custom slots and tables, optional alloc.

## Library surface

- `coaptic::message` — decode/encode a CoAP datagram (`&[u8]`), empty ACK/RST constructors, RFC 7252 option value codecs (`message::value`), Observe (RFC 7641 option 6) uint helpers, Block/Q-Block/Size2 (`BlockValue` NUM/M/SZX; RFC 7959 / RFC 9177), and `OptionsBuilder` for out-of-order option insertion. No `Engine` required.
- `coaptic::storage` — bounded `Engine` / `Memory` / pools (main types are also at the crate root). `Engine` decodes/encodes occupied datagram slots when the backend implements `DatagramSlots`. Datagram slots hold CoAP bytes only; `Endpoint` is sidecar metadata. The Dedup Table stores typed `DedupEntry` rows. Pending CON matching (`PendingCon`) is sidecar on TX slots. Token matching (`ExchangeEntry`) is a compact table keyed by Token + remote Endpoint, sized from the TX pool count. Observe interest (`ObserveInterest`) fills the existing `ObserveTable` (Token + remote Endpoint). Classic Block1 / Block2 body assembly uses a `BlockTransfer` sidecar on Incoming / Outgoing Body Pool slots when `.block_wise(true)`. None of those is a seventh area.
- `coaptic::profiles` — `Default` (1472-byte datagrams) and `Constrained` (1152).

Crate: `coaptic` (`#![no_std]`; optional `alloc` and `std`). Licensed MIT OR Apache-2.0. See [CONTRIBUTING.md](CONTRIBUTING.md). Knowledge: [knowledge/](knowledge/). Plugtest harnesses are not in this crate yet.

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
