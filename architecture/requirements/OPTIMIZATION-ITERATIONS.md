# Reliable optimization across library revisions

Direction agreed October 27, 2026: reliability first, performance second,
ease of use third. Reduce overhead while preserving supported behavior. A
proven memory, stack, code-size, CPU or reliability improvement can be delivered
independently of the open 50% CPU objective in [#650](https://github.com/jeffglousher/coaptic/issues/650).
The [full acceptance requirements](PERFORMANCE-ACCEPTANCE.md) remain unchanged.

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

[Validation #204](https://github.com/jeffglousher/coaptic-validation/issues/204)
is implemented in [companion #205](https://github.com/jeffglousher/coaptic-validation/pull/205)
as a prepared development screen using
prebuilt, source-identified binaries and existing verification paths. Target
about one minute for a complete warm run on the declared host. Setup, compilation
and first-build costs are reported separately. This duration is a development
target, not a promise of #650 qualification. Two complete Windows revision runs
measured 8.03 and 7.86 seconds; preparation/build costs are separate.

Start from four existing resource configurations:

- SmallDevice fixed/Constrained, no body pools, 32-byte response, C1 CON.
- SmallDevice with blockwise explicitly selected, one RX and one TX 4 KiB body,
  4 KiB response, C1 CON.
- Service fixed, four RX and four TX 16 KiB bodies, 16 KiB response, C1 CON.
- The bounded allocated equivalent of Service with identical logical capacities,
  payload and concurrency.

Run unprotected and established OSCORE separately. Preserve exact route, Call,
deferred, security-binding, packet and body dimensions from the full acceptance
matrix. Host runs of Constrained storage do not establish physical-device timing.
Independent-endpoint CON, saturation and refusal/recovery remain focused extended
checks; finite C1 success does not establish their coverage or peak throughput.

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
profile-specific oversize/refusal/recovery, eight socket cells and six supported
core cells. SmallDevice blockwise still lacks a socket-free core probe and is
reported unsupported. Profile selection now drives RX dimensions and startup
ledger validation. Exact source/build/binary/configuration identities and a frozen
client are required for revision comparison; hidden Cargo build settings are
refused. Preparation settles owned compiler auxiliaries before accepting artifacts;
benchmark workers retain fatal descendant and cleanup checks.

The [suite guide](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/benchmark/development_suite.md)
contains preparation, run and comparison commands. Windows custody and paired
runs are validated; real Linux execution of this new wrapper remains unverified.
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
