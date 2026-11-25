# Where Coaptic spends a read

The original Windows server and the integrated packet-path changes served the same bytes with the same Tokio service, security policy and pool capacities. The integrated build completed more work per second at capacity in 15 of 16 paired windows. The same build still took longer than libcoap to serve both the seven-byte value and the 2,000-byte representation on the Waveshare. Each conclusion applies to a different deployment. [Process windows, boot-level observations and source identities](CHAINED-READS-20261125.json)

## Current checkout against the original

The release build with source diff `3eb1b84701050813796a3f07aa283aff75a1425b7e29d6c56c012727808b59a5` and the original project baseline each used the frozen Tokio server fixture, the same verifying client, one outstanding confirmable request and unchanged service capacities. Four AB/BA pairs per workload produced 658,944 verified reads with the profiler off. The figures are medians of process windows. The original appears first in each comparison.

**At capacity**

- **64 B unprotected.** Throughput 56,686 to 61,913 reads/s. Server CPU 17.2 to 16.2 and client CPU 17.2 to 16.2 microseconds/read. Median latency 16.7 to 15.2 and p99 25.0 to 23.2 microseconds.
- **4 KiB unprotected.** Throughput 13,413 to 14,761 reads/s. Server CPU 72.9 to 67.1 and client CPU 74.2 to 67.7 microseconds/read. Median latency 71.2 to 64.3 and p99 106.0 to 98.0 microseconds.
- **64 B established OSCORE.** Throughput 50,812 to 51,705 reads/s. Server CPU 20.0 to 19.6 and client CPU 20.0 to 19.6 microseconds/read. Median latency 18.4 to 19.4 and p99 28.2 to 29.6 microseconds.
- **4 KiB established OSCORE.** Throughput 10,238 to 11,137 reads/s. Server CPU 93.8 to 87.9 and client CPU 97.0 to 89.2 microseconds/read. Median latency 95.8 to 87.1 and p99 130.2 to 122.3 microseconds.

**At 800 offered reads per second**

- **64 B unprotected.** Throughput 800 to 800 reads/s. Server CPU 28.1 to 41.5 and client CPU 37.8 to 64.7 microseconds/read. Median latency 48.3 to 45.6 and p99 79.8 to 77.6 microseconds.
- **4 KiB unprotected.** Throughput 800 to 800 reads/s. Server CPU 119.6 to 116.0 and client CPU 140.4 to 127.0 microseconds/read. Median latency 135.1 to 127.2 and p99 318.8 to 309.1 microseconds.
- **64 B established OSCORE.** Throughput 800 to 800 reads/s. Server CPU 53.7 to 36.6 and client CPU 70.8 to 74.5 microseconds/read. Median latency 54.9 to 52.1 and p99 87.0 to 84.6 microseconds.
- **4 KiB established OSCORE.** Throughput 800 to 800 reads/s. Server CPU 183.1 to 181.9 and client CPU 216.1 to 208.7 microseconds/read. Median latency 192.7 to 181.1 and p99 287.6 to 271.3 microseconds.

The original server used 6.13 to 6.28 MiB of working set and 1.21 to 1.24 MiB private. Its client used 5.02 to 5.12 MiB of working set and 0.92 to 0.93 MiB private. The current server used 6.01 to 6.15 MiB of working set and 1.17 to 1.19 MiB private. Its client used 5.02 to 5.12 MiB of working set and 0.91 to 0.93 MiB private. These ranges cover the four capacity cells.

Throughput was higher in 15 of 16 capacity pairs. At 800/s, small unprotected reads took more charged CPU on both processes in this campaign. At capacity, small protected median and tail latency increased. A 50% reduction in server CPU remains open independently for both security modes. The executable-to-executable comparison includes the recorded dependency changes. [Complete windows and source identities](CHAINED-READS-20261125.json)

Validation on the retained source passed 1,716 all-feature workspace tests and 745 heap-free, no-default library tests. Strict Clippy, Rust 1.89, release Xtensa OSCORE compilation and the executable examples passed. Fixed and allocated protected clients each verified a 200-byte reply. Cases include authentication failure, duplicate replay, Call retry, cancellation and expiry, block bounds, short-buffer refusal, reuse and Observe replacement. Across the included Windows read campaigns and controls, 4,152,064 operations verified. Timings of the original project, the integrated server, the exact checkout and rejected candidates retain their distinct source and binary identities.

