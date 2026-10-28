# Reliable optimization across library revisions

Direction agreed October 27, 2026: reliability first, performance second,
ease of use third. Reduce overhead while preserving supported behavior. A
proven memory, stack, code-size, CPU or reliability improvement can be delivered
independently of the open 50% CPU objective in [#650](https://github.com/jeffglousher/coaptic/issues/650).
The [full acceptance requirements](PERFORMANCE-ACCEPTANCE.md) remain unchanged.

## First bounded memory iteration

Core #693 keeps two isolated reductions: unused fixed-table row-pin backing and
local OSCORE AAD backing. The exact eight-profile Windows comparison records
8/32/128-byte enclosing-App reductions for Device/Gateway/fixed Service; smaller
SmallDevice owners and allocated Service do not shrink. AAD object layout falls
72 to 40 bytes, separately from generated stack. Configured capacities, actual
pool pins and protocol behavior stay fixed. Source-specific timing arrays remain
descriptive; no CPU or latency acceptance follows from these small runs.

Keep the complete 15.40-second candidate run and its second startup-failed attempt.
Repair the concrete readiness bind race independently under validation #208;
the missing server log prevents assigning the observed failure's cause. Do not
replace that failure with a passing cohort. The separate allocated-table
counterpart in #695 omits only unused startup pin
arrays and retains real pool pins. Matched Service requested heap is 128 bytes
smaller; inline owners stay unchanged. Fault-injection/startup-request checks and
four complete development runs pass. The extra reversed pair was predeclared
because large fixed-Service timing shifts raised a concern; their signs do not
repeat, and every observation remains visible without a performance verdict.
Reject the simple lazy dedup replay enum: retained inactive
bytes are observable through current equality/hashing, so discarding them changes
behavior. These dispositions preserve useful work without reviving failed batches.

## Compare implementations without changing the workload

Keep a small, versioned set of configurations. Compare each configuration across
source revisions on the same host/toolchain/build settings, with the same client,
transport, capacities, payload, security mode and concurrency. Report absolute
costs and changes against the previous accepted iteration. Retain the original
source baseline separately for #650; a rolling baseline never replaces it.

Preparation must resolve the actual dependency path and record core/companion
commits and trees, selected lock input, compiler/target/features/build flags and
binary hashes. A clean checkout and executable hash alone do not prove their
relationship. Refuse source, binary or configuration mismatches. Freeze the harness
and loopback client across a comparison; never automatically replace a baseline.

Memory/concurrency choices belong to the implementer. A smaller configured body
pool may deliberately support fewer simultaneous transfers; a larger pool may
support more. Those are distinct configuration rows, not an optimization gain
between unequal workloads. Never silently enlarge SmallDevice to buy CPU savings,
shrink a preset to pass a memory gate, or enable automatic pool growth.

For every proposed mechanism record:

- Exact source and configuration, intended benefit, and a concrete counterexample.
- Verified complete operations, refusal/recovery, cancellation and supported sync/
  async behavior affected by the change; authentication and nonce/replay ownership.
- CPU and latency evidence with uncertainty; useful work and failures separately.
- Disjoint enclosing owners: inline App, requested pool heap, transport, runtime,
  initialization and stack costs. Nested buffers are not additional allocations.
- Complexity and portability impact, disposition, and the smallest remaining proof.

A known regression does not disappear because another metric improves. Accept an
explicit bounded tradeoff only within the declared configuration and its reviewed
budgets. Unknown material costs remain unknown. A failed experiment remains failed;
a new isolated mechanism needs a new rationale and predeclared comparison, not a
replacement run of the same rejected candidate.

## Fast development feedback and full qualification

The prepared development screen is implemented in companion #205 and expanded
by [validation #206](https://github.com/jeffglousher/coaptic-validation/issues/206)
and [companion #207](https://github.com/jeffglousher/coaptic-validation/pull/207).
Keep about one minute as the full warm-run target; setup/build are separate.
The original four-profile Windows runs took 8.03/7.86 seconds; preserve that
version-1 evidence separately from the expanded version-2 configuration.

Eight unchanged configurations cover constrained and capable implementations:

- SmallDevice fixed/Constrained: 32 bytes, blockwise/body pools off, C1.
- SmallDevice blockwise: 4 KiB, one RX and one TX body, C1.
- SmallDevice blockwise/Observe: the same body bounds, one Observe entry, C1.
- Device fixed/Default: 4 KiB, two RX/TX bodies, C2.
- Gateway fixed: 8 KiB, two RX/TX bodies, C2.
- Service fixed without blockwise: 1 KiB, no body pools, C4.
- Service fixed blockwise: 16 KiB, four RX/TX bodies, C4.
- Service allocated: identical Service capacities and 16 KiB workload, C4.

All run unprotected and established volatile OSCORE separately. Preset route,
Call, deferred, security, packet, dedup and Observe dimensions remain explicit;
these are implementer choices, not a ranking of unequal memory configurations.
C2/C4 require distinct actual client socket pairs, observed overlap and per-lane
complete-body verification while preserving one nonce owner and NSTART=1 per
endpoint. Host runs do not establish physical-device timing or saturation.

Observe coverage requires a separate real-UDP proof at the constrained one-interest
server bounds: full registration and changed notification, explicit deregistration,
Call refusal and recovery. Local cancellation controls one deliberately undelivered
registration; it does not establish reversal of an already delivered registration
or server Observe-overflow behavior. Ordinary GET timing is not notification timing.

Use optimizer-resistant core probes and native Windows loopback as distinct
measurements, with Linux loopback in its own platform lane. Keep verified counts,
owner sizes, sample arrays and source/configuration identities in a compact result.
Compare only matching rows. Report unavailable/ineligible rows and exhausted time
budgets explicitly; never silently omit a failed cell or pool unlike platforms.

Use `std::hint::black_box` around measured inputs/results with independently
verified observable results. Add Criterion only for selected repeatable hot-path
comparisons where statistical baselines answer a concrete implementation question;
keep it out of production dependencies. Inspect generated code when optimization
elision or stack movement is plausible. Statistical wall-time samples, instruction
counts and server CPU/op are different metrics; none substitutes for the others.

The combined suite reuses existing `black_box` probes and native verification.
One prepared run performs independent response corruption/security recovery,
profile-specific oversize/refusal/recovery, sixteen socket cells and six existing
C1 core cells. Core probes remain separate C1 supporting measurements for the three
matching storage configurations; ten other core cells are unsupported. Profile
selection now drives RX dimensions and startup
ledger validation. Exact source/build/binary/configuration identities and a frozen
client are required for revision comparison; hidden Cargo build settings are
refused. Preparation settles owned compiler auxiliaries before accepting artifacts;
benchmark workers retain fatal descendant and cleanup checks.

The [suite guide](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/benchmark/development_suite.md)
contains preparation, run and comparison commands. Windows custody was validated
in #205. The expanded paired proof passes in
14.27/14.23 seconds after preparation: nine reliability phases, both Observe
lifecycles, sixteen socket cells (512 measured plus 64 warmups) and six C1 core
cells per run. Preparation takes 96.66/98.09 seconds separately. The
[expanded receipt](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/eight-profile-suite-20261027)
identifies tested companion `cc6edfb`, before core `b9b83d8` and after core
`f4810d7`, with both bundles frozen before cells and one baseline-built socket
client. Real Linux execution of this wrapper remains unverified.
Short samples establish functionality and routine duration, not qualified CPU
savings or extreme tails. Socket startup, combined client startup/warmup,
measurement and cleanup are recorded; core startup/warmup/calibration remain
combined at worker level. Criterion has no nominated isolated cost to measure yet
and remains optional. Existing `small_device_quick.py` remains a distinct six-peer
Python-client screen.

Keep eBPF opt-in in a separate Linux diagnostic lane, with tracing-off controls,
bounded ownership and explicit overhead accounting. Counts do not establish time
or removable CPU share. Do not add privileged tooling, competitor libraries or
HTTP comparisons to every quick run. Use the existing broader library campaign
when those comparisons answer an actual product question.

Full qualification retains source-matched real-socket CPU windows, original
eligibility/latency guards, representative concurrency, saturation and recovery,
whole-memory accounting and separate physical checks. A fast green result is an
early regression screen, not full performance acceptance.

## Retained mechanisms and immediate disposition

The direct Linux receive control is already committed in the companion's
`tools/benchmark/native/servers/src/direct_udp.rs`. Run `40042232531` measured
App-including-adapter owners of 16,008 bytes for ordinary Mio and 13,960 for direct
receive: a 2,048-byte structural saving in that historical host configuration.
The control retains Mio readiness and immediate sends, uses the Engine destination
only during the synchronous receive, and detects truncation before parsing.
Keep this mechanism and its proof. Mixed CPU results do not erase that owner-size
result; they also do not justify a universal speedup or default promotion. The
historical owner sizes are not current SmallDevice or whole-memory measurements.

The unmerged level-readiness control at companion `4f7869b` removes separate
selector/Event owners, but both measured SmallDevice Apps remain 14,016 bytes.
Its failed OSCORE latency guard and unmeasured aggregate savings do not justify
shipping the old candidate. Preserve the source/result; park the whole transport.

The unmerged validated-option batch in core PR #675 has no demonstrated enclosing
memory reduction or qualified CPU gain and adds an outlined validator frame.
Preserve its independent semantic/error-precedence tests and source for a narrowly
justified extraction; do not merge the aggregate-return implementation unchanged.

The quick read-only adversarial pass found no new concrete correctness defect
and no evidence-supported correction to the two rejected candidates. Counterexamples
remain important: direct receive must reject an oversized datagram's valid-looking
prefix; a receive-credit transport can add waits during queued bursts; and option
reuse must preserve complete 4.02 validation before a later 4.00 refusal, including
authenticated Inner processing and replay behavior.

Borrowed maintenance (`e0586d1`), borrowed option validation (`120a622`), lazy
Inner scratch (`ff08926`) and lazy discovery scratch (`7ab6949`) are already
delivered. Keep their recorded costs, including discovery's additional 16-byte
local frame; an emitted frame is not a whole-stack peak. Recovery changes also
stay in the accepted source. No rewrite or removal is justified merely because
the overall CPU target remains open. The pass reused existing proofs and source
inspection; it ran no new builds, tests or benchmark workloads.

## Delivery and cleanup

Keep source, benchmark definitions and compact public results committed and pushed.
Use immutable source references for rejected experiments. Remove settled owned
local branches and exclusive generated output after checking for consumers;
preserve evidence and mixed/shared build lanes. Keep routine reporting lightweight:
one disposition per mechanism, one coherent implementation batch, focused checks,
independent review and authorized squash delivery. No PR watchers or recurring
campaigns are required for continuous use of the developer suite.
