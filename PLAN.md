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

Core `120a622` validates each request path once, encodes selections lazily,
borrows dedup rows during timer scans, and validates request options in one pass.
The [isolated option-validation run](https://github.com/jeffglousher/coaptic-validation/actions/runs/39825129898)
verified 952,000 operations at C1 CON, 64 bytes and 11,900 operations/s with
matched Rust 1.99.0. Total server CPU changed -1.22% unprotected and +1.17% with
OSCORE, only two pooled 10 ms ticks in either mode. No material repeatable total
gain is established. OSCORE p99 increased 3.205 microseconds; retain that observed
cost. Separate inclusive profiles resolved kernel syscall/scheduler symbols,
but sampled self counts do not identify stage costs or prove redundant wakeups.

The explicit [blocking UDP control](https://github.com/jeffglousher/coaptic-validation/pull/157)
stages one packet in existing scratch before calling App with fresh time. Its
[qualified 4,000/s comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/39843237773)
verified 1,344,000 operations at the same core, compiler and frozen client.
All 32 clean windows and four pooled latency guards passed. Server CPU per
operation fell 17.61%/26.30% unprotected and 23.75%/30.89% with OSCORE for
64/1024-byte responses. Unprotected 64-byte median latency increased 1.8435
microseconds, close to the +2 guard. These are host scheduling gains; both
representative 50% targets remain open.

Retain the [11,900/s pacing failures](https://github.com/jeffglousher/coaptic-validation/actions/runs/39839988246):
only 13 of 16 windows qualify, so both cohort ratios remain withheld. The latest
[scheduler-accounting check](https://github.com/jeffglousher/coaptic-validation/actions/runs/39867544358)
verified 320,000 operations and passed 106 Linux tests. One unprotected window
missed 212 of 20,000 starts, above the unchanged 200 limit; its cohort ratio is
withheld. OSCORE qualified at 29.5 to 23.5 microseconds per operation, 20.34%
lower. Do not pool different hosts or substitute passing windows.

The control preserves core capacities and default adapters. It uses a distinct
synchronous congestion policy with soft read/write timeouts, not async
cancellation or exact absolute send deadlines. Linux inline sizes are 56 bytes
of metadata inside the 2,112-byte adapter inside the 16,064-byte App; nested
owners must not be summed. Whole-system memory and runtime congestion behavior
remain unqualified.

Recorder fences and optional scheduler snapshots are delivered in the companion
runner. Scheduler runtime agrees with process CPU within 10 ms in all four
latest captures, but they contain only 4/3/57/14 samples. No user/kernel mode
reaches 100 resolved samples, so there is no supported hotspot ranking or
accounting correction. The next task is bounded transport and idle-work counting
through existing diagnostic/test seams before removing work. Keep clean CPU
metrics and acceptance gates unchanged; further profiler infrastructure needs a
concrete reason.

The optional [native async fixture](https://github.com/jeffglousher/coaptic-validation/pull/154)
remains available. Its [same-core comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/39807483103)
used 9.28-18.83% more server CPU and failed the +2 microsecond median-latency guard
in all four 64/1024-byte security strata. This measures application scheduling,
not isolated core processing. [Borrowing App from its setup owner](https://github.com/jeffglousher/coaptic-validation/pull/155)
reduced the complete outer future from 120,696 to 56,536 bytes in a controlled
Windows Rust 1.99.0 layout check. This does not establish CPU, peak-stack or
whole-RAM savings; pools, allocator, body storage and kernel costs remain separate.

Concurrent Block2 response assembly is isolated by App Call. Two live Calls with
the same ETag previously shared a body slot and one could stop before sending
its follow-up ([#655](https://github.com/jeffglousher/coaptic/issues/655)). Both
original security cases now complete at unchanged capacities. Fixed/allocated
storage and classic/Q Block2 have focused coverage; raw Engine cross-token
assembly stays intact. The [bounded native driver](https://github.com/jeffglousher/coaptic-validation/pull/153)
keeps CON/NON cohorts separate and reports actual overlap, capacity refusal and
failed work. Further concurrency qualification must preserve NSTART and nonce
ownership, and include refusal and recovery.

The OSCORE copy cleanup and source-matched library comparison are delivered in
[core #648](https://github.com/jeffglousher/coaptic/pull/648) and
[companion #149](https://github.com/jeffglousher/coaptic-validation/pull/149).
Authenticated Inner reconstruction encodes once, opened requests avoid RX
reserialization, and fresh protected TX slots avoid packet staging. Retained-TX
replacement preserves transactional staging and retransmission bytes on refusal.

The [source-matched Windows report](https://github.com/jeffglousher/coaptic-validation/tree/98b54fe8fd4c9b76301d2d22a533af40ecade744/tools/benchmark/evidence/comprehensive-windows-20261022)
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
