# Throughput and security-context lookup

The Windows capacity test times verified full reads with one Call outstanding. Both peers sustained 800 offered reads/s. The ESP32-S3 has a separate comparison because it used a different board runtime, radio and response exchange.

Capacity means one verified read finishes and the next starts immediately. At an offered 800/s, compare CPU, cycles and latency because the scheduler fixes the requested rate. Higher throughput and lower CPU per read are both desirable. Server and client times belong to separate processes and may overlap. Each table reports medians across runs for each peer. Paired changes and win counts compare peers in the same round, so the percentage from two table medians can differ from the median of the paired percentages.

## Bounded table search

The final candidate keeps the original request admission order. When managed policy has never been selected (generation zero), a lookup sees at most the primary slot. After managed policy has been enabled it sees the full array, including occupied slots after a hole. The install and free-slot search still uses every configured slot. The new UDP regression removes the first of two managed peers, collects a cancelled one-slot client Call and verifies a new two-block protected read against the remaining association.

| Association slots | Body B | Control ns/read | Bounded ns/read | Paired change | Faster pairs | Bounded slot visits |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 7 | 352 | 349 | -0.5% | 6/10 | 7 |
| 1 | 2000 | 1041 | 1039 | -0.1% | 5/10 | 16 |
| 32 | 7 | 397 | 358 | -9.8% | 10/10 | 7 |
| 32 | 2000 | 1167 | 1042 | -11.3% | 10/10 | 16 |

The independent Windows comparison rotated the source control, bounded candidate and libcoap through six orders at each size, load and security mode. Times are microseconds per verified read. The receipt contains all 144 windows and paired differences.

| Load | Security | Body B | Control rate/s | Bounded rate/s | C rate/s |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 40,179 | 39,995 | 33,771 |
| Capacity | unprotected | 4096 | 9,543 | 9,918 | 8,457 |
| Capacity | oscore | 64 | 34,830 | 35,940 | 16,541 |
| Capacity | oscore | 4096 | 8,089 | 8,097 | 3,839 |
| 800/s | unprotected | 64 | 800 | 800 | 800 |
| 800/s | unprotected | 4096 | 800 | 800 | 800 |
| 800/s | oscore | 64 | 800 | 800 | 800 |
| 800/s | oscore | 4096 | 800 | 800 | 800 |

| Load | Security | Body B | Control server / client CPU | Bounded server / client CPU | C server / client CPU |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 25.39 / 25.39 | 24.41 / 24.41 | 22.95 / 29.30 |
| Capacity | unprotected | 4096 | 103.20 / 105.42 | 100.99 / 100.99 | 95.44 / 117.63 |
| Capacity | oscore | 64 | 28.32 / 28.81 | 27.34 / 27.83 | 53.22 / 60.06 |
| Capacity | oscore | 4096 | 123.18 / 124.29 | 122.07 / 124.29 | 226.38 / 257.46 |
| 800/s | unprotected | 64 | 43.95 / 65.92 | 68.36 / 80.57 | 43.95 / 73.24 |
| 800/s | unprotected | 4096 | 161.13 / 180.66 | 170.90 / 187.99 | 146.48 / 192.87 |
| 800/s | oscore | 64 | 65.92 / 53.71 | 75.68 / 83.01 | 92.77 / 119.63 |
| 800/s | oscore | 4096 | 185.55 / 187.99 | 217.29 / 236.82 | 273.44 / 327.15 |

| Load | Security | Body B | Control p50 / p99 | Bounded p50 / p99 | C p50 / p99 |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 22.7 / 52.7 | 22.6 / 54.0 | 27.3 / 58.6 |
| Capacity | unprotected | 4096 | 96.2 / 207.7 | 95.2 / 198.3 | 110.0 / 230.6 |
| Capacity | oscore | 64 | 27.2 / 60.3 | 27.0 / 52.5 | 57.2 / 111.4 |
| Capacity | oscore | 4096 | 116.5 / 238.2 | 116.2 / 237.4 | 244.9 / 467.0 |
| 800/s | unprotected | 64 | 63.6 / 139.9 | 64.2 / 130.9 | 64.5 / 126.7 |
| 800/s | unprotected | 4096 | 146.2 / 369.6 | 145.4 / 377.2 | 162.6 / 370.4 |
| 800/s | oscore | 64 | 67.2 / 149.8 | 69.0 / 147.1 | 99.5 / 193.8 |
| 800/s | oscore | 4096 | 167.4 / 405.2 | 167.4 / 398.1 | 319.1 / 605.3 |

