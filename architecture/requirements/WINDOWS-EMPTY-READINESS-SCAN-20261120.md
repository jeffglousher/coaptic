# Windows empty readiness scan experiment

Keep the accepted App wake path. The candidate leaves resource dimensions
unchanged and verifies all 96,000 measured reads. Only one of 16 matched
pairs meets the pacing, latency and server-cycle gates.

## Scope

The [Windows thread snapshot](WINDOWS-PRIMARY-THREAD-CYCLES-20261120.md)
locates cycle charges on the primary server thread. One repeatable shared
operation there is the App readiness query. The candidate computes the
original protocol timer result, then returns it directly if RX datagrams,
TX bodies and Observe interests are all empty. It uses the current pool
counts and leaves all occupied-pool scans, timer deadlines and priorities
intact. It adds no retained state and establishes no kernel attribution.

The baseline is accepted `285b7fbbf4d25581297ad5df34a963649ded3909`.
The candidate is `b4433194e8169d555d5b52354da06f19f94e4bf6`, retained with
its patch, binaries and raw windows in the bounded private evidence.
Both use the same native fixture from companion `7636b22`, selected
Syn 3 lock, compiler and frozen client. All 963 core unit tests pass on
the candidate, including Call, replay, expiry and pool cases. A separate
matched preflight verifies 2,560 complete reads.

Two rotated pairs per cell use fixed Service capacity and C1 CON, with
Mio and Tokio async, both security modes, and 64 B at 800 starts/s or
16 KiB at 200 starts/s. Each of 32 measured windows lasts approximately
six seconds with 32 separate warmups. The harness requires equivalent
capacity and accounting, at most 0.1% missed starts and 0.5% rate
difference, no more than +2 µs median and +10 µs p99, and at least 5%
fewer server process cycles in every pair to advance.

## Outcome

The windows report 94 missed periods. Ten pairs pass pacing. Five pairs
pass pacing and latency together. In both unprotected Tokio 16 KiB
pairs, server cycles fall, by 6.65% and 3.86%. The second result misses
the advancement threshold. Across all pairs, server-cycle changes
range from 7.21% fewer to 6.64% more, and client-cycle changes from 6.66%
fewer to 5.35% more. Preserve all pairs. The
[numeric receipt](WINDOWS-EMPTY-READINESS-SCAN-20261120.json) retains both
processes' separate CPU time and memory snapshots, the measured rates,
window hashes and individual guard decisions.

This rejects the isolated App change. No production, CPU-time,
throughput or peak-memory improvement is claimed. Both original #650
targets and peer and device scope stay open.