## First single-extraction build against the original

The first single-extraction source diff is `1fec14b58f2d0cc91c712dc7d876818e5363dcb5f63cf1034d1c56ccb72aa77e`. A separate four-pair AB/BA comparison alternated this release binary with the original. It verified 658,944 reads. At 800 offered reads/s the observations below give server CPU, client CPU and median/p99 latency in microseconds, first for the original and then for that build. Achieved rates were approximately 800/s throughout.

- **64 B unprotected.** Server CPU 48.8 to 36.6, client CPU 59.8 to 41.5, and latency 48.6/81.8 to 45.6/79.2.
- **4 KiB unprotected.** Server CPU 131.8 to 128.2, client CPU 156.2 to 153.8, and latency 135.9/307.9 to 128.1/300.8.
- **64 B established OSCORE.** Server CPU 51.3 to 31.7, client CPU 61.0 to 51.3, and latency 54.8/87.8 to 52.0/84.8.
- **4 KiB established OSCORE.** Server CPU 175.8 to 148.9, client CPU 207.5 to 194.1, and latency 191.6/279.1 to 180.8/270.5.

Removing the rate cap produced the following medians. CPU is server/client µs per verified read and latency is p50/p99 in µs.

- **64 B unprotected.** Throughput 56,214 to 61,063/s, CPU 17.2/17.2 to 15.7/16.2, latency 16.8/26.0 to 15.4/22.6.
- **4 KiB unprotected.** Throughput 13,460 to 14,712/s, CPU 73.6/74.2 to 67.1/67.7, latency 70.5/101.3 to 64.9/92.5.
- **64 B established OSCORE.** Throughput 48,708 to 50,640/s, CPU 19.6/20.0 to 19.6/20.0, latency 20.9/30.4 to 19.6/32.1.
- **4 KiB established OSCORE.** Throughput 10,226 to 10,866/s, CPU 95.7/97.7 to 87.9/91.8, latency 95.8/127.1 to 89.0/132.3.

At capacity its server working set was 6.0–6.2 MiB, the client 5.0–5.1 MiB. Private memory was about 1.2 MiB for the server and 0.9 MiB for the client. The protected p99 rose by about 6% for 64 bytes and 4% for 4 KiB even though median latency fell. Source-matched controls support a throughput improvement and leave the protected tail and both 50% server CPU targets open. This server comparison uses the frozen baseline client with its original collection policy. The borrowed-client and HKDF figures below measure separate mechanisms.

## Windows service

Each round started a fresh server and verifying client. One confirmable read was outstanding. An ordinary read used one request and response. Each 4 KiB read used four of each. Warmup excluded 32 reads. The original core was `b8f44fa`, the server fixture was `eea0374b`, and the integrated core was `161a225` plus source diff `e412562a`. The original and integrated builds have different dependency locks, recorded in the evidence. For example, the original core pins AES 0.9.3 and the later builds pin 0.9.4. Their comparison measures the project over time. The independent deadline and OSCORE options experiments use the integrated lock and isolate their respective source changes.

The longer capacity rotation put every build in every starting position. Values below are medians of four windows per build. CPU figures are server then client microseconds per verified read. Latency is the median of each window's p99 in microseconds.

- **64 B unprotected.** CPU went from 17.2 and 17.2 to 16.2 and 16.2. Throughput went from 56,542 to 61,208/s and p99 from 24.9 to 22.4 µs.
- **4 KiB unprotected.** CPU went from 72.3 and 72.9 to 67.1 and 68.4. Throughput went from 13,580 to 14,634/s and p99 from 104.4 to 96.8 µs.
- **64 B established OSCORE.** CPU went from 20.0 and 20.0 to 18.6 and 18.6. Throughput went from 51,821 to 52,966/s and p99 from 30.4 to 28.5 µs. Median latency rose from 18.1 to 18.5 µs.
- **4 KiB established OSCORE.** CPU went from 94.4 and 97.7 to 85.3 and 91.8. Throughput went from 10,187 to 10,931/s and p99 from 137.2 to 123.2 µs.

