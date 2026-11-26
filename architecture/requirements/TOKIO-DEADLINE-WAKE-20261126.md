# Windows separate responses on the shipped Tokio adapter

This comparison uses the Coaptic Tokio UDP socket and clock shipped with core revision `7687f92`. It follows the [matched acknowledgement comparison](EMPTY-ACK-CONTROLS-20261126.md), whose native server used its own Tokio adapter. In both services the server acknowledges each CON GET with an empty ACK and sends a separate confirmable response. The client then sends the final empty ACK. The 64-byte read takes four datagrams and the 4 KiB read takes sixteen. Established OSCORE protects content on both peers. An independent wire verifier checks both ACKs and their Message IDs. The frozen timing client checks the representation, its ETag and the expected datagram counts. Both protected fixtures used AES-CCM-16-64-128 and a 32-entry replay window. The C peer also sent its optional Size2 on blockwise responses.

The Coaptic service used four RX and four TX datagrams, eight dedup entries, four Observe entries, one route, four Calls, zero deferred replies, one receive body and two send bodies. Body slots were rounded to whole KiB. One OSCORE context and four request bindings were available in the protected service. Libcoap allocated its PDUs dynamically.

All 144 timing windows completed, verifying 1,497,600 reads with zero failures, cancellations or unattempted operations. The native client ran on Windows loopback with one outstanding Call and 32 warmup reads. Each load, security mode and body size has all six orders of the shipped adapter, the timer experiment and stock C. The capacity windows cover 28,000 small reads or 7,200 blockwise reads. Each paced window covers 3,200 reads. The unprotected libcoap service uses its public per-block separate-response handler. The established OSCORE comparison uses the stock libcoap encryption allocation. The earlier report retains the [right-sized C allocation control](MATCHED-ACK-20261125.md).

CPU is charged separately to the server and client processes per completed, verified read. Median and p99 refer to the complete read. Private bytes and working set are sampled at the end of each window. Medians in the tables are medians of windows. Both peers use their own server runtime, and this comparison leaves each peer's normal transport and storage ownership intact.

## What repeated

The shipped Coaptic server and stock libcoap used similar unprotected server CPU at capacity. Each server used less CPU in three of the six small-read pairs, even though Coaptic’s client used less CPU and its median read completed faster in all six pairs. At 4 KiB, median capacity was 9,752 versus 8,082 reads/s. An earlier unprotected cohort on a fixture adapter gave libcoap a small server CPU lead. These data support a throughput advantage on this host. They leave the unprotected server CPU ranking sensitive to operating point and cohort.

The clearest unprotected loss was the small read at 800/s. Coaptic used 78.12 µs of server CPU versus 46.39 µs for C, while client CPU was 83.01 versus 90.33 µs. Both kept the requested pace. Coaptic used more server cycles in all six pairs, with a median excess of 34.3%. At 4 KiB the median paired cycle excess was 8.1%, also higher in all six. The latency difference was small at 64 bytes. At 4 KiB Coaptic returned the verified body sooner in every paced pair.

Stock C used about twice the server CPU for established OSCORE at capacity. The previous controlled C experiment narrowed that difference by changing one transient encryption allocation from 8,388,880 bytes to the actual bounded ciphertext size. Its C server used 29.79 and 117.63 µs for the two sizes. That experiment used an earlier cohort. The stock comparison here and the earlier allocation control answer different questions. An allocation request also says little about resident memory. In the direct capacity run the stock C server held 0.94–0.99 MiB private and 5.40–5.58 MiB working set. Coaptic held 1.16–1.18 MiB private and 6.16–6.32 MiB working set. [Earlier three-way allocation comparison](MATCHED-ACK-20261125.md)

### At capacity

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | Coaptic unprotected | 25.39 | 25.39 | 23.6 / 57.2 | 38,956 | 1.18 / 1.01 | 6.16 / 5.18 |
| 64 | libcoap unprotected | 26.51 | 32.09 | 29.1 / 63.8 | 31,400 | 0.94 / 1.02 | 5.40 / 5.18 |
| 4,096 | Coaptic unprotected | 103.08 | 103.08 | 99.2 / 178.7 | 9,752 | 1.18 / 0.87 | 6.21 / 5.05 |
| 4,096 | libcoap unprotected | 106.34 | 123.70 | 115.0 / 205.6 | 8,082 | 0.96 / 0.87 | 5.43 / 5.05 |
| 64 | Coaptic OSCORE | 28.18 | 28.46 | 28.6 / 55.4 | 34,781 | 1.16 / 1.01 | 6.25 / 5.27 |
| 64 | libcoap OSCORE | 56.36 | 63.34 | 58.5 / 106.0 | 15,817 | 0.97 / 1.01 | 5.56 / 5.27 |
| 4,096 | Coaptic OSCORE | 122.61 | 123.70 | 120.2 / 220.3 | 8,060 | 1.18 / 0.87 | 6.32 / 5.14 |
| 4,096 | libcoap OSCORE | 238.72 | 272.35 | 260.6 / 411.0 | 3,654 | 0.99 / 0.87 | 5.58 / 5.14 |

