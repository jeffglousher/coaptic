# Project plan

Coaptic serves direct UDP clients and servers, from embedded devices to host
services. The accepted implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, guided commissioning, durable
ownership and application receipts. Remaining qualification is listed below;
it is not silently treated as finished software work.

## Next software work

Follow the [performance investigation (#341)](https://github.com/jeffglousher/coaptic/issues/341).
Repeated request planning is removed. Authenticated Inner reconstruction now
encodes once, and fresh protected TX slots avoid packet staging. Retained-packet
replacement keeps transactional staging so refusal preserves retransmission bytes.

1. Finish the source-matched protected/unprotected library comparison and separate
   offline packet grading. Keep client, security and runtime strata distinct.
2. Use optimizer barriers and clock calibration for in-process probes; retain
   complete-response verification and all failures in network runs.
3. Change further hot-path work only for measured waste, preserving authentication,
   replay checks, refusal/recovery, bounded ownership and memory limits.

This is an optimization task, not a target speedup or a reason to weaken checks.
The [latest study](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/hot-path-windows-20261021)
contains exact sources and all samples. It found client-runtime overhead as well
as real library waste; it did not establish a historical 2–3× core regression.

Future hardening [#647](https://github.com/jeffglousher/coaptic/issues/647) studies
volatile plaintext/key lifetime and bounded zeroization. Copy removal does not
promise secure erasure; the study is separate from the accepted delivery milestone.

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