At 800 offered reads/s, all four configurations completed about 800/s. Small unprotected reads used **more** charged server time in all four pairs in the four-build rotation. Small protected reads split two pairs each way. Both large bodies had lower median server time, with two of four pairs lower unprotected and three of four protected. The first shorter campaign produced different charged-time estimates. The process clock advanced in 15.625 ms increments and translated poorly into elapsed service cost at light load. Process cycles, elapsed latency and saturated CPU are separate observations. Neither security mode has established a 50% server CPU-time reduction.

The x64 App including its UDP adapter shrank from 16,048 to 15,928 bytes. The separate server-loop future borrowing that App shrank from 19,184 to 11,824 bytes. The original server working set was about 6.1–6.3 MiB across capacity cells and the integrated server about 6.0–6.2 MiB. The verifying client used about 5.0–5.1 MiB. Each record includes private bytes, working set and peak working set for each process. Component sizes overlap their enclosing owners.

## Executing, waiting and observer overhead

Only the benchmark server thread was sampled. At 800/s, independent thread-state samples of the integrated server found 1,095 waits, 72 running, five standby and two unknown. A completion-port wait means the reactor can sleep until readiness or a deadline. Standby means runnable and awaiting dispatch. A blocked thread consumes no thread CPU while it waits. The Windows client used the fixture's spin policy in every matched window.

The separate stack captures included 13,569 wall samples. Receive dispatch appeared in 8,355 stacks and `send_tx_async` in 5,186. Socket control appeared in 5,286 and the completion-port path in 2,191. Parents and children share samples. On the protected 4 KiB run, AEAD sealing appeared in 252 stacks and opening in 79. Suspending and unwinding the thread moved integrated p99 from 85–278 µs in the four matched controls to 2.96–11.94 ms. Those traces identify paths for investigation. The performance comparisons above ran with the profiler off. Shared host trace settings stayed intact.

The repeat capture on the exact retained release recorded 13,764 wall stacks. Receive dispatch appeared in 8,172 and the TX send path in 5,157. Independent thread-state sampling found 1,091 waits, 78 running and 12 standby observations. On the protected 4 KiB cell, sealing appeared in 253 stacks and opening in 77. The instrumented p99 across cells ranged from 4.64 to 11.30 ms. The corresponding profiler-off p99 medians ranged from 77.6 to 309.1 microseconds. Counts include nested frames and the two sampling methods have different observers. The waits measure elapsed thread state. Only scheduled execution can contribute to thread CPU time.

## The same ESP32-S3 against libcoap

Three rounds rotated integrated Coaptic, the deadline-cache experiment and Espressif's libcoap 4.3.5~8 through the same board. Each boot ran eight randomized cycles sweeping the body and requested block size. The client checked the token, message ID, ACK, response code, block offsets, completion and every payload byte. There were 576 timed reads plus 18 identity checks. Automatic 2,000-byte reads used exactly two exchanges, 26 request bytes and 2,020 reply bytes with either implementation. The C advantage survives matching the wire traffic.

- **One seven-byte value.** The median was 5.85 ms for Coaptic and 4.05 ms for libcoap. Across the observed active read time they served 147 and 212 verified reads/s respectively.
- **One 2,000-byte value.** With automatic Block2, Coaptic took 16.91 ms and libcoap 13.29 ms, or 55 and 75 reads/s. Requesting 1,024-byte blocks gave 17.00 and 14.38 ms. Requesting 256-byte blocks gave 63.04 and 41.95 ms.
- **The same bytes in 32 exchanges.** With 64-byte blocks, the medians reached 242.48 ms and 176.10 ms. Moving from two to 32 exchanges added about 7.5 ms per extra exchange to Coaptic and 5.4 ms to libcoap. This is end-to-end time including Wi-Fi, scheduling and socket work. It makes the per-packet service path worth more attention than a single final body copy.
- **Memory and capability.** These images reserve 196,608 and 16,384 bytes of network worker stack. Coaptic had roughly 68.6 kB of running free heap. The C fixture had about 233–235 kB free immediately before cleanup. Coaptic reserves bounded duplicate history, eight route slots, two slots per packet direction, one 4 KiB body per direction, two Observe rows, four Calls, four security request bindings and one association. The client checked every returned byte for both images. Only Coaptic supplies the full bounded service and durable protected endpoint in this fixture. The C measurements cover unprotected reads.

