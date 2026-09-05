# Constrained CoAP — Reference Architecture

## Scope

Applicable IETF CoAP specifications define protocol behavior. This document defines only the bounded-memory architecture, ownership and lifetimes, application boundary, capacity posture, and reference progress contract used to implement that behavior.

Rust is the intended reference implementation language, but the architecture does not depend on Rust-specific ownership, async, allocation, or socket mechanisms. It should map cleanly to bare-metal, RTOS, and operating-system environments.

Q-Block is preferred when supported. Classic Block remains a fully capable standards-compliant path. Client and server roles are both first-class. Security integration is deferred without constraining later OSCORE or DTLS integration.

Where the IETF defines terminology, this architecture uses it rather than creating project synonyms. Protocol state transitions, wire behavior, timers, response rules, and option semantics remain in the applicable RFCs and are not duplicated here.

## Bounded-memory contract

The protocol core requires no runtime allocator. Capacity is provisioned before activation and remains bounded while active.

- Core-managed memory does not grow at runtime.
- Capacity exhaustion is an expected operating condition.
- Each progress invocation performs bounded work and returns.
- No hidden queue, private overflow area, or runtime allocation is used to escape configured limits.
- Very small configurations are valid; capacities are tuning and isolation controls rather than a fixed RAM floor.
- Storage may be static, caller-provided, platform-provided, or allocated by an outer integration before activation.

## Core-managed memory areas

The reference architecture has six core-managed memory areas:

- Incoming Datagram Pool
- Outgoing Datagram Pool
- Incoming Body Pool
- Outgoing Body Pool
- Dedup Table
- Observe Interest Table

A new core memory area is justified only by a demonstrated protocol lifetime that cannot be represented cleanly by these areas.

### Incoming Datagram Pool

Each RX Datagram Slot holds one complete received CoAP datagram plus only the state needed while that datagram is being handled.

RX slots are primarily ingress processing workspace. A received datagram is normally parsed, acted on, and released promptly after any surviving state has moved to the bounded owner that legitimately outlives the datagram.

An RX slot may remain occupied across progress invocations when its own bounded processing state requires that. Retention and release are governed by the slot's state machine and its defined boundary conditions.

The application may receive temporary safe read access. Application state that must outlive that access is application-owned.

### Outgoing Datagram Pool

Each TX Datagram Slot holds one complete outgoing CoAP datagram together with the bounded state whose lifetime is naturally tied to that outgoing message or operation.

The application may receive temporary safe writable access and may serialize directly into the slot. No mandatory intermediate payload buffer is required.

Unlike RX workspace, a TX slot may legitimately remain occupied while applicable outbound protocol processing still requires the outgoing bytes or associated state. It becomes reusable when those obligations and temporary application access have ended.

### Incoming Body Pool

Incoming Body Slots exist only for bodies that require block-wise transfer.

Each slot contains one contiguous complete-body byte area and the Block or Q-Block state required for that body. Admission of an incoming block-wise body acquires one complete Body Slot. Subsequent individual CoAP messages continue to use ordinary RX Datagram Slots and place their body ranges into the admitted Body Slot.

When the body is ready for the application, the application normally decodes directly from the Body Slot using temporary safe access. The Body Slot remains the bounded owner for the required block-wise lifetime.

### Outgoing Body Pool

Outgoing Body Slots exist only for bodies that require block-wise transfer.

The application normally serializes the complete body directly into the contiguous Body Slot. Block/Q-Block processing selects ranges of that body and uses ordinary TX Datagram Slots for every individual CoAP message.

The Body Slot remains the bounded owner of the body and required transfer state until that block-wise operation no longer needs them. Q-Block and classic Block use the same Body storage; changing transfer mechanism does not require a second complete-body buffer.

### Body sizing and direction

Each Body Pool has independent configuration for:

- slot count — admitted block-wise bodies in that direction;
- slot capacity — maximum complete body represented by one slot.

Incoming and outgoing values may differ.