| Load | Security | Body B | Control server private / working MiB | Bounded server private / working MiB | C server private / working MiB |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 1.17 / 6.15 | 1.17 / 6.15 | 0.94 / 5.40 |
| Capacity | unprotected | 4096 | 1.19 / 6.21 | 1.19 / 6.21 | 0.96 / 5.43 |
| Capacity | oscore | 64 | 1.17 / 6.25 | 1.16 / 6.24 | 0.96 / 5.56 |
| Capacity | oscore | 4096 | 1.19 / 6.32 | 1.19 / 6.32 | 0.98 / 5.58 |
| 800/s | unprotected | 64 | 1.17 / 6.15 | 1.17 / 6.15 | 0.94 / 5.40 |
| 800/s | unprotected | 4096 | 1.18 / 6.21 | 1.18 / 6.21 | 0.96 / 5.43 |
| 800/s | oscore | 64 | 1.17 / 6.26 | 1.17 / 6.25 | 0.96 / 5.56 |
| 800/s | oscore | 4096 | 1.19 / 6.32 | 1.18 / 6.32 | 0.98 / 5.58 |

| Load | Security | Body B | Control client private / working MiB | Bounded client private / working MiB | C client private / working MiB |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 0.93 / 5.10 | 0.93 / 5.09 | 0.92 / 5.09 |
| Capacity | unprotected | 4096 | 0.86 / 5.05 | 0.87 / 5.05 | 0.87 / 5.05 |
| Capacity | oscore | 64 | 0.92 / 5.18 | 0.93 / 5.18 | 0.93 / 5.18 |
| Capacity | oscore | 4096 | 0.88 / 5.14 | 0.87 / 5.14 | 0.88 / 5.15 |
| 800/s | unprotected | 64 | 0.83 / 5.00 | 0.83 / 5.00 | 0.83 / 5.00 |
| 800/s | unprotected | 4096 | 0.85 / 5.03 | 0.85 / 5.03 | 0.85 / 5.04 |
| 800/s | oscore | 64 | 0.84 / 5.09 | 0.83 / 5.09 | 0.82 / 5.09 |
| 800/s | oscore | 4096 | 0.85 / 5.12 | 0.85 / 5.12 | 0.85 / 5.12 |

## Interpreting the Windows and device results

The stock C server was cheaper to run for unprotected reads in several cells. In the bounded build's 64-byte capacity comparison, Coaptic used 24.41 microseconds of server CPU and 24.41 of client CPU per read. Libcoap used 22.95 and 29.30. The complete exchange therefore finished sooner on Coaptic even though libcoap charged less time to its server. At 800/s, a 64-byte unprotected read took 33% more Coaptic server cycles in the median pair, with more cycles in all six. A worker waiting between packets and a worker consuming CPU while sending measure different things. Socket and completion-port frames in the process-local traces do not assign the excess to an individual protocol function.

The stock protected libcoap control requests 8,388,880 bytes of temporary ciphertext capacity per response. That is a virtual allocation request. In a separate matched comparison, right-sizing the C request reduced its server CPU from 55.18 to 27.83 microseconds for 64 bytes and from 239.70 to 128.73 for four blocks. Neither server had a consistent CPU advantage against that C build. The roughly twofold capacity throughput advantage in the current stock comparison is specific to the C build tested. [Right-sized C control](MATCHED-ACK-20261125.md).

The final narrow build and its control contained the same 7,851,745 bytes of .text by length. They differed at 161 byte positions. All 161 belonged to one Windows unwind-table function. The matching release PDB identified it as the four-request, one-association specialization of inbound_route. Both had the same inline App and loop future sizes. The client executable was identical for every arm. The fixed-pool diagnostic counted 7 visits per 7-byte read and 16 visits per 2,000-byte read at both association capacities. The original 32-slot control counted 224 and 512. These are visits to the association store. The socket and protocol still do the rest of the work on every read.

At one association the final candidate's unprotected 4 KiB capacity rate rose 5.3% in the median pair and was higher in five of six. Protected 4 KiB throughput changed by +0.5%, with three of six positive, and server CPU had a median paired change of zero. The broad trial's protected blockwise regression did not repeat on the narrow candidate. One-slot fixed-pool timings were within 1%.

