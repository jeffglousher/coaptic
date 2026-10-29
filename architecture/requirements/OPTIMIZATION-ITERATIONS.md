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

## Empty-work iteration

Core #697 wraps only pre-receive terminal-body expiry in a fresh documented O(1)
occupied-count check after deferred expiry. Preserve retained bodies after Observe
removal, pin-aware due refusal/retry, exact/regressed/MAX clocks and unconditional
later progress. No cached emptiness, public backend redesign or diagnostic
infrastructure is introduced. Tests and unchanged eight-profile functional
screens pass with unchanged measured owners. The predeclared focused two-pair
wall screen has a small consistent unprotected direction, with OSCORE mixed and
inconclusive. End sampling without replacement. Retain the narrow empty-work
reduction and its limited observation; do not present it as a CPU gain, protected
improvement or satisfaction of either 50% target. Existing `black_box` complete
verification suffices for this screen; no Criterion dependency is needed here.

## October 29 hot-path screen

Seven candidates were bounded; six were implemented in the staged
[combined PR #699](https://github.com/jeffglousher/coaptic/pull/699).
These were source-work nominations, not measured hotspot attribution.
No feature, configured capacity or authentication rule was removed.

1. **Select notification scratch independently.** `encode_notification` can
   append only ETag, Observe, Content-Format and Max-Age. Its private builder
   now has four entries instead of 24. The generic response encoder and public
   `OptionsBuilder` remain unchanged. Actual Windows x64 type layout is
   584 to 104 bytes, a 480-byte local-object reduction. This does not prove an
   emitted-frame, peak-stack, CPU or whole-memory reduction.
2. **Hold datagram acquisition reset removal.** Constructors and successful
   releases already clear owned fixed/allocated metadata. Removing the repeated
   acquire reset passes lifecycle checks, but an independent runtime benefit
   has not been established. Keep the patch and pin/refusal/recovery tests.
3. **Hold retained-response lookup shortcuts.** A fresh zero occupied count
   can skip a body-table scan. An occupied pool instead gains a count query
   before its unchanged scan. The refinement restores the original lookup for
   zero/single-slot pools and keeps the shortcut only above one slot. Both
   tested variants remain staged; neither establishes a net performance win.
4. **Hold the combined OSCORE request-binding lookup.** One borrowed row yields
   latest and original Observe references, preserving latest-first authentication
   and the distinct-original fallback. Functional proof passes; client reception
   cost is outside the existing server-only timing probe.
5. **Hold parsed response-PIV reuse.** Carrying authenticated response metadata
   removes a second option search/header parse in client admission. Replay and
   failed-ACK checks pass. Independent runtime benefit remains unmeasured.
6. **Reject isolated direct Observe route indexing in #711.** Checked inversion
   preserves holes/reuse behavior, but the later full-notification poll comparison
   fails both benefit gates and ten regression guards. Keep its raw evidence and
   source; the one-route quick GET fixture never established a scaling benefit.
7. **Park exact/fallback body-search fusion without implementation.** Extra
   identity comparisons before a later exact hit could worsen that workload.

The unchanged eight-profile suite ran two predeclared opposite-order pairs for
each combined source: `f4fb8e8` and refined `c39b100`, against `28d9f1a`, with
companion `dd74a00` and the same frozen loopback client. All eight runs pass
functional, security, refusal/recovery and owner checks in 14.00–14.69 seconds
for the first cohort and 13.92–14.20 seconds for the refinement. Each includes
16 socket cells, six C1 core cells, ten explicitly unsupported core cells and
both separate Observe proofs. App inline and requested-pool owners are unchanged.

Do not promote the combined timing result. The first cohort's unprotected
SmallDevice/Observe GET median increases by 6.85/4.45 us in both pairs. The
refinement changes that sign pattern, but Gateway increases by 11.80/14.45 us
unprotected and 23.35/13.50 us under OSCORE. Other rows include improvements
and mixed signs. These short observations do not attribute a cause or qualify
CPU/latency acceptance; adverse observations still block broad promotion here.
No replacement runs or further measurement variants were taken.

Library executions belong to `f4fb8e8`: 1,212 all-feature and 715 no-default
passes. Initial alias, allocated-body alignment and feature-gating fixture
failures remain retained. The notification-only subset reuses the unchanged
encoder, outbound adapters/protection and three exact oracle/test bodies after
hash comparison. Its fresh Clippy checks type-check all targets/tests with all
features and no defaults; its Rust 1.89 production checks pass in both modes.
No new library-test or quick-suite binary was built for that subset. The earlier
whole-batch passing suites are not relabeled as notification-only measurements.

[Raw cohorts, layout/reuse receipts and staged patch](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/hot-path-efficiency-20261029)
preserve these distinctions. Existing `std::hint::black_box` complete-operation
probes stay in use. No Criterion dependency or new benchmark platform was added
for this bounded memory result. Future CPU nominations need an isolated affected
path and a declared decision rule before measurement; the original #650 baseline,
both 50% targets and whole-memory qualification remain unchanged.

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

## October 29 isolated response-binding lookup

**REJECT; #705 is not delivered to main.** Core `3474c106` compares the actual
client response-opening entrypoint against `869d4354`'s exact two-search method
in one executable. [Retained data](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/oscore-binding-lookup-20261029)
includes all 72 Criterion JSON members, selected lock and identity limits.

Normal-entry mean changes are +2.871% / +1.009%, with both 95% intervals above
zero. First-slot changes are +12.893% / +0.830%; round 1's upper95 +19.539%
violates the +2% guard. Fallback changes +0.483% / +0.099% establish no benefit.
Lookup alone improves 21.154% / 21.166%. The full-entry regression cause remains
unestablished; fewer source searches do not establish a caller improvement.

The frozen two-round AB/BA plan requires at least 2% normal improvement and
upper95 below zero in both rounds, improving isolated lookup direction, and
first-slot/fallback upper95 regression at most 2%. Rejection ends sampling;
no replacement run or eight-profile expansion follows. All four arms and 134
nonignored correctness tests pass. Sixteen arrays contain 30 samples each;
each sample aggregates repeated immutable opens, not socket transactions.
Fixture equality, tamper and obsolete-binding rejection are checked outside
clocks. Rust 1.99.0, Criterion 0.5.1, Linux x86_64 CPU0, release thin LTO and
`std,oscore` without defaults are explicit. No replay-admission, process-CPU,
socket, emitted-stack, hardware or whole-memory claim follows.

Two earlier compile-only failures remain retained with zero samples and all
correctness/timing arms NOT RUN. Setup corrections used Cargo-managed LTO and
two temporary explicit byte-slice assertions; cases and thresholds stayed fixed.
Prior delivered notification-scratch savings remain intact. The separate
transient-cipher prototype is dispositioned below.
## October 29 transient cipher reuse

**REJECT; #707 is not delivered to main.** Candidate `636ad4e2` compares the
actual client response-opening path against `869d4354` in one executable. The
baseline includes both the original bound method and its original free
candidate loop; both retain the original two binding searches. The prototype
retains one transient lazy AES schedule across authentication attempts, then
drops it before processing authenticated Inner bytes. Ordinary one-shot
AEAD, capabilities, capacities, authentication ordering and errors stay intact.

The [retained data](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/oscore-transient-cipher-20261029)
includes all 126 Criterion JSON members, exact collector receipt, selected lock,
frozen plan, correctness output and explicitly derived identity/fixture projections.
All four arms and 136 OSCORE correctness tests pass. Twenty-eight arrays contain
840 positive finite time/iteration sample pairs; each sample aggregates repeated
immutable opens, not socket operations. Forty-eight fixture records preserve
identical wires, complete plaintext/reference checks, true fallback, obsolete
binding rejection and tampering. Independent key contexts and fresh encryption
nonces separate absent/explicit response PIV; repeated decryption is not replay
admission. Setup and correctness assertions remain outside timed closures.

Explicit-PIV fallback improves 10.544% / 10.662%; absent-PIV fallback improves
10.409% / 10.517%. Rejected two-attempt traffic improves 15.505% / 15.269% with
PIV and 15.989% / 15.898% without. These pass the frozen fallback benefit and
rejection guards. However, ordinary responses slow 3.452% / 3.149%, first-slot
responses 2.816% / 2.605%, and latest-bound Observe responses 2.189% / 1.839%.
All six normal upper95 bounds exceed the 2% limit. Passing fallback gains cannot
rescue those failures, and the specific normal-regression cause is unestablished.

The predeclared AB then BA plan requires both fallback means to improve at least
5%, with upper95 below zero in both rounds; each of the other five metrics must
have upper95 regression at most 2% in both rounds. Source, lock, executable and
fixture identities and correctness are mandatory. No replacement, changed gate,
pooling or eight-profile expansion follows. Preserve the measured fallback
opportunity for a separately justified normal-path-preserving design; do not
merge this lazy-state implementation unchanged.

Rust 1.99.0, Criterion 0.5.1 without default features, Linux x86_64 CPU0, release
thin LTO, one codegen unit and `std,oscore` without defaults are explicit. The
299-second hosted job includes a reported 228-second release build; instrumented
authentication intervals are separate. Earlier quick checks on the identical
tree use Rust 1.99.1: 1,208 all-feature and 711 minimal-feature library tests,
formatting and strict Clippy pass. That is separate producer evidence, not MSRV
or this benchmark's compiler. Compiler/executable identity and process cleanup
remain producer evidence; no ELF was independently inspected. No process/server
CPU50, socket, replay-admission, emitted-stack, hardware or whole-memory gain
follows. Both CPU objectives and remaining qualification remain open.

## October 29 response-only cipher continuation

**REJECT; #709 is not delivered to main.** Candidate `905a3977` uses one plain
local cipher and a lazy continuation reached only after the first raw AEAD
failure. The continuation refills existing AAD backing and overwrites the
existing nonce; both attempts reuse immutable ciphertext and private plaintext
scratch. The cipher drops before Inner/code/stitch/decode. Original one-shot
opening, both binding searches, capacities, security and terminal errors remain
unchanged. This is a separate design and comparison from #707, whose rejected
samples and disposition remain intact; it is not a replacement run.

The [retained data](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/oscore-cipher-continuation-20261029)
contains all 126 Criterion JSON members, exact collector receipt, selected lock,
frozen plan, correctness output and explicit derived input/fixture projections.
Four arms pass with 48 matching fixture records and 139 focused correctness
tests. Twenty-eight sample records contain 840 positive finite time/iteration
pairs; the collector's 56-array count counts times and iterations separately.
Full response/reference/error/tail checks remain outside clocks; immutable
reopening is not App replay admission.

Explicit-PIV fallback improves 17.038% / 14.638%; absent-PIV fallback improves
16.922% / 15.105%. Rejected traffic improves 19.053% / 16.873% with PIV and
18.398% / 17.021% without. Round-robin ordinary changes are -9.148% / +0.285%,
and latest-bound Observe changes are -0.784% / +0.691%; those guards pass.
First-slot ordinary responses regress +2.727% / +8.411%, with normalized 95%
intervals [+0.865%, +5.431%] / [+4.591%, +13.257%]. Both upper bounds violate
the unchanged +2% guard. The first-slot cost cause is not established by source
structure; stronger fallback gains cannot rescue this result.

The frozen AB/BA plan requires both fallback means <= -5% with upper95 below
zero and every other upper95 regression <= +2%, in both rounds. Source, lock,
binary, fixture and correctness gates are mandatory. Rejection ends this campaign:
no replacement samples, relaxed thresholds, eight-profile expansion or another
cipher variant. Existing delivered reductions are retained; no functionality or
capability is removed to obtain a performance result.

The timing executable uses Rust 1.99.0, Linux x86_64 CPU0, Criterion 0.5.1,
release thin LTO/one codegen unit and `std,oscore` without defaults. Each arm has
30 samples, 0.5-second warmup and 1-second measurement. The 219-second hosted
job includes a reported 155-second release build; the proof step is 205 seconds
and four timed libtest processes total 46.77 seconds. These are nested costs.
Separate quick run `40957984918` uses Rust 1.99.1 on the exact same tree and
passes formatting, strict all-target/all-feature Clippy, 1,211 all-feature and
711 no-default library tests. MSRV, the full feature matrix and
`extended_oscore` integration were not run; rejection stops further qualification.

Compiler/executable identity, resource enforcement and process settlement remain
producer evidence, with no uploaded ELF or independent kernel cleanup replay.
The collector checks complete source maps; publication retains selected identity
projections, not those full maps. No server/process CPU50, socket, replay
admission, emitted-frame/peak-stack, physical or whole-memory qualification
follows. Both CPU objectives and whole-memory acceptance remain open.


## October 29 checked Observe route lookup

The separately isolated #711 candidate is rejected and remains unmerged. Its
private checked inverse of the App-local slot identity replaces only
`Site::observe_source`'s ascending route scan. Populated/hole/replacement/removed
and foreign same-slot aliases, Plain/Selected callbacks and all capacities remain
supported. The original one-route GET fixture does not time notifications; this
screen clocks one complete already-due ordinary NON notification through
`App::poll`, including normal polling work and a bounded in-memory transport.

[Run 40970202099 attempt 1](https://github.com/jeffglousher/coaptic/actions/runs/40970202099)
uses core `96505cf772ffb01c3f0037ada79f1570034021c5`, tree
`68c64993e4e4ba8cf8f376d93cb8c60fd7257277`, against frozen accepted baseline
`176c337d350a4f601e4c2a02b9bdfdcaf0792a92`. Production blobs equal
`2349287f0bf78212125d94ab7316e4bcf604c326`. Frozen direct copies of poll, engine
and the original Site scan change only recorded identifiers and one due-source
call. This preserves static-call opportunities without a Site dispatch field;
symmetric noinline outer wrappers do not prove identical emitted production code.

Eight cells cross 4/32 fully populated server routes, first/last targets and
unprotected/OSCORE. Constrained-derived fixed storage has RX2/TX2 at 1152 bytes,
dedup4, Observe1, Call1, deferred0, four request bindings and one association;
blockwise/body pools are disabled. The independent verifier App retains four
routes and its own resources. Full 32-byte payload, wire metadata and
OSCORE authentication/admission are checked outside timing. Epochs use real
registration/deregistration/draining and distinct keys, at most 4096 NON
notifications including calibration/warmup, three-second virtual pacing and
actual confirmation-deadline checks. No state/sequence reset substitutes for
protocol progress. Logical pair/Rc owners measure 24080/30128 bytes including
RefCell metadata; allocator overhead, generated frames and peak stack are unknown.

The same frozen binary runs AB then BA on Linux x64 CPU0 (allowed 0-3), AMD EPYC
7763, Rust 1.99.0, temporary Criterion 0.5.1, `std,oscore` without defaults,
thin LTO and one codegen unit. Thirty samples per arm use 0.2-second warmup,
0.5-second nominal measurement, 10000 resamples and 95% intervals. PerIteration
setup, verification and drops are outside clocks; clock/wrapper/borrow overhead
remains and nothing is subtracted. Two fixture-only processes agree before any
sample; all six processes produce 48 matching fixture receipts. Fixed preludes
verify 196608 notifications and 196704 server packets, separately from dynamic
calibration/warmup/samples. All 877 nonignored release correctness tests pass;
same-head quick checks pass 1209 all-feature/716 minimal tests, strict Clippy/fmt.

Both 32-route last-slot cases require mean change <= -5% and upper95 < 0 in both
rounds. Actual unprotected changes are +3.297% / +1.510%; OSCORE changes are
-0.613% / -1.398%. All four benefit checks fail. Ten of twelve other upper95
<= +2% guards also fail; first-slot four-route unprotected means rise
+5.623% / +8.045%. All 144 Criterion JSON files, 64 literal arrays, 960 positive
finite sample pairs, 32 means and 16 AB/BA-normalized intervals are retained.
Producer status remains `INCONCLUSIVE/REJECT`; complete evidence supports REJECT.
Cause is unknown. Stop without replacement samples, changed gates, a new source
variant or eight-profile expansion. No runtime optimization is delivered.

The hosted job is 276 seconds, with setup to runner 13 seconds, runner 258 seconds
and upload 2 seconds. Nested inside the runner are Cargo-reported build time
226 seconds, correctness 0.31 seconds, fixture aggregate 0.468458 seconds and
sampling aggregate 28.896279 seconds, plus metadata/preparation/finalization.
Do not add nested durations. The complete
[retained selection and collector receipt](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/observe-route-index-20261029)
include 166 exact producer texts and two byte-exact input-receipt reconstructions
from the pinned Git tree, avoiding duplicate source identity maps. All raw samples,
timing/fixture logs, selected dependencies and the earlier Clippy-layout failure
remain lossless. Omitted duplicate source/build/meta payloads are identified.
Compiler/executable bytes and independent cleanup/usage receipts are unavailable;
those identities, settlement, deadlines and sampled resource guards remain
producer-attested. No CPU50, real-UDP/endpoint, hardware, emitted-frame, peak-stack,
allocator or whole-memory benefit follows. Existing accepted reductions and both
open CPU targets remain unchanged.