A body that fits in one ordinary CoAP datagram uses the Datagram path directly and does not consume a Body Slot.

### Dedup Table

The Dedup Table is compact bounded duplicate history retained after an RX datagram no longer needs to remain in its slot.

It is not a general response cache. Response-sized retention or regeneration may be supplied by an implementation or application when useful, but it does not create another required core memory area.

### Observe Interest Table

The Observe Interest Table owns bounded long-lived Observe relation state and small pending/coalescing state.

It does not own notification payload storage. Notifications use the ordinary Outgoing Datagram path or, when block-wise, the Outgoing Body path.

## Application memory access

Core byte areas are directly usable through temporary safe access.

```text
ordinary incoming:    RX Datagram Slot -> application decode
ordinary outgoing:    application encode -> TX Datagram Slot
block-wise incoming:  Incoming Body Slot -> application decode
block-wise outgoing:  application encode -> Outgoing Body Slot
```

A core area cannot be reused while temporary application access remains valid. Applications that need longer-lived data decode or copy it into application-owned state.

The library owns CoAP protocol mechanics. The application owns resource selection, resource semantics, and deferred application work. The library may provide constrained-friendly dispatch/routing tools without imposing one routing model. The App façade is a routing table over request/response — not a global mutable shared application bag. Per-slot protocol state machines and bounded progress live on the Engine (reactor).

## Datagram and Body capacity relationship

Body capacity and Datagram capacity control different limits.

- Body Slot count bounds admitted block-wise body concurrency.
- Datagram Slot count bounds concurrent individual-message handling and strongly influences attainable throughput and pacing.

Adding Body Slots alone does not imply that all admitted block-wise operations can progress at full concurrency.

For a reference starting profile, define:

- `B = incoming Body Slot count + outgoing Body Slot count`
- `RX_base` and `TX_base` as Datagram capacity provisioned independently of block-wise bodies

A simple starting heuristic is:

- `RX Datagram Slots = RX_base + B`
- `TX Datagram Slots = TX_base + B`

This is a provisioning heuristic only. Datagram Slots remain shared; no Datagram Slot is bound or reserved to a Body Slot, and the relationship is not a claim of maximum socket throughput.

Performance-oriented targets should measure and balance Datagram and Body capacities against the useful packet rate of the platform and socket/network stack. Deliberately smaller Datagram pools are valid when reduced concurrency and stronger load isolation are desired.

## Reference progress contract

The architecture does not prescribe threads, tasks, async functions, interrupts, callbacks, or one monolithic reactor.

Logical progress exists for:

- transport/socket integration;
- Datagram processing;
- Block/Q-Block processing;
- Observe processing;
- application participation.

Block/Q-Block and Observe use the common Datagram machinery for network traffic; neither creates an alternate socket path.

### Bounded progress

Each progress invocation:

- inspects bounded configured state or a fixed bounded subset;
- performs bounded work for each visited owner;
- acquires only already-provisioned bounded resources;
- leaves surviving state in the bounded area that legitimately owns it;
- does not create a private queue to escape capacity limits;
- returns after bounded work rather than draining until empty.

Repeated invocation provides throughput.

### Fair iteration

The reference iteration strategy is rotating rather than repeatedly starting at slot zero. Each bounded iterable domain retains enough cursor state to resume subsequent passes after the prior stopping point.

This provides recurring opportunity under sustained load without introducing traffic classes or priorities. The architecture does not prescribe one global traversal order.

### Bounded state-machine lifetime

Every occupied slot or table entry is governed by explicit state transitions and defined conditions for continued retention and release/termination.

Where timing is required to govern that lifetime, timing state is colocated with the bounded owner: Datagram Slot, Body Slot, Dedup entry, Observe entry, or application-owned deferred state.

The architecture does not reproduce protocol-specific state machines or timer rules. Those remain defined by the applicable IETF specifications. Where the standards leave a local resource-boundary policy open, that policy is an implementation/profile tuning point.

