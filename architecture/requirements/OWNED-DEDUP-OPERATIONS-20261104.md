# Allocated reply retention and expiry batch

PR #733 implements one combined allocated-storage change. It is a draft, not
delivered on main: the development screens retain failed fixed-storage guards.

App previously repeated key searches while replacing a reply or inserting into
a full table. AllocMemory now selects the existing row or first oldest eligible
victim in one walk, releases its TX pin before its row, and acquires through the
original occupancy cursor. Its private table enforces unique keys, permitting
direct row release without changing public duplicate-insertion behavior.
Expiry also releases due rows and refreshes the live deadline in one pass.
Generic fixed/custom storage retains its original operations through default
non-mutating hooks. No persistent fields, buffers or capacities are added.

## Observed allocated CPU-work improvement

Baseline: delivered #732, `cf25b30fca5f8f06392aaa51a70c2498fe974a35`.
Candidate: `a5bc973e1dd65b4b97c37028086307a08f8f7d8a`.
Later documentation changes are not another runtime measurement.

The paired six-cell allocated integration ratio is **0.9488096000**, with
one-sided95 upper **0.9573070457**: 5.12% fewer measured thread cycles.
Every allocated cell individually has an upper bound below1:

- Idle: unprotected ratio0.9267147356/upper0.9348013993;
  established OSCORE0.9764154528/0.9863707318.
- Steady: unprotected0.8921691982/0.9048781673;
  OSCORE0.9613612258/0.9796645307.
- Burst: unprotected0.9710680209/0.9912176116;
  OSCORE0.9680777291/0.9995294785.

This is instrumented complete server App::poll CPU-cycle evidence on one local
Windows Ryzen7 5800X3D, CPU0. The counter is Windows
[QueryThreadCycleTime](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-querythreadcycletime),
including user/kernel work and counter-call overhead. Calibration is retained
separately and never subtracted. Microsoft warns that timer frequency behavior
varies: cycles are not converted to elapsed time or CPU seconds. Request
preparation, partner polling and authenticated complete response verification
remain outside the counter boundary. Fixture receive/send copies remain inside.

## Failed guards remain failed

The CPU-cycle screen is **NOT_ESTABLISHED overall**: 11 of14 individual guards
pass and three fixed-Service guards fail their required upper bound1.02:

- Fixed idle unprotected: ratio1.0082825377, upper1.0212945780.
- Fixed steady OSCORE: ratio1.0182067071, upper1.0404744649.
- Fixed burst unprotected: ratio1.0225556188, upper1.0378427411.

The preceding wall comparison of the same integrated source also remains
NOT_ESTABLISHED: primary allocated ratio0.9897054042/upper1.0517875697,
seven of14 guards pass. The separate counter condition answers a CPU-work
question; it does not rescue that wall result or establish a general latency
gain. Large process-mean spikes and individual stalls were observed, but their
cause is not proved. No observation is excluded or replaced.

Two earlier per-row API variants failed; bulk expiry alone improved its primary
allocated wall estimate but failed five guards. All are retained separately,
without pooling, changing their gates or presenting them as delivered wins.

## Reproduction and affected checks

Both arms use the same private matched source path/fixture over companion
`a012247212c80402ce30731fce061fa3626f635b`, Rust/Cargo1.99 MSVC release thinLTO,
codegen-units1, frame pointers, debug1, incremental off, jobs2. Both binaries and
PDBs, exact build inputs/locks, commands, sources and raw observations remain
under the private phases named below. Builds used the existing registered lane;
no shared checkout, lock, CPU affinity or runtime was changed.

Each final condition has eight process pairs, AB/BA alternating, across14 cells:
allocated/fixed Service idle/steady/burst in both security modes, plus SmallDevice
steady in both. Each process has128 verified warmups,24576 measured polls and
4096 calibrations. All28 one-operation qualifiers and224 timing processes
completed per condition:5,505,024 measured polls and917,504 calibration values.
Use each process's complete mean, paired log ratios and upper
`exp(mean + 1.894578605061305 * sampleSD / sqrt(8))`. The primary integrates the
six allocated log ratios equally within each pair before that calculation.
It is a declared workload mix, not a production traffic distribution.

Service uses routes32/populated1, Calls4/deferred8, association1/bindings4,
RX/TX32x1472, dedup64, Observe32, RX/TX bodies4x16384, payload1024. SmallDevice
uses Constrained storage, routes4/populated1, Call1/deferred0, association1/
bindings4, RX/TX2x1152, dedup4, Observe2, no bodies, payload32. Steady keeps the
original logical clock; idle advances300000ms per operation; burst advances
300000ms every64 operations. Nonsteady checks exact dedup and TX ownership
outside timing. App, keys, PIV/replay state and identifiers remain continuous.
Synthetic gaps are not real throughput.

Local checks pass:1211 all-feature/713 minimal library tests,8 dedup-capacity
tests,13 footprint checks, strict all-target/all-feature Clippy and formatting.
Two differential mutation tests compare rows, replay bytes, occupancy, cursor,
deadline behavior and TX pin ownership against the original algorithms, including
raw occupied-empty slots, zero/full capacity, ties and failed TX release.
All14 paired emitted memory ledgers are identical. Peak stack, initialization
and emitted code costs remain unmeasured. Review used general inline
correctness/security guidance, not an independent reviewer.

Private evidence, retained once with per-file hashes and independent raw-value
recalculation records:

- `D:/CoapticCurrent/dedup-known-row-batch-20261104`: both rejected per-row cohorts.
- `D:/CoapticCurrent/dedup-bulk-expiry-20261104`: expiry-only cohort.
- `D:/CoapticCurrent/dedup-integrated-store-20261104`: combined wall cohort,
  source review and local checks.
- `D:/CoapticCurrent/dedup-integrated-cycles-20261104`: separate counter cohort.

Each phase stays below its96MiB ceiling; shared compiler caches are preserved.
The counter probe had preparation-script quoting/exact-source assertion errors
before compilation; no partial native samples resulted. No device, socket,
whole-memory, general CPU-time or original50% objective is established.