The two 50% server CPU-time targets use the historical original-project baseline at matching capacity, security and offered load. The previous comparable capacity changes were −9.4% and −8.2% for small and large unprotected reads and −1.7% and −6.1% with established OSCORE. Both targets remain open. The new local improvement applies to applications configured for more association slots while the App is in primary or unprotected mode with generation zero. The 32-slot case and the stock C comparison use different baselines. [Original-baseline control](EMPTY-ACK-CONTROLS-20261126.md).

## Longer 800/s CPU windows

The first narrow cohort raised a concern on 64-byte unprotected reads. Its server CPU rose 27.9% in the median pair, with more charged CPU in all six pairs, while cycles rose 1.5%. Longer two-arm windows alternated the same one-slot binaries for eight more pairs per affected case. Small rows verified 8,000 reads per window, and four-block rows verified 7,040 to stay within the original 60,000 outbound-datagram budget including warmup and setup. Across 368,640 verified reads, the small unprotected server CPU difference fell to +1.3%, split four increases and four decreases. Cycles changed +0.1%. Protected small-read server CPU changed −6.8%, and protected four-block CPU changed +2.8% (six of eight higher), while four-block server cycles changed −0.6%. The later run did not reproduce the large paced penalty. It does not establish an improvement in paced CPU.

On 4 KiB unprotected reads, the unchanged Coaptic binary completed 7,471 reads/s in the first six-round cohort and 9,543 in the second. The matching C medians were 7,518 and 8,457. Comparing the first Rust result to the second C result would manufacture a lead. The server/client/harness and packet checks match within each cohort. The measurements do not identify the cause of the difference between cohorts.

For the same unchanged binaries in the second unprotected 4 KiB capacity cohort, the Coaptic server used 23.30 microseconds of user and 81.01 of kernel time in the median window, versus 31.07 and 67.69 for C. The operating system charged more time in the kernel for Coaptic even though its user-time median was lower. At an offered 800/s on that body the split was 75.68/92.77 versus 46.39/97.66. The relative cost shifts with load. The reported totals and components are independent medians.

Two original correctness logs from the reused Cargo output lane included a replay-builder test present only in a rejected private fixture. Those logs were excluded. The replacement checks require Cargo to recompile the Coaptic crate from the active checkout, execute the two-association UDP regression, and exclude the private replay-builder test. The old log hashes are retained for audit. Each timed release build has its own executable hash.

On the final narrow build, counting row visits changed measured fixed-pool time by +3.2% small and +1.1% large at 32 slots in the median of six alternating pairs. The corresponding one-slot differences were −1.2% and −0.5%. The uninstrumented binary supplies every quoted timing gain.

## Physical attribution fixture

A private, source-matched overlay places stamps at actual Rust executor poll entry, protocol and dispatch entry, handler entry and return, completed encoding, and transport and native socket entry and return. The C fixture marks corresponding boundaries. The fixed ledger takes at most 16 KiB and correlates both sides with the 24 message IDs in sixteen verified reads. The grader accepted both detailed and endpoint-only traces for both implementations and rejected twenty damaged traces. All 16 modified source files reproduced against the archived firmware input. Device measurements require an ordinary boot and both instrumented boot modes so that the cost of collecting those stamps is measured.

## Equal 32-slot over-the-network qualification

Both release builds used the shipping Tokio adapter, the same request handler, one route and four Calls, four RX and four TX slots, eight duplicate rows, four Observe slots, four security bindings per context and 32 configured associations. Both asserted the configured packet and body capacities at startup. An unprotected server had an empty association table. An established OSCORE server used a primary context. Their inline Apps both used 29,816 bytes and loop futures 11,848 bytes. Memory for the additional provisioned capacity was identical across the two arms. Eight alternating pairs verified 993,280 reads after an independent wire check of both security modes and sizes.

For unprotected 4 KiB reads the rate went from 9,481 to 10,077/s and was higher in seven of eight pairs. Median paired server and client CPU both fell 4.3%, server cycles fell 5.5% and p99 fell 8.4%. The independent server CPU medians were 105.42 and 99.88 microseconds. The small unprotected row had a longer tail. Its p99 rose 7.8% in the median pair, rising in six of eight, while charged CPU held flat. Protected 4 KiB capacity had a +0.6% median paired rate change and a zero median paired server-CPU change. The local 9.8–11.3% fixed-pool timings do not represent the change in complete UDP read throughput.

## Repeating the profile on the retained lookup

