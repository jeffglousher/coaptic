# Allocated dedup expiry: scoped improvement

PR #731 keeps one conservative deadline bound in `AllocMemory`'s dedup table.
Successful fresh insertions lower it; removals may leave it conservatively early.
Before that deadline App skips the table expiry walk. At the boundary it refreshes
from occupied payloads, then uses the original expiry/pin-release loop.

Fixed `Memory` and `DedupTable` source/layout are unchanged. Custom and fixed
storage use the default request for the original scan. Allocated storage retains
one extra `u64` inline, with no new allocation, per-row metadata or capacity.
The existing whole-App and heap ceilings remain unchanged.

## Measured successes

Accepted comparison source A: `c655b3aac485c3b88d3811b4da836b56210fce5b`.
Measured candidate B: `2321164e2ab97f004c48243954e46a9391dcc474`.
Unmodified companion `bench-core`: `a012247212c80402ce30731fce061fa3626f635b`.
Later documentation commits do not inherit another runtime measurement.

Allocated Service, 1,024-byte C1 CON responses:

- Unprotected: B/A 0.9347474864, a 6.53% reduction; one-sided 95% upper B/A
  0.9476177355 (5.24% reduction at that bound).
- Established OSCORE: B/A 0.9823348245, a 1.77% reduction; upper B/A
  0.9887759885 (1.12% reduction at that bound).

These are conditional local Windows complete-server-App poll wall results.
Request preparation, partner polling and authenticated response verification are
outside the clock; fixture receive/send copies remain inside. They establish
neither server CPU, socket/device benefit nor a general host-performance guarantee.
Expiry-heavy timing remains unmeasured; a boundary refresh adds a bounded pass.

## Preserve the regression uncertainty

The six-cell confirmation did **not** pass every prospective percentage guard.
The primary allocated/unprotected benefit and allocated/OSCORE guard passed.
Fixed Service point changes were +0.74% unprotected and +0.90% OSCORE, with
upper bounds +2.41% and +2.46%, exceeding the 2% guard. SmallDevice unprotected
was +0.06%, upper +2.13%; SmallDevice OSCORE passed (upper +1.73%).
Do not describe this as an all-green performance screen, no-regression proof,
or a fixed-storage speedup. The scoped allocated gains, unchanged fixed
storage representation, correctness review and tests are the delivery evidence;
fixed-path percentage stability remains an open performance limitation.
No failed result was pooled, removed or relabelled as passing.

## Reproduce the workload

Use Rust/Cargo 1.99.0, x86_64 Windows MSVC, release thin LTO, one codegen unit,
frame pointers, debug=1, incremental off, jobs=2. Build each revision serially
from the **same physical dependency/manifest path**, retaining each EXE/PDB
before the next build. Use the source-matched native lock and `--locked`.

For each retained executable, use these unchanged CLI routes:

```text
bench-core 16384 unprotected --service-allocated 1024
bench-core 16384 oscore --service-allocated 1024
bench-core 16384 unprotected --service-fixed 1024
bench-core 16384 oscore --service-fixed 1024
bench-core 16384 unprotected --small-device
bench-core 16384 oscore --small-device
```

Service: routes32/populated1, Calls4, deferred8, association1/security bindings4,
RX/TX32x1472, dedup64, Observe32, RX/TX bodies4x16384. The receiving partner
uses the unchanged bounded allocated client. SmallDevice: routes4/populated1,
Call1, deferred0, association1/bindings4, RX/TX2x1152, dedup4, Observe2,
no bodies, 32-byte representation. No pool capacities grow.

One one-operation qualification per arm/cell precedes timing. Then execute
12 process pairs, alternating AB/BA, in fixed Service, allocated Service,
SmallDevice order, each unprotected then OSCORE: 144 timing processes,
2,359,296 measured polls and 589,824 separate calibration values. Each process
also verifies 128 warmups. Keep every value; never subtract calibration.
Compute a mean per process, then log(B/A) per pair. Report exp(mean) and the
one-sided upper exp(mean + 1.7958848187 * sample_sd / sqrt(12)).

Raw matched data: 156 exact native JSON outputs, gzip SHA-256
`11832d4938c1030a7e45eaa23cd95ecbe9ca87e4f53e71a348bf508298717ee4`. The owner's retained raw pack and member SHA manifest include
all qualifiers, samples, capacities and memory reports. The earlier broad-cache
failure, initial allocated-only build-path mismatch, and first caller-assertion
failure are retained separately; none contributes to the matched result.

Review used the installed correctness/security guidance inline; no independent
reviewer was invoked. Strict Clippy, library tests and unchanged footprint/capacity
checks passed on the measured source. Authentication/replay ordering, replay
bytes, slot cursor, key-removal and TX pin release remain unchanged.
