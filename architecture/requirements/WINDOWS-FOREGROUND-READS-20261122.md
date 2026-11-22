# Windows foreground UDP reads

The README server at `37d7b7d` slept for one millisecond after every App poll. A poll processes at most one eligible datagram. On this Windows host that sleep delayed the next block by about 1.5 ms. The revised loop polls the existing nonblocking UDP socket for normal reads until the next absolute App deadline. A queued datagram wakes the server immediately. An elapsed application deadline returns directly to the App. Both paths retain the same receive guard, capacities and protocol logic.

## Verified workload

Windows 11 x64, Ryzen 7 5800X3D, native loopback, release with line-table symbols, thin LTO, one codegen unit. The two foreground builds share the same standard-thread runtime, application and lockfile. Both come from companion `8215e251` with its native host-control representation handler and `coaptic-reusable` transport. One build sleeps after each poll. The other calls the example's readiness helper. A separate build with both policies selected at process startup provided longer clock and stack controls.

One frozen client, one outstanding CON, 32 warmups, 800 offered starts/s or closed-loop capacity. The client checked status, Content-Format, ETag, payload length and every byte. Warmup preceded measurement. Each measured 64 B read required one request and one response, each 4 KiB read four of each. OSCORE contexts were established before measurement. Each pair kept one route, four Calls, zero deferred operations, four RX and four TX datagram slots at 1,472 bytes, eight duplicate rows, four Observe rows, one RX body and two TX bodies. The fixture used the same 2,049-byte UDP scratch in both arms. The standalone README retains its own fixed storage and route set.

## Foreground results

Each cell gives original sleep / changed readability. Server and client CPU are process user plus kernel microseconds per verified read. p50 and p99 are end-to-end microseconds. Memory columns show closing working set and private memory in MiB for each process. Each value is the median of three profiler-off windows. Arm order reverses on the second pass.

**800 offered starts/s, longer windows.** Each small window verified 6,400 reads and each blockwise window 3,200. The original sleep loop exhausted its service capacity. The unchanged client spins while waiting for a reply, so its CPU cost includes the server delay.

| Security, body | Server CPU | Client CPU | p50 | p99 | Reads/s | Server WS | Client WS | Server private | Client private |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Unprotected, 64 B | 22.0 / 24.4 | 1,528.3 / 39.1 | 1,530.5 / 30.5 | 1,573.6 / 70.1 | 653 / 800 | 5.57 / 5.58 | 4.90 / 4.91 | 1.02 / 1.02 | 0.84 / 0.84 |
| Unprotected, 4 KiB | 73.2 / 43.9 | 6,113.3 / 175.8 | 6,131.4 / 100.5 | 6,217.1 / 181.3 | 163 / 800 | 5.61 / 5.62 | 4.91 / 4.92 | 1.02 / 1.03 | 0.84 / 0.84 |
| OSCORE, 64 B | 26.9 / 46.4 | 1,533.2 / 70.8 | 1,535.2 / 35.9 | 1,573.8 / 73.9 | 651 / 800 | 5.68 / 5.69 | 4.99 / 4.99 | 1.02 / 1.02 | 0.85 / 0.85 |
| OSCORE, 4 KiB | 107.4 / 97.7 | 6,147.5 / 146.5 | 6,152.4 / 127.4 | 6,244.6 / 227.0 | 163 / 800 | 5.71 / 5.72 | 5.00 / 5.01 | 1.03 / 1.04 | 0.84 / 0.84 |

**Separately linked executables, closed loop.** Each small window verified 1,536 reads and each blockwise window 768. These short windows resolve throughput and latency well. A single Windows CPU accounting tick is about 10.2 or 20.3 microseconds per read at those respective counts. The receipt also includes the 800-offered comparisons using these binaries.