At 800 small unprotected reads/s the narrow build had 1,174 completion-port wait stacks, 1,037 including a socket send, and 6 naming an association path in 3,181 wall stacks. Separate scheduler observations found 168 waits, 12 running states and one standby. At 4 KiB under established OSCORE the narrow profile had 259 security stacks and 1,059 socket sends in 3,234 observations. Its separate state sample recorded 142 waits, 39 running and two standby. The sampler raised small-read p99 from 130.4 to 4,695.4 microseconds and protected blockwise p99 from 404.2 to 10,904.0. The sparse association samples cannot support an exclusive microsecond estimate. The fixed-store counters and profiler-off peer comparisons supply that change's work and end-to-end measurements.

## Retained change

A second equal-capacity 32-slot UDP cohort verified 496,640 more unprotected reads across eight independently alternating pairs per size. For 4 KiB, median paired throughput improved 4.8%, server CPU fell 6.3%, client CPU fell 4.3% and p99 fell 7.5%. Six of eight pairs completed more 4 KiB reads per second. For the small read, throughput rose 2.2% and p99 fell 4.3% in the median pair, reversing the first cohort’s tail movement. We retain the bounded lookup for the repeatable unprotected blockwise improvement at 32 configured slots. The small-read tail, the protected server and the physical ESP32-S3 earn no speedup claim from this change.

## What the device results say

On the November 25 Waveshare firmware, mixed whole-window rates were 91.7 verified reads/s on Coaptic and 91.1 on libcoap. Summing active read intervals within each route gives 164.7 versus 134.4 for 7 bytes, 53.9 versus 57.0 for 2,000 bytes, and 133.5 versus 136.5 for the 32-byte identity route. Each route had 16 reads per peer across two boots. The 2,000-byte winner changed between boot pairs. An earlier sweep on different images favored C at every tested block size. Rates computed with different denominators or images cannot be combined. [Device reads and boots](PACKET-WORK-20261125.md) and [earlier sweep](CHAINED-READS-20261125.md).

For the same landed device image, the 7-byte exchange used 2.857 versus 1.587 ms of worker CPU from receive return to send return. The first and second large replies cost Coaptic 1.278 and 1.378 ms more than C. Median blocked time in those intervals was zero. The worker clock includes interrupt time charged to the task. No fresh device image or protected libcoap run was available in this study. [Matched device intervals](LANDED-LIBCOAP-20261125.md).

## First Windows comparison (broad trial)

The source control is 9c632ba. The first three-way comparison includes a broad lookup trial that was subsequently rejected. Both Coaptic binaries used the same source directory, frozen driver, lockfile, release settings and stock Mio. The three executables rotated through every starting position. Counts exclude 32 warmups per window. The frozen client required one request, two empty ACKs and one separate response per block, verified each payload and allowed one Call outstanding. The C implementation used its stock OSCORE encryption allocation. [Prior control of the allocation size](MATCHED-ACK-20261125.md).

Each rate is the median number of verified reads/s over six process windows. Higher is better.

| Load | Security | Body B | Control | Lookup trial | libcoap |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 30,694 | 31,642 | 25,045 |
| Capacity | unprotected | 4096 | 7,471 | 8,148 | 7,518 |
| Capacity | oscore | 64 | 28,526 | 32,437 | 14,020 |
| Capacity | oscore | 4096 | 7,070 | 6,440 | 3,441 |
| 800/s | unprotected | 64 | 800 | 800 | 800 |
| 800/s | unprotected | 4096 | 800 | 800 | 800 |
| 800/s | oscore | 64 | 800 | 800 | 800 |
| 800/s | oscore | 4096 | 800 | 800 | 800 |

Each CPU cell shows server / client microseconds charged per verified read. Lower is better.

| Load | Security | Body B | Control | Lookup trial | libcoap |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 31.25 / 31.74 | 31.74 / 31.74 | 31.25 / 39.06 |
| Capacity | unprotected | 4096 | 133.17 / 130.95 | 118.74 / 122.07 | 105.42 / 132.06 |
| Capacity | oscore | 64 | 34.18 / 34.67 | 31.25 / 31.25 | 62.50 / 69.34 |
| Capacity | oscore | 4096 | 140.94 / 140.94 | 150.92 / 154.25 | 255.24 / 291.86 |
| 800/s | unprotected | 64 | 73.24 / 95.21 | 73.24 / 73.24 | 68.36 / 95.21 |
| 800/s | unprotected | 4096 | 175.78 / 168.46 | 197.75 / 207.52 | 148.93 / 197.75 |
| 800/s | oscore | 64 | 70.80 / 70.80 | 92.77 / 112.30 | 95.21 / 126.95 |
| 800/s | oscore | 4096 | 219.73 / 244.14 | 190.43 / 207.52 | 317.38 / 390.62 |

