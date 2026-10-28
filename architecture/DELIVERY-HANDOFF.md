# Delivery handoff — October 23, 2026 (updated October 24 UTC)

Accepted protocol and integration software is implemented. The remaining work
is performance implementation/qualification and the physical checks
in [PLAN.md](../PLAN.md). Both 50% server CPU targets remain open under
[#650](https://github.com/jeffglousher/coaptic/issues/650). Release publication
requires separate authorization under #132.

## Allocated-table iteration, October 27

Core #695 tests `f4585c6` against accepted `7ee2734` with identical companion
`1841076` fixtures and one frozen baseline-built socket client. Both actual
bundles are frozen before cells. Service requested pool heap is 282,640 to
282,512 bytes, with unchanged 34,752-byte inline App. Separate C1 core-client
requested storage is 37,076 to 37,060; separate Observe-client storage is 13,746
to 13,739. Do not combine different configurations or nested owners. Fixed Apps
retain the prior #693 sizes. Real pool pins, capacities, security and growth
policy stay unchanged; only three unused allocated row-pin backings are omitted.

Fault-allocator checks observe Constrained/Default startup request counts of
16/20 without bodies and 24/30 with bodies. Old counts 19/23 and 27/33 are
source-derived, not new baseline observations. Every actual failure position,
partial rollback, exact requested layouts, budget refusal and recovery passes.
263 storage plus 57 integration executions, strict release Clippy, formatting
and MSRV1.89 alloc/OSCORE checks pass. The independently reviewed actual diff is
separate from an earlier malformed private digest, caught before application.

AB Windows development runs pass in 14.69558/14.76390 seconds; a single
predeclared reversed BA pair passes in 14.27606/14.11815 seconds in execution
order. Every run covers nine reliability phases, both Observe lifecycles,
sixteen socket and six separate C1 core cells; ten exact core cells remain
unsupported. AB fixed-Service median changes are -522.750/+620.200 us for
unprotected/OSCORE; BA changes are +45.200/-247.350 us. Preserve each observation:
the signs do not repeat and no causal performance verdict follows. No replacement
cohort or CPU/latency acceptance is claimed. Preparation takes 123.13/108.48
seconds separately. The candidate four-binary set grows 2,048 file bytes.

The [allocated iteration receipt](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/allocated-table-startup-20261027)
retains arrays, owners, exact sources and both finite pairs. The original 64 MiB
compiler-output guard refusal is retained, including its inherited misleading
128 MiB error label; the script threshold is authoritative. An explicit manual
revision bounds source plus prepared copies to 128 MiB. Sampled phase-positive
release growth is 88,497,988 bytes and two bundles occupy 19,816,508 bytes; private
proof has a separate 2 MiB cap. These are not physical/transient peak guarantees.
The source/lock restore and idle-lane checks pass. Allocator/kernel/runtime costs,
emitted/peak stack, whole memory, Linux wrapper and physical checks remain open.
Both #650 CPU targets and their original acceptance gates remain unchanged.

## Bounded memory iteration, October 27

Core #693 tests source `e1e27f1` against retained `f4810d7` using the unchanged
`cc6edfb` benchmark fixtures and frozen `b9b83d8`-built socket client. The baseline
protocol, crate and Cargo inputs are identical to accepted `ba8d085`; its actual
`f4810d7` identity is retained. Device App is 57,192 to 57,184 bytes, Gateway
96,312 to 96,280, fixed Service datagram 174,088 to 173,960, and fixed Service
blockwise 314,872 to 314,744. All configured dimensions remain fixed. SmallDevice
App owners and allocated Service (34,752 inline plus 282,640 requested pool
bytes) are unchanged. The private AAD object is 72 to 40 bytes on this x64 build;
its 32-byte backing accommodates the 31-byte maximum supported encoding. This
is neither a peak-stack nor whole-memory measurement.

Release validation passes 217 fixed-storage, 136 OSCORE, 259 allocated-storage,
23 affected integration and two std-only/no-allocation test executions. These
are feature-dependent executions, not 637 distinct tests. Formatting, strict
Clippy for no-default and std/alloc/OSCORE all-targets, alloc-only checking, and
MSRV 1.89 checks without defaults and with OSCORE pass. The old const/pool pin
behavior, authentication, nonce/replay and capacities remain unchanged.

One prepared candidate suite passes in 15.40422 seconds after 122.57 seconds of
preparation: nine reliability phases, both Observe lifecycles, sixteen socket
cells and six C1 core cells. Ten exact core cells remain unsupported. The next
attempt fails common protected startup in 0.369 seconds, leaving all sixteen
cells NOT RUN. Its server log was discarded, so its cause cannot be assigned;
source inspection separately identifies a readiness bind probe racing server
startup, repaired in validation #208. The passive Mio-ledger repair passes finite
checks with both frozen actual server binaries and the same client; 91 focused
Python tests pass with one platform skip. This changes only the common proof
fixture and does not replace the old failed attempt or its timing cells. No blind
suite rerun or CPU/latency gain is claimed. The original 128 MiB compiler-output guard also refused the first MSRV
phase; that failure is retained before a documented 256 MiB aggregate continuation.
Observed phase growth totals 203,625,528 bytes; this is sampled logical growth,
not a physical/transient peak guarantee. One new prepared bundle contains
9,699,840 binary bytes. No new target/cache or dependency download was used.

The [iteration receipt](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/bounded-memory-overhead-20261027)
retains exact build/source identities, descriptive timing differences and the
failed attempt. SmallDevice alignment erases its table-field saving; no constrained
whole-App reduction is claimed. Both 50% CPU targets, emitted stack, whole memory,
remaining Linux/representative and physical qualification remain open.

## Saved work

The October 27 user direction and quick adversarial salvage pass are recorded in
[Reliable optimization across library revisions](requirements/OPTIMIZATION-ITERATIONS.md).
Useful direct-receive, borrowed-maintenance and lazy-scratch mechanisms are already
retained; the unsuccessful aggregate option-return and level-readiness candidates
remain unmerged. Companion #205 delivered the original four-profile screen,
whose Windows runs took 8.03/7.86 seconds after preparation. That version-1
evidence remains unchanged. Companion #207 expands coverage to eight fixed
configurations, sixteen socket cells in both modes, actual C2/C4 independent
endpoints and a constrained blockwise/Observe lifecycle proof. Six existing C1
core cells remain separately scoped; ten other core cells are unsupported.
The expanded Windows proof passes in 14.27/14.23 seconds after preparation;
preparation takes 96.66/98.09 seconds separately. Nine reliability phases, both
Observe lifecycles, 512 measured socket operations plus 64 warmups and six core
cells pass per run. The
[expanded receipt](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/eight-profile-suite-20261027)
identifies companion `cc6edfb` and cores `b9b83d8` / `f4810d7`; both actual bundles
were frozen before cells, with one baseline-built loopback client. Python passes
248/257 with nine skips; 123 native tests per core combine 17 current Observe
tests with 106 unchanged core/client/server tests reused from valid receipts.
Formatting and strict all-target release Clippy pass. The initial author-toolchain
check and incorrect Observe-proof outer-code guard failures remain retained;
the latter left all sixteen timing cells NOT RUN. The guard repair adds exact
Observe-dependent code and real protected-lifecycle regressions; library protocol
source is unchanged. Linux execution of the suite ownership wrapper, whole memory
and both 50% CPU targets remain open. Original
failed cohorts and separate physical qualification remain intact.

The bounded Linux level-readiness experiment has ended without integration.
Companion #202 retains the implementation at
`4f7869be939292b0fce340c58dc6729c0a2faf4c` for exact reconstruction; it is unmerged.
The [retained result](https://github.com/jeffglousher/coaptic-validation/tree/e28a52c075dcffc0b4343bdaefe82a8dff6440c8/tools/benchmark/evidence/level-readiness-20261024)
records the failed guard and full source/host/owner identities. No retry,
replacement cohort or further transport expansion is planned from this result.

The eBPF syscall implementation and two concrete prerequisite/control repairs
are delivered in companion #197/#198/#199 (latest squash
`35a86de963503cd93cd0a579f0fcfbec85ce0b00`, tree equal reviewed `4d5a211`).
[Companion #200](https://github.com/jeffglousher/coaptic-validation/pull/200)
records the completed measurement, failed overhead guards and retained earlier
failures.
The diagnostic is opt-in; protocol Rust remains identical to accepted `d23f93c`.

## Ended level-readiness comparison

[Run 40130199038, attempt 1](https://github.com/jeffglousher/coaptic-validation/actions/runs/40130199038)
uses the same accepted core `d23f93c0699b75ec17e692999d62ff89bb71463d` on both
labels, ordinary Mio versus the Linux `ppoll` receive-credit control, and one
frozen native client. SmallDevice fixed/Constrained, 32-byte GET, C1 CON,
1,000 starts/s, thirty seconds and four rotated pairs per security mode:
sixteen windows verify 480,000 measured operations plus 320 warmups, zero
operation failures. Existing offered/matched-rate, pacing, tick and cleanup
gates pass. OSCORE zero-based repeat 2 median rises from 27.431 to 29.823 us:
+2.392 us exceeds +2 us. All p99 and both pooled guards pass; they do not rescue
the failed pair. **Full-cohort CPU comparisons are withheld.** OSCORE pair tick
directions are also mixed. The predeclared stop rule ends expansion; ordinary
Mio stays the reference. This same-core adapter experiment does not replace
original `b8f44fa` acceptance or the earlier failed cohorts.

The AMD EPYC 9V74 host has four assigned logical CPUs, Linux 6.17.0-1022-azure,
glibc 2.39 and 100 Hz CPU accounting; governor unavailable. Locked Rust/Cargo
1.99.0 GNU release uses std/alloc/oscore, defaults off, thin LTO/one codegen unit,
frame pointers, debug=1/strip=none, incremental=0/jobs2. Tracing and profiling are
off. Each window has 30,020 sends including warmup; ticks span 39 to 47 and
achieved rates 1,000.0302 to 1,000.0306/s. This is paced load, not saturation.

Both Linux Apps are 14,016 bytes, including their nested adapters/scratch and
the level credit. The ordinary adapter is 2,056 bytes; the level adapter 2,060.
Ordinary Poll four bytes/Events header 24 bytes and its capacity-eight backing
are separate; the level control instead uses a separate temporary eight-byte
borrowed pollfd and adds no selector, worker or pending kernel operation. The
32-byte representation is separate. Fixed App requested heap zero is not total
heap zero. Allocator, initialization, peak stack, runtime/kernel and whole-memory
acceptance remain open; no CPU share is inferred from prior syscall counts.

Functional evidence is retained separately: Linux `40127468237` passes 104
native tests, 207 Python tests, formatting, strict Clippy, release builds and
six cells (12 measured operations plus six warmups). Windows `40129245779`
passes 96 native tests, 199 Python tests/eight skips, formatting, strict Clippy,
release builds and four cells (eight plus four), including ordinary fixed-Service
startup and authenticated 16 KiB responses. Windows App sizes are 14,032 bytes
SmallDevice and 314,872 bytes fixed Service, with separate Service representation
16,384 bytes. Earlier compile, Clippy and Windows sampler failures remain
retained. Finite proof establishes neither all kernel EINTR behavior nor peak
stack, whole memory or CPU acceptance. Both 50% targets remain open.

## eBPF syscall counts and withheld overhead

[Run 40112478137, attempt 1](https://github.com/jeffglousher/coaptic-validation/actions/runs/40112478137)
uses native core `d23f93c0699b75ec17e692999d62ff89bb71463d`, BPF build core
`1b93171035fa4b9506ec5e798ccef5d0773e1109` and companion
`35a86de963503cd93cd0a579f0fcfbec85ce0b00` (tree `3265cb3a972aef3258826c8eda56d217bf61b63e`).
It completes all sixteen SmallDevice fixed/Constrained 32-byte, ordinary-Mio,
C1 CON windows at 1,000 starts/s for thirty seconds, four rotated OFF/ON pairs
per security mode: 480,000 verified operations plus 320 warmups, zero operation
failures. Both server arms use CPU0 and identical frozen native binaries.
OFF has no tracing helper or attached programs. ON attaches disabled before
warmup, enables before CPU/run, and disables/detaches after measured snapshots
before client report formatting. Capacities remain routes four/one GET, Call
one, deferred zero, association one, bindings four, RX/TX two x 1,152 bytes,
dedup four, Observe two and no body pools; each window sends 30,020 datagrams
under the unchanged 60,000 ceiling.

All eight ON captures, normalized by verified completions, record `sendto` one
entry/success, `recvfrom` two entries with one success and one EAGAIN, and
`epoll_wait` one entry/success per operation. No individual event order was
captured. Other bins and both anomaly counters are zero. Independent gated
entry/exit counts do not imply syscall durations, vDSO-clock coverage, removable
kernel cost or a transport percentage. The window totals match the empty-receive path in source; removing it still needs a safe source design and matched evidence.

Unprotected repeat 3 fails the +2 us median guard by a delta of +2.084 us;
OSCORE repeats 0/1/2 have +2.875/+16.642/+2.514 us deltas. All pair p99 and both
pooled guards pass; pooled success cannot rescue failed pairs. Both overhead
ratios are withheld. Every diagnostic row vetoes primary CPU acceptance. No
replacement cohort or gate relaxation is planned; both 50% targets stay open.

Actual Linux validation passes 201 Python tests, three classifier tests, all
nine loader tests (seven new real-pipe regressions), strict loader Clippy and
locked helper/BPF/native builds. The workflow does not run native Rust tests or
native Clippy. Retained ACK/counter/detach and process-group settlement support
normal cleanup, without a separate system-wide final link census. Earlier
`40107158379` stopped at the undersized archive guard with zero windows;
`40109321826` verified one OFF window, then failed ON enable ACK with fourteen
later windows NOT RUN. Both failures and their reviewed repairs remain retained.

The EPYC 7763 host has four assigned logical CPUs, Ubuntu 24.04/Linux
6.17.0-1022-azure, glibc 2.39, TSC and 100 Hz process accounting; governor
unavailable. Native Rust/Cargo 1.99.0 GNU release uses locked std/alloc/oscore,
defaults off, thin LTO, one codegen unit, frame pointers, debug=1/strip=none,
incremental=0/jobs2. BPF nightly-2026-10-13 and linker 0.12.0 are separate
pinned inputs. Actual native syn3 lock input is `cb9a96e1...`, canonical restored
input `3ed89aed...`. Exact compiler/lock/binary receipts and reproduction are in
the [companion evidence report](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/ebpf-syscalls-20261024).

Map values total 2,632 bytes across four possible CPUs; both programs together
report 8,192 bytes memlock (JIT/translated lengths are nested, not additive).
Map/link metadata remains unknown. Separate helper lifetime CPU is
53,721 to 56,762 us and maximum RSS 49,220 to 51,852 KiB through its pre-final-format
snapshot, including setup/warmup/wait/control. Its 1 MiB stack limit is within
the 128 MiB address-space ceiling. These are not native-window overhead or
whole-memory requirements; global prefilter callback and kernel worker CPU
remain unassigned. App is 14,016 bytes, including adapter 2,056/scratch 2,049
and fixed pools, pool heap zero; representation 32/Poll 4/Events header 24
(capacity eight) are separate. Events heap, allocator/init/peak-stack/runtime
and kernel costs remain open. Client owners and non-atomic process observations
are retained without adding nested scratch or treating RSS as minimum RAM.

Artifact `12566918509` contains 141 text members, 3,667,819 expanded bytes,
SHA-256 `997f2065b5ac53545ebeffa814946105fb8978edeb4bb18bdc9b22e617aff9c1`.
All member CRC/SHA and immutable source records are checked; binaries are not
uploaded, so captured build hashes/control-flow freezes do not constitute fresh
collector ELF rehashes. Hosted embedded late-step states remained inconsistent;
original API responses/refusals are retained. Completed-success job/log, final
artifact, raw sixteen-window campaign and quotas corroborate completion; no
terminal API step states are invented. Whole-memory acceptance remains open.

## Earlier delivered work


- Core main includes the acceptance matrix from #674. Transient validated-option
  reuse was reviewed in [#675](https://github.com/jeffglousher/coaptic/pull/675),
  measured source `7c043046097e01e8e8f9f95fc51ee70dde3ed860`. The cumulative
  baseline comparison and incremental isolation fail OSCORE p99 guards. The
  experiment is ended and #675 closed without merging; its source remains saved.
  The current host ranking measured clean core
  `b9b83d8663cbdcb7a80bf143670a05019f25cf25`; subsequent `11cf5b6` updates a
  dependency and is outside that measurement.
- [Current comparative evidence](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/current-windows-20261023)
  preserves 2,610 cell receipts, 348,000 verified operations and all latency
  samples. It includes a standalone report, CSV, exact identities and regrading
  command. Captures containing fixture credentials and private operational
  records remain retained privately; they are not public source artifacts.
- [Companion #170](https://github.com/jeffglousher/coaptic-validation/pull/170)
  is merged as `4d65781`. The server-App CPU mode has completed focused native
  checks, independent source review and finite Linux window qualification.
  Its tested companion source is `ff46fd5`, with identical merged file bytes.
- Companion [#172](https://github.com/jeffglousher/coaptic-validation/pull/172)
  is merged as `428f906ad300ec61ad93cbedb811c80f098c67af`; #173 is merged as
  `8b25a2579fabbbb016e2798d67b076bf4022e41d`. Both squash trees match their
  reviewed/tested source trees. Delivered main was clean and pushed, and both
  settled owned branches are removed. Existing shared compiler output is retained.
- The companion's bounded C1 profile fixture extension is merged in
  [#174](https://github.com/jeffglousher/coaptic-validation/pull/174) as
  `9185c8f297801f522f71be8fa36a479d0fec9642`, with the same tree as reviewed/tested
  source `7ed5211950f413b762a4b9cd239d0a83c5b7a4cf`. Unchanged SmallDevice,
  configured SmallDevice blockwise, and fixed/allocated Service share the Mio
  loop. Focused Windows validation verifies 104 complete operations in 52 cells
  across core `b8f44fa`/`7c04304` and both modes, with refusal/recovery checks.
  Forty-five release server tests, strict Clippy and formatting pass; Python runs
  171 tests with eight platform skips. Independent review has no findings. Main
  was clean/pushed; the settled owned branch is removed. This is fixture support,
  not representative CPU qualification of current core main.
- The bounded independent-endpoint CON fixture is delivered in companion
  [#176](https://github.com/jeffglousher/coaptic-validation/pull/176), squash
  `b5d6e9a6b2691756043028b4fef8c9856cb86aaf`. Its tree matches reviewed/tested
  `d64e52b11bb36abaa15514823220280b6b1f88a5`. One client App owns C2?C4 socket
  lanes, one context/nonce sequence and four request bindings. Existing C1 inputs,
  shared-endpoint NON and server capacities stay unchanged. Each lane preserves
  NSTART=1 to the same actual server. Reports separate lifetime traffic, measured
  datagram deltas and fully verified per-lane completions; CPU eligibility also
  requires observed overlap and verified service on every lane.
- On Windows/Rust 1.99.0, #176 passes 30 client tests independently against
  `b8f44fa` and accepted protocol source `d23f93c`, strict release package Clippy
  and formatting. Python runs 173 tests with eight platform skips. Eight final
  metric-functional cells verify 24 measured operations plus 24 warmups using
  one baseline-built client. Earlier 28-cell/88-operation functionality is
  retained for the unchanged operation path. R3 independent review has no
  actionable findings; prior test defects and raw failures remain retained.
  Ordinary Service GET proof has four Calls/four client RX bodies and zero
  retained server TX bodies. Boundary tests use explicitly synthetic server
  body occupancy plus real socket refusal/retry/cancellation/recovery; they do
  not establish five real clients or four retained ordinary server snapshots.
  Shared replay-window skew beyond 32 remains an explicit counterexample.
- The #176 author released source, refs, junction and build lane after validation;
  delivered main was clean/pushed and the settled owned branch removed.
  [#178](https://github.com/jeffglousher/coaptic-validation/pull/178) delivers
  [#175](https://github.com/jeffglousher/coaptic-validation/issues/175) as
  `4d5e81af85a4401c8b2af717ef3dd74f599a0217`, with reviewed/tested tree matching
  `b87b808a4acc5dd2ce8f3898ac7d8605dad548b5`. Blocking/staged-Mio driver turns
  recover only from consumed oversized-input refusal; unrelated receive/send
  errors retain priority. No packet pool, worker or transport capability is added.
  Stage remains 32 bytes on Windows, with one transient 32-byte prior-stage
  snapshot per poll; generated peak stack remains unknown. Windows passes six
  focused and 53 full server tests, strict release Clippy and formatting.
- [#179](https://github.com/jeffglousher/coaptic-validation/pull/179) delivers
  [#177](https://github.com/jeffglousher/coaptic-validation/issues/177) as
  `8dc65fd4e09b4f9440b307085c5b9848174fe4fd`, matching reviewed
  `a79e3984434001e0a9c660a55d2677e722e82ffe`. The existing manual Linux workflow
  exposes independent-endpoint CON and runs focused endpoint/staged tests plus
  strict native Clippy before measurement. Default C1 behavior and all gates
  remain unchanged. Actionlint and exact input-validation cases pass. Both batches
  have no actionable independent review findings, are pushed, and their settled
  owned branches are removed. Linux proof is recorded below.
- PR watches and recurring tasks are off. The finite
  [incremental comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/40045613509)
  has completed: current main `d23f93c` versus `7c04304`, same Mio,
  64/1024 bytes, 4,000/s, twelve seconds and four rotated pairs, profiling off.
  It isolates #675 and fails one pair p99 guard; the original-baseline failure
  below remains retained. Disposition review found no supported correction; no further
  campaign is scheduled.

## Delivered measurement support and Linux qualification

Companion [#184](https://github.com/jeffglousher/coaptic-validation/pull/184) is
merged as `430722d73818e6b23ceecc6e6edb567a20111958`, tree-identical to reviewed
`8274bf004ee2a6b78d8610de94c165b9eea7aef8`. Explicit long partitions select one
representative profile/payload/rate/security mode, ordinary Mio on both labels,
C1 CON, four rotated pairs/eight clean windows, 31..1500 seconds and at most
12,000 scheduled clean seconds. Perf, admission diagnostics and independent
endpoints are off. Default short/both-mode operation remains. The complete
operation deadline stays 3000 ms and the one sender counter still charges all
warmup, measured, retry and control datagrams against 60,000. Every window,
every pair AND pooled stratum retain their original guards. One absolute long
measurement wait is `ceil(D/0.95)+13`, capped at 1600 seconds; default/long job
ceilings are 60/360 minutes. A single-security partition is partial evidence.
The 256 MiB whole-upload plan now includes actual copied binaries, bounded
metadata/profiles and a combined 16 MiB setup-log reserve. These are retained-file
bounds, not build-cache, transient-disk or memory peaks.

Retained Windows checks reconcile 290 Python passes/22 platform skips, 33 native
client tests, strict release package Clippy, fmt and actionlint. Native checks use
Rust/Cargo 1.99.0, MSVC, release thin LTO/one codegen unit, debug=0, incremental=0
and two build jobs. Locked original
`b8f44fa` and accepted `d23f93c` builds plus eight short functional cells verify
176 operations including warmups, using one baseline-built client. These Windows
checks do not qualify long Linux behavior; the first finite Linux result follows.

Companion [#186](https://github.com/jeffglousher/coaptic-validation/pull/186) is
merged as `c1fa5bf8e08a0811e187cefb61ee1dc1b1db5059`, tree-identical to reviewed
`75a37d1ee12f8c56d87cb25d12d158a567d9ff52`. Existing before/after process snapshots
attempt Linux status/limits observations after their original CPU/memory reads:
approximate lifetime resident/virtual high water, stack mapping, thread count and
configured stack limits. Reads are bounded to 16 KiB and compact metadata to
1 KiB per process snapshot; parent temporary byte payloads stay below 32 KiB,
with Python object/allocator costs separate. No new sampler or worker is added.
Missing/refused/malformed reads are explicit unavailable data. These non-atomic
observations include startup/warmup and cannot prove used stack, physical peaks,
minimum RAM or an aggregate ceiling. Boundary work can affect raw resource
windows; all CPU/latency gates remain. Windows runs 178 unittest cases: 170 pass,
eight skip. Independent general reviews for both PRs have no actionable findings.
The limited Linux proof follows; whole-memory acceptance remains open.

### Qualified low-load SmallDevice result

[Run 40065560596](https://github.com/jeffglousher/coaptic-validation/actions/runs/40065560596),
attempt one, uses companion `430722d73818e6b23ceecc6e6edb567a20111958`, original
`b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and candidate
`5bd6f272cc2b3dc993e1eaa2d34eee025f918773` (protocol-identical to accepted
`d23f93c`). Both labels use ordinary Mio and the fixed SmallDevice configuration
specified below: one populated GET/four route capacity, one Call, zero deferred,
one association/four request bindings, RX/TX each two by 1,152 bytes, dedup four,
Observe two and no body pools. Responses are 32 bytes, unprotected C1 CON,
100 starts/s, 500 seconds and four rotated pairs. Perf, admission diagnostics
and independent endpoints are off.

All eight windows verify 400,000 measured operations plus 160 warmups, with zero
operation failures. Every window, pair AND pooled guard passes. Each window has
243..252 server ticks at 100 Hz, zero missed starts and exactly 50,020 total
client sends including warmups, within the unchanged 60,000 ceiling. Achieved
rate is 100.001949..100.001964 operations/s. This paced rate does not establish
saturation or CPU-limited throughput.

Baseline/candidate server ticks are 1,004/973 over 200,000 completions per label:
**50.20 to 48.65 microseconds/op**, ratio 0.969123506 (-3.09%). User/system
components are 273/731 to 284/689 ticks; these counters do not nominate a causal
hotspot. Client CPU is separately 123.3 to 121.9 microseconds/op. Pooled median is
100.869 to 99.476 microseconds and p99 131.937 to 128.790. Pair median deltas are
-1.532/-1.172/-1.885/-1.282 microseconds; pair p99 deltas are
-2.144/-2.234/-6.522/+0.951. Process-clock sidecars have separate boundaries and
do not rescue eligibility or replace ticks. No optimization or transport-cost
percentage is inferred from socket-free App timings.

The host is AMD EPYC 7763, four assigned logical CPUs, Ubuntu 24.04/Linux
6.17.0-1022-azure, glibc 2.39, TSC and 100 Hz accounting; governor data is
unavailable. This differs from the EPYC 9V74 of the 1,000/s result below. Do not
combine their costs, gains or clocks, or infer a load curve across hosts.
Rust/Cargo 1.99.0/LLVM 23.1.1 build release `--locked` for
`x86_64-unknown-linux-gnu`, thin LTO/one codegen unit, debug=1, strip=none,
forced frame pointers, incremental=0 and two build jobs; core features remain
`std,alloc,oscore` with defaults disabled. Source-specific syn-2/syn-3 lock inputs,
compiler/flags and clean restoration match.

The frozen baseline-built client SHA-256 is
`c61bb04ca2de20424274a13a67f8c95066137f415fa5e6087f06a270ada337f2`.
The actual baseline/candidate server hashes are the same as the retained
`40054024038` ELFs below; this equality does not make the hosts or workloads
equivalent. Linux setup runs 312 Python tests: 310 pass/two skip. Ten selected
endpoint and 37 staged tests, strict release/locked/all-target package Clippy,
mixed-UID/live-thread cleanup and real launcher success/timeout proofs pass.
This does not claim a whole-client test run or new long-window unit execution.

Startup App is 14,016 bytes including the 2,056-byte adapter and its 2,049-byte
scratch; fixed requested-pool heap is zero. Poll 4 bytes, Events header 24 bytes,
the pinned-source 96-byte Events payload request and 32 static representation
bytes are separate as described below. Snapshot RSS spans 4,702,208..5,103,616
bytes, not a peak requirement. No server worker or pending kernel operation is
introduced; attributable process user/system CPU is included, while machine
kernel-worker cost remains unmeasured. Allocator, initialization, runtime,
kernel owners and peak stack still prevent whole-memory acceptance.

The retained ZIP is 22,089,530 bytes, SHA-256
`50b7fa3282f7ecefa052fe008618bdda3507339d2a50eb31240572b37c1dafe7`.
Parent verification independently reproduces all 60 member hashes/CRCs, 24
source-file/Git-blob matches, raw counts/ticks and all latency guards. Four ELFs
are streamed as data, never extracted or executed. Uncompressed upload is
82,940,019 bytes within the 256 MiB whole-upload bound; retained partition output
is about 23.2 MB within its existing 30 MiB cap. The initial SHA-ref dispatch
refusal created no job and remains retained; the corrected verified-main request
created the sole measured attempt. This qualifies one unprotected slice, fails
the required 0.5 CPU ratio, and replaces no historical failed cohort.

Reproduce from clean companion `430722d` with the pinned compiler/flags above,
`psutil==7.0.0`, an existing compatible lane and fresh bounded output:

```sh
python3 -B tools/benchmark/linux_cpu.py \
  --baseline-sha b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 \
  --candidate-sha 5bd6f272cc2b3dc993e1eaa2d34eee025f918773 \
  --server-kind coaptic-mio --resource-profile small-device \
  --sizes 32 --rates 100 --seconds 500 --repeats 4 \
  --long-window --security-mode unprotected --concurrency 1 --message-type con \
  --build-lane /existing/compatible/lane --output /fresh/small-device-long
```

### Failed low-load OSCORE partition

[Run 40073484846](https://github.com/jeffglousher/coaptic-validation/actions/runs/40073484846),
attempt one, uses companion `c1fa5bf8e08a0811e187cefb61ee1dc1b1db5059`, original
`b8f44fa` and accepted `5bd6f27` (protocol-identical to `d23f93c`). It selects
the same SmallDevice fixed32/Mio/C1 CON/100 starts/s/500-second/four-pair plan
above, with OSCORE alone and profiling, admission diagnostics and independent
endpoints off. All eight windows verify 400,000 operations plus 160 warmups,
zero operation failures, zero missed starts and 50,020 sends per window. Every
window passes source, compiler, selected-lock, pacing, tick and cleanup gates.

Latency acceptance fails. Zero-based repeat one has median +6.6925 microseconds
and p99 +20.238; repeat three has median +15.931. Pooled median is +5.732 and
p99 +5.410 microseconds. The unchanged +2/+10-microsecond guards apply to every
pair AND pooled stratum. All CPU comparisons are withheld. Raw server ticks
are 1,303/1,258 and separate client ticks 3,192/3,210 over 200,000 completions per
label; these are retained accounting, not an accepted reduction. No retry,
cohort replacement or timing-cause claim follows.

The host records EPYC 7763/four assigned logical CPUs, Ubuntu 24.04/Linux
6.17.0-1022-azure, glibc 2.39, TSC, 100 Hz accounting and unavailable governor
data. It is a separate run/host boundary from the unprotected result, despite
the same CPU model. Compiler/flags, source-specific locks and the actual server
and frozen baseline-client hashes match those recorded above. Linux passes
317 Python checks with two skips, ten endpoint tests, 37 staged tests and strict
release/locked/all-target package Clippy, including the actual launcher and
mixed-UID/live-thread cleanup proof. Dependency warnings remain retained.

All 32 process-memory observations are available and PID-coherent. Server
lifetime VmHWM is 4,870,144..5,210,112 bytes, VmPeak 9,605,120..9,625,600 and
VmStk 241,664..249,856. These approximate, non-atomic observations include
startup/warmup and do not establish used stack, minimum RAM or whole-memory
acceptance. Startup App remains 14,016 bytes with nested adapter/scratch/fixed
pools and zero requested pool heap; separate Poll/Events/static owners and
unknown allocator/runtime/kernel/initialization/peak costs remain as above.

The remote ZIP is 22,219,844 bytes, SHA-256
`0f8c925dfa5a46aa7531b843159949e30503febdb35b3c993eaf328420086843`,
uncompressed 83,124,165 bytes. The collector checks all sixty member CRC/SHA
values, streaming four ELFs as data without saving or executing them. Parent
verification independently checks all 56 retained text members, 24 source
Git blobs, raw 400,000 latency samples, ticks and failed guards; it reuses the
collector's remote archive/ELF hashes. Retained output is 3,842,623 bytes within
8 MiB, with no third local ZIP/ELF/perf artifact. Both CPU targets, this cell and
the remaining matrix stay open. Reproduce the preceding command from companion
`c1fa5bf`, selecting `--security-mode oscore` and a fresh output directory; the
recorded failed cohort remains unchanged.

### Fixed-Service blockwise partition

[Run 40081182117](https://github.com/jeffglousher/coaptic-validation/actions/runs/40081182117),
attempt one, compares original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`
with accepted `d23f93c0699b75ec17e692999d62ff89bb71463d`, using companion
`73216630df57496e50bcc53189301cb74890f2e6`. Both labels use ordinary Mio and
the same frozen baseline-built client. The fixed Service profile serves complete
16,384-byte blockwise representations, C1 CON, 50 starts/s, 74 seconds and four
rotated pairs. Profiling, admission diagnostics and independent endpoints are off.
All eight windows verify 29,600 unprotected operations plus 160 warmups, with
zero operation failures. Complete bodies and metadata verify; this unprotected
partition makes no OSCORE authentication claim. Sources, selected locks,
compiler, capacities, rate, pacing, CPU resolution and cleanup pass.

Acceptance fails. Pair median deltas in zero-based repeats 0/1/3 are
**+93.4885/+27.681/+32.200 microseconds**; pooled median is +3.154. Repeat two
median passes at -84.216. All pair and pooled p99 guards pass. Pooled median
is 748.8985 to 752.0525 microseconds and p99 1,145.009 to 1,113.309. The existing
pair AND pooled +2/+10-microsecond guards remain unchanged. **All CPU ratios
are withheld.** Raw server ticks total 702 to 692 and separate client ticks
1,250 to 1,238 over 14,800 completions per label; these are accounting, not an
accepted reduction or causal cost ranking. There is no rerun or replacement.

Each window completes 3,700 measured operations and twenty warmups, using
59,520 total client sends including the sixteen Block2 requests per operation.
The live 60,000-send ceiling retains 480 sends of headroom for retries/controls;
it is not enlarged. Zero starts are missed, achieved rates span
50.0127465..50.0130277/s, and server CPU is 165..180 ticks/window at 100 Hz.
Paced completion does not establish saturation or maximum throughput. The
3,000 ms complete-operation deadline and 91-second measurement wait are retained.

The host records EPYC 7763, four assigned logical CPUs, Ubuntu 24.04/Linux
6.17.0-1022-azure, glibc 2.39, TSC, 100 Hz accounting and unavailable governor
data. This is a separate host/run boundary from every other result, even where
CPU model strings match. Rust/Cargo 1.99.0/LLVM 23.1.1 build release `--locked`
for `x86_64-unknown-linux-gnu`, thin LTO/one codegen unit, debug=1, strip=none,
forced frame pointers, incremental=0 and two jobs; `std,alloc,oscore` features
with defaults off. Source-specific syn-2/syn-3 locks and clean restoration match.
Linux passes 317 Python tests/two skips, ten endpoint and 37 staged tests,
strict release/locked/all-target package Clippy and actual mixed-UID/live-thread
and production-launcher success/timeout cleanup proofs. The ten endpoint tests
include the path-included ownership, replay, cancellation and Service-boundary
proofs; they are not merely adapter tests or a whole-client-suite claim.

Both actual Linux App owners are **314,856 bytes**, including fixed pools,
security, the 2,056-byte adapter and its 2,049-byte scratch. Requested pool heap
is zero. Service capacities are routes 32/one populated GET, Calls 4, deferred
8, association 1/request bindings 4, RX/TX each 32 x 1,472 bytes, dedup 64,
Observe 32 and RX/TX bodies each 4 x 16,384 bytes; blockwise is on. The nested
225,280-byte packet/body payload total is already inside App. Separate owners
are 16,384 requested representation bytes, Poll 4 bytes and Events header 24
bytes/capacity eight. The pinned-source 96-byte Events payload request described
below remains separate; allocator/runtime/kernel/initialization and peak-stack
costs remain unresolved. No worker or pending kernel operation is introduced.

The baseline client used for both labels reports App 15,448 bytes including
its 1,473-byte scratch, requested core heap 37,076 bytes, verification storage
32,768 bytes, timing storage 29,600 bytes and pending metadata 288 bytes. These
client costs are separate from the server. All 32 process snapshots are available
and PID-coherent. Server lifetime VmHWM spans 5,627,904..5,922,816 bytes, VmPeak
10,297,344..10,321,920 and VmStk 933,888..946,176, with one observed thread and
a 16 MiB soft stack limit/hard unlimited. Approximate non-atomic lifetime/mapping
observations do not establish used stack, peak RAM or minimum requirements.

Actual baseline/candidate server ELF SHA-256 values are
`5c4730ddc7e72eaeb05cfd50ee5647f75005cdbf00fc774c181a39ca71a968dc`
and `48b0432fa03cf14beadcc54db271848c9482c0903ee659409a9881f17481d1a2`.
The frozen baseline client is
`c61bb04ca2de20424274a13a67f8c95066137f415fa5e6087f06a270ada337f2`.
The remote archive is 21,201,546 bytes, SHA-256
`451161431758a5275c83d3fd7e44a6bdd9b29a8d2986a8ada2d1656eb70fc582`,
with sixty members and 80,215,926 uncompressed bytes. The collector verifies all
member CRC/SHA values, including four ELFs streamed as data. Parent independently
rehashes all 56 retained text members and 24 source Git blobs and reproduces raw
latency/count/tick/capacity/memory checks (614 checks); remote archive/ELF hashes
are transparently reused. A private verifier's initial metadata-type assumption
was corrected with its failure retained; no evidence, source, gate or benchmark
attempt changed. Sealed output `service-fixed16384-u50-74s-40081182117` under the
existing evidence root is 936,498 bytes within 2 MiB, with no local ZIP/ELF/perf.

The first required OSCORE counterpart,
[run 40083610413](https://github.com/jeffglousher/coaptic-validation/actions/runs/40083610413),
attempt one, passes all eight windows on the same declared sources, workload
and unchanged guards. All 29,600 complete responses plus 160 warmups are
authenticated and verified by the frozen baseline-built client. Sources,
selected locks, capacities, pacing, ticks, every pair/pooled latency guard and
cleanup pass. Server CPU is **610.811 to 561.486 microseconds/op**, ratio
**0.919247788 (-8.08%)**. This exceeds the required 0.5 ratio; both 50% targets
remain open. This is original-baseline versus accepted source, not an isolated
optimization experiment. No confidence interval or repeatability claim follows.

Server user/system ticks are 332/572 to 357/474 (904 to 831 total), over 14,800
completions per label. Separate client totals are 1,520 to 1,399 ticks. Pair
median deltas are -83.5265/-205.9915/-28.640/-8.549 microseconds; p99 deltas
-28.102/-50.241/-54.360/-126.542. Pooled median is 894.332 to 849.8195
microseconds and p99 1,421.318 to 1,366.653. All starts are met, each window
uses 59,520 sends and has 201..238 server ticks. This paced rate is not a
saturation result. Tick quantization, scheduling and machine kernel-worker
uncertainty remain visible; counter components identify no causal hotspot.

This second host also reports EPYC 7763/four logical CPUs, Linux
6.17.0-1022-azure/glibc 2.39/TSC/100 Hz and unavailable governor data. It remains
a separate run/host boundary from the failed unprotected result; do not combine
their costs or infer a security/transport-cost percentage. Compiler/build/lock
inputs, all four actual ELF hashes and preflight coverage match those above,
with the binary hashes freshly checked in this second artifact. Both startup
App owners again measure 314,856 bytes with zero requested pool heap and separate
16,384-byte representation backing. All 32 snapshots are available/PID-coherent:
server lifetime VmHWM 5,713,920..6,045,696 bytes, VmPeak 10,301,440..10,321,920
and VmStk 937,984..946,176. Owner nesting and all unresolved peak/runtime/kernel
and whole-memory limits remain unchanged.

The OSCORE remote archive is 21,201,183 bytes, SHA-256
`aec1eb5acc3b599b292dd5d894bae5baab598f75f94f02b42ba1d96a1684b501`,
sixty members and 80,220,019 uncompressed bytes. All member CRC/SHA values,
including four streamed ELFs, are checked anew by the collector. Parent
verification independently rehashes 56 retained text members and 24 source Git
blobs and reproduces all 29,600 raw samples, guards, ticks, capacities and
memory observations (617 checks); remote archive/ELF stream hashes are reused
transparently. Sealed `service-fixed16384-os50-74s-40083610413` output is 942,484
bytes within 2 MiB, with no local ZIP/ELF/perf. No retry or replacement occurred.

Both targets, the failed unprotected acceptance cell, other representative cells
and whole-memory acceptance remain open. The bounded Service core probe is now
delivered in companion #191 and its explicit Linux workflow in #192; the
functional proof and its limits are recorded below. It cannot rescue socket
guards or establish a transport-cost percentage.
Reproduce from clean companion `7321663`, the compiler/flags above and a
compatible lane. The command below selects the unprotected cohort; select
`--security-mode oscore` and another fresh output for its separate counterpart:

```sh
python3 -B tools/benchmark/linux_cpu.py \
  --baseline-sha b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 \
  --candidate-sha d23f93c0699b75ec17e692999d62ff89bb71463d \
  --server-kind coaptic-mio --resource-profile service-fixed \
  --sizes 16384 --rates 50 --seconds 74 --repeats 4 \
  --long-window --security-mode unprotected --concurrency 1 --message-type con \
  --build-lane /existing/compatible/lane --output /fresh/service-fixed-long
```

### Service complete-operation core probe

Companion [#191](https://github.com/jeffglousher/coaptic-validation/pull/191)
delivers explicit fixed/allocated Service modes in the existing `bench-core`
executable. Merge `c7887f6b56fe37cbc8d25ad9402a02a34c9a22ea` matches reviewed
tree `6b721c7986272ef147e29eae5d775d183abe5451`. Companion
[#192](https://github.com/jeffglousher/coaptic-validation/pull/192) delivers the
opt-in `service-proof` Linux workflow as
`7a3214e311c0e63949e9514a4d2e2ed4f17b2e75`, matching reviewed tree
`31982ab7c91899b05010703c0330dcd797a806ae`. Both independent general reviews
have no findings. Probe Git blob `dbffc02293898336087a640cd4b61cca5eb70d5e`
has Linux LF SHA-256
`5ff878db5d1f0eadc0e85da789cb12fe1e5508146874d5440f07d60f147051c6`.
The earlier #187 proof remains pinned to its original probe and suite.

Two real Apps exchange complete C1 CON representations through two bounded
single-packet ports. Server capacities retain Service's 32 routes/one GET,
four Calls, eight deferred rows, one association/four bindings, RX/TX each
32 x 1,472 bytes, dedup 64, Observe 32 and RX/TX bodies each 4 x 16,384 bytes.
The separate allocated client has four Calls/one active, zero deferred rows,
one association/four bindings, RX/TX each 4 x 1,472 bytes, dedup eight, Observe
four, one 16,384-byte RX body and one 1,024-byte TX body. Each App owns its
OSCORE state; contexts are not cloned between operations.

Optimization barriers and observable full-body results protect the probe.
Each wire ACK, MID and Token is checked against its request. Assembled Block2
metadata retains the first fragment's header/MID; final collection is checked
against that snapshot, including authenticated content, code, format and ETag.
The operation has an absolute three-second deadline and at most 32 rounds.
Both ports share the 60,000 accepted-send ceiling, including 128 warmups,
continuations, retransmissions and control. At 16 KiB the measured-operation
ceiling is 1,747; configured storage is allocated at startup and reused.

The measured quantity is the sum of every server `App::poll` wall interval per
verified complete operation. Server fixture receive/send copies, dispatch,
maintenance, body work and cryptography are inside. Bridging, client preparation,
polling, protection, authentication, assembly and verification are outside.
The 4,096 clock calibrations per process remain separate and are never subtracted.
These intervals are not process CPU, socket latency or a transport-cost share.

Both pinned Windows builds (`b8f44fa`/`d23f93c`, Rust 1.99.0, locked release)
pass sixteen native tests and strict binary-target Clippy each. Both actual
executables and their inputs freeze before eight cells: both sources and modes
with fixed/allocated 16 KiB storage, sixteen measured operations and 128 warmups
per cell. All 1,152 exchanges verify, with 32,768 separate calibrations. Formatting
passes. Legacy Default64/Small32 and batch code/tests retain their recorded
source regions; the earlier proof is not retroactively repinned.

[Linux run 40090187411](https://github.com/jeffglousher/coaptic-validation/actions/runs/40090187411),
attempt one, passes all eight cells with no operation failures or NOT RUN cells:
128 measured operations plus 1,024 warmups, 32,768 separate calibrations,
18,432 server polls and 36,864 combined accepted sends. Both cores pass sixteen
unfiltered native release tests and strict binary-target Clippy. Python runs
eighty tests: 75 passes and five skips. Both actual copied ELF/compiler/input/
selected-lock identities freeze before cells. Captured records and immutable
control flow establish that order, without an independently timestamped trace.
All eight owned-process settlement receipts and source/lock restoration pass.
Arbitrary escaped groups and abrupt-job-death restoration are outside that proof.

Linux uses the same pinned b8/d23 pair, Rust/Cargo 1.99.0/LLVM 23.1.1 and GNU
target with locked release, thin LTO, one codegen unit, debug=1, strip=none,
forced frame pointers, incremental=0 and two build jobs; features are
`std,alloc,oscore`, defaults off. CPU model, kernel and affinity are not captured
by this functional workflow and must not be inferred from another host. Original
ELF SHA-256 is `2df1c00a2bbbb9aa049fb9fa7a01bfbb0eff62b1d81e331158725eba6dd39560`
(7,778,576 bytes); accepted is
`b946498bb6c27b9954b4fcc58b416b24c8b8603139e1443949fb7ab6d044835e`
(7,792,264 bytes). No cross-build binary-reproduction assumption is needed.

Actual GNU owners match across both cores and security modes for each storage
choice. Fixed server App is 315,784 bytes with zero requested pool heap;
allocated App is 35,664 bytes plus 282,640 requested pool bytes. The client App
is 16,936 bytes plus 37,076 requested pool bytes. Each App includes a 2,984-byte
port containing 1,472-byte input and output buffers: four nested packet buffers
in total. Server pool payload 225,280 bytes and client payload 29,184 bytes
are already inside their App or requested-heap owners and must not be added.
Separate owners are static representation 16,384 bytes, verification request
16,384, timing request 256, poll counts 64, calibration request 65,536, Vec
headers 96, pending state 80 and clock 16. Requested pool heap includes pool
metadata/alignment, with allocator overhead unresolved. These are this fixture's
actual GNU owners, not Mio equivalence, aggregate RAM requirements or peaks.

The 4,456,645-byte remote archive has fifty members and SHA-256
`ec59556f38c73f6805da43c912cf84721e362836328520e5dfea16b3fe22a0dd`.
The collector checks every member CRC/SHA, including both streamed ELFs as data.
Parent verification independently rehashes all 48 retained text members and
twenty source blobs and reproduces raw counts, scopes, owner matching,
restoration and descriptive wall statistics (483 checks); remote archive/ELF
stream hashes are reused transparently. No binaries are extracted or executed
during receipt review. Sealed output `linux-service190-40090187411` retains
980,393 bytes within its 2 MiB budget; whole-upload and existing phase bounds
pass. Software/workflow evidence remains in `service-core-probe-190-20261024`
and `linux-service-probe190-integration` under the existing evidence root.

This completes the narrow Windows/Linux functional probe obligation in #190.
Fault/cancellation tests use allocated 16 KiB fixtures; identical fixed-mode
fault coverage and actual continuation deadline expiry remain unproved.
Finite no-growth observations do not establish generated peak stack, allocator,
initialization or runtime bounds. Both CPU targets, whole-memory acceptance and
failed socket cohorts are unchanged. No CPU ratio, gain, transport share or
socket guard is established by these short wall diagnostics.

From the recorded source-specific locked build, run each command for both cores:

```sh
bench-core 16 unprotected --service-fixed 16384
bench-core 16 oscore --service-fixed 16384
bench-core 16 unprotected --service-allocated 16384
bench-core 16 oscore --service-allocated 16384
```

For hosted functional reproduction, select a branch at recorded companion
`7a3214e311c0e63949e9514a4d2e2ed4f17b2e75` and pass its branch name to
`gh workflow run core-cost-probe.yml --repo jeffglousher/coaptic-validation --ref BRANCH -f mode=service-proof`.
The original dispatch used `main` at that exact commit. Record the actual run
SHA; a later `main` is a different input. This selects the frozen eight-cell
functional plan, without replacing any failed acceptance cohort.

### Limited Linux memory proof

[Run 40066937573](https://github.com/jeffglousher/coaptic-validation/actions/runs/40066937573)
uses companion `c1fa5bf8e08a0811e187cefb61ee1dc1b1db5059` and identical core
`37c3389bd560638e92b0b76243428246a4b54d76` on both labels (protocol-identical to
`d23f93c`). It ran after the long partition in the unchanged serial concurrency
group. Ordinary Mio, fixed SmallDevice 32 B, both modes, C1 CON, 100/s, two
seconds and two pairs verify 1,600 measured operations plus 160 warmups, zero
operation failures and resolved cleanup. This is a functional control, not an
optimization experiment.

All 32 actual `coaptic-proc-memory/1` objects have available status/limits data,
strict numeric/unit fields and PIDs coherent with the recorded owned server/client
identities. Server approximate lifetime VmHWM is 4,833,280..5,038,080 bytes,
VmPeak 9,601,024..9,613,312 and VmStk 237,568..249,856. Client values are
2,871,296..3,026,944, 3,997,696 and 139,264 bytes respectively. All snapshots report
one thread and configured stack soft limit 16,777,216 bytes/hard `unlimited`.
These non-atomic observations include startup/warmup. VmStk is a virtual stack
mapping, not used stack; limits are configuration, not allocation. One stable
functional path does not fully prove dynamic-thread, exit, permission or race
behavior, physical peaks or whole-memory ceilings.

All eight server tick deltas are zero and fail the twenty-tick floor. Rate/pacing
guards pass with no missed starts, but OSCORE repeat zero median rises 6.0445
microseconds, failing the +2-microsecond guard. All pair p99 and pooled latency
guards pass; they do not erase that failure. All CPU comparisons and gains are
withheld. The separate process clocks do not rescue the floor, and identical
core labels establish no optimization even if timing guards were to pass.

This host is EPYC 9V45 with four assigned logical CPUs, Linux 6.17.0-1022-azure,
glibc 2.39 and 100 Hz accounting; governor data is unavailable. Compiler, build
flags and capacities match the declared fixture, not the hardware of the long
run. Linux setup runs 319 Python tests: 317 pass/two skip; ten endpoint and 37
staged tests and strict release/locked/all-target package Clippy pass. Dependency
warnings remain retained. Both labels build server hash
`48b0432fa03cf14beadcc54db271848c9482c0903ee659409a9881f17481d1a2` and client
`97dd6b1baa2b40466e8acd08e171dddce2c4c82bfc23783ed37167d8d9f3956f`.

All 253,877 bytes of raw nonbinary artifact text are retained locally with source,
build, snapshot and failure receipts. The remote archive is 21,086,753 bytes,
SHA-256 `e91e09b68c2c38fe615be131cd28c54115a663cefe8a025f8eaeee77cb8095d8`;
all 60 members were streamed for CRC/SHA verification. No local ZIP/ELF/perf is
saved for this proof; executable bytes remain only in the remote artifact's
fourteen-day retention. Collector archive byte payloads are bounded to
three times 26 MiB plus a 1 MiB chunk, with Python/JSON/runtime overhead separate.
Those offline collection bounds do not measure server RAM or CPU. Total retained
proof metadata is under 2 MiB, inside the unchanged 64 MiB evidence phase.
The older long run predates #186 and contains none of these new memory fields.

Reproduce this functional control from clean companion `c1fa5bf`, using the same
setup as above, both core SHA arguments set to `37c3389bd560638e92b0b76243428246a4b54d76`,
`--sizes 32 --rates 100 --seconds 2 --repeats 2 --concurrency 1 --message-type con`,
the same Mio/SmallDevice options and fresh output. Omit `--long-window` and
`--security-mode unprotected`; default `both` is required for this proof.

Both 50% targets, the remaining representative matrix and material memory
unknowns remain OPEN. Exact software
limits and command options are in the source-identified companion
[guide](https://github.com/jeffglousher/coaptic-validation/blob/c1fa5bf8e08a0811e187cefb61ee1dc1b1db5059/tools/benchmark/linux_cpu.md).

## Retained baseline memory-owner caps

The retained startup-ledger consolidation covers 28 configuration rows and 112
raw observations from 55 hashed sources. Freeze the following original `b8f44fa`
**owner-only no-regression caps** before future candidate qualification, under
the same declared target/compiler, adapter, full capacities and workload. They
are host-fixture owner references, not aggregate ceilings or device RAM needs.

For x86_64 Windows/MSVC with Rust/Cargo 1.99.0, release thin LTO/one codegen unit,
debug=0, incremental=0 and two jobs, the measured App-including-transport sizes
and source-calculated requested pool layouts are:

- SmallDevice fixed: App 14,032 bytes; pool request zero.
- Explicit SmallDevice blockwise: App 27,760 bytes; pool request zero.
- Service fixed: App 314,872 bytes; pool request zero.
- Bounded Service allocated: App 34,752 bytes; pool request 282,640 bytes.

The capacities are exactly those in the
[representative matrix](requirements/PERFORMANCE-ACCEPTANCE.md#representative-execution-matrix).
All four App owners include the 2,072-byte adapter and its 2,049-byte scratch.
Poll (8 bytes) and Events header (40 bytes, capacity eight) are separate;
Windows Events payload/allocator costs remain unknown. Fixed pool payloads and
metadata are inside App; allocated pool payloads and metadata are inside the
requested layout, not additional owners. Nested packet/body payload totals are
4,608/0 bytes for SmallDevice, 4,608/8,192 for explicit blockwise, and
94,208/131,072 for both Service storage choices. The allocated figure is a checked
layout request, not allocator consumption. Separate representation backing is
32 borrowed static bytes for SmallDevice, otherwise the selected representation
size (64/1024/4096 or 64/1024/16384 bytes) in the fixture's process-lifetime boxed
slice. Initialization and transient construction costs remain unresolved.

The Windows baseline rows come from the #174 C1 fixture freeze at companion base
`8b25a25`; its old candidate is failed `7c04304` and is excluded from accepted
claims. Actual accepted `d23f93c` SmallDevice/blockwise figures are independently
observed in #184's short compatibility cells, and Service figures in #176's CON
receipts at base `9185c8f`. Those accepted observations match the above owners;
they do not qualify their CPU or whole-memory requirements. Host-control pool
requests vary with configured body capacity: 23,462 bytes for 1024-byte body
pools versus 216,998 for 65536-byte pools. A single host row cannot cover both.

On Linux/GNU, qualified run `40054024038` supplies only SmallDevice fixed:
App 14,016 bytes, pool request zero, Poll 4 bytes and Events header 24 bytes,
with 32 static representation bytes. Its separate 96-byte Events payload is a
pinned-source requested-allocation bound, not measured allocator use. The next
section records exact sources/compiler/flags. Run `40081182117` additionally
supplies matched Linux fixed-Service App 314,856 bytes and zero requested pool
heap, with separate 16,384-byte representation backing under its exact declared
configuration above. Linux SmallDevice blockwise and allocated Service owner
caps remain OPEN; Windows values do not establish them. Do not add
nested scratch/pools again, or add stack/image residents again to their enclosing
reservation/image. Material allocator, initialization, runtime, kernel and peak
requirements still prevent aggregate/whole-memory acceptance.

Exact source/binary/configuration mappings and raw row selectors are retained in
`retained-owner-ledger/ledger.json`, SHA-256
`92b10f872ad75a52579e4afda1d35fda3cd93e99ba4b7ed105ec0d0bfb2fd9ec`,
under the existing evidence root. Parent verification reproduces all 55 source
hashes and all 112 owner/capacity/representation observations. This consolidation
runs no build, benchmark or new diagnostic and retains all original failures.

## First representative SmallDevice C1 result

[Run 40054024038](https://github.com/jeffglousher/coaptic-validation/actions/runs/40054024038)
uses companion `8dc65fd4e09b4f9440b307085c5b9848174fe4fd`, original baseline
`b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and accepted protocol source
`d23f93c0699b75ec17e692999d62ff89bb71463d`. Later core main changes through
`907fb72` are documentation only. Both labels use Mio and the baseline-built
client `fe46f5e0ee2737441cb9b925064d6806dcca5c87064f085a796c09b64a3a6baf`.
Source-specific syn-2/syn-3 locked builds and clean restoration pass.

The unchanged fixed SmallDevice/Constrained configuration has route capacity four
(one GET), one Call, zero deferred rows, one association/four request bindings,
RX/TX each two by 1,152 bytes, dedup four, Observe two and no body pools. Complete
32-byte responses use C1 CON at 1,000 starts/s, thirty seconds and four rotated
pairs per mode, with profiling/admission diagnostics off. All sixteen windows
verify 480,000 measured operations plus 320 warmups with zero failures. Every
operation, source, rate, pacing, twenty-tick, cleanup and pair/pooled latency guard
passes. Missed starts range from zero to two and server ticks from forty to
forty-nine per window. Achieved throughput is 1,000.0301–1,000.0305 operations/s;
this paced workload does not establish CPU-limited throughput or saturation.

Unprotected server CPU is 172 to 164 pooled ticks over 120,000 completions per
label: 14.333 to 13.667 microseconds/op, ratio 0.953488 (-4.65%). OSCORE is
191 to 185 ticks: 15.917 to 15.417 microseconds/op, ratio 0.968586 (-3.14%).
Each mode's final pair has zero tick improvement. Pooled median/p99 latency is
25.677/35.262 to 24.896/35.172 microseconds unprotected and 28.812/39.939 to
27.851/42.032 OSCORE. All individual pair guards also pass. Client CPU is separate:
33.167 to 32.583 microseconds/op unprotected and 36.250 to 36.000 OSCORE.
These small aggregate changes do not establish either 50% target, a confidence
interval, an isolated optimization gain or a transport share. Historical failed
cohorts remain unchanged; other representative cells and profile-matched core
probes remain open. This socket run collected no profiles.

The host is AMD EPYC 9V74 with four assigned logical CPUs, Ubuntu 24.04,
Linux 6.17.0-1022-azure, glibc 2.39 and 100 Hz CPU accounting. Governor information
is unavailable. Rust/Cargo 1.99.0 uses release, thin LTO, one codegen unit,
debug=1, strip=none, forced frame pointers, incremental=0 and two build jobs;
core features are `std,alloc,oscore`, defaults disabled. Do not pool this host
with other runs or subtract socket-free App timings.

Linux preflight passes ten endpoint and 37 staged tests, including both actual
oversized-driver-turn recovery tests, plus strict locked release all-target
native-package Clippy. Python runs 301 tests with two skips (299 passes).
Mixed-UID/live-thread cleanup and actual production launcher success/timeout
proofs pass. This closes the focused Linux proof obligations in #175/#177;
it does not qualify representative concurrent performance.

Every server startup reports 14,016 bytes App inline, including the 2,056-byte
adapter and its 2,049-byte scratch. Fixed pools are nested in App; requested App
heap is zero. Borrowed static representation storage is 32 bytes; Poll and Events
are separate four- and 24-byte inline owners. Pinned Linux Mio 1.2.4/libc 0.2.191
source gives a requested Events payload of eight packed twelve-byte `epoll_event`
entries, 96 bytes, separate from the Events header. This is a source-derived
allocation request, not measured allocator consumption. No added worker or
pending kernel operation exists. Snapshot RSS spans 4,874,240–5,292,032 bytes;
it is neither peak RAM nor an additive owner. Allocator overhead, initialization,
peak stack, runtime and socket/epoll kernel costs remain unknown. Do not count
stack-resident owners again inside a stack reservation. Whole-memory acceptance
remains open; the 96-byte Linux bound does not apply to Windows Mio.

A read-only inspection of these exact retained server ELFs with LLVM 22.1.8
(`llvm-readobj --elf-output-style=JSON --program-headers -` and
`llvm-nm --print-size --demangle --defined-only -`, bytes supplied on stdin)
accounts for additional image owners. Original `b8f44fa` declares 5,607,380 file
bytes and 5,610,604 memory bytes across its four nonoverlapping `PT_LOAD` ranges;
accepted `d23f93c` declares 5,594,852 and 5,598,284 bytes. Their respective 4096-byte
page envelopes are 5,623,808 and 5,611,520 bytes. These are declared image extents
and a page model, not measured resident memory, minimum RAM or a memory saving.
Both images contain the 32-byte `SMALL_BODY` and 24-byte `BODY` metadata static;
these are nested in the load image and must not be added again to an image total.
The TLS header declares 168 file/304 memory bytes on both labels; it does not
establish dynamic TLS instance requirements. Dynamic loader/libraries, TLS
instances, allocator overhead, initialization, stacks and kernel costs remain
unresolved. The inspection neither extracted nor executed either ELF. Exact
segments, tools, hashes and accounting scope are retained in
`parent-linux-image-static-owner-receipt.json` under the evidence root below.

Private evidence is retained at
`D:/CoapticCurrent/cpu-linux-20261024/representative-small-c1-20261024` under its
declared 64 MiB phase ceiling; earlier evidence retains its separate 128 MiB
ceiling. The 22,183,226-byte archive SHA-256 is
`941861f93320394ebf35e3f7a73086b65f02e11848f554eb4b06930e789c2ff7`.
Raw window/latency math and all 100 member hashes were independently checked;
four ELFs were stream-hashed without extraction or execution. Source/compiler,
locks, binary manifests, startup ledgers and exact commands accompany the run.

Reproduce on an idle Linux host from companion `8dc65fd` with the pinned compiler,
`psutil==7.0.0`, existing compatible build lane and fresh output, following its
[CPU guide](https://github.com/jeffglousher/coaptic-validation/blob/8dc65fd4e09b4f9440b307085c5b9848174fe4fd/tools/benchmark/linux_cpu.md):

```sh
python3 tools/benchmark/linux_cpu.py \
  --baseline-sha b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 \
  --candidate-sha d23f93c0699b75ec17e692999d62ff89bb71463d \
  --server-kind coaptic-mio --resource-profile small-device \
  --sizes 32 --rates 1000 --seconds 30 --repeats 4 \
  --build-lane /existing/compatible/lane --output /fresh/small-device-c1
```

## Completed bounded Linux I/O control

[Run 40042232531](https://github.com/jeffglousher/coaptic-validation/actions/runs/40042232531)
uses identical core `b9b83d8663cbdcb7a80bf143670a05019f25cf25` and tested companion
`fd3f3fd26b332a7510ba46129fc5e3f0b024bf1c`. Mio is the reference; direct Linux
`recvfrom(MSG_TRUNC | MSG_DONTWAIT)` borrows the existing Engine RX destination.
There is no pending operation, retained pointer, future, extra packet owner,
worker or ring. Immediate exact-length sends and the 32-pass Mio/deadline loop
remain shared. Oversize input is consumed, refused without prefix parsing, and
the loop continues draining and recovers. Socket saturation/refusal/recovery tests pass.

The host is AMD EPYC 9V74, four assigned logical CPUs, Ubuntu 24.04,
Linux 6.17.0-1022-azure, glibc 2.39 and 100 Hz process ticks. Rust/Cargo 1.99.0
uses release, thin LTO, one codegen unit, debug=1, strip=none, frame pointers,
incremental=0 and two build jobs; core features are `std,alloc,oscore` without
defaults. The frozen baseline-built client drives both labels. The allocated
Default fixture has one route, four Calls, zero deferred rows, one context/four
bindings, RX/TX four by 1,472 bytes, dedup eight, Observe four, RX body one and
TX bodies two, each 1,024 bytes in these cells.

Both modes use complete 64/1024-byte GET responses, C1 CON, 4,000 starts/s,
twelve seconds and two rotated pairs, perf/diagnostics off. All 16 windows and
every original pair/pooled latency guard qualify: 768,000 measured completions
plus 320 warmups, zero failed operations. Direct versus Mio pooled server CPU/op:

- Unprotected 64 B: 12.813 versus 13.646 microseconds; ratio 0.93893.
- Unprotected 1024 B: 13.958 versus 13.646 microseconds; ratio 1.02290.
- OSCORE 64 B: 14.688 versus 14.479 microseconds; ratio 1.01439.
- OSCORE 1024 B: 16.042 versus 16.354 microseconds; ratio 0.98089.

Pooled median deltas range from -0.681 to +1.121 microseconds and p99 deltas
from -0.691 to +3.226. Achieved rates range 4,000.051–4,000.056/s; missed starts
6–98 per 48,000 operations stay below 480. Tick windows contain 61–81 ticks;
the mixed changes are only 2–8 pooled ticks. Client CPU stays separate. No
CPU-limited throughput, transport-cost share or consistent gain is established.
Stop expanding this approach; retain its explicit selector and current defaults.
Both representative 50% targets remain open.

Startup App-including-adapter storage is 16,008 bytes for Mio and 13,960 for
direct. Adapter sizes (2,056/8 bytes) and scratch (2,049/0) are nested in those
owners, not additional. Poll and Events are separate inline owners of 4 and
24 bytes, with eight event capacity. App/event heap, allocator overhead, peak
stacks, initialization and kernel socket/selector requirements still need their
separate measurements or justified bounds. RSS snapshots of 4.49–4.81 MB do
not establish those requirements or whole RAM. No added worker CPU is displaced
from the server process; descendant/unattributed machine work is outside its
all-thread clocks.

The earlier [40041518786](https://github.com/jeffglousher/coaptic-validation/actions/runs/40041518786)
stopped before measurement because pinned Clippy was absent. Its zero-window
failure and successful native preflight remain retained. Installing that declared
component was the only source delta before the unchanged comparison above.

Reproduce from companion `8b25a2579fabbbb016e2798d67b076bf4022e41d` on an idle
Linux host with the pinned compiler and `psutil==7.0.0`, using the existing
compatible lane and a fresh evidence destination. Follow the
[CPU guide](https://github.com/jeffglousher/coaptic-validation/blob/8b25a2579fabbbb016e2798d67b076bf4022e41d/tools/benchmark/linux_cpu.md)
for setup and artifact interpretation:

```sh
python3 tools/benchmark/linux_cpu.py \
  --baseline-sha b9b83d8663cbdcb7a80bf143670a05019f25cf25 \
  --candidate-sha b9b83d8663cbdcb7a80bf143670a05019f25cf25 \
  --server-kind coaptic-mio --candidate-server-kind coaptic-mio-direct \
  --sizes 64,1024 --rates 4000 --seconds 12 --repeats 2 \
  --build-lane /existing/compatible/lane --output /fresh/direct-rx-evidence
```

The linked run retains raw samples, all pair results, source/compiler/lock/binary
identities and reproduction inputs. A new host is a new comparison; do not pool
its results with this run or subtract socket-free App timings.

## Retained original-baseline result

[Run 40043745872](https://github.com/jeffglousher/coaptic-validation/actions/runs/40043745872)
uses companion `8b25a2579fabbbb016e2798d67b076bf4022e41d`, accepted core baseline
`b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and candidate `7c04304`, same Mio
and Default/AllocMemory capacities. The source-specific Linux locked builds
verify canonical syn-2 versus alternate syn-3 selection, clean restoration,
compiler/flag/source/binary identities and the frozen baseline client.
This is an Intel Xeon Platinum 8573C host with four assigned logical CPUs,
Ubuntu 24.04/Linux 6.17.0-1022-azure, glibc 2.39, performance governor, TSC and
100 Hz process accounting. Rust/Cargo 1.99.0 and the Linux release flags listed
above apply. It is a separate host from the direct RX and server-App runs.

All 32 clean windows complete 1,536,000 operations plus 640 warmups with zero
failed operations. Rate, pacing, twenty-tick and cleanup gates pass. OSCORE
p99 fails the +10-microsecond limit:

- 64 B repeat 1: +17.861 microseconds; pooled +13.306 microseconds.
- 1024 B repeat 2: +15.664 microseconds; pooled +11.093 microseconds.

All median guards and unprotected latency guards pass. Full-cohort CPU claims
are withheld. Diagnostic pooled server ticks (baseline/candidate) are 226/214
and 234/222 unprotected, 258/246 and 295/286 OSCORE, for 64/1024 bytes and
192,000 verified operations per label/size. These raw counts do not rescue
latency acceptance or establish either target.

Sixteen separate five-second profiling windows complete 320,000 operations
plus 320 warmups. Eight cpu-clock records have 195–320 samples each and zero
reported lost samples; eight stat outputs are retained. Kernel scheduling,
socket/syscall/poll and server dispatch/AES/CCM symbols are leads, with no
causal ranking or percentages: inline coverage and throttling remain unproven,
and cycles/instructions are unsupported. Profile timings never enter clean
CPU/latency. This comparison spans cumulative core changes, so it does not
attribute its latency failure to #675. The incremental control is reported below.

Reproduce from companion `8b25a25` with the same setup as the CPU guide, preserving
all required windows and failures:

```sh
python3 tools/benchmark/linux_cpu.py \
  --baseline-sha b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 \
  --candidate-sha 7c043046097e01e8e8f9f95fc51ee70dde3ed860 \
  --server-kind coaptic-mio --sizes 64,1024 --rates 4000 \
  --seconds 12 --repeats 4 --perf \
  --build-lane /existing/compatible/lane --output /fresh/core-evidence
```

## Cleanup and source-matched build delivery

[Validation #169](https://github.com/jeffglousher/coaptic-validation/issues/169)
is closed. The sole owner reserves the unreaped leader through final group
signals, always invokes privileged TERM/KILL for elevated cleanup, and observes
complete process/thread-group exit before reap or capture handoff. Reaped or
uncertain identities cannot authorize signals. Primary exceptions survive
independent cleanup failures, which remain fatal. Markers no longer authorize
numeric cleanup; the builder watchdog requests cancellation from the sole owner.

The observer bounds two `/proc` scans to 4,096 entries/pass, 64 process pidfds,
4 KiB/stat, and one five-second observation deadline checked between bounded
syscalls. Linux mixed-UID TERM-resistant live-thread/zombie-leader and actual
sudo/prlimit/timeout --foreground/perf stat --null/setpriv launcher success/timeout
proofs pass, with unrelated sentinel/independent teardown. Python passes 276
tests with two optional native-binary skips; four direct socket tests and strict
release Clippy pass. This proves inherited groups in the same visible PID
namespace with one owner/waiter, not arbitrary escaped-group containment.

Companion #173 retains canonical syn-2 `Cargo.lock` and one committed syn-3 input
whose only dependency difference is the `coaptic-io` edge. The selected core
manifest determines the exact supported requirement. Builds record selected
bytes/digests while installed and restore before clean identity/measurement;
unexpected edits, restoration failures and unsupported requirements refuse.
Actual Windows Rust 1.99.0 locked native builds pass for `b8f44fa`, `b9b83d8`,
`d23f93c` and `7c04304`. This is build compatibility, not performance evidence.

## Ended core option experiment

The unmerged #675 experiment reuses transient validated Block1/Block2/Q2 metadata and 4.00 decisions
without persistent state or another packet buffer. Complete 4.02 validation still
precedes 4.00 refusal; authentication, replay/checkpoint ordering and Inner/Outer
separation are unchanged. Independent legacy/field oracles, synchronous and
authenticated precedence/ciphertext-replay tests, 398/370/392 affected App tests
and strict release Clippy across all-features/no-default/OSCORE pass. Two general
correctness/security reviews have no unresolved findings.

Matched Windows d23/7c checks pass 54 existing layout/future checks with identical
reported owners: SmallDevice one-association datagram 12,072 bytes, blockwise
25,800 bytes, and the one-route Constrained async progress future 17,128 bytes.
These are separate configurations/owners, never additive. Native release emitted
dispatch/poll reservations stay 9,368/8,312 bytes plus 64 saved-register bytes
each. Candidate validation is outlined with 200 bytes plus 64 saved-register
bytes; baseline validation is inlined. Return addresses/call nesting, other
callees, peak stack and whole RAM remain unqualified.

The unchanged optimizer-resistant `bench-core` fixture verifies 161,024 exchanges
across d23/7c, both modes and two rotated pairs on Windows. Per-poll wall clocks
include measurement perturbation and establish no server CPU gain. Emission uses
Rust 1.99.0, native release thin LTO/one codegen unit, default debug=0, no RUSTFLAGS,
features `std,alloc,oscore`; it is separate from the Linux socket build profile.
The original-baseline Linux comparison above retains its OSCORE p99 failures;
the current-main incremental comparison also fails its latency guard.

Run `40045613509` uses the same companion source, Linux build flags, host-control
capacities, payloads, rate, duration and four-pair plan, with profiling and
diagnostics off. Its host is Intel Xeon 6973P-C with four assigned logical CPUs,
Ubuntu 24.04, Linux 6.17.0-1022-azure, glibc 2.39 and 100 Hz accounting; it is a
separate host from the original-baseline run. Both labels select the verified
syn-3 lock and restore canonical
bytes before recording clean identities. All 32 windows verify 1,536,000 measured
operations plus 640 warmups with zero failures; source, lock, rate, pacing, tick
and cleanup gates pass. The second OSCORE 1024-byte pair has baseline p99 50.112
and candidate p99 72.139 microseconds (+22.027), exceeding the unchanged +10 guard.
Its median delta is +0.187 microseconds. All pooled guards pass, including this
stratum's +6.121-microsecond p99 delta; the pair failure still vetoes acceptance.
Qualified incremental CPU claims remain withheld. This result establishes no
cause for the tail increase. Independent disposition review reproduced the
failure from raw samples, found no concrete source defect or supported correction,
and recommended ending the experiment. PR #675 is closed without merging.
Do not force inlining, exclude the failed pair or rerun without new evidence.
The reviewed source and tests remain saved; core main retains its preceding
implementation. Both targets stay open.

Reproduce this incremental control with the preceding original-baseline command,
replacing `--baseline-sha` with `d23f93c0699b75ec17e692999d62ff89bb71463d`, omitting
`--perf`, and using a fresh evidence destination. Retain both failed cohorts.

## Current server-App CPU result

The [qualified run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40024678302)
verifies 200,512 exchanges across four serial processes, including 512 warmups.
All four CPU windows qualify. Unprotected server-App driver CPU is
0.901244–0.916253 microseconds/op; OSCORE is 2.032878–2.034727 microseconds/op.
These are two observations per mode on one EPYC 7763 runner, not confidence
intervals or an optimization comparison. Core is `b9b83d8`, Rust is 1.99.0,
and full source/compiler/clock/fixture receipts are in the run artifact.

The probe prepares requests outside a server-App CPU window, runs full App polls
and captures complete responses inside it, then authenticates/verifies every
response outside. It retains protocol maintenance and server crypto. This is
a server-driver measurement including fixture copies, monotonic reads and
loop/gate work. It cannot replace socket acceptance, establish either target
or yield a transport percentage by subtracting another run's timing.

The earlier [short-window run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40022418841)
retains its failed unprotected 10 ms qualification. One 50,000-operation window
now replaces five 10,000-operation windows, with unchanged total work and guards.
Each fixed-workload capture buffer is 256 bytes; a complete row contains two
buffers plus 40 bytes of metadata and padding, totaling 552 bytes. Actual
App/CoreIo datagrams stay 1,472 bytes. Enclosing requested fixture plus App is 27,658,654 bytes below
32 MiB, including metadata/padding and a 16 KiB scratch allowance. Allocator
overhead, generated peak stack and whole RAM remain unknown.

Six release Rust tests, strict Clippy, formatting and build pass. Linux passes
the 54-test Python set with two optional skips, then both native worker-death/EOF
checks; 1,001 independent evidence checks pass. Those EOF tests alone did not
prove production marker/error cleanup. The later #169 repair and production
failure-path proof are described above; arbitrary descendant containment remains
outside their scope.

Use [DEVELOPMENT.md](../DEVELOPMENT.md), the registered compatible build wrapper,
and existing compiler output. Disk space is tight. Mixed shared lanes, evidence,
credentials and device state are retained; an idle lease does not make them
disposable. Remove only settled exclusive output with no consumer.

## Matched original-to-accepted core probe

The unchanged optimizer-resistant `bench-core` fixture at companion
`0732772e94e184f454b91b958925ba7949af312b` now verifies 322,048 complete exchanges
between original `b8f44fa` and accepted `d23f93c` on Windows. Sixteen serial
windows use four AB/BA pairs per mode, each with 20,000 measured operations and
128 warmups. Every response verifies metadata and its complete body; OSCORE
responses also authenticate. Both actual binaries and source-selected lock
inputs were frozen before measurement. Parent checks reproduce all raw counts,
ordering and descriptive statistics, and verify all 71 retained evidence files.

This is Default/Alloc, one GET route, four Calls, zero deferred rows, one
association capacity, four request bindings, C1 CON and 64 response bytes. RX/TX
each have four 1,472-byte datagrams, dedup eight, Observe four, one 1,024-byte RX
body and two 1,024-byte TX bodies. It is distinct from SmallDevice. Post-run host
inventory identifies Ryzen 7 5800X3D (eight cores/sixteen logical CPUs), Windows
11 Pro build 26200; during-window CPU frequency and scheduler state were not
captured. Rust 1.99.0/LLVM 23.1.1 targets `x86_64-pc-windows-msvc`, native release,
thin LTO, one codegen unit, debug=0, incremental=0, two build jobs, features
`std,alloc,oscore` without defaults and no RUSTFLAGS. The actual original
binary SHA256 is `196e3091c7afa9a9cc2fa402e47ee57e67791e0ba76a532c0c6275b66e95b538`;
accepted is `835204fcd4c3f01b40caed003a2c37fd06e6103be05ed41baf82e22418b9012a`.
The earlier historical-PE reproduction attempt remains stopped with zero
windows after its expected binary hash differed; this distinct proof freezes
the actual source-matched builds and does not revise that failure.

Per-poll wall intervals include fixture copies, App polling, handler and server
cryptography; request preparation/protection and full response verification run
outside. Optimization barriers cover inputs and observable results. Pooled
median/p99 is 1,000/2,000 to 700/1,000 ns unprotected, and 1,900/2,500 to
1,600/2,000 ns OSCORE. Observed 100 ns quantization and clock/barrier overhead
limit interpretation. Each window retains 4,096 separate calibration samples;
none are subtracted. These are wall diagnostics, not server CPU windows, socket
latency acceptance, confidence intervals or evidence of either 50% target.
Configured App/CoreIo storage is reused; this probe supplies no allocator, peak
stack or whole-memory acceptance.

Build the same fixture with the recorded source-specific locks and flags for
each core source, freeze both binary identities, then run each mode in the
recorded four-pair order with a thirty-second per-process deadline:

```powershell
bench-core.exe 20000 unprotected
bench-core.exe 20000 oscore
```

## SmallDevice core probe

Companion [#188](https://github.com/jeffglousher/coaptic-validation/pull/188)
delivers `--small-device` in the existing `bench-core` fixture as
`51e1d2fb3769500eb8fdc6fd8e1b67b18a68a9f8`, tree-identical to reviewed/tested
`98eebdb701e91e5567d34e0fdfd033c1eb830541`. It constructs the exact fixed
SmallDevice/Constrained 32-byte C1 profile: route capacity four/one GET, one Call,
zero deferred rows, one association/four bindings, RX/TX two by 1,152 bytes,
dedup four, Observe two and no body pools. The socket-free transport still owns
1,472-byte input/output buffers; it is not equivalent to a Mio adapter. Legacy
Default64 CLI/seven-field JSON and fixed batch CPU code/six tests are unchanged.

Locked original `b8f44fa` and accepted `d23f93c` Windows builds each pass ten
release tests, strict package Clippy and formatting. Rust/Cargo 1.99.0/LLVM
23.1.1 targets MSVC with release thin LTO/one codegen unit, debug=0,
incremental=0, two build jobs and no RUSTFLAGS; features are `std,alloc,oscore`
with defaults disabled. Both actual executable/source/selected-lock identities
are frozen before eight functional cells: two cores, both modes, Small32 and
Default64, each sixteen measured operations plus 128 warmups. All 1,152 exchanges
verify; 32,768 separate clock calibrations are retained without subtraction.
Optimization barriers cover inputs, App and observable results. Preparation and
full authenticated response/metadata verification are outside the timed poll;
fixture copies and App maintenance/server crypto remain inside. No CPU gain is
inferred from these short wall diagnostics.

The Windows Small32 App is 14,936 bytes including nested CoreIo 2,976 bytes and
its two 1,472-byte buffers; fixed pool heap requests are zero. At sixteen
operations the separate timing/calibration payload requests are 256/65,536
bytes, with 48 bytes of vector headers; verification scratch is 1,472 bytes,
optional client context 368 bytes and static representation 32 bytes. Nested
components are not additive. Allocator overhead, initialization, generated
peak stack and whole memory remain unqualified. Parent verifies all raw cells,
final paired executable receipts and restoration; independent general source
review has no findings. Observed stage-boundary positive compiler-output growth
is 24,918,637 bytes, below 128 MiB; this is not a transient peak bound.

Companion [#189](https://github.com/jeffglousher/coaptic-validation/pull/189)
delivers the opt-in `small-device-proof` mode in the existing core-cost workflow
as `73216630df57496e50bcc53189301cb74890f2e6`, tree-identical to reviewed
`0176259fdd222aadacc8afb468c9ecc21d02e1f6`. Default original/batch pins and
behavior, the 25-minute job and serialized `manual-linux-cpu` group remain.
The fixed Linux plan requires twenty actual release tests and both copied ELF
input freezes before the same eight cells, verified cleanup/restoration and a
128 MiB whole-upload bound. Actionlint, four heredoc AST checks and 81 tests pass;
ten platform tests skip. The initial private harness failure is retained and
corrected without changing frozen source. Fresh general correctness/security
review has no findings. The single Linux run
[`40079192247`](https://github.com/jeffglousher/coaptic-validation/actions/runs/40079192247)
passes its narrow Linux functional proof: twenty native tests, 75 Python passes
and five skips, all eight cells, 1,152 fully verified exchanges and 32,768 separate
calibrations. Both copied ELF/source/lock/compiler identities are frozen before
the cells; final source/lock restoration and all owned-process cleanup pass.
The legacy Default64 output remains seven fields. The actual Linux Small32
ledger matches the listed Windows owner sizes; this is observed output, not a
Windows-size assertion or equivalence to the Mio fixture. This completes #187's
narrow SmallDevice32 C1 functional/probe proof. Both CPU targets, other
profile/concurrency probe obligations and whole-memory acceptance remain open.

Actual Linux original/accepted ELF SHA-256 values are respectively
`07cd940a383a25cb9890dcc97655255b2a076b474702c1e88a094a4839bbdc2a`
and `6635c4b149355a25aa705bc6922f2c6a4d02b5773092415f531f01dcbae4bdbb`,
5,541,848/5,550,224 bytes. The builds use the recorded Rust/Cargo 1.99.0 GNU
target, release `--locked`, thin LTO/one codegen unit, debug=1, strip=none,
forced frame pointers, incremental=0 and two build jobs. The finite proof uses
the same App poll boundary with complete authentication/response verification.
Its sixteen wall samples per cell establish no statistical CPU or latency claim;
separate calibration is never subtracted.

The 3,330,559-byte remote archive, SHA-256
`53b02c55b666f742e201283ec2846194df58c7c8406f672d72378f1f16458ba2`,
contains 46 members and 11,318,087 uncompressed bytes. The collector checks all
member CRC/SHA values, including both ELFs streamed as data. Parent verification
rehashes all 44 retained text members and twenty source Git blobs, checks raw
arrays/counts, selected locks, paired identities and restoration; it transparently
reuses the remote archive/ELF stream hashes. Retained output is 789,274 bytes
within 1.5 MiB, with no local ZIP/ELF/perf. The native index's own empty placeholder
is consistent with its reviewed write ordering; its actual archived CRC/SHA is
checked separately. An initial collector assumption and a parent local-path
assumption were corrected in private verifiers, with original failures retained;
no source, evidence, acceptance gate or benchmark attempt changed.

Linux process limits are 512 MiB address space, thirty seconds and 512 KiB per
raw output file. They are limits, not allocations or peak requirements. Reproduce
from companion `7321663` using `small-device-proof` in the existing manual
core-cost workflow, verifying the exact suite SHA; or use the recorded locked
build inputs and the following commands without `.exe` on Linux. Original and
batch diagnosis modes were preserved in source and were not run by this proof.

From the recorded companion source and source-selected locked build, reproduce
the finite functional cells for each security mode and each pinned core:

```powershell
bench-core.exe 16 unprotected --small-device
bench-core.exe 16 oscore --small-device
bench-core.exe 16 unprotected
bench-core.exe 16 oscore
```

## Current cost screen

### Bounded socket attribution readout proof

Companion [#194](https://github.com/jeffglousher/coaptic-validation/pull/194)
delivers source `37b9df383e245f06644c91d53d5c02bd8b26378a`, tree
`50ae8eddf10ad2782081124b645710c58ae12f37`, equal to reviewed `bcef384`.
It preserves the flat report and default controls, and explicitly requests two
additional PID-filtered inline readouts for socket record captures. Each uses
the existing post-sudo file quota, foreground timeout, reserved-group cleanup
and disabled debuginfod. Five output owners per profile slot fit the unchanged
whole-upload bound. Windows source validation passes 306 checks with 22 skips;
independent general correctness/security review has no findings.

The single [Linux run 40096580435](https://github.com/jeffglousher/coaptic-validation/actions/runs/40096580435),
attempt 1, establishes **readout functionality only** under companion #193.
Original core `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and accepted
`d23f93c0699b75ec17e692999d62ff89bb71463d` use ordinary Mio, the same
baseline-built client, fixed Service capacities, 16,384-byte responses and C1 CON.
This predeclared diagnostic uses 300 starts/s, ten-second clean windows, two
rotated pairs per mode and separate five-second 997 Hz record/stat windows.
It is not a representative CPU acceptance stratum or a replacement cohort.
All sixteen windows verify 36,000 measured complete operations plus 320 warmups,
with zero operation failures and resolved cleanup. The transport accepts 581,120
client sends across the campaign; each window stays within its unchanged
60,000-send limit, including warmups and controls.

All four clean pairs fail a retained latency guard. Candidate-minus-baseline
deltas, with zero-based repeats, are: unprotected repeat 0 median +35.839
microseconds; unprotected repeat 1 p99 +370.780; OSCORE repeat 0 median +2.0775
and p99 +389.349; OSCORE repeat 1 p99 +217.699. Pooled OSCORE median +3.325
and p99 +555.641 also fail. Pooled unprotected passes and cannot erase pair
failures. Rate, pacing, twenty-tick eligibility and cleanup checks pass.
**All primary CPU comparisons remain withheld.** The short-run summary's
`comparable` field covers operation/rate/tick eligibility, not these latency
guards; retained runner arithmetic is not an accepted CPU claim.

All four record captures, their flat reports and eight new caller/sample helpers
complete successfully. Each helper returns zero below its 4,105,632-byte quota,
with an explicit positive SUT PID, inline output, 25-second TERM/five-second KILL
timeout and thirty-second owner deadline. Exact decimal flat headers agree with
the unchanged strict raw reader. Total/user/kernel samples are respectively
291/76/215 and 282/60/222 unprotected, 346/135/211 and 331/111/220 OSCORE,
ordered baseline then candidate. Observed loss/throttle counts are zero;
requested PID and executable mappings match. Every user total is below 200
before App filtering. Kernel totals are not eligible App-rooted counts.
Inline-complete App counts remain unknown, as do full operation exposure and
coverage. Samples span about 4.98 seconds; ACK processing fences do not establish
sampling exposure. Unknown frames and warnings are retained. **Cost nomination
is withheld**; no percentage, transport share or optimization is selected.

The host is AMD EPYC 9V45 with four assigned logical CPUs, Linux
6.17.0-1022-azure, glibc 2.39, TSC/100 Hz accounting and perf 6.17.13; governor
information is unavailable. Rust/Cargo 1.99.0 GNU builds use release `--locked`,
defaults off, `std,alloc,oscore`, forced frame pointers, debug=1, strip=none,
incremental=0, two build jobs, thin LTO and one codegen unit. Source-specific
syn-2/syn-3 lock inputs and actual binaries are frozen before windows. Canonical
lock restoration is verified per build; a final hosted core checkout restoration
to its initial revision is not separately recorded. No cross-host pooling or
socket-free subtraction is valid.

Actual server App owners match at 314,856 bytes, including fixed pools and the
2,056-byte adapter/2,049-byte scratch; requested pool heap is zero. Representation
backing is a separate 16,384 bytes, Poll is four bytes and Events header is 24
bytes with capacity eight. Client App is 15,448 bytes, requested core heap 37,076,
verification 32,768, timing 24,000 clean/12,000 profiling and pending metadata
288 bytes; its 1,473-byte scratch is nested in App. Sixty-four available lifetime,
non-atomic process observations are supporting evidence, not peak RAM or used
stack. Allocator, initialization, runtime, kernel and whole-memory costs remain
open; profiler/helper CPU is not a server saving.

Linux receipts pass 326 Python checks with two skips, ten endpoint and 37 staged
tests, strict release package Clippy, and production launcher/mixed-UID cleanup
proofs. Artifact `12561520726` is 21,844,876 compressed bytes, SHA-256
`b92de7ce8880f64e2120fbb07f548a88e23b307298f7bbf300698174ed65d577`;
all 128 members total 92,221,303 expanded bytes and pass streamed CRC/SHA checks.
All 12,013,487 raw text bytes are retained; four ELF and four raw perf owners
were inspected as data without local extraction or execution. Parent verification
passes 712 checks over retained files, source blobs, raw arrays and readouts,
explicitly reusing the collector's remote binary/raw-perf integrity receipts.
Private verifier assumption failures and their corrections remain retained;
no source, raw result, gate or benchmark attempt changed.

Private evidence is retained under
`D:/CoapticCurrent/cpu-linux-20261024/service-attribution-193-20261024/collection/`
with `REPORT.txt`, `analysis.json`, `perf-analysis.json`, `commands.txt` and the
complete evidence index. Its 13,449,396 bytes remain below the 56 MiB reserve
inside the 64 MiB phase. No local archive, ELF or raw perf file was created.
The following is the recorded reproduction shape from the exact companion
source and locked toolchain; it does not schedule a replacement run:

```sh
python3 -B tools/benchmark/linux_cpu.py \
  --baseline-sha b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 \
  --candidate-sha d23f93c0699b75ec17e692999d62ff89bb71463d \
  --server-kind coaptic-mio --resource-profile service-fixed \
  --sizes 16384 --rates 300 --seconds 10 --repeats 2 --perf \
  --security-mode both --concurrency 1 --message-type con \
  --build-lane "$BUILD_LANE" --output "$OUTPUT"
```

No replacement run, parser expansion, higher sampling rate or diagnostic
framework is planned. Both 50% targets, whole-memory acceptance and remaining
representative obligations stay open; all earlier failed cohorts remain intact.

### Earlier cost screens

An audit of eight retained `40043745872` raw captures confirms requested PID and
executable mappings, user/kernel sample modes and zero recorded loss/throttle
events. Unprotected user counts total only 136 for the original label and 101
for the unmerged option candidate, before filtering to App ancestry. They do
not support the existing 200-App-sample nomination floor or a causal ranking.
Offline inspection of the accepted SmallDevice Linux ELF also confirms a
1,152-byte outer RX scratch clear followed by a filled-prefix copy. Instruction
presence alone supplies no CPU percentage or safe material optimization.

Companion [#181](https://github.com/jeffglousher/coaptic-validation/pull/181)
introduced explicit 997/9970 Hz selection in source
`0c2f94f82f6c97bd61ede50fdfd6fdfa6e97b251`, with the same tree as reviewed
`b34e04aa802721924075a0ee3cdd642b60fae6c6`. It preserved requested-rate checks,
requested-period burst validation and existing bounds. Independent general
review had no findings, 288 distinct Python checks passed with 22 platform
skips, both workflow checks passed, and eight retained 997 Hz raw captures
regraded unchanged while refusing a mismatched 9970 request.

The single [control run 40060666280](https://github.com/jeffglousher/coaptic-validation/actions/runs/40060666280)
failed at its first cpu-clock continuous control: perf printed a rounded
`Samples: 4K`, which the existing exact raw/report count check refused. The raw
reader independently verifies 4,990 samples (2,828 user and 2,162 kernel),
500,497,000 ns aggregate period and zero recorded loss/throttle events. Process
CPU is 500,047,912 ns; these counts per CPU second remain descriptive, not
coverage or an App cost nomination. The other three controls did not run.
One continuous block exceeds the requested 100,300 ns period; continuous
validation permits this, and no burst invariant was proved. Linux runs 79 tests
with no failures or skips. The host is Xeon Platinum 8573C, four online CPUs,
Linux 6.17.0-1022-azure, perf 6.17.13 and Python 3.12.3.

The report already requested sample counts; the
[Linux 6.17 header formatter](https://github.com/torvalds/linux/blob/v6.17/tools/perf/builtin-report.c#L482)
scales its total unconditionally. The higher-rate approach is ended without
parser expansion, rate escalation, gate changes, retries or a communication
capture. [Companion #182](https://github.com/jeffglousher/coaptic-validation/pull/182)
restores the qualified 997 Hz implementation, workflows and tests as
`bff8859c3f637736b1a326f518aac8d9a3600f79`, with the same tree as reviewed
`4ee141fc834cbec8b22298144d22170c3c404a0d`. Independent general review has no
findings. Main is clean/pushed and the settled owned branch is removed. Its six
restored source/workflow/test blobs match qualified `8dc65fd` exactly;
the existing guide retains the failed experiment. The 87,961-byte archive's
eleven members and raw result are independently checked without extraction or
execution. Both CPU targets and representative/whole-memory qualification remain
open. No sample count establishes CPU gain, a transport share or complete delivery.

## Independent-endpoint client memory and qualification limits

The #176 Windows C4/16,384-byte client reports 15,720 bytes App inline,
91,656 requested core-pool heap bytes and 32,768 verification bytes. Its
four-operation functional window requests 32 timing bytes. The 1,768-byte adapter,
its 1,473-byte scratch and socket handles are nested in App. Window metadata is
368 bytes; the counter snapshot, peer keys and pending rows are separate
64/112/288-byte local owners. Five RX/five TX slots are specific to this client
mode; server pools and legacy client capacities remain unchanged. Socket buffers,
allocator overhead, initialization and peak stack remain unknown. The exact
Windows executables reserve a 1 MiB main stack with 4 KiB initial commit; this is
configured virtual reservation, not observed peak or a minimum requirement.
Stack-resident App/local owners must not also be added to that stack reservation.
No whole-memory or CPU acceptance follows from these functional receipts.

The SmallDevice 32-byte C1 measurement strata at 1,000/s in both modes and at
100/s unprotected, plus fixed-Service 16 KiB/C1 CON/50/s OSCORE, qualify as
recorded above. Their CPU ratios remain above 0.5;
both reduction targets and the remaining representative Linux CPU/latency cells
remain open. Throughput/saturation remains unqualified. Default
clean windows remain 2–30 seconds; delivered #184 adds explicitly selected
31–1,500-second single-security partitions with the scope and bounds above.
Each window still needs at least twenty server CPU ticks. The one global
60,000-send ceiling includes warmups, retries and controls.
With 20 warmups and 1,024-byte blocks, even ideal traffic permits at most 59,980
one-block, 14,980 four-block or 3,730 sixteen-block measured operations. Additional
traffic lowers those maxima. Extending duration alone cannot qualify a cell that
needs more operations than that budget; pooling individually sub-floor windows
or substituting process clocks cannot rescue it. Source-frozen run `40065560596`
now passes the 500-second SmallDevice/unprotected partition and every pair and
pooled latency guard recorded above. It covers only that partial matrix and
does not meet the required 50% CPU reduction. No
representative workload or failed cohort is being replaced.

## Socket integration today

The core owns neither an OS socket nor an executor. `DatagramIo` and
`AsyncDatagramIo` move complete packets between caller transports and bounded
Engine slots. Sync/async protocol behavior shares the implementation. The
platform integration supplies clocks, timers and wakeups; pools are allocated
at startup and reused, with automatic growth off.

The benchmark has several explicitly named drivers:

- Bare and reusable std UDP use nonblocking sockets and continuously call
  `App::poll`. Both spin; reusable storage avoids constructing receive scratch
  on every receive. Both still copy accepted bytes into an Engine RX slot.
- Mio uses nonblocking receive/send, drains at most 32 App passes, then waits
  for socket readability or the next App deadline. Its OS selector is epoll on
  Linux and IOCP on Windows.
- Tokio ready uses synchronous App polling with Tokio socket readiness waits.
  Native async uses `wait_async` followed by freshly timed `progress_async`;
  accepted ingress is retained in an existing RX slot. Sends and cancellation
  keep the existing deadline, pin and commit rules.
- Staged-ready, staged-Mio and blocking variants are optional controls.
  Staging did not establish consistent savings; blocking has distinct soft
  send-timeout semantics. They are not defaults or substitutes for qualification.

The normal receive adapters use an N+1 scratch guard to detect oversize input,
then copy accepted bytes to the bounded RX destination. Sending borrows encoded
TX bytes. Async APIs alone do not imply io_uring or zero-copy networking.

## Assessing io_uring

io_uring is a possible Linux-specific completion adapter, not a new protocol
implementation for each CPU architecture. Keep the portable core and existing
Windows/embedded integrations. Rust/OS bindings handle the target ABI; Linux
kernel capabilities still require explicit checks and a fallback.

The potentially useful mechanisms are submission/completion batching and bounded
multishot receives. They do not remove UDP stack work, authentication, replay
checks or required message ownership. At C1, little independent traffic may be
available to batch. Existing system-CPU charge is not proof that all such work
is avoidable, and present measurements do not predict an io_uring speedup.

The difficult seam is lifetime: the current async contract requires dropping a
pending receive to end all access to the borrowed destination. A pending send
must not already have accepted the packet. Kernel-submitted operations cannot
simply retain those short loans. A safe integration needs bounded owned stable
storage and completion/drain rules, including forgotten futures, cancellation
races and exact send acceptance. Count ring memory, all configured packet owners,
kernel resources and any worker CPU; do not hide new buffers or polling threads.

Compare a simple batched socket control and a completion adapter only when a
specific measurable cost justifies the experiment. Preserve clean CPU accounting,
capacities, latency/fairness and refusal/recovery. This is a design consideration,
not an accepted implementation or completed optimization.

References: [Mio portability](https://docs.rs/mio/latest/mio/struct.Poll.html),
[io_uring model](https://man7.org/linux/man-pages/man7/io_uring.7.html),
[cancellation](https://man7.org/linux/man-pages/man7/io_uring_cancelation.7.html),
[ownership in Tokio-uring](https://docs.rs/tokio-uring/latest/tokio_uring/).