### At 800 offered reads/s

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | Coaptic unprotected | 78.12 | 83.01 | 67.0 / 104.2 | 800 | 1.17 / 0.83 | 6.15 / 5.00 |
| 64 | libcoap unprotected | 46.39 | 90.33 | 65.0 / 103.9 | 800 | 0.94 / 0.83 | 5.40 / 5.01 |
| 4,096 | Coaptic unprotected | 151.37 | 170.90 | 151.9 / 293.6 | 800 | 1.19 / 0.84 | 6.22 / 5.03 |
| 4,096 | libcoap unprotected | 158.69 | 205.08 | 179.4 / 317.1 | 800 | 0.96 / 0.84 | 5.43 / 5.03 |
| 64 | Coaptic OSCORE | 61.04 | 70.80 | 72.0 / 111.7 | 800 | 1.17 / 0.83 | 6.26 / 5.09 |
| 64 | libcoap OSCORE | 100.10 | 139.16 | 104.0 / 160.3 | 800 | 0.97 / 0.83 | 5.56 / 5.08 |
| 4,096 | Coaptic OSCORE | 146.48 | 166.02 | 174.4 / 354.0 | 800 | 1.20 / 0.83 | 6.33 / 5.11 |
| 4,096 | libcoap OSCORE | 300.29 | 332.03 | 334.7 / 559.8 | 800 | 0.99 / 0.84 | 5.58 / 5.11 |

## What the timer cost

