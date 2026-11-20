# Windows completion dequeue follow-up

Keep Tokio's default completion dequeue bound. A 64-entry bound saved
7.4–12.3% server cycles in the 64-byte pairs, but seven of sixteen matched
pairs missed the advancement gates. The isolated Windows API test confirms
that the maximum-count argument affects cost on this host. Its result cannot
predict the complete operation cost under load.

## Isolated Windows control

One thread used one private completion port and the same 1,024-entry array
for every call. It verified exactly one previously posted completion or
zero entries on an empty nonblocking call. Eight alternating order sets
covered bounds 8, 64 and 1,024 for both cases. Each of the 48 windows had
30,000 calls. The prequeued case includes posting, validation and Python
and FFI work. Neither case uses Winsock, Tokio or the App.

Against 1,024, the 64-entry argument uses 12.1–18.4% fewer thread cycles on
prequeued calls and 21.0–25.5% fewer on empty calls. The 8-entry argument
uses 15.2–21.2% and 22.2–27.7% fewer cycles respectively. The receipt
separates elapsed time, thread CPU time and cycles. This keeps the array's
startup memory fixed while varying the dequeue argument. It leaves blocking
socket wakeup cost and the exact kernel code responsible unresolved.
See [Microsoft's completion-port API contract](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getqueuedcompletionstatusex).

## Matched complete operations

The release build uses core `58eecc1`, tree-equal to accepted main
`7c93123`, and companion `3fbce2d` with the
[oversized receive recovery fix](https://github.com/jeffglousher/coaptic-validation/pull/268).
The same binary runs both bounds on the Windows host documented in the
[phase attribution](WINDOWS-UDP-PHASES-20261119.md). The only fixture
experiment is the startup selection of the reactor bound. The App resources,
both drivers, worker count, workload and frozen client match within pairs.
The locked build explicitly resolves the intended core and its macro crate.

Two reversed passes cover Tokio sync and async App execution, unprotected
and established OSCORE, and 64 and 4,096-byte complete reads. All 256,000
reads in 32 windows verify at 800 offered starts/s, with 131 missed start
periods. The [numeric receipt](WINDOWS-IOCP-BOUND-20261119.json) preserves
accounting, latency, estimated process memory, CPU time, cycles and the
predeclared guards for each pair.

- Eight small-read pairs use 7.4–12.3% fewer server cycles. Client cycles
  range from 2.3% less to 2.3% more.
- The blockwise pairs range from 10.5% fewer to 2.6% more server cycles
  and from 3.4% fewer to 7.3% more client cycles. In one async unprotected
  blockwise pair, the smaller bound raises median latency by 4.85 µs and
  p99 by 88.7 µs and misses ten start periods. One async OSCORE small-read
  pair raises p99 by 68.5 µs. Process CPU time varies independently and the
  results do not qualify a CPU-time gain.
- Post-window server working-set snapshots are 6.15–6.56 MB at 64 and
  6.19–6.61 MB at 1,024. These samples describe the whole observed process.
  Startup allocation, runtime stack and kernel ownership remain separate.

## Socket-free scale check

Four rotated gated batch runs per security mode verify 400,000 complete
64-byte App operations using the frozen core fixture from the
[preceding screen](WINDOWS-RUNTIME-SCREEN-20261119.md). Its production Rust
matches the current runtime. The work gate covers synchronous App polls and
in-memory packet copies. Response verification follows the gate. There is no
UDP, kernel readiness or Tokio driver in this workload. Server process
cycles are 1,890–1,950 per operation unprotected and 4,656–5,384 with OSCORE.
The receipt preserves raw time and cycle samples and all empty gate controls.
The different workload and boundaries prevent computing a network residual
or treating this check as the original #650 comparison.

Keep both rejected cohorts and their guards. Further queue-bound tuning
requires evidence of a distinct mechanism. The async fixture's extra root
poll every 32 passes is directly observed. A bounded follow-up can test
whether explicit yielding still costs cycles when the preceding App wait
has already returned Pending. Keep the work budget for continuously ready
ingress and recheck complete operations and recovery before adoption.

The original #650 CPU-time baseline, both 50% targets, peer speed and hardware
gates remain open.