| Security, body | Server CPU | Client CPU | p50 | p99 | Reads/s | Server WS | Client WS | Server private | Client private |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Unprotected, 64 B | 20.3 / 20.3 | 1,536.1 / 20.3 | 1,530.7 / 17.2 | 1,564.5 / 25.4 | 653 / 56,865 | 5.55 / 5.55 | 4.86 / 4.86 | 1.00 / 1.00 | 0.81 / 0.81 |
| Unprotected, 4 KiB | 101.7 / 20.3 | 6,144.2 / 61.0 | 6,134.6 / 74.6 | 6,233.9 / 106.2 | 163 / 13,428 | 5.59 / 5.59 | 4.89 / 4.89 | 1.02 / 1.01 | 0.83 / 0.82 |
| OSCORE, 64 B | 30.5 / 10.2 | 1,525.9 / 20.3 | 1,533.1 / 21.0 | 1,569.6 / 31.8 | 652 / 46,985 | 5.67 / 5.67 | 4.95 / 4.95 | 1.00 / 1.00 | 0.82 / 0.81 |
| OSCORE, 4 KiB | 101.7 / 61.0 | 6,144.2 / 101.7 | 6,146.5 / 94.6 | 6,227.2 / 129.0 | 163 / 10,423 | 5.70 / 5.69 | 4.98 / 4.98 | 1.01 / 1.02 | 0.82 / 0.82 |

**Equal delivered rate, separate executable pairs.** The client offered 550 reads/s for 64 B and 140 reads/s for 4 KiB, within the measured service capacity of both loops. Each window measured 3,200 or 1,280 complete reads. Both arms reached the target at the median in every cell. Two original windows delivered 539.0 of 550 unprotected small reads/s and 131.8 of 140 protected blockwise reads/s. All twelve paired comparisons recorded fewer server cycles with readability. Server CPU time varied across repetitions.

| Security, body | Server CPU | Client CPU | p50 | p99 | Reads/s | Server WS | Client WS | Server private | Client private |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Unprotected, 64 B | 29.3 / 34.2 | 1,157.2 / 68.4 | 1,537.1 / 47.3 | 1,711.9 / 71.8 | 550 / 550 | 5.55 / 5.55 | 4.89 / 4.88 | 1.00 / 1.00 | 0.82 / 0.82 |
| Unprotected, 4 KiB | 134.3 / 97.7 | 5,871.6 / 170.9 | 6,109.4 / 152.6 | 8,727.6 / 301.3 | 140 / 140 | 5.59 / 5.59 | 4.91 / 4.91 | 1.02 / 1.02 | 0.84 / 0.84 |
| OSCORE, 64 B | 34.2 / 39.1 | 1,191.4 / 63.5 | 1,533.2 / 54.7 | 1,609.7 / 81.4 | 550 / 550 | 5.67 / 5.66 | 4.98 / 4.98 | 1.01 / 1.00 | 0.82 / 0.83 |
| OSCORE, 4 KiB | 158.7 / 109.9 | 5,810.5 / 195.3 | 6,070.4 / 185.9 | 6,580.5 / 349.1 | 140 / 140 | 5.70 / 5.70 | 4.99 / 5.00 | 1.02 / 1.02 | 0.82 / 0.84 |

## Execution and waiting

A bounded process-local sampler walked the foreground server thread for 950 ms in each measured workload, with symbols from the same release image. The original sleep path appeared in 528/981, 564/943, 589/911 and 635/914 wall stacks for unprotected small/blockwise and OSCORE small/blockwise. The changed readability path appeared in 589/961, 488/1,004, 558/983 and 517/927. The samples also show Winsock send/receive, App dispatch and AES-CCM. A blocked Winsock call can appear at a sampling instant. Stack shares describe elapsed time. The independent process clocks above quantify execution. The stack walk changed server cycles per read by −0.3% to +21.9% in the 1,600-read observer-on/off controls. The performance comparisons ran with the observer off. Shared host tracing settings stayed intact.

On the separate current-thread Tokio control, an unprotected 64 B App burst took 32.5 microseconds and 107,060 server-thread cycles per send. The readiness await took 1,215 microseconds and 73,403 cycles per send. The difference includes time off CPU. This observer cannot separate the time before the socket became ready from time spent runnable before dispatch. Winsock send took 35,432–62,865 thread cycles per datagram across the four cells. Receive attempts together took 11,607–19,803. These calls nest inside the App burst.

## Comparison with libcoap
A common independent client ran three unpaced rotations against the checkout Tokio control and libcoap 4.3.5b at `851533c3`. Their respective median latencies were 65.1 and 77.0 µs for unprotected small reads, and 395.4 and 393.4 µs for unprotected 4 KiB. With established OSCORE the medians were 191.7 and 286.5 µs, and 1,059 and 1,334.7 µs. The wire capture showed an empty ACK and separate confirmable response for each protected libcoap block. Coaptic piggybacked its responses. This peer client and these protocol exchanges differ from the foreground capacity fixture.

