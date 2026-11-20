# Windows async UDP adapter and original-project CPU comparison

Direct send and receive polling each reduced the targeted poll's thread cycles in three of four traces. Neither lowered server CPU time consistently across complete reads. Both patches were reverted. The retained change tests refusal of an oversized outgoing datagram and immediate reuse of the bound socket.

## What ran

We measured 1,094,400 verified CON reads on Windows loopback. Each release used MSVC x86-64, debug level 1, thin LTO, one codegen unit and the same frozen verifying client. Each window warmed up with 32 reads and offered 800 starts/s with one outstanding Call. The client checked the response code, Content-Format, ETag and complete representation. The 64 B and 4 KiB bodies required one and four request/response datagram pairs respectively. Protected traffic used an established OSCORE context.

The two adapter comparisons used the actual checkout `coaptic-tokio::Udp` in companion `8215e251`’s async App on a current-thread Tokio runtime. Both used checkout `f8ffae43`, its Syn 3 companion lock plus the local adapter package, the same socket and timer configuration and the same bounded host-control capacities. Each arm allocated one route, four Calls, zero deferred entries, one association, four request bindings, eight duplicate entries, four Observe entries and four RX and four TX datagram slots of 1,472 B each. One RX and two TX body slots each held 1,024 B or 4,096 B. Receive scratch held 2,049 B.

Each adapter experiment alternated four adjacent control/candidate pairs of 9,600 verified reads per cell with profiling off. Odd passes reversed both arm and cell order. The historical comparison used three adjacent original/current pairs, 16,000 reads per arm for small bodies and 14,400 for blockwise. A requested 16,000-read blockwise window exceeded the harness’s 60,000-datagram planning bound before either server started. The reduced blockwise count applied to both arms.

The CPU window opened after warmup and closed at the client’s measured signal, before report formatting. CPU time includes user and kernel time charged to each process. Windows recorded it in 15.625 ms increments. The comparison checked both processes for at least 20 ticks, rates within 0.5% of 800 and each other, at most 0.1% missed periods, and increases of at most 2 µs p50 and 10 µs p99. Pooled latency includes both arms of every complete pair, including guard failures. Memory values are closing process snapshots. All windows and source identities are in the [numeric receipt](WINDOWS-TOKIO-ADAPTER-PROFILE-20261120.json).

## Execution and waiting

The [process-scoped stack capture](WINDOWS-READ-PROFILE-20261120.md) found the Winsock send and receive paths, Tokio/Mio completion waits and smaller protocol and cryptographic costs. For the production adapter we added thread-cycle and wall-clock stamps around each individual send and receive poll in the single server thread. The gap between a Pending return and the next poll of the same future includes off-CPU socket and scheduler delays plus other executor work. Canceled pending futures contribute no resume interval. Separating socket-blocked time from runnable time would require further event attribution. Both phases report per datagram, including the extra receive polls. Shared host tracing settings stayed intact.

| Checkout case | Send cycles | Receive cycles | Send / receive active wall (µs) | Receive resume gap (µs) | Receive Pending / total polls |
|---|---:|---:|---:|---:|---:|
| Unprotected 64 B | 57,747 | 20,670 | 17.1 / 6.2 | 1216.0 | 2,048 / 4,096 |
| Unprotected 4 KiB | 32,980 | 11,832 | 9.9 / 3.5 | 292.1 | 2,039 / 4,087 |
| OSCORE 64 B | 54,918 | 19,964 | 16.3 / 5.9 | 1215.1 | 2,048 / 4,096 |
| OSCORE 4 KiB | 35,980 | 12,750 | 10.7 / 3.8 | 287.6 | 2,045 / 4,093 |

Each trace covers 2,048 completed datagrams between two fixed snapshots. Empty timestamp pairs cost 1,010 to 1,064 cycles and 0.33 to 0.35 µs. Across all 12 same-executable on/off cells, whole-process server cycles changed by -20.0 to +16.8%. This includes run-to-run variation. The receipt also retains per-cell CPU-time and latency changes with the probe enabled.

## Candidate results

**Send.** The candidate passed the numeric destination directly to `poll_send_to` through `poll_fn`. The checkout already supplied a numeric address. All sends in both sets of four traces completed in their first poll. No send-side scheduler wakeup was removed. For protected 4 KiB reads, whole-process server cycles fell in all four clean pairs, while server CPU time rose in three. The raw send patch is retained in the receipt.

