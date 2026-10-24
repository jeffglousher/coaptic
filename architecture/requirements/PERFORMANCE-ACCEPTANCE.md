# CPU efficiency acceptance

Both targets in [#650](https://github.com/jeffglousher/coaptic/issues/650)
remain **OPEN**. Each security mode must independently reduce server CPU per
successfully verified complete operation by at least 50%. An eligible host
control, a core probe or one favorable payload cannot complete the representative
target. EDHOC establishment is a separate workload.

## Baselines and source matching

The retained initial runtime baseline is core
`b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`. A retained historical comparison against that
source used candidate `7ab6949d9604133c7f8ed8904e424b8657630d50`, companion
`eea0374b60c93098a7f0dcf24a999e49d753d624`, the same Mio adapter on both labels,
64/1024-byte responses, C1 CON, 4,000 offered operations/s, twelve seconds and
four rotated pairs. Run `39924741885` retains 16 pacing failures in 32 windows;
all four CPU ratios are withheld. Two pair latency failures remain. This cohort
must not be replaced by a passing rerun or a lower offered load.

The separately declared SmallDevice 32-byte/C1 CON/1,000/s stratum in
[run 40054024038](https://github.com/jeffglousher/coaptic-validation/actions/runs/40054024038)
compares that original baseline with accepted `d23f93c`, same Mio, companion
`8dc65fd`, thirty-second windows and four rotated pairs per mode. All sixteen
windows and pair/pooled guards pass; server CPU ratios are 0.953488 unprotected
and 0.968586 OSCORE. This cell does not meet the required 0.5 ratio, qualify the
remaining matrix or replace the failed host cohort. The delivery handoff records
its exact sources, host, capacities, memory limits and reproduction command.

The separate low-load SmallDevice 32-byte/C1 CON/100/s partition in
[run 40065560596](https://github.com/jeffglousher/coaptic-validation/actions/runs/40065560596)
uses companion `430722d`, original `b8f44fa` versus `5bd6f27` (protocol-identical
to accepted `d23f93c`), the same Mio and a frozen baseline-built client. All eight
500-second unprotected windows, four rotated pairs and pooled guards pass:
400,000 verified operations plus 160 warmups; 50.20 to 48.65 microseconds/op,
ratio 0.969124. This remains above 0.5 and covers only that slice. Its EPYC 7763
host differs from the EPYC 9V74 used by `40054024038`; do not pool their costs,
multiply ratios or infer a load curve from them.

The first OSCORE partition at that 100/s, 500-second SmallDevice configuration,
[run 40073484846](https://github.com/jeffglousher/coaptic-validation/actions/runs/40073484846),
uses companion `c1fa5bf`, the same original/accepted core pair, ordinary Mio and
frozen baseline client. All eight windows verify 400,000 operations plus 160
warmups, but zero-based repeat one fails median (+6.6925 microseconds) and p99
(+20.238), repeat three fails median (+15.931), and pooled median fails (+5.732).
CPU comparisons are withheld. The cell remains open; this failed cohort is
retained without replacement or relaxed guards.

The first fixed-Service 16 KiB blockwise/C1 CON/50 starts/s partition,
[run 40081182117](https://github.com/jeffglousher/coaptic-validation/actions/runs/40081182117),
uses companion `7321663`, original `b8f44fa` versus accepted `d23f93c`, ordinary
Mio, a frozen baseline-built client and four rotated pairs of 74-second windows.
All eight windows verify 29,600 unprotected complete operations plus 160 warmups;
sources, capacities, pacing, ticks and cleanup pass. Median deltas fail in
zero-based repeats 0/1/3 (+93.4885/+27.681/+32.200 microseconds) and pooled
(+3.154); every p99 guard passes. All CPU comparisons are withheld. This cell
and both reduction targets remain open. The first required OSCORE counterpart,
[run 40083610413](https://github.com/jeffglousher/coaptic-validation/actions/runs/40083610413),
uses the same source/configuration plan on a separately recorded host. All eight
windows verify 29,600 authenticated operations plus 160 warmups and pass every
eligibility, latency and cleanup guard. Server CPU/op is 610.811 to 561.486
microseconds, ratio 0.919248 (-8.08%), above the required 0.5. This measurement
qualifies only that OSCORE slice and replaces no failed cohort. Neither an
isolated optimization nor a confidence interval is established. See the handoff
for exact Linux owners, host boundaries and reproduction.

The separate identical-core memory functional control `40066937573` verifies
1,600 measured operations, 160 warmups and 32 available process snapshots.
All eight CPU windows miss the twenty-tick floor and one OSCORE pair median
guard fails. CPU comparisons remain withheld; approximate lifetime high-water,
stack mappings and configured limits do not establish whole-memory acceptance.

Historical run `39924741885` used Ubuntu 24.04, Linux 6.17.0-1022-azure, Xeon Platinum 8370C,
four assigned logical CPUs, glibc 2.39, performance governor and 100 Hz process
accounting. A fresh comparison must build both labels on the same recorded host;
the host name is not a guarantee that two hosted runners are equivalent.

Use Rust/Cargo 1.99.0, release, `--locked --target x86_64-unknown-linux-gnu`,
`-C force-frame-pointers=yes`, release debug=1, strip=none, incremental=0, two
build jobs, thin LTO and one codegen unit. Core features are `std,alloc,oscore`
with defaults disabled for the existing host fixture. Retain full compiler,
manifest, lock, source/tree and binary identities. The baseline-built client
drives both server labels. Detailed diagnostics are off for acceptance.

Compare adapters with identical core SHAs; compare core revisions with the same
adapter. Do not multiply ratios from separate experiments. Retain the original
runtime baseline even when a newer source is useful as a local experiment
control. An old measurement does not become current when a workflow pin changes.

## Existing host cells and guards

The eBPF syscall diagnostic `40112478137` uses the same accepted native core
`d23f93c` in OFF/ON arms, companion `35a86de`, and BPF build core `1b93171`.
It completes 480,000 verified SmallDevice/32-byte/C1 CON operations plus 320
warmups across sixteen 30-second, 1,000/s windows. The eight ON snapshots each
record one successful send, one successful receive plus one EAGAIN receive,
and one epoll wait per operation; anomalies are zero. This proves counts only.
Unprotected repeat 3 median is +2.084 us; OSCORE repeats 0/1/2 are
+2.875/+16.642/+2.514 us. Those pair failures withhold both overhead ratios even
though all pair p99 and both pooled guards pass. Every syscall-diagnostic row
permanently vetoes primary CPU acceptance. Counts are not durations, a CPU share
or an optimization gain; helper lifetime CPU has a different scope and global
prefilter/worker costs remain unknown. The [companion report](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/ebpf-syscalls-20261024)
retains sources, raw-statistic receipts, both earlier failures and resource gaps.
Neither 50% target nor whole-memory acceptance is completed by this experiment.


The existing allocated fixture has one populated route, four Calls, zero deferred
rows, one OSCORE context and four request bindings. RX and TX each have four
1,472-byte datagrams; dedup has eight entries and Observe four. RX body count is
one and TX body count two, each rounding the selected representation to a
1,024-byte block boundary. Normal security and protocol maintenance remain on.

Preserve these eligibility rules from companion `linux_cpu.py`:

- Every operation verifies complete representation, response metadata and, for
  OSCORE, authentication. All required windows/pairs and cleanup must complete.
- Achieved rate is within 5% of offered rate; labels match within 5%.
- Missed starts are at most `max(1, completed // 100)` per window.
- Server user plus system CPU is at least twenty accounting ticks per window.
- Instrumented admission cohorts, including mixed campaigns, veto CPU ratios.
- Candidate median minus baseline median is at most 2 microseconds and p99 at
  most 10 microseconds, for every pair and pooled stratum. Pooled results cannot
  erase a failed pair.

Compute CPU/op from summed server user plus system CPU divided by verified
completions within one eligible stratum. Keep client cost separate. Process
clocks supplement ticks; they do not rescue an ineligible window or include
descendants/unattributed kernel workers. Do not subtract empty controls or
socket-free timings. Keep profiling separate from clean measurement.

Companion #194 supplies bounded inline caller and per-sample readouts, with
functional Linux proof in diagnostic run `40096580435`. Availability does not
establish App ancestry, inline completeness, exposure or eligible sample counts.
That run's user-mode counts are 76/60 unprotected and 135/111 OSCORE for
baseline/candidate, below the 200-sample nomination floor before App filtering.
Kernel mode totals are not eligible App-rooted counts. Every clean pair also
fails a retained latency guard; primary CPU comparisons and cost nomination
remain withheld. The 300/s, two-pair diagnostic is not a representative acceptance
stratum and cannot replace any failed cohort. See the
[handoff](../DELIVERY-HANDOFF.md#bounded-socket-attribution-readout-proof) for
exact sources, failures and reproduction.

Also require matched optimizer-resistant core probes for the changed request
paths. Identify both source revisions, compiler, features, profile, capacities,
payload and work count. Inspect the probe's observable inputs/results and
optimization barriers; every operation must produce a fully verified response,
including authentication in OSCORE mode. Keep client preparation/verification
and driver overhead scopes explicit. Probe evidence accompanies real-socket
acceptance and cannot replace it.

The original-to-accepted Default/Alloc 64-byte Windows probe now verifies
322,048 exchanges in sixteen windows with both modes and frozen matched inputs.
Its per-poll wall samples are supporting functional/optimizer-resistant evidence;
they establish no server CPU gain or SmallDevice equivalence. The delivery
handoff records exact sources, compiler, verification and timing boundaries.
Representative profile/payload/concurrency probe obligations remain open.

Companion #188 adds the explicit fixed SmallDevice 32-byte C1 probe and preserves
the Default64 seven-field output and batch CPU fixture. Exact b8/d23 Windows
builds pass twenty native tests and eight functional cells verify 1,152 exchanges
with optimization barriers and full response verification. Companion #189 adds
the fixed Linux proof to the existing workflow; run `40079192247` passes twenty
native tests and eight cells with 1,152 verified exchanges and 32,768 separate
calibrations. Actual Linux SmallDevice App is 14,936 bytes including its nested
2,976-byte CoreIo; this is a different fixture from Mio. The narrow SmallDevice32
C1 probe obligation is now proved, with a Default64 regression control.
Neither wall samples nor this functional proof qualify socket
CPU, latency, remaining profiles/concurrency or whole memory.

## Representative execution matrix

Issue #650 did not freeze all representative cells or numerical overload and
memory criteria. The following execution choices make that remaining scope
explicit; they are not claims that historical runs covered these configurations.
Collect matched baseline evidence where it is absent, using equivalent resource
configuration if an older source predates a preset name. Do not reduce a preset
to make its comparison pass.

- **SmallDevice:** fixed Constrained pools; four route capacity, one populated
  GET route, one Call, zero deferred rows, one association, four security request
  bindings. RX/TX each have two 1,152-byte datagrams, dedup four, Observe two,
  no body pools. Compare 32-byte complete responses at C1 CON.
- **SmallDevice with blockwise selected:** the same resource dimensions with
  blockwise explicitly enabled; one RX and one TX body of 4,096 bytes. Compare
  64, 1,024 and 4,096-byte complete representations. This is distinct from the
  body-free preset, which cannot be credited with blockwise coverage.
- **Host service:** the fixed Service preset and its explicitly bounded allocated
  equivalent; 32 route capacity, one populated route, four Calls, eight deferred
  rows, one association, four request bindings. RX/TX each have 32 1,472-byte
  datagrams, dedup 64, Observe 32; four RX and four TX bodies of 16,384 bytes.
  Compare 64, 1,024 and 16,384-byte complete representations.
- **Retained host control:** preserve the existing Default/AllocMemory
  64/1,024-byte cohort and the separate 65,536-byte blockwise fixture. The latter
  has explicitly enlarged body capacity and is not the unchanged Service preset.

Run every cell separately for unprotected and established OSCORE. Use matched
100/1,000 offered operations/s for small representations and 10/50 for blockwise,
plus the retained 4,000/s host cohort. Use four rotated AB/BA pairs for target
acceptance. A smaller two-pair experiment may reject a hypothesis but cannot
complete the targets. Keep low-resolution or pacing-ineligible cells open.

Completion is a conjunction: candidate server CPU/op must be at most 50% of its
matched baseline in **every required security/profile/payload/load/concurrency
stratum**, with all eligibility and latency guards satisfied. No averaging across
strata, compensating one mode with another, or dropping failed cells is allowed.
Report all pair results and counter quantization/observed variation. If their
uncertainty can reverse the claimed 50% reduction, that stratum remains unproven.

Qualify C1 CON and independent endpoints up to the supported request/pool bound.
Each endpoint retains NSTART=1. Share nonce ownership correctly; cloned sender
contexts starting at the same sequence are forbidden. Concurrent NON is a
separate cohort and cannot replace CON concurrency. A fixture limitation is
unfinished evidence, not unsupported core behavior.

For throughput, increase a predeclared offered-load ladder through the first
refusal or eligibility failure; report the last eligible matched rung and the
first failed rung, client utilization and all failure counts. Do not call a
closed-loop library ranking server saturation. At overload, every admitted
operation must complete or explicitly refuse within the existing client deadline;
no endpoint may starve while another continues indefinitely. Prove pool refusal
without consuming unowned input, cancellation without retained loans, exact TX
retention on ambiguous/refused sends, release and fresh successful operation.
Preserve the supported sync and async paths.

## Memory, evidence and limits

Account for disjoint enclosing owners: App inline state, configured requested
heap, packet/body pools, metadata/alignment, security contexts/bindings, transport
scratch/staging, selector/ring storage, runtime/task allocations, stacks and
initialization scratch. Include allocator overhead and kernel requirements as
measured values or justified bounds; mark unknowns explicitly. Do not sum nested
components again. Count attributable worker CPU and memory, including work
outside the server thread. No automatic growth or capacity reduction is allowed.

Before qualifying a candidate, freeze named enclosing-owner and aggregate memory
ceilings for each unchanged-capacity configuration against its matched baseline.
Preserve existing configured ceilings. Any proposed additional bounded owner
requires an explicit startup budget and review of its aggregate SmallDevice and
service impact before acceptance. An unexplained ceiling overrun, or unresolved
material allocator, stack, runtime, kernel or worker cost, keeps memory acceptance
and the overall target open; labeling such a cost unknown does not pass it.
Retained host-fixture App/requested-pool reference caps are now named in the
[delivery handoff](../DELIVERY-HANDOFF.md#retained-baseline-memory-owner-caps).
They cover Windows profiles, Linux SmallDevice and the matched Linux fixed-Service
configuration, with exact source/build scope. Linux SmallDevice blockwise,
allocated Service and aggregate/whole-memory ceilings remain open.

Companion #191's Service fixed/allocated complete-operation core probe and
#192's Linux functional workflow use different transport and client owners from
Mio. Their App/pool ledgers must retain that distinction. Supporting wall probes
cannot qualify socket CPU or latency, replace failed cohorts, or establish
aggregate/whole-memory ceilings.

The global 60,000 outbound-datagram client transport ceiling, campaign output limits and
twenty-tick CPU gate currently limit long efficient windows. Companion #184
supports explicit 31..1500-second single-security representative partitions with
four pairs, unchanged send/operation/latency/eligibility limits and profiling off;
its existence does not qualify a cell. Companion #186 lifetime memory observations
similarly do not establish required whole-memory ceilings. If a representative
cell cannot collect sufficient CPU inside the retained bounds, it remains ineligible.
Any remedy needs bounded ownership and MID/nonce lifetime proof; raising a send
ceiling or relaxing an eligibility guard is not a remedy.

Retain raw failed cohorts, latency samples, profiles and reproduction commands.
The server-App run `40024678302` is diagnostic only: EPYC 7763, core `b9b83d8`,
companion `ff46fd5`, 200,512 verified exchanges, four qualified driver windows.
Its 0.901244–0.916253 microseconds/op unprotected and 2.032878–2.034727 OSCORE
include full App polls, copies, clocks and loop/gate work; client preparation,
authenticated verification and sockets are outside. These observations establish
neither improvement nor transport share.

Production cleanup identity and settlement proof is delivered under
[validation #169](https://github.com/jeffglousher/coaptic-validation/issues/169),
closed by companion #172 after Linux run `40042232531`. It covers reserved
inherited groups in the same visible PID namespace with one owner/waiter;
arbitrary escaped-group or namespace containment is not claimed.
Physical #333/#365/#341 and publication authorization #132 stay separate.
