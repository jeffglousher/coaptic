# Delivery handoff — October 23, 2026 (updated October 24 UTC)

Accepted protocol and integration software is implemented. The remaining work
is performance qualification and the physical checks
in [PLAN.md](../PLAN.md). Both 50% server CPU targets remain open under
[#650](https://github.com/jeffglousher/coaptic/issues/650). Release publication
requires separate authorization under #132.

## Saved work

- Core main includes the acceptance matrix from #674. Transient validated-option
  reuse is reviewed in draft [#675](https://github.com/jeffglousher/coaptic/pull/675),
  measured source `7c043046097e01e8e8f9f95fc51ee70dde3ed860`. The cumulative
  baseline comparison fails OSCORE p99 guards; incremental isolation is pending.
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
- A child now owns the companion's bounded C1 profile fixture extension from
  `8b25a25`: unchanged SmallDevice, configured SmallDevice blockwise, and
  fixed/allocated Service on the existing Mio loop. That implementation owns
  its source/refs/local build lane until explicit release. Independent-endpoint
  CON remains a separate pending client slice.
- PR watches and recurring tasks are off. One finite
  [incremental comparison](https://github.com/jeffglousher/coaptic-validation/actions/runs/40045613509)
  has one finite collector: current main `d23f93c` versus `7c04304`, same Mio,
  64/1024 bytes, 4,000/s, twelve seconds and four rotated pairs, profiling off.
  It isolates #675; the original-baseline failure below remains retained.

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
attribute its latency failure to #675. The incremental control above is pending.

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

## Pending core option batch

Draft #675 reuses transient validated Block1/Block2/Q2 metadata and 4.00 decisions
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
the current-main incremental comparison remains pending. Both targets stay open.

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