Each latency cell gives complete-read p50 / p99 in microseconds. Lower is better.

| Load | Security | Body B | Control | Lookup trial | libcoap |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 25.0 / 95.5 | 24.8 / 88.0 | 30.8 / 106.0 |
| Capacity | unprotected | 4096 | 101.3 / 293.4 | 99.8 / 325.2 | 114.4 / 320.4 |
| Capacity | oscore | 64 | 28.8 / 86.3 | 27.7 / 73.5 | 62.9 / 165.6 |
| Capacity | oscore | 4096 | 119.0 / 324.7 | 123.4 / 355.2 | 257.6 / 577.4 |
| 800/s | unprotected | 64 | 70.3 / 182.2 | 66.4 / 150.4 | 68.6 / 172.1 |
| 800/s | unprotected | 4096 | 170.0 / 523.0 | 160.2 / 459.4 | 199.0 / 502.5 |
| 800/s | oscore | 64 | 80.1 / 207.8 | 73.9 / 169.8 | 107.8 / 234.4 |
| 800/s | oscore | 4096 | 206.5 / 570.3 | 180.8 / 461.5 | 345.5 / 755.2 |

Server memory at the end of the measured window, private MiB / resident working-set MiB.

| Load | Security | Body B | Control | Lookup trial | libcoap |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 1.17 / 6.15 | 1.16 / 6.11 | 0.94 / 5.40 |
| Capacity | unprotected | 4096 | 1.19 / 6.21 | 1.18 / 6.18 | 0.96 / 5.43 |
| Capacity | oscore | 64 | 1.17 / 6.26 | 1.16 / 6.21 | 0.97 / 5.56 |
| Capacity | oscore | 4096 | 1.18 / 6.32 | 1.19 / 6.29 | 0.98 / 5.58 |
| 800/s | unprotected | 64 | 1.16 / 6.15 | 1.16 / 6.11 | 0.93 / 5.40 |
| 800/s | unprotected | 4096 | 1.19 / 6.21 | 1.18 / 6.18 | 0.96 / 5.43 |
| 800/s | oscore | 64 | 1.17 / 6.26 | 1.16 / 6.22 | 0.96 / 5.56 |
| 800/s | oscore | 4096 | 1.19 / 6.32 | 1.18 / 6.29 | 0.99 / 5.58 |

Client memory at the end of the measured window, private MiB / resident working-set MiB.

| Load | Security | Body B | Control | Lookup trial | libcoap |
| --- | --- | --- | --- | --- | --- |
| Capacity | unprotected | 64 | 0.93 / 5.10 | 0.92 / 5.09 | 0.93 / 5.10 |
| Capacity | unprotected | 4096 | 0.88 / 5.06 | 0.87 / 5.06 | 0.87 / 5.06 |
| Capacity | oscore | 64 | 0.92 / 5.18 | 0.93 / 5.18 | 0.94 / 5.19 |
| Capacity | oscore | 4096 | 0.87 / 5.14 | 0.88 / 5.14 | 0.87 / 5.14 |
| 800/s | unprotected | 64 | 0.84 / 5.00 | 0.83 / 5.01 | 0.83 / 5.00 |
| 800/s | unprotected | 4096 | 0.85 / 5.03 | 0.85 / 5.03 | 0.84 / 5.03 |
| 800/s | oscore | 64 | 0.83 / 5.09 | 0.83 / 5.09 | 0.84 / 5.09 |
| 800/s | oscore | 4096 | 0.84 / 5.12 | 0.85 / 5.12 | 0.85 / 5.12 |

## First lookup trial (rejected)

A zero generation permits only the primary context, which lives in slot zero. Explicitly requiring managed associations or installing one sets a nonzero generation, which remains nonzero after revocation. The trial uses that existing state in ingress, context selection and handler metadata. Managed traffic continues to match context, recipient, endpoint and live bindings. No new cache, allocation or change of capacity is needed.

A separate host test drove the fixed-pool App without sockets, using eight configured routes, four Calls, zero deferred jobs, two 1,152-byte datagrams in each direction, four dedup rows, two Observe slots, one 4 KiB RX and one 4 KiB TX body. Each 7-byte or two-block 2,000-byte read verified MID, token, code and every payload byte. Each arm processed 60,000 reads per window. The A=1 column represents device association capacity, and A=32 measures capacity dependence. These synchronous in-process times include request injection and verification. They cannot be converted into Wi-Fi throughput.