The independent Windows wire client used median 244 µs of charged process CPU per mixed read against integrated Coaptic and 488 µs against C. Each boot contains only 64 timed reads, so one Windows accounting tick is 244 µs per read. Working sets were about 26.7 MiB and private memory 18.4–18.5 MiB. The ordinary firmware exposes heap and stack observations. The separate diagnostic images below add a per-operation worker CPU clock. There was no power meter.

The retained Rust firmware and C source both borrow the static application representation. On Coaptic, secure state, duplicate handling and bounded allocation are live service constraints. The compiled async runtime also self-wakes on each cooperative progress iteration and calls the platform yield after 32 polls. On this port that yield is a one-tick FreeRTOS delay. Its mere presence proves neither how often it runs per read nor how much of the measured gap it explains. [Installed service source audit](ESP32-GET-SOURCE-AUDIT-20261124.md)

## Scheduled CPU on the same board

Eight boots used a native verifying Windows client against Coaptic and libcoap. Each image ran twice with the worker clock enabled and twice with it off. The second round reversed the order. Across the 192 full reads, the two instrumented builds supplied 128 request/reply intervals matched to the independent client's message IDs and datagram lengths. Each interval starts after a successful UDP receive and ends after the corresponding send. The 2,000-byte totals add both intervals. This boundary excludes waiting for the next datagram, Wi-Fi airtime between blocks and other tasks. Interrupt time charged to the worker is included.

- **Seven bytes.** Coaptic used 2.94 ms of worker CPU and libcoap used 1.62 ms. Median elapsed time inside the measured interval was 3.28 and 1.93 ms. The workers spent 0.35 and 0.33 ms ready to run. Median blocked time was zero in both.
- **2,000 bytes.** Coaptic used 6.72 ms of worker CPU and libcoap used 3.74 ms. Median elapsed time inside the two intervals was 7.33 and 4.37 ms. Ready time was 0.67 and 0.63 ms. Median blocked time was zero in both.
- **Ordinary firmware.** The client saw 7.15 and 6.44 ms for seven bytes, and 18.42 and 16.50 ms for 2,000 bytes, for Coaptic and libcoap respectively. Across the mixed identity, seven-byte and large reads, the median per-boot rates were 88.4 and 94.7 verified reads/s. Host CPU was 651 and 326 microseconds per mixed read. Maximum sampled host working sets were 5.51 and 6.04 MiB. One Windows clock tick across 24 reads is 651 microseconds/read.
- **Observer cost.** Each meter reserved 2,560 bytes. With it enabled, the client saw 7.04 and 6.78 ms for seven bytes, and 16.89 and 17.40 ms for 2,000 bytes. The latency change varied in both direction and size across boots. This small cohort cannot separate a sub-millisecond observer cost from Wi-Fi and scheduling differences. The worker CPU ordering repeated in both pairs at both sizes. The first capture used a host filter that dropped the timing rows. It contributed no CPU observations. The corrected collector captured every row from both implementations. The board's normal firmware identity was verified after both attempts.

Coaptic used about 1.8 times the worker CPU for the same bytes in both cases. Scheduler-ready time accounts for only a small part of the difference. The measured interval includes protocol dispatch, response construction and the UDP send call. It does not distinguish instruction-cache effects, a particular Rust function, or work performed in the network stack on behalf of the sending task. [All individual observations and image identities](CHAINED-READS-20261125.json)

### How much of the unprotected gap is in the UDP send?

A second trace split the same interval immediately before the socket send. Four boots in Coaptic, libcoap, libcoap, Coaptic order matched all 384 timestamps to 128 exchanges and 96 verified reads. The ledger used 3,840 bytes and emitted its records after the measured cohort. Each firmware used the same native client and the same application data as the preceding control. The normal image identity was verified on restoration.

- **Seven bytes.** From UDP receive return to send entry, Coaptic used **2.079 ms of worker CPU** and libcoap used **0.834 ms**. Inside the send, the figures were **0.826 and 0.782 ms**. Median scheduler-ready time before the send was zero in both. Inside the send it was 0.345 and 0.298 ms.
- **2,000 bytes.** Across both datagrams the workers used **4.916 and 2.095 ms before send**, then **1.715 and 1.595 ms inside send**, respectively. Median ready time inside the sends was 0.571 and 0.596 ms. Median blocked time was zero in both stages for both implementations.

