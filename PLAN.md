# Project plan

Coaptic serves direct UDP clients and servers, from embedded devices to host
services. The accepted implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, guided commissioning, durable
ownership and application receipts. Remaining qualification is listed below;
it is not silently treated as finished software work.

## Current work

The [current delivery handoff](architecture/DELIVERY-HANDOFF.md) identifies saved
work, pending checks and transport choices. The finite option-batch comparison
has finished; its failed latency guard is retained. The bounded C1 profile fixture
is delivered in companion #174 and independent-endpoint CON in #176. Staged
fixture recovery (#175/#178) and Linux workflow integration (#177/#179) are now
delivered and pass focused Linux proof. The first SmallDevice C1 comparison
passes every guard, with modest CPU reductions; remaining implementation and
representative qualification continue. Companion #184 now delivers bounded long
low-load partitions and #186 bounded process-memory observations. The first
500-second unprotected SmallDevice partition passes every guard, with a 3.09%
server CPU reduction. The separate identical-core memory control verifies
complete operations and 32 available snapshots; its CPU results remain withheld
for low ticks and one failed OSCORE median guard. The first 500-second OSCORE
SmallDevice partition verifies 400,000 operations but fails pair and pooled
latency guards; its CPU comparisons are withheld. Both 50% targets and
whole-memory acceptance remain open. PR watches and recurring tasks remain off.

The refreshed [Windows library comparison](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/current-windows-20261023)
contains 348,000 verified complete operations across 2,610 cells and 90 workload
views, with zero failed operations. It includes unprotected, OSCORE, DTLS and
separate HTTP/3 strata. Raw receipts, latency samples and a regrading command
are published. These closed-loop rankings do not establish server saturation,
whole-system memory requirements or the CPU target below. The measured core is
`b9b83d8`; the later dependency update is outside that measurement.

[CPU efficiency #650](https://github.com/jeffglousher/coaptic/issues/650) remains
open: halve server CPU time per verified complete operation, separately for
unprotected and established OSCORE communication. Preserve behavior, security,
capacities, refusal and recovery. A passing host control does not complete the
representative SmallDevice/Constrained and cloud targets.

The [performance acceptance matrix](architecture/requirements/PERFORMANCE-ACCEPTANCE.md)
records the retained baseline, unchanged host gates and explicit remaining
representative cells. Missing baseline or sufficient-resolution evidence keeps a
cell open. Production cleanup under validation #169 is repaired and qualified.
The bounded Linux direct RX comparison qualifies all windows and latency guards
but shows no consistent CPU saving. Companion #172 and #173 are merged; the
latter preserves source-specific locked builds across the syn dependency change.
Core option reuse was reviewed in #675 and closed without merging. The cumulative original-baseline
comparison and current-main isolation both retain OSCORE p99 failures. The batch
was ended after an independent disposition review found no evidence-supported
correction; no further campaign is scheduled. Neither CPU target is complete.

The [SmallDevice C1 run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40054024038)
compares original baseline `b8f44fa` with accepted `d23f93c`, using companion
`8dc65fd`, the same Mio adapter and frozen baseline client. Its 32-byte, C1 CON,
1,000/s, thirty-second, four-pair cells verify 480,000 operations plus 320 warmups.
All sixteen windows and every pair/pooled latency guard pass. Server CPU/op is
14.333 to 13.667 microseconds unprotected (-4.65%) and 15.917 to 15.417 OSCORE
(-3.14%). This one representative stratum leaves both 50% targets open and does
not replace failed host cohorts. See the handoff for sources, memory limits and
reproduction; low-load, blockwise, Service, concurrency and throughput remain open.

The matched original-to-accepted Windows core probe now verifies 322,048
exchanges across sixteen windows. Its Default/Alloc 64-byte per-poll wall
diagnostics do not establish server CPU gain or SmallDevice acceptance.
Companion #188 adds the explicit fixed SmallDevice 32-byte mode without changing
the legacy Default64 schema or batch CPU mode. Locked b8/d23 Windows checks and
eight functional cells verify 1,152 exchanges. Companion #189 delivers its
bounded opt-in Linux workflow; the single functional proof is pending review.
These wall probes do not establish either CPU target or whole-memory acceptance.
The bounded 9970 Hz screen from companion #181 stopped at its first control's
rounded report count; the other three controls did not run. The approach is
ended without a retry or communication capture. Companion #182 restores the
qualified 997 Hz source as `bff8859`. See the handoff for sources and limits.

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

The historical clean socket comparison tests core `7ab6949` against baseline
`b8f44fa`, using companion `eea0374`. Commands, compiler settings and limits
are in the
[Linux CPU guide](https://github.com/jeffglousher/coaptic-validation/blob/eea0374b60c93098a7f0dcf24a999e49d753d624/tools/benchmark/linux_cpu.md).

The [server-App CPU diagnostic](https://github.com/jeffglousher/coaptic-validation/pull/170)
is implemented and merged. Its [qualified Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40024678302)
uses core `b9b83d8` and companion `ff46fd5`: four serial processes, each with
50,000 measured operations and 128 warmups, a 64-byte representation and no UDP.
All 200,512 exchanges verify, all four CPU windows qualify, and 1,001 independent
evidence checks pass. The two unprotected observations are 0.901244 and 0.916253
microseconds/op; OSCORE observations are 2.032878 and 2.034727 microseconds/op.
These are server-App driver windows including full App polls, fixture copies,
monotonic reads and loop/gate work. Client preparation and complete authenticated
response verification run outside them. Two observations per mode do not establish
a confidence interval, an optimization gain or either 50% target.

The runner is Ubuntu 24.04, Linux 6.17.0-1022-azure, AMD EPYC 7763 with four
assigned logical CPUs, glibc 2.39 and Rust 1.99.0. Release uses thin LTO, one codegen
unit, debug information and frame pointers; detailed diagnostics are disabled.
The prior [five-window control](https://github.com/jeffglousher/coaptic-validation/actions/runs/40022418841)
also verifies every exchange, but unprotected windows are below the unchanged
10 ms CPU minimum. Those failures remain retained. Consolidating the same 50,000
operations into one window preserves the 10 ms, 100-resolution and 1% empty-control
guards. Empty controls are never subtracted.

Each diagnostic row has two 256-byte capture buffers and 40 bytes of metadata
and padding: 552 bytes total. App/CoreIo datagrams remain 1,472 bytes with
unchanged pools. Enclosing requested
fixture plus App is 27,658,654 bytes, including all rows, metadata/padding,
requested App heap, inline owners and a 16 KiB scratch allowance. This is below
the 32 MiB diagnostic ceiling, not a production memory requirement or whole-RAM
measurement. Allocator overhead and generated peak stack remain unknown. Native
worker-death/control-EOF checks pass. Later Linux run `40042232531` also passes
production launcher success/timeout and mixed-UID live-thread cleanup proofs for
reserved inherited process groups. Arbitrary escaped-group containment remains
outside that proof.

### What the measurements support

The [new original-baseline comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/40043745872)
uses accepted `b8f44fa` versus candidate `7c04304`, the same Mio adapter and the
retained 4,000/s, C1 CON, 64/1024-byte, twelve-second, four-pair workload.
All 32 clean windows verify 1,536,000 operations plus 640 warmups with zero
operation failures; operation/rate/pacing/tick/cleanup gates pass. OSCORE p99
fails two pairs (+17.861 and +15.664 microseconds) and both pooled sizes
(+13.306 and +11.093 microseconds for 64/1024 bytes). Full-cohort CPU claims
are withheld. All 16 separate profile windows verify 320,000 operations plus
320 warmups; their timings do not enter clean comparisons. Sampling provides
kernel scheduling/socket/syscall and dispatch/AES/CCM leads, with insufficient
coverage/inline/throttle evidence for a causal hotspot ranking.

This cumulative comparison does not isolate #675. The predeclared
[incremental comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/40045613509)
uses d23f93c versus the same 7c04304 and otherwise the same clean plan, with
profiling off. All 32 windows verify 1,536,000 operations plus 640 warmups with
zero failures; source, locks, pacing, ticks and cleanup pass. The second OSCORE
1024-byte pair fails p99: 50.112 versus 72.139 microseconds, a +22.027-microsecond
delta. Passing pooled guards cannot erase that failure. Qualified incremental
CPU claims are withheld; the original-baseline failure remains retained and
neither target can be completed by this local control.

The [direct RX control](https://github.com/jeffglousher/coaptic-validation/actions/runs/40042232531)
compares identical core `b9b83d8` using Mio versus direct Linux MSG_TRUNC receive:
64/1024 bytes, both modes, 4,000/s, C1 CON, twelve seconds and two rotated pairs.
All 16 windows, eight pair guards and four pooled latency guards pass; 768,000
measured operations plus 320 warmups verify with zero operation failures.
Direct/Mio CPU ratios are 0.93893/1.02290 unprotected and 1.01439/0.98089 OSCORE
(64/1024 bytes). This mixed result does not justify expanding the approach or
changing defaults. The earlier missing-Clippy setup failure `40041518786` remains
retained with zero campaign windows. See the handoff for memory and reproduction.

The [retained same-Mio comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/39924741885)
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

- Keep the client pacer unchanged: source inspection found no missing deadline
  recheck or demonstrated defect. Preserve the original failed windows and gates;
  interval counters establish no server, sleep or scheduler cause.
- Reuse the merged server-App CPU mode for core diagnosis. Its source-matched
  native checks and finite Linux clock/window qualification pass. Keep its
  driver scope and cleanup limits explicit; subtracting it from another run's
  socket timing cannot establish transport cost.
- The bounded direct RX experiment against Mio is complete. Keep it explicit;
  its mixed result does not justify further transport complexity. Any later
  completion/batching experiment needs a demonstrated cost and bounded ownership,
  cancellation, kernel/worker accounting and refusal/recovery proof first.
- The transient validated-option experiment is ended without merging #675 after
  its failed incremental p99 guard. Retain its sources, tests and measurements.
  Existing
  Windows object/future checks remain unchanged; generated dispatch/poll frame
  reservations are unchanged, but validation is now outlined with a 200-byte
  reservation and 64 bytes of saved registers. Record that cost without claiming
  peak-stack acceptance. The optimizer-resistant probe verifies all 161,024
  exchanges; per-call wall clocks establish no CPU gain.
- Use the delivered C1 fixture extension for unchanged SmallDevice, explicitly
  configured SmallDevice blockwise, and fixed/allocated Service profiles using
  the existing Mio loop/client. Independent-endpoint CON is delivered in
  companion #176 with one shared context and bounded socket lanes. Actual overlap
  and fully verified service on every lane remain mandatory; concurrent NON
  cannot substitute. Functional Windows checks are not Linux CPU qualification.
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
  uncertainty and instrumentation overhead with all failures retained. The
  default runner retains its 2..30-second windows. Delivered companion #184
  additionally permits one representative single-security Mio C1 CON partition
  with four rotated pairs and 31..1500 seconds per window, perf/diagnostics off.
  All eligibility and pair/pooled latency gates remain. The global 60,000 client
  sends can still leave cells below twenty server ticks regardless of duration;
  pooling sub-floor windows cannot qualify them. Run `40065560596` now qualifies
  the 100/s, 500-second unprotected SmallDevice slice at 50.20 to 48.65 microseconds
  per complete operation; it does not meet the 50% target or replace failed cells.
  OSCORE run `40073484846` completes all eight equivalent low-load windows but
  fails repeat-one median/p99, repeat-three median and pooled median guards.
  Keep its CPU comparisons withheld and that acceptance cell open; no retry or
  replacement cohort is planned.
  Use these retained results before extending remaining plans. Companion #186's
  bounded lifetime memory/stack-limit observations pass the limited Linux proof
  in `40066937573`; they do not establish whole-memory ceilings or used stack.

Count whole-system memory: configured pools, initialization, metadata, scratch,
security, transport, runtime, stacks and kernel costs. Pools are allocated at
startup and reused; automatic expansion stays off. Inline components and RSS
snapshots do not establish peak or whole RAM. The direct RX control reports
App-including-adapter owners of 16,008 bytes for Mio and 13,960 for direct; Poll
and Events are separate inline owners of 4 and 24 bytes (eight event capacity).
Heap, allocator, generated peak stacks and kernel owners remain separate or
unknown. The lazy local scratch adds one tag byte in measured Windows layouts,
with no persistent App field or extra payload buffer. Do not sum nested owners;
whole-memory acceptance remains open.
The retained SmallDevice Linux ELF inspection now identifies the declared load
image and its nested 32-byte representation/24-byte metadata statics in the
handoff. Image extents and TLS headers do not establish resident, dynamic TLS or
peak requirements; keep the remaining enclosing-owner ceilings open. The handoff
now freezes source-identified host App/requested-pool owner caps from retained
startup ledgers. Linux blockwise/Service and aggregate/whole-memory caps remain
open; the Windows host figures do not establish device RAM requirements.

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