| Association slots | Body B | Control ns/read | Trial ns/read | Paired time change | Faster pairs | Control row visits | Trial row visits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 7 | 365 | 360 | -1.6% | 7/10 | 7 | 2 |
| 1 | 2000 | 1073 | 1085 | +1.5% | 3/10 | 16 | 5 |
| 32 | 7 | 409 | 361 | -12.5% | 10/10 | 224 | 2 |
| 32 | 2000 | 1215 | 1076 | -11.9% | 8/10 | 512 | 5 |

The diagnostic counts every row yielded by association-table iterators and every successful first-slot access. Timing windows use binaries with no counters. Each of six separately alternated diagnostic pairs measures the overhead of the row counter against its corresponding uninstrumented binary. The receipt retains all timing windows, counter costs and individual samples.

## Checking the suspected Windows regression

The broad lookup trial changed the order of context selection and the response guard as well as the association walks. In the first six pairs its median protected 4 KiB capacity rate fell 10.8%, and server CPU rose 7.4%. An independent two-arm, eight-pair run verified 276,480 more reads. The protected blockwise server used 5.0% more CPU and completed 2.7% fewer reads/s in the median pair. Both measures moved against the trial in seven of eight rounds. The unprotected 4 KiB rate gained 2.6% in the median pair. We rejected the combined change and kept the original context-selection and dispatch ordering.

The first three-way rotation had one server exit before readiness. The first two-arm confirmation also had one server exit before readiness. Both occurred before any timed read. Completed windows were preserved and the affected cell was rerun with the same settings. The first failures were reported by the harness without the server exit reason.

## What the profiles found

At 800/s the Windows worker often waited between datagrams. In 3,068 wall-stack samples for a small unprotected read, the lookup trial included 1,529 completion-port waits and 583 socket sends. A separate state sample observed 143 waits, seven running states and one standby. The corresponding C worker included 1,959 socket-select stacks and 1,325 send stacks among 4,215 samples. Its separate state observations found 146 waits, four running and one standby. Application symbols were available for Rust. C stacks resolved operating-system functions only. The wall samples include waiting and cannot be converted to exclusive CPU per function.

For protected 4 KiB reads on the trial, 217 of 2,819 stacks included security work and 876 included a socket send. The independent state sample saw 83 waits, 25 running states, two standby and one unknown. The sampler perturbed p99 on both implementations. On unprotected small reads it increased from 173 to 4,782 µs on Coaptic and from 170 to 4,708 µs on C. The throughput comparisons ran without the sampler. The association counters for the original 32-slot store added 63% and 50% median paired time on small and large reads. Only counter-free binaries supplied the fixed-pool timing rows.

## Duplicate history on a pinned response

The archived ESP32-S3 executable initializes a 256-byte inline replay buffer before testing the response length. Installing its history record also copies 312 bytes. A large response retains its wire bytes in a TX slot. The inline buffer therefore contributes no replay bytes on either large block. On the same image, the compiler reduced the apparent temporary BlockTransfer constructor to scalar size and capacity checks. It emitted no sidecar construction on the successful borrowed path.

A separate trial selected a compact representation for pinned replies and four-byte ACKs, preserving the previous output of replay, equality, hashing and Debug after row reuse. The real fixed-pool App ran uninstrumented in ten alternating pairs for each route and association capacity. At the device's one-slot capacity, two-block 2,000-byte reads took 1.9% longer in the median pair and were slower in eight of ten pairs. At 32 slots the median regressions were 4.9% small and 4.4% blockwise. The trial passed its storage and protocol checks. We rejected that representation.

Two archive-only ESP32-S3 builds used the same recovered firmware companion, OSCORE-enabled feature set, O3/thin-LTO release profile and current library control. In the Constrained specialization the original remember_tx_reply reserved 368 stack bytes. The compact trial reserved 640 and its function text grew from 339 to 406 bytes. Both insertion functions still loaded 0x138 and called memcpy. The trial moved the zeroing call into the inline-payload path. The control library archive was 3,281,964 bytes and the trial 3,286,490 bytes. Archive size includes symbols and other objects. These binaries were not flashed.

[Per-window results, source changes, executable hashes, counter overhead and process profiles](THROUGHPUT-AND-LOOKUP-20261128.json).
