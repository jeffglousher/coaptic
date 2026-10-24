# Delivery handoff — October 23, 2026 (updated October 24 UTC)

Accepted protocol and integration software is implemented. The remaining work
is performance implementation/qualification and the physical checks
in [PLAN.md](../PLAN.md). Both 50% server CPU targets remain open under
[#650](https://github.com/jeffglousher/coaptic/issues/650). Release publication
requires separate authorization under #132.

## Saved work

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

## Current cost screen

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

Apart from the SmallDevice 32-byte/1,000/s C1 stratum above, representative Linux
CPU/latency cells remain open; throughput/saturation remains unqualified. The current
runner limits clean windows to 30 seconds; each needs at least twenty server CPU
ticks. The one global 60,000-send ceiling includes warmups, retries and controls.
With 20 warmups and 1,024-byte blocks, even ideal traffic permits at most 59,980
one-block, 14,980 four-block or 3,730 sixteen-block measured operations. Additional
traffic lowers those maxima. Extending duration alone cannot qualify a cell that
needs more operations than that budget; pooling individually sub-floor windows
or substituting process clocks cannot rescue it. No long-window campaign is
scheduled, and no representative workload or failed cohort is being replaced.

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
