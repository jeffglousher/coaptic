# Fused allocated dedup expiry and deadline refresh

PR #732 removes a second table pass at the cached expiry boundary. App's existing
expiry walk now accumulates the conservative deadline bound while processing
rows in their original order. The allocated predicate reads only the existing
u64. No persistent field, buffer, allocation or capacity is added.

Pre-removal observations include expired rows: key-based removal can choose an
earlier duplicate in a custom backend. Including the observed deadline prevents
hiding a surviving row. A stale early bound can request another later walk; it
cannot postpone expiry. Fixed/custom storage uses the original untracked scan.
The pre-release `DedupSlots` contract and known callers are updated together.

## Confirmed improvement

Measured baseline: delivered #731, `b6a74d9f5bf9abf5d703ead1457157027ffb2b65`.
Measured candidate: `1c8636d68bff82e624b9c33750b201bf32a2cef5`.
Later documentation-only commits do not represent another runtime measurement.

Allocated Service, 1,024-byte C1 CON, after a synthetic 300-second logical gap:

- Unprotected B/A 0.9354143581: 6.46% less server App poll wall time.
  One-sided 95% upper ratio 0.9612320448.
- Established OSCORE B/A 0.9772917194: 2.27% less poll wall time.
  Upper ratio 0.9973823940.
- Steady allocated guards passed: unprotected upper1.0066438590 and
  OSCORE upper1.0023913424, each within the prospective1.02 bound.
- All four descriptive fixed/SmallDevice controls also remained within that
  bound. This does not establish a general fixed-storage benefit.

This separately declared comparison used CPU0 on a local AMD Ryzen7 5800X3D,
Windows, Rust/Cargo1.99 release thinLTO/codegen-units1, frame pointers, debug1,
incremental off, jobs2. The benchmark worker confirms its own CPU0 affinity
before spawning the inheriting native process. System/caller affinity is unchanged.
These are conditional single-CPU App wall results, not CPU/socket/device or
whole-memory qualification. Do not add these percentages to #731's gains:
workloads and processor conditions differ.

## Matched fixture and counts

Both sources built serially at the same physical archive/manifests path in the
existing reusable compiler lane. The private matched patch over companion
`a012247212c80402ce30731fce061fa3626f635b` changes only Service logical-time
preparation, adds out-of-clock ownership checks and a report label. The original
App poll timer, optimizer barriers, authentication and complete varied-body/
metadata verifier remain. The exact patched source and hash are retained.

`COAPTIC_EXPIRY_PROBE=idle` adds300000*operation milliseconds to protocol time;
`steady` uses the original clock. The offset is prepared outside the timer.
App, keys, PIV sequence, replay window and body/Call ownership stay continuous;
there is no reset between transfers. Every idle transfer checks exactly one
retained dedup reply and one TX owner afterward, proving expiry/release/reuse.
The offset costs a separate8-byte fixture static; the mode String is startup
fixture state. Neither is App memory.

Use the existing CLI with8192 measured operations and128 verified warmups:
`bench-core 8192 unprotected|oscore --service-allocated 1024`, the same
`--service-fixed 1024`, or `--small-device` (32bytes, steady only).

Service has routes32/populated1, Calls4/deferred8, association1/bindings4,
RX/TX32x1472, dedup64, Observe32, RX/TX bodies4x16384. SmallDevice retains
Constrained pools, routes4/populated1, Call1/deferred0, association1/bindings4,
RX/TX2x1152, dedup4, Observe2, no bodies. The partner uses unchanged bounded
allocated client pools. No automatic growth is enabled.

Eight cells: allocated idle unprotected/OSCORE, allocated steady both,
fixed Service idle both, SmallDevice steady both. One one-operation qualifier
per arm/cell precedes12 process pairs, AB on even pairs, BA on odd, cells in
that order:192 timing processes,16 qualifiers,1,572,864 measured poll values,
786,432 separate measured-process calibration values. All values are retained.
Per cell, compute each process mean,12 log(B/A) ratios, exp(mean), and upper
exp(mean +1.7958848187*sampleSD/sqrt12). Both allocated idle upper ratios must
be below1; both allocated steady upper ratios must be at most1.02. Fixed
controls are descriptive with a1.02 warning. All eight passed in this condition.
No pooling, outlier exclusions, calibration subtraction or replacement samples.

## Preserve limitations and earlier results

The preceding unpinned condition remains separate: both allocated idle benefits
passed, but the steady unprotected upper1.038647315 missed its guard despite
its slightly faster point estimate. Observed process-mean variation was4-22%
in several arms. That does not prove CPU migration caused it. The separately
specified affinity condition removes one uncontrolled input; none of the
unpinned observations is included in the single-CPU result.

Initial private-probe compilation failed because its outside-clock occupancy
check needed mutable Engine access. The exact failure/source is retained; it
was fixed before any native qualification or timing. Core source was unchanged.
The retained matched raw pack has208 exact JSON members, SHA256
`1aad6a289f0b9e8cf351d2c10e10d3fdfc9448079f361ef013f2ac992ea97f88`, plus per-member sizes/SHA values, both EXE/PDBs and build inputs.

Strict Clippy and1209 all-feature/713 minimal library tests passed, as did
8/5 dedup-capacity and13/11 footprint checks. Every emitted memory ledger is
identical between arms. Review used installed correctness/security guidance
inline; it was not an independent agent review. Transient emitted stack cost
and broader CPU/socket/device behavior remain unproven.