**Receive.** The Windows-only candidate used `poll_recv_from` with a `ReadBuf` over the same guarded scratch. Review rejected the initial cross-platform form because Tokio 1.53.3’s read-direction poll and its `recv_from` future have different error readiness masks. Linux can report a socket error alone. The pinned Windows Mio 1.2.4 backend includes its connection-failure flag in the readable mask. The experiment kept the existing path on other platforms. Oversize refusal, buffer ownership and cancellation remained covered by the adapter suite.

| Case | Send cycles/datagram, checkout → send candidate | Receive cycles/datagram, checkout → receive candidate | Whole-server CPU change, send candidate | Whole-server CPU change, receive candidate |
|---|---:|---:|---|---|
| Unprotected 64 B | 57,747 → 52,558 | 20,670 → 18,788 | +50.0%, +111.8%, -38.5%, -70.3% | -80.7%, -3.2%, +81.8%, +10.0% |
| Unprotected 4 KiB | 32,980 → 35,646 | 11,832 → 12,172 | +19.7%, -23.7%, +21.6%, -45.1% | +44.7%, +72.7%, -23.2%, +2.5% |
| OSCORE 64 B | 54,918 → 50,028 | 19,964 → 17,444 | -40.5%, -4.3%, -37.2%, +233.3% | +42.9%, -15.4%, -38.3%, +300.0% |
| OSCORE 4 KiB | 35,980 → 32,844 | 12,750 → 12,384 | +9.5%, +43.8%, +9.3%, -15.3% | -42.5%, +9.5%, +111.4%, -25.0% |

## Checkout cost at the offered rate

Medians across the eight profiler-off production adapter control windows per cell follow. Each latency column is the median of window percentiles. Throughput is complete verified reads per second. Both memory columns show closing working set and private bytes, in that order.

| Case | Server CPU µs/read | Client CPU µs/read | p50 / p99 µs | Reads/s | Server MiB WS / private | Client MiB WS / private |
|---|---:|---:|---:|---:|---:|---:|
| Unprotected 64 B | 49.6 | 57.0 | 53.5 / 101.8 | 800.06 | 6.11 / 1.18 | 4.98 / 0.87 |
| Unprotected 4 KiB | 118.0 | 139.2 | 120.9 / 327.6 | 800.05 | 6.16 / 1.19 | 5.03 / 0.88 |
| OSCORE 64 B | 39.9 | 57.0 | 58.0 / 140.4 | 800.06 | 6.22 / 1.18 | 5.06 / 0.87 |
| OSCORE 4 KiB | 152.2 | 183.1 | 144.6 / 391.5 | 800.06 | 6.26 / 1.20 | 5.11 / 0.89 |

## Original project baseline

Both historical arms compiled the same companion `eea0374b` async App and its embedded `ReadyUdp` against original core `b8f44fa1` and checkout core `f8ffae43`. The historical lock differs only by the checkout-required AES update from 0.9.3 to 0.9.4. The source hashes and resolved paths were checked at build time. The original-baseline rows measure this common fixture. Ratios from the separate production adapter experiments cannot be combined with them.

| Case | Original → checkout server CPU µs/read | Original → checkout client CPU µs/read | Server CPU deltas by pair | Pacing, pair latency and CPU floor pass |
|---|---:|---:|---|---:|
| Unprotected 64 B | 50.8 → 53.7 | 71.3 → 77.1 | +31.1%, -15.6%, +5.8% | 1 / 3 |
| Unprotected 4 KiB | 143.2 → 103.1 | 170.4 → 140.0 | -37.5%, +82.4%, -62.9% | 1 / 3 |
| OSCORE 64 B | 56.6 → 41.0 | 74.2 → 53.7 | -3.5%, -76.3%, -25.0% | 2 / 3 |
| OSCORE 4 KiB | 141.1 → 143.2 | 166.0 → 171.4 | +32.9%, +1.5%, +12.5% | 2 / 3 |

Each CPU column shows the median of three windows for its own arm. The receipt also gives the paired latency, throughput, working set and private memory of each process. The unprotected and established OSCORE 50% server CPU-time targets are assessed independently against the original source.

- **Unprotected remains open.** Server cycles fell in all six pairs. Neither body size showed a consistent direction for server CPU time.
- **Established OSCORE remains open.** All three small-read pairs used less server CPU time, with reductions of 3.5%, 76.3% and 25.0%. All three blockwise pairs used more.

## Regression checks

All 12 adapter tests passed on the checkout control, send candidate and Windows receive candidate. They cover outgoing oversize refusal followed by same-socket delivery, incoming oversize refusal with the caller buffer intact, cancellation of a pending receive, protected binding refusal and reuse, duplicate replay and blockwise body recovery. The retained implementation passed strict adapter Clippy and rustfmt. The preceding source-matched core runs covered Call retry, cancellation and expiry among 1,711 all-feature tests.
