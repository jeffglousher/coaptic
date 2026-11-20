# Windows UDP phase attribution

At 800 offered complete reads per second, direct Mio used fewer server process
cycles than either Tokio path. With synchronous App polling, Mio and Tokio
made the same number of socket API calls, and their measured socket and App
spans were close. The difference motivates testing the reactor and scheduling
path. The measurement cannot locate the additional cycles inside Windows or
Tokio.

## Scope and provenance

The instrumented release fixture uses clean core `416ffc3` and companion
`a5e49ed`, tree-equal to accepted main `42100e1` and `571aa85` respectively.
Production Rust is `b875088`. A private manifest overlay points Cargo to that
core and its `coaptic-io`. Cargo metadata confirmed both paths and the lock
matches the companion's committed Syn 3 lock. Build inputs are Rust 1.99.1 MSVC,
thin LTO, one codegen unit, Tokio 1.53.3 and Mio 1.2.4. The recorded machine is
Windows 11 Pro build 26200 on a Ryzen 7 5800X3D. The
[numeric receipt](WINDOWS-UDP-PHASES-20261119.json) has the source trees, input
and binary digests, phase counts and both processes' measurements per window.

All windows use the frozen client from the
[preceding screen](WINDOWS-RUNTIME-SCREEN-20261119.md), C1 CON, 64-byte
representations, the host-control profile and fresh fixture credentials for
established OSCORE. The fixed host-control configuration has one route, four
Calls, zero deferred entries, one association, four security request bindings,
eight dedup and four Observe entries, four RX and four TX datagrams of 1,472
bytes, and one RX and two TX bodies of 1,024 bytes. Each window has 32 warmups
and 6,560 verified operations. The client verifies status, format, ETag and
complete body. All 282,080 measured reads in 43 windows pass. There are 121
start-period misses in aggregate. The receipt retains individual pacing and
latency observations.

## Attribution and probe cost

The probe keeps fixed process-local counters on the single server thread and
writes two cumulative snapshots, after successful sends 2,048 and 6,144. It
uses a monotonic wall clock around completed calls or individual future polls.
The interior delta spans 4,096 sends. Timed async polls exclude time between
polls. Mio's blocking poll includes idle wait. App spans contain the socket
spans. Nested timers, preemption and probe bookkeeping affect the measurements.
Socket API calls and pending polls do not establish Winsock syscall counts.
The client and server process counters use the full client CPU gates, outside
warmup and report formatting.

- Both sync arms make two App polls, two receive API calls (one empty), one send
  call and one deadline lookup per reply. The timed repeats show 4.06–4.34 µs
  across receive calls and 13.61–14.82 µs during send. Tokio's inclusive sync
  App polls take 23.39–25.48 µs and Mio's take 22.17–24.98 µs. Tokio's readiness
  wait polls add 1.60–1.74 µs of active wall time per reply.
- Tokio async makes two App wait polls and two receive polls, with one pending
  in each phase. It makes one App progress poll and one send poll per reply.
  The root polls average 1.031 per reply, consistent with the explicit yield
  after every 32 passes. Summed wait and progress wall time is 27.23–29.56 µs
  including the socket polls. The root future spends 28.66–31.00 µs per reply
  inside polls. The runtime work outside the root polls is unmeasured by the
  probe.
- With instrumentation disabled, server cycles per verified operation span
  97,048–105,971 for Mio, 135,550–144,828 for Tokio sync, and 148,790–159,373
  for Tokio async. The probe binary with the probe disabled still uses
  2.0–10.6% more server cycles than the earlier frozen Tokio server in paired
  windows. Count-only varies from −4.5% to +4.2% against disabled. Timing varies
  from −2.4% to +7.0% against disabled. The observed differences include host
  variation and binary layout. The call counts support comparison of the paths.
  Small timing differences should be interpreted within these limits.

Server process time ranges from 4.76 to 76.22 µs per verified operation across
all windows. Two frozen async OSCORE windows report 57.16 and 4.76 µs with
essentially equal process cycles. The cycle advantage for Mio is useful for
screening on this machine. The Windows counters measure different quantities,
and wall timings inside a phase include descheduling. They cannot be combined
to infer a CPU-time residual. Working-set and private-memory snapshots describe
the observed Windows processes. They do not bound runtime or App peak memory.
See [Microsoft's process cycle counter contract](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryprocesscycletime).

## Reactor dequeue experiment

Direct Mio configures room for eight events per dequeue and Tokio's default
is 1,024. The bound becomes the maximum number of I/O completion entries
dequeued in a reactor call on this Windows path. A private same-binary
experiment varied only this bound on the host-control Tokio server and kept
the enabled I/O and timer drivers. Both App execution modes and protections
used 64 and 4,096-byte complete reads and the same capacity within each
matched pair. Both body ceilings were 4,096 bytes in the larger workload.
The build uses the accepted fixture source with only the startup selector
added. The intervention isolates the setting. Exact kernel attribution
requires additional evidence.
See [Microsoft's completion-port dequeue contract](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getqueuedcompletionstatusex).

Two reversed passes cover 16 adjacent pairs. All 256,000 reads verify, with
8,000 per window and 32 warmups. Each small read uses one datagram exchange.
Each blockwise read uses four. Achieved rates are 800.04–800.10 reads/s,
with 110 start-period misses in aggregate. The screen required at most 0.1%
missed starts per window, matching achieved rate within 0.5%, p50 regression
at most 2 µs and p99 regression at most 10 µs, and at least 5% fewer server
cycles per operation in every pair. The threshold was for advancing the
candidate to more trials.

The eight-entry setting saves 10.4–16.0% server cycles and 1.0–6.9% client
cycles in the 64-byte pairs. In blockwise pairs it saves 1.7–10.1% server
cycles, with up to 8.7% more client cycles. Four blockwise pairs miss the
advancement gate. Async unprotected repeat 2 exceeds the p99 guard by
reporting +34.3 µs. Async OSCORE repeat 2 reports +15.4 µs at p50 and
its control misses 18 periods. Three other low-gain pairs, including the
async unprotected repeat 2 pair, save less than 5% server cycles. Across all
blockwise pairs the median direction varies. Process CPU time also varies.
The receipt retains both processes, every window and the observed memory
outlier in the default-bound arm. Working-set samples cannot establish
peak allocation or runtime memory savings.

**Decision:** keep the default dequeue bound. This screen demonstrates a
repeatable small-read cycle effect and leaves the wider Windows runtime
candidate unqualified. A related reliability check reproduced oversized
receive refusal terminating several benchmark server paths. Preserve that
evidence and verify same-process reuse before further adapter screening.

The original `b8f44fa` baseline and both 50% CPU-time targets remain governed
by the [acceptance matrix](PERFORMANCE-ACCEPTANCE.md). The Windows phase
experiment adds no peer, device, overload, or full capacity qualification.
