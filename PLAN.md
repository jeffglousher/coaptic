# Project plan

Coaptic serves direct UDP clients and servers, from embedded devices to host
services. The accepted implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, guided commissioning, durable
ownership and application receipts. Remaining qualification is listed below;
it is not silently treated as finished software work.

## Current software status

The current priority is [CPU efficiency #650](https://github.com/jeffglousher/coaptic/issues/650):
halve CPU time per verified operation separately for unprotected and established
OSCORE traffic. Establish matched-load process CPU baselines, separate idle
scheduling from active processing, and profile before expanding instrumentation.
Keep client costs, failures, latency and whole-system memory visible. The prior
throughput report does not prove this target; it remains open until representative
device and cloud configurations have relevant evidence.

The latest [matched Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/39786342261)
completed 1,536,000 verified operations with no failures. At concurrency one,
64/1024-byte payloads and matched Rust 1.99.0, CPU changes ranged from a 3.45%
reduction to a 2.70% increase; neither 50% target is established. Idle receive
polling is a measured cost, with a latency tradeoff when using readiness. The
[earlier Linux report](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/cpu-cost-linux-20261022)
retains raw profiling evidence; its different compiler and host prevent isolating
later changes by comparing the two runs.

Dispatch now validates the path once and encodes a request selection only when
Observe, deferral or retained Q-body state needs it. Timer maintenance borrows
dedup rows and checks Call deadlines before inbox/Observe lookups. These changes
add no retained storage; generated stack/future size and the latest dispatch CPU
effect remain unmeasured. Next, measure the merged source and extend bounded
concurrent workloads. Keep NON traffic separate from CON qualification, preserve
NSTART and nonce ownership, and report actual overlap, refusal and recovery.

The OSCORE copy cleanup and source-matched library comparison are delivered in
[core #648](https://github.com/jeffglousher/coaptic/pull/648) and
[companion #149](https://github.com/jeffglousher/coaptic-validation/pull/149).
Authenticated Inner reconstruction encodes once, opened requests avoid RX
reserialization, and fresh protected TX slots avoid packet staging. Retained-TX
replacement preserves transactional staging and retransmission bytes on refusal.

The [current report](https://github.com/jeffglousher/coaptic-validation/tree/98b54fe8fd4c9b76301d2d22a533af40ecade744/tools/benchmark/evidence/comprehensive-windows-20261022)
contains all named protected/unprotected configurations, resource ledgers,
independent packet grades and retained failures. Separate client/runtime strata
and the patched-toolchain control are explicit. The copy reductions did not
establish a large core-poll speedup. Further optimization needs measured waste,
with authentication, replay, refusal/recovery and bounded ownership preserved.

No accepted protocol implementation gap remains in this batch. Required physical
qualification stays below. Future [#647](https://github.com/jeffglousher/coaptic/issues/647)
studies volatile plaintext/key lifetime and bounded zeroization; copy removal
does not promise erasure. The open third-party
[Go comparator defect](https://github.com/jeffglousher/coaptic-validation/issues/146)
is tracked separately from Coaptic implementation.

## Qualification waiting for physical access

These stay open. Continue software work while their prerequisites are unavailable.

- **Power-loss recovery — [#333](https://github.com/jeffglousher/coaptic/issues/333):**
  cold-power and interrupted persistent-write checks on the reserved ESP32.
  Done when retained identity, authorization and unresolved effects recover
  correctly, or the device explicitly refuses unsafe recovery.
- **Backup-owner recovery — [#365](https://github.com/jeffglousher/coaptic/issues/365):**
  execute installed recovery with three distinct credentials, then refuse reuse
  of the consumed backup. Needs the operator's deferred touch action.
- **Reverse LAN comparison — [#341](https://github.com/jeffglousher/coaptic/issues/341):**
  execute MCU-to-host unprotected and protected cells. Needs scoped Windows
  inbound UDP permission. Host-to-MCU execution and reverse software/build
  checks already exist; they do not complete these two runtime cells.

Preserve the reserved device's keys, NVS, custody history and unresolved effects.
Missing history never authorizes resetting it to first use.

## Scope and completion

[Protocol scope](architecture/IETF-RFC-PLAN.md) defines the supported roles.
The [qualification checklist](architecture/requirements/VALIDATION-PLAN.md)
defines the remaining runs and points to retained proof.
[#130](https://github.com/jeffglousher/coaptic/issues/130) coordinates delivery;
[Project 2](https://github.com/users/jeffglousher/projects/2) shows its queue.

The milestone is complete when accepted behavior is implemented, relevant finite
checks pass, the onboarding flow is ready to try, and the report states measured
performance, resource requirements and limits. An unavailable prerequisite is
still unfinished qualification.

Publication is separately authorized under
[#132](https://github.com/jeffglousher/coaptic/issues/132).
It requires a selected candidate and release review; it is not the next automatic
development step. Unsupported transports and roles are contribution opportunities,
not hidden completion gates.