This general state-machine rule is the architectural mechanism for bounded lifetime under saturation; no additional saturation-specific memory mechanism is required.

### Ownership by progress domain

Transport progress moves datagrams between the platform/network boundary and Datagram Slots.

Datagram progress handles individual CoAP messages and slot-local state. Surviving block-wise state belongs in Body Slots, duplicate history in the Dedup Table, Observe relation state in the Observe Interest Table, and longer-lived application state in application-owned memory.

Block/Q-Block progress owns active Body Slots. If it cannot obtain a Datagram Slot, the Body Slot remains the owner and the transfer can wait there according to its state machine.

Observe progress owns relation/pending state. Complete notification bodies are never queued in the Observe table.

## Saturation and failure posture

Maximum load is an expected operating region, not an exceptional memory-management path.

When configured limits are reached:

- state remains bounded;
- progress passes remain bounded;
- existing slot/entry state machines continue to receive fair execution opportunity;
- protocol-visible errors, timeouts, retransmissions, refusal, or packet loss may occur as defined or permitted by the applicable protocol and platform behavior;
- the core does not allocate more memory, create hidden queues, reserve special traffic classes, or invent another state area to avoid saturation.

There is no universal admission threshold. Exact capacity balance and local boundary tuning are measurement-driven.

The design goal is stability and predictable resource use through saturation. Successful completion rate and packet loss under deliberate undersizing are performance/configuration outcomes, not reasons to compromise the bounded architecture.

## Locked architecture

The following decisions are current and normative for the reference architecture:

- no required runtime allocator;
- six core-managed memory areas listed above;
- separate RX/TX Datagram Pools and Incoming/Outgoing Body Pools;
- ordinary payloads use Datagram Slots directly;
- block-wise bodies use complete contiguous Body Slots and ordinary Datagram Slots for every individual message;
- Body Slots are admitted as complete bounded resources and are not paired with dedicated Datagram Slots;
- Q-Block and classic Block share the same Body storage architecture;
- application access to core byte areas is temporary and safe;
- duplicate history remains compact rather than becoming a response-sized cache;
- Observe owns relation/pending state, not notification bodies;
- each occupied bounded owner has explicit state-machine retention/release conditions, with timing colocated where required;
- progress invocations are bounded;
- iteration is rotating so repeated passes do not restart at slot zero;
- fairness is provided without traffic priorities;
- Datagram Slots remain shared; the architecture defines no fixed slot-reservation scheme;
- no hidden continuation queue or runtime growth is used to escape saturation;
- Datagram/Body sizing controls performance, pacing, and isolation and is validated by measurement rather than treated as a correctness formula.

## Validation and tuning

Architecture flow walking has been used to test ownership and lifetime assumptions without duplicating the protocol behavior defined by the RFCs.

The six-area model has been exercised against ordinary client/server operations, Block and Q-Block request/response bodies, Observe, simultaneous load, and deliberately undersized Datagram configurations. No additional core memory area has been justified.

The reference progress contract is present in this crate: temporary application `Access` pins; bounded `Engine::progress` (CON retransmit poll, rotating RX, Observe notify, Observe Max-Age / client-OFF lifetime, incoming Q-Block recover); BERT multi-block payloads on the existing Body Slot path; Request-Tag / ETag body identity (`BodyTag` on the Body Slot sidecar). An in-crate harness covers the Block/Q-Block SZX × 1–25-block sweep and in-memory CoAP#4 plugtest.

Further measurement across capacity combinations and network conditions — throughput, occupancy, turnover, loss, completion, and CPU work per bounded pass — should tune defaults and local boundary policies without changing the six-area ownership model unless a demonstrated lifetime requires it.

## Standards sources

Protocol semantics are defined by the applicable IETF CoAP documents, including RFC 7252, RFC 7641, RFC 7959, RFC 9175, and RFC 9177. They are implementation references, not content to duplicate into this architecture document.
