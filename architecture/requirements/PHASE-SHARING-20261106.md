# Shared dispatch and response phases

The integrated batch at native source `3e592adb1fd52c5e6918af1b4e810bb874413bad`
reduces complete server-App CPU cost by 0.625% in synchronous operation and
0.625% in asynchronous operation against pinned
`bd63199b652193b4c0a882e5840e72d992cf1362`. Both selected Service progress futures
shrink from 16,912 to 15,496 bytes: 1,416 bytes, or 8.37%. Selected App and pool
owners remain unchanged, including the 32-bit App layouts. Capacities, startup
allocation counts and supported security behavior are preserved.

## Changes and ownership

Blockwise dispatch uses the storage that holds the collected response body in a
later phase. The body borrow prevents another mutable App operation while that
view is live. Datagram-only dispatch retains its temporary scratch. Five retained
send helpers borrow immutable response metadata from their callers, reducing
nested async ownership without changing ACK, send or cancellation ordering.

Resident scratch capacity is declared on its backing type rather than stored in
every App. A nonzero encoded body length lets the shared enum use the existing
body storage without growing its owner. Custom memory profiles now specify
`ResidentRxScratch`, whose `ResidentScratch::CAPACITY` must equal its stable
writable length. Admission checks actual readable and writable slices before
entropy or I/O. Temporary `RxScratch` bounds remain unchanged.

The original replay implementation, owned client inbox and retained-chunk copies
remain. Their experimental replacements failed the frozen CPU checks and are
excluded from this integration. Preallocation bounds allocations; it does not
automatically make independent locals or async futures share storage. The shared
phase owner removes one such overlap without reducing configured capacity.

Large custom scratch backings switch initialized variants when collecting a
body. Startup and these custom switching costs are not measured by the steady
cohort. The 4,096-byte collection limit and body-slot release timing are unchanged.

## Measurement and limits

The retained comparison contains 64 process pairs for each of 28 matched
workloads, using AB/BA/BA/AB order repeated 16 times. It includes SmallDevice,
fixed and allocated Service, unprotected and established OSCORE, ordinary and
retained transfers, and actual sync poll and async wait/progress. No samples are
trimmed or replaced. Every workload's one-sided 95% upper CPU ratio is at most
1.02; both ordinary and combined sync/async groups pass the same bound.

Combined sync and async upper ratios are 0.996182 and 0.995725, respectively.
Ordinary sync/async point changes are -0.833%/-0.541%; retained sync/async are
-0.105%/-0.834%. Retained sync's upper ratio is 1.002490, so that subgroup shows
preservation within the declared tolerance, not an established CPU reduction.
The integrated aggregate result is not a claim that every workload gets faster.
The slowest point result is retained fixed OSCORE sync: +0.851%, with a +1.299%
upper bound. It passes the declared 2% preservation tolerance.

The clock measures complete server-App thread cycles with immediate-ready memory
transport and a pending timer. Client preparation, authentication and verification
are outside the measured interval. These results do not establish socket latency,
maximum throughput, executor scheduling cost, peak RAM or hardware qualification.
There is no new cross-library ranking. Equivalent-capability libcoap comparison
remains separate under the [runtime policy](../ASYNC-RUNTIMES.md).

Host SmallDevice App remains 14,936 bytes. Fixed Service remains 315,656 bytes;
allocated Service remains 35,672 inline plus 282,512 requested pool bytes.
SmallDevice's complete progress future remains 15,952 bytes. Encoder frames remain
1,280/1,296 bytes and all selected reclaim frames are nongrowing. Pool fields and
future components are nested owners, not additional totals. Requested heap
excludes allocator bookkeeping; compiler future/frame sizes are not peak stack or
grounds for reducing task reservations.

SmallDevice uses Constrained with 4 routes, 1 Call, no deferred work, 4 security
requests, 1 association, 2 RX/2 TX datagrams of 1,152 bytes, 4 dedup entries,
2 Observe entries and no bodies. Ordinary Service uses 32 routes, 4 Calls,
8 deferred entries, 4 security requests, 1 association, 32 RX/32 TX datagrams of
1,472 bytes, 64 dedup entries, 32 Observe entries and 4 RX/4 TX bodies of 16,384
bytes. Reported fields are compared directly; the pinned unchanged fixture/source
preserves capacities absent from individual outputs.

## Regression evidence

The focused source passes 926 library, 11 allocation/failure-injection,
27 retained-Q, 13 footprint and 5 scratch-admission checks. Cases cover zero,
boundary, oversized and 65,536-byte scratch, construction/drop timing, declared
capacity mismatch before entropy, and datagram-only bypass. Existing asynchronous
cancellation, refusal/retry, duplicate handling and security tests remain active.
The final native source adds one rustdoc line to the tested source; exact byte
comparison supports reuse of those tests. The original missing-doc lint failure
is retained, with passing formatting and strict lint after its correction.

General independent static review found no actionable correctness issue. A
separate evidence review checked source pins, accounting, claims and the complete
portable bundle. All nine final qualification lanes pass: no-default strict lint, all-feature
tests (1,681 passed; one explicitly ignored idle microbenchmark), no-default,
alloc, oscore and std library tests, both Rust 1.89 feature checks and rustdoc.
These results are retained with the evidence and do not replace the runtime
comparison.

## Reproduction and rejected experiments

The [companion evidence](https://github.com/jeffglousher/coaptic-validation/tree/3e03f251877cd44c7a6e8515ac1225500da95a0a/tools/benchmark/evidence/phase-sharing-static-20261106)
contains exact source patches, fixture hashes, binaries/PDBs, raw observations,
32-bit layouts, compiler frames, check logs and the comparable HTML report.
`verify.py` recomputes the result without executing the probes; `prepare.py`
reconstructs the native source. The initial host CRLF reconstruction mismatch is retained; the portable helper
now normalizes LF and verifies every recorded native source hash.

The untimed capacity-field precursor grew both selected 32-bit Service Apps by
8 bytes; its source, checks and failure are preserved. Earlier replay, inbox,
chunk and combined variants retain their original failed verdicts. No failed
hypothesis was resampled unchanged or relabelled as accepted. The prospective
rule for this distinct lifetime batch required selected owner/frame nongrowth
and strict Service future shrink, with the same CPU bound and pinned baseline.