## Other measured changes

- libcoap requests writable readiness when it has output pending. A Mio 1.2.4 receive-rearm experiment with Tokio 1.53.3 eliminated the observed writable-only events after UDP receive. Across four profiler-off capacity pairs per workload, median throughput moved from 59,139 to 47,002 and from 14,486 to 11,582 reads/s for unprotected small/blockwise. OSCORE moved from 47,842 to 41,570 and from 10,062 to 9,269. The stock dependency remains in use.
- Admission consults TX occupancy before and after trying to demote cached replies. A candidate removed repeated occupancy checks when no mutation intervened. Its full, minimal and strict-lint checks passed. Server CPU medians at capacity were equal in all four cells. Protected small-read p99 regressed by 7.3%, 19.5% and 7.3% in order. The candidate was reverted.
- A direct four-byte empty ACK encoder reduced the duplicate-victim function from 1,069 to 827 code bytes and its stack reservation from 776 to 392 bytes. At eight rows, measured demotion time moved from 75.3 to 70.7 ns. Replacing the selected allocated row also reduced duplicate key scans. The combined code lowered sampled victim appearances from 82/3,256 to 54/3,255 unprotected and from 53/3,335 to 31/3,157 protected in blockwise traffic. The direct ACK reduced unprotected small capacity in four of four pairs. The combined candidate reduced protected blockwise capacity and increased p99 in four of four pairs. Both experiments were excluded from the foreground change.
- The protected receive stack included `stitch_inner` after AES-CCM authentication. An experiment copied validated inner wire bytes directly when there were no retained outer options to merge. In 13,590 differential cases the result and every destination byte matched the original at short capacities, including empty codes, extended tokens and option overflow. Across six alternating runs, median reconstruction cost was 61.5 to 40.6 ns with no payload, 64.2 to 44.7 ns with 64 B, and 85.6 to 61.9 ns with 1 KiB. The cell held the option limit, AEAD, replay and wire validation fixed. Post-change stacks showed reconstruction in 9/902 versus 16/888 small-read samples and 6/858 versus 12/888 blockwise samples. Protected closed-loop medians fell from 53,630 to 52,602 and from 10,857 to 10,481 reads/s. Median server CPU increased from 18.2 to 18.9 and from 82.8 to 89.1 µs/read. At 800 offered reads/s small-read p99 went from 79.5 to 88.9 µs. The isolated saving did not repeat end to end, so the original reconstruction remains in place.
- With the common historical Tokio fixture `eea0374b` and its original release profile (debug info disabled), three-way alternating 9,600-read runs at 800/s compared the original project `b8f44fa`, direct ACK `835bd6c` and combined candidate `110fb53`. Median server CPU in that order was 39.1 / 37.4 / 58.6 and 135.1 / 127.0 / 122.1 µs/read for unprotected small/blockwise, and 58.6 / 53.7 / 57.0 and 154.6 / 166.0 / 140.0 for OSCORE. The historical fixture preserves the project-required AES 0.9.3 baseline and 0.9.4 checkout locks. Both original 50% server CPU-time targets remain open. The foreground deployment uses a different runtime from that historical series.

The [per-window receipt](WINDOWS-FOREGROUND-READS-20261122.json) identifies the executables, raw window digests, process CPU, cycles, latency, throughput, pacing and process memory. The accepted foreground patch has byte-identical library sources to the previous `f8ffae4` historical Tokio comparison. Against original project `b8f44fa`, median server CPU for that core went from 50.8 to 53.7 µs/read unprotected at 64 B and from 143.2 to 103.1 at 4 KiB. OSCORE went from 56.6 to 41.0 and from 141.1 to 143.2 respectively. The paired timing, pacing and CPU accounting guards leave each security target open. [Historical Windows baseline measurements](WINDOWS-TOKIO-ADAPTER-PROFILE-20261120.md)

## Correctness

The Windows example's three socket/deadline tests passed, including idle suspension until an absolute deadline, oversized-datagram refusal, an empty datagram and reuse of the same socket for a valid datagram. The full all-feature suite passed 1,711 tests, including Call retry, cancellation, expiry, duplicate replay and security checks. A release build of the standalone README server returned `21.5` for 16 fresh GETs and all 16 duplicate GETs on its own route. Blockwise and OSCORE figures above use the native fixture's representation route.