The difference in median pre-send CPU was 1.25 ms for the small read and 2.82 ms for the blockwise read. Differences inside send were 0.045 and 0.120 ms. The extra work is concentrated after receive and before the socket call. On the Rust path that interval covers ingress handoff, protocol dispatch, routing, encoding and duplicate bookkeeping. The experiment identifies their combined cost. Ranking the individual functions requires subdivision within that interval. [Per-stage data and source fingerprints](CHAINED-READS-20261125.json)

## What changed in the library

**Completed response ownership.** `App::take_response_with` lets a caller verify, decode or act on the bytes in the slot that already owns them. The callback can return an owned result. On return the slot is reusable. The public snapshot and copy-into-caller-storage APIs still work. Tests check pointer identity in both a 200-byte datagram and a 6,000-byte assembled body. Other tests cover local cancellation, a remote error, Observe followed by another notification, an unavailable full representation followed by snapshot collection, short-buffer refusal and slot reuse. Both the ordinary client example and the fixed and allocated protected example now consume their payload this way. [Client collection](../../src/app/client.rs) and [regressions](../../src/app/tests.rs)

A single client executable alternated copying and borrowing against the same integrated server in four pairs per workload. Both modes compared every returned byte. For a 4 KiB read the verifier reserved 8,192 bytes when copying and 4,096 when borrowing. The remaining 4,096 bytes hold the expected test data. At capacity, unprotected client CPU went from 67.71 to 67.06 µs/read and throughput from 14,923 to 14,946/s. Protected client CPU was 87.89 µs/read in both arms and throughput was 11,356 versus 11,353/s. Median client working set was effectively unchanged. The reliable gain is eliminating the destination and its body copy. Ingress still moves the admitted packet into dispatch scratch, the client retains bounded inline metadata and payload for snapshots, and each accepted block is copied once into the assembly slot. Neither a 4 KiB working-set saving nor a general CPU improvement follows from that result.

**OSCORE context creation.** Association setup extracts the shared secret and salt once. Sender key, recipient key and common IV still use distinct RFC 8613 inputs. The first version reduced Windows setup time from 1,031 to 581 ns, but also increased the deepest identified sequence of Xtensa release frames from 1,360 to 1,776 bytes. Two distinct function boundaries now let the extraction temporaries unwind before building the outputs. The checked-in version uses 32 bytes of encoding scratch for the longest supported derivation info. Its corresponding Xtensa frame sum is 1,360 bytes, including SHA-256 compression. The context object occupies 368 bytes in all three builds. These are compiler layout observations. Interrupt stack, system runtime and whole-task reservation are separate. A fresh set of ten alternating Windows pairs measured 1,028 to 585 ns per completely verified setup, a 43.1% reduction. Setup occurs before established reads. [Context derivation](../../src/oscore/context.rs) and [vector tests](../../src/oscore/tests.rs)

### What association setup costs on the ESP32-S3

Both versions ran in the same physical image using the same RustCrypto dependencies and compiler. Each sample started a fresh, core-pinned 16 KiB worker. The worker created 128 complete contexts and verified the sender key, recipient key and common IV on every iteration. Six alternating pairs per input on each of two boots gave 6,144 verified contexts. Logging and stack-watermark collection happened outside the timed loop. The board was restored to its normal image and identified afterward.

- With the published RFC 8613 C.1 inputs, median worker CPU per setup fell from **7.161 to 3.389 ms**. Elapsed time fell from 7.298 to 3.451 ms.
- With a 64-byte secret and salt, two seven-byte IDs and a 16-byte ID context, CPU fell from **7.368 to 3.460 ms**. Elapsed time fell from 7.489 to 3.522 ms. Independent host HMAC-SHA-256 calculations supplied all three expected outputs for these inputs.
- Every matched worker used 96 fewer stack bytes at its measured high-water mark. The original used 2,756 or 2,764 bytes depending on boot. The revised setup used 2,660 or 2,668 bytes. These whole-worker observations include the verifier and task entry. Both arms had the same 16 KiB reservation and 368-byte context.

The CPU reductions are 52.7% and 53.0% for **association creation**. The established-read targets use a separate workload. [Individual worker observations and image fingerprints](CHAINED-READS-20261125.json)