On the shipped adapter, a datagram send first polls its deadline. Tokio registers a sleep, wakes the I/O driver and posts to the Windows completion port. An immediately writable socket can finish within the same poll. The trial asked the timer for an owned clock check and evaluated it after constructing both futures. A strictly future Tokio deadline allowed the socket one first poll. A pending send armed the real timer before yielding. Subsequent polls and custom timers retained timer priority. The 14 core async tests and 13 adapter tests passed, including eager construction, a simultaneous deadline and ready send, cancellation, duplicate replay, pool refusal and reuse. The modified core and adapter also compiled on Rust 1.89. The full suite for the preceding Call ownership change passed 1,724 all-feature and 753 no-default-feature tests. [Ownership fix and regression coverage](https://github.com/jeffglousher/coaptic/pull/807)

A diagnostic binary counted accepted sends, message type and timer polls at the window boundaries. Across 256 small reads, polls fell from 512 to zero in each security mode. Across 256 four-block reads, they fell from 2,048 to zero. The independent verifier checked message types, payloads and ACK identities on both builds. The diagnostic counters confirmed the same number of sends. Enabling these counters on the same binary changed median large-read server cycles by +2.7% unprotected and +3.8% protected over four on/off pairs. Counters ran in separate processes from the timing trials.

The process-scoped wall stacks also located the missing wakeup. At 800 reads/s, unprotected 4 KiB sends accounted for 105 sleep-to-completion-post samples out of 3,213 baseline samples and zero out of 3,142 trial samples. Protected sends accounted for 107 of 3,003, then zero of 3,112. Other timer paths still posted to the completion port. The baseline traces contained 406 unprotected and 184 protected samples inside the completion-port wait. The trial contained 287 and 271. These waits include time when the server thread could sleep. Charged process CPU and process cycles were measured separately with profiling off. Kernel execution versus waiting inside a sampled syscall remains unresolved.

The largest remaining leaf in all four traces was `NtDeviceIoControlFile`. Of the 1,497 baseline unprotected samples at that leaf, 1,041 came through socket send, 334 through receive and 122 through Mio updating readiness interests. Of the 1,041 send samples, 564 were on the empty ACK path and 477 on the block response path. The protected baseline split was 994 sends, 285 receives and 141 readiness updates. The earlier unprotected C trace on the same peer binary also spent substantial wall time submitting sends and waiting in select. These samples are observations of call paths. They do not identify an avoidable send or measure exclusive kernel CPU. The JSON includes the weights and callers for both peers.

## Why 800 small unprotected reads/s cost more

Each peer completed 3,200 verified reads in a server-thread stack trace and another 3,200 under independent scheduler-state sampling. The Windows scheduler reported the Coaptic worker waiting in 195 of 204 observations and running in nine. The C worker was waiting in 201 of 208 and running in seven. Neither sample identified a worker waiting ready to run. The sparse running samples cannot resolve individual functions or exclude short ready intervals. This rules against attributing the paced gap to continuous server polling. The verifying client used the same spin setting with each server.

Coaptic had 1,272 socket-send stacks and 869 completion-port wait stacks among 3,343 wall samples. Libcoap had 1,664 socket-send and 1,461 Winsock select stacks among 4,686. Coaptic posted to the completion port in 119 samples. Its readiness-interest update appeared in 36. Final ACK routing appeared in 16. The observations put the next investigation at the send, rearm and wake boundaries shared by every datagram. Neither the number of samples in a socket call nor subtracting the two peers' wait counts estimates executing CPU.

The wall-stack sampler raised small-read p99 from roughly 104 µs to 5.68 ms on Rust and 6.48 ms on C. With the scheduler-state observer alone p99 was 105 and 107 µs. That lighter observer changed server cycles by +0.6% and −2.1% relative to each profiler-off median in these single windows. The full timed comparisons use only the unobserved runs.

## Checking the measurements

The first profiler-off timer experiment mistakenly reused a Python interpreter that had installed the stack sampler. All 96 affected windows and their 706,560 reads were withdrawn. The four intentionally profiled windows serve only for attribution. With the sampler running, 4 KiB p99 was 8.38–11.35 ms, versus 0.39–0.46 ms in the clean paced runs. Client CPU rose 402–536%. The sampler’s perturbation was large enough to reverse apparent workload conclusions.

The replacement capacity and paced phases ran in separate fresh interpreters. Each allowed exactly one release server and the frozen client per window and confirmed the two executable paths at the pre-measurement gate. Those 96 windows verified 706,560 reads. The independent three-arm comparison also verified both live peer paths, allowed only the peer binaries and Git provenance queries, and checked every raw digest and packet count. Earlier published cohorts used separate timing and profiling launchers according to a twelve-file source audit. Those earlier runs lack the new per-window subprocess records. The JSON retains the invalidated-manifest digest, all replacement digests, symbol and source identities, individual paired differences and the fixture and trial patches.

A separately linked control had the same optimized instruction bytes as the baseline. Four more pairs per cell verified 471,040 reads. Even with identical code, short 800/s windows could give very different Windows CPU charges. For example, median small unprotected server CPU moved from 75.7 to 95.2 µs, while the median within-pair change in cycles was −0.7%. Comparing adjacent pairs, process cycles, latency and throughput helps identify changes that survive that variation.

## Timer trial decision

The first clean A/B and the independent three-way comparison both found a real capacity gain. For 64-byte unprotected reads, the trial used 4.9% and 4.7% fewer paired server cycles. At 4 KiB it used 4.5% and 3.6% fewer. Protected 4 KiB throughput rose 5.8% and 2.7% in the median pair. Those results describe a server with a read immediately available after each response.

At 800/s, large unprotected reads used 16.1% and 11.6% more charged server CPU in the median pair. Small unprotected p99 rose 10.1% and 0.6%. Large unprotected median latency rose 7.8% in the first group and fell 1.8% in the second. Cycles changed +3.7% and −0.9%. Windows CPU accounting and the same-code control limit the precision of the paced comparison. No consistent paced blockwise saving emerged. The loop future grew from 11,848 to 11,872 bytes, while the App stayed at 15,928 bytes. We rejected the timer API change under the existing cross-workload regression criterion.

The table entries are medians calculated separately for each implementation. The percentages here are medians of changes within each chronological pair. A ratio of the two arm medians can even point in the other direction when the host varies across rounds. The receipt includes both calculations and individual repeats.

## Device and original-project targets

The last physical unprotected Waveshare comparison verified 192 reads across eight boots with piggybacked responses. For a seven-byte read, Coaptic and libcoap used 2.857 and 1.587 ms of server worker CPU. A 2,000-byte read used 6.53 and 3.82 ms. The excess was 1.27 ms for the small datagram and 1.28 and 1.38 ms for the first and following blocks. Median blocked time inside those measured worker intervals was zero. The almost fixed cost across packet sizes makes entry to the async poll and dispatch a stronger measurement target than bulk payload copying. Its cause remains unresolved. [Physical worker, latency, memory and throughput measurements](LANDED-LIBCOAP-20261125.md)

The C and Rust separate-response firmware images passed host wire checks. The board management endpoint timed out again on November 26 at 14:06 UTC, Windows enumerated zero serial ports, and the last inspected managed-device and router data showed the board unavailable. A physical comparison with both ACKs, and a protected C comparison with matched durable state, remain open. No image was uploaded during this qualification.

Relative to the original project, earlier matched capacity measurements reduced server CPU by 9.4% small and 8.2% blockwise in unprotected mode, and 1.7% and 6.1% under established OSCORE. The 50% reduction remains open independently in each security mode. [Original release, later release and same-source controls](EMPTY-ACK-CONTROLS-20261126.md)

[Full per-window CPU, latency, throughput, memory, call paths, build identities and reproduction patches](TOKIO-DEADLINE-WAKE-20261126.json)
