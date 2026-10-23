# Project plan

Coaptic serves direct UDP clients and servers, from embedded devices to host
services. The accepted implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, guided commissioning, durable
ownership and application receipts. Remaining qualification is listed below;
it is not silently treated as finished software work.

## Current work

[CPU efficiency #650](https://github.com/jeffglousher/coaptic/issues/650) remains
open: halve server CPU time per verified complete operation, separately for
unprotected and established OSCORE communication. Preserve behavior, security,
capacities, refusal and recovery. A passing host control does not complete the
representative SmallDevice/Constrained and cloud targets.

Dispatch constructs responses directly and applies Observe updates in place.
Local Inner scratch initializes only after protected input is selected. Windows
emitted code confirms removal of the helper return copy and the ordinary
608-byte response join copy. A qualified CPU gain remains unestablished.
OSCORE builds AAD directly in its existing output buffer. Canonical bytes and
refusal behavior are preserved. Windows emission confirms removal of the
48-byte intermediate, its 44-byte clear and staging copy, with more branches
and register saves. Net CPU benefit remains unestablished.

Discovery catalog scratch initializes only for the selected built-in GET.
Windows emission removes the ordinary 128-byte clear; scratch-owner layouts
and the measured 17,128-byte constrained async future stay unchanged. The
local dispatch frame grows by 16 bytes. This is a recorded stack cost, not
a whole-memory saving or measured CPU gain.

The latest clean socket comparison tests current core `7ab6949` against baseline
`b8f44fa`, using companion `eea0374`. Commands, compiler settings and limits
are in the
[Linux CPU guide](https://github.com/jeffglousher/coaptic-validation/blob/eea0374b60c93098a7f0dcf24a999e49d753d624/tools/benchmark/linux_cpu.md).

### What the measurements support

The [current same-Mio comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/39924741885)
verified 1,536,000 operations plus 640 warmups with no operation failures:
4,000/s, C1 CON, 64/1024 bytes, twelve seconds and four rotated pairs.
Both labels use the same capacities and frozen baseline client. Sixteen of
32 windows exceed the unchanged 1% missed-start limit. All four full-cohort
CPU ratios are withheld; this run establishes no speedup.

All pooled latency guards pass numerically, but two pair guards fail:
unprotected 1024-byte median +2.212 microseconds and OSCORE 64-byte p99
+10.402 microseconds. Both pairs also fail pacing eligibility. Preserve these
failures; pooled values do not establish overall latency acceptance.

The native Linux process-clock smoke test and all 32 bounded clock sidecars
pass. These optional whole-process, all-thread readings supplement the
original tick metrics. They do not change eligibility or provide replacement
ratios; reported nanosecond resolution is not nanosecond accuracy. They
exclude descendants and unattributed whole-machine work. Profiling was off.

A separate [admission diagnostic](https://github.com/jeffglousher/coaptic-validation/actions/runs/39937330058)
uses companion `c123d8b4` and identical core `1629c853` on both labels: Mio,
64 bytes, both security modes, 4,000/s, C1 CON, twelve seconds and two rotated
pairs. All eight windows pass the original operation, rate, pacing, tick and
cleanup gates: 384,000 verified operations plus 160 warmups, with no operation
failures. Linux checks pass 111 runner tests and 19 diagnostic tests; the
workflow builds Rust binaries but does not run native Rust tests.

The fixed counters reconcile 293 missed starts: 287 were already at least one
period late at the preceding verified completion, and six crossed that limit
after traversing the idle-wait branch. No first-admission or no-idle crossing
occurred. Prior completion includes inherited delay; these categories do not
identify a server, client, sleep or scheduler cause. The idle branch can include
a zero-duration sleep. This instrumented cohort does not explain or replace the
prior 16 pacing failures.

Both diagnostic CPU ratios remain withheld even though the original gates pass.
One descriptive OSCORE pair median guard fails at +13.615 microseconds. No
latency acceptance or instrumentation-overhead estimate follows. The diagnostic
state and its containing Option each occupy 208 bytes on Linux; those nested
sizes are not additive or whole-memory measurements. Profiling was off.

The prior [same-Mio core comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/39902235048)
qualified all 32 windows: 640,000 operations plus 640 warmups at 4,000/s,
C1 CON, 64/1024 bytes, five seconds and four rotated pairs. Baseline `f23d69c`
and candidate `ff089264` use the same adapter, capacities and frozen client.
Server CPU changes were -1.80%/-2.70% unprotected and -4.07%/-0.76% with OSCORE.
These are only 1â€“5 pooled counter ticks; no material repeatable saving is
established. Pooled latency guards pass, but three pair guards fail:
unprotected 64-byte p99 +80.892 microseconds, unprotected 1024-byte median
+2.634 microseconds and OSCORE 1024-byte median +4.427 microseconds.
Preserve all three failures. Overall latency acceptance is not established.
Profiling was off, so this run does not identify the next hotspot.

The earlier [Ready-versus-Mio control](https://github.com/jeffglousher/coaptic-validation/actions/runs/39891201686)
supports a limited host CPU saving of 8.28â€“14.29%, with two failed pair latency
guards. [Receive staging](https://github.com/jeffglousher/coaptic-validation/actions/runs/39895566785)
established no consistent gain and failed latency guards. Fewer empty App polls
did not establish lower total cost. Full reports and failures remain in #650;
existing selectors and defaults remain unchanged.

These are separate host comparisons. Do not combine their ratios, replace failed
windows or subtract timings with different scopes. Process CPU includes host I/O
and scheduling; sparse earlier profiles did not establish a hotspot ranking.
Per-call probe clocks and logical transport counts alone cannot prove CPU gains.

### Next steps

- Review the client's idle admission path and absolute-deadline recheck against
  the retained interval counters before changing pacing. Preserve the original
  failed windows and gates; the new diagnostic establishes no cause or CPU gain.
- Separate core processing from host I/O and scheduling using the existing
  optimizer-resistant core probe. Keep client preparation and response
  verification visible; whole-probe CPU is not isolated server CPU. Inspect
  optimized emitted code to establish which memory operations actually remain.
- Optimize the largest supported cost in a coherent batch. Reuse storage and
  parsing, preserve authentication, replay/nonce safety, cancellation, retained
  bytes and fresh-clock ordering. Add no persistent timer cache or hidden growth.
- Validate the change with matched, complete operations and clean CPU windows.
  Keep diagnosis separate from clean measurement. Reuse the validated bounded
  whole-process clock supplement alongside the original coarse ticks;
  preserve eligibility, ratios and failures. Reuse existing counters and
  runners; add bounded observability only to answer a concrete cost question.
  Keep kernel/eBPF work proportional to measurable value.
- Complete representative SmallDevice/Constrained and cloud qualification:
  small messages, blockwise bodies, CON concurrency, CPU-limited throughput,
  saturation, fairness, cancellation and recovery. Report client limits,
  uncertainty and instrumentation overhead with all failures retained.

Count whole-system memory: configured pools, initialization, metadata, scratch,
security, transport, runtime, stacks and kernel costs. Pools are allocated at
startup and reused; automatic expansion stays off. Inline components and RSS
snapshots do not establish peak or whole RAM. The latest Mio comparison has no
server resource ledger. The lazy local scratch adds one tag byte in measured
Windows layouts, with no persistent App field or extra payload buffer. Its
async-future delta is unknown. Do not sum nested owners; runtime allocations,
peak stack and whole-memory costs remain unqualified.

### Delivered behavior and boundaries

No accepted protocol implementation gap remains in this batch. Fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, guided commissioning and durable
ownership are implemented. App Calls have isolated concurrent Block2 assembly
([#655](https://github.com/jeffglousher/coaptic/issues/655)); nonce ownership and
per-endpoint NSTART still constrain concurrency qualification.

The native async and blocking controls remain optional. Earlier async work did
not establish a CPU gain; the blocking control has a distinct soft write-timeout
policy. Neither is a substitute for immediate-refusal or async cancellation
qualification. The adjacent deadline-query cleanup removes repeated work but
establishes no measured CPU gain. Retained reports and #650 hold the full
experiment ledger.

[#647](https://github.com/jeffglousher/coaptic/issues/647) studies volatile
plaintext/key lifetime and bounded zeroization; removing copies does not promise
erasure. The third-party [Go comparator defect](https://github.com/jeffglousher/coaptic-validation/issues/146)
is tracked separately from Coaptic implementation.

## Qualification waiting for physical access

These stay open. Continue software work while their prerequisites are unavailable.

- **Power-loss recovery â€” [#333](https://github.com/jeffglousher/coaptic/issues/333):**
  cold-power and interrupted persistent-write checks on the reserved ESP32.
  Done when retained identity, authorization and unresolved effects recover
  correctly, or the device explicitly refuses unsafe recovery.
- **Backup-owner recovery â€” [#365](https://github.com/jeffglousher/coaptic/issues/365):**
  execute installed recovery with three distinct credentials, then refuse reuse
  of the consumed backup. Needs the operator's deferred touch action.
- **Reverse LAN comparison â€” [#341](https://github.com/jeffglousher/coaptic/issues/341):**
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