## Rejected server experiments

**Duplicate-history deadline cache.** Maintaining the earliest expiry during admission made repeated timer queries cheaper on allocated tables. At 64 entries, the median query fell from 69.0 to 15.9 ns, replacement rose from 120.2 to 161.2 ns, and a replacement plus two queries fell from 273.7 to 185.9 ns. At the device's four fixed entries, the combined case changed only from 49.9 to 46.6 ns. The Windows and device binaries have distinct linked implementations. The device's automatic 2,000-byte reads took longer in all three boot pairs. The pooled median increased from 16.91 to 19.14 ms. The cache was removed.

The first local expiry comparison accidentally linked identical executable code into two files with different whole-file hashes. Those timings were excluded. The replacement used independent Cargo path dependencies, checked both `.text` sections, and asserted that the deadline-specific method returned the stored deadline before timing its use. The physical comparison used separately compiled Rust archives. The linked `next_timeout_after` implementation in the two ESP images changed from 4,552 to 4,707 bytes. Source, test size and binary identity were each checked separately.

**Reusing the protected option.** One build carried the validated outer OSCORE option range forward on authenticated receive and reused the successful response binding. Window order reversed in the second Waveshare pair. The first pair favored reuse on both seven-byte and 2,000-byte reads. The second favored the integrated server on both. The 12 fresh client processes verified 288 operations. Both arms served a median 30.9 mixed reads/s. The integrated server's pooled small and large medians were 17.97 and 46.37 ms. With the option change they were 18.24 and 45.63 ms. At Windows capacity, the change lowered protected throughput at both sizes. The option change was removed.

**Borrowing block-transfer metadata during role lookup.** The private candidate forwarded role lookup to the resident fixed and allocated pools. This avoided taking an owned transfer snapshot from every occupied slot just to compare its key and role. Eight alternating local pairs reduced a last-slot incoming lookup from 11.4 to 8.2 ns at one fixed slot and from 65.0 to 56.3 ns at 64 allocated slots. The normal small GET does not use this client block lookup. Even the complete four-block client read showed no repeatable gain against the same server. Across four profiler-off pairs, unprotected 4 KiB client CPU went from 65.10 to 66.41 microseconds/read and throughput from 15,376 to 15,087/s. Protected 4 KiB client CPU was 87.24 microseconds/read in both arms, with throughput 11,498 to 11,443/s. The full four-cell comparison verified 454,144 reads. The change stayed out of the checkout. [Isolated probes and every client/server process window](CHAINED-READS-20261125.json)

## Durable protected execution on the Waveshare

A separate diagnostic image linked the identical integrated Rust archive and a private instrumented kernel. A bounded trace matched all 32 exchanges to 24 verified reads. It captured running-task time, time blocked and time ready to run while waiting for a core. The task clock includes interrupt time charged to the worker. A private copy of the kernel compiled the instrumentation. Normal firmware was restored and its image identity verified.

From the worker observing readiness to its final send return, the seven-byte read used a median **14.60 ms of task CPU**, with 0.74 ms blocked and 0.37 ms ready but unscheduled. The 2,000-byte read used **38.40 ms of task CPU**, with 3.25 ms blocked and 1.52 ms ready but unscheduled. Those component medians need not add to the median elapsed time. Across the whole cohort, the 64 storage reads and 32 commits occupied 289.7 of 602.3 ms of measured task CPU. Every storage span lay within its request's observation interval. The device checks the old record and verifies the committed replacement before exposing the operation. This accounts for a substantial portion of protected execution. Any change that removes a read needs an equally strong way to verify the precondition or the committed replacement.

The ledger reserved 28,672 bytes. Against the same native client on the ordinary integrated firmware, median client-observed latency changed from 17.46 to 18.21 ms for the small value and from 45.67 to 48.76 ms for the large one. These are two uninstrumented control boots and one instrumented boot. The difference combines observer overhead and between-boot variation. The ordinary integrated and option runs had 68.5–68.6 kB of running free heap. Native client CPU per mixed read was 326 and 651 µs with maximum sampled working sets of 6.83 and 5.73 MiB respectively. Client CPU includes process startup and durable host state and remains subject to Windows tick quantization. [Per-boot and per-operation evidence](CHAINED-READS-20261125.json)
