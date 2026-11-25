# Packet work on Windows and the Waveshare

The changed checkout borrows deferred state through completion, scans the Call table once for expiry, reuses packet admission decisions, and shortens option parsing and encoding. During upload it compares only the encoded operation bytes and carries the selected body slot into the write. A continuation whose condition is already known leaves its 512-byte binding in place. The fixed and allocated pools also reuse the transfer they just validated.

The original project control is `b8f44fa`. The preceding checkout, Call/deferred batch, additional codec batch and final combined source on `161a225` are identified by diff hashes `3eb1b847`, `69b8cb93`, `3a63c585` and `510df7f0`. The source identity covers `git -c core.safecrlf=false diff --binary --no-ext-diff HEAD -- src examples`. [Every process window, device observation, executable identity and method](PACKET-WORK-20261125.json)

## What the measurements establish

Verified uploads through the real fixed-pool App took about 11% less time against the preceding checkout. On Windows the cumulative project beats the original baseline in median capacity throughput at all four read sizes/modes. The incremental read gain from this batch is much smaller. At 800/s even the original can meet the offered rate, so throughput there mainly checks delivery. The six capacity rounds and four paced rounds provide the independent comparisons. The main cohorts completed 2,348,800 profiler-off reads. Including the queue candidate, the dependency control and the process-local observers, the complete record contains 4,310,400 verified reads over 456 Windows process windows.

The ESP32-S3 still spends about 1.8 times libcoap’s worker CPU on the seven-byte read and 1.7 times on the 2,000-byte read. Ready time on the small request is nearly identical and median blocked time within the measured exchanges is zero. Earlier phase markers on the preceding firmware placed most of the gap between receive return and send entry. Its Rust and C large reads spent 4.92 and 2.10 ms of scheduled CPU in that interval, then 1.72 and 1.60 ms inside send. The current metered firmware measures total worker time across both stages. [Earlier phase boundaries and measurements](CHAINED-READS-20261125.md)

## What the changed paths cost

Each probe used six order-balanced, fixed-affinity rounds with the first pass discarded. Each PUT contained eight 16-byte fragments, two URI path segments and two query options. The App verified every reply and all 128 assembled bytes. The Engine also tested out-of-order Q-Block1 with NUM0 arriving second. Time includes verification and uses the host monotonic clock.

| Operation | Prior checkout | With option changes | Final | Faster than prior |
|---|---:|---:|---:|---:|
| Validate and walk options (ns/packet) | 95.67 | 45.65 | 43.17 | 6/6 |
| Encode four short options (ns/packet) | 53.93 | 29.97 | 30.81 | 6/6 |
| App PUT, untagged (µs/body) | 6.03 | 5.96 | 5.38 | 6/6 |
| App PUT, tagged (µs/body) | 6.32 | 6.23 | 5.59 | 6/6 |
| Fixed Engine, classic (µs/body) | 3.31 | 2.40 | 2.01 | 6/6 |
| Alloc Engine, classic (µs/body) | 3.71 | 2.50 | 2.12 | 6/6 |
| Fixed Engine, tagged (µs/body) | 3.46 | 2.58 | 2.21 | 6/6 |
| Alloc Engine, tagged (µs/body) | 3.64 | 2.69 | 2.35 | 6/6 |
| Fixed Engine, q (µs/body) | 3.55 | 2.63 | 2.29 | 6/6 |
| Alloc Engine, q (µs/body) | 3.59 | 2.74 | 2.36 | 6/6 |

The registered Constrained App used 36,504 bytes with four Calls and 88,920 with 32 in the local read fixture. Empty polls took 47 to 42 ns and 91 to 53 ns respectively in the Call/deferred batch. The async deferred-completion future fell from 10,496 to 9,032 bytes. The four-Call synchronous GET went from 403 to 417 ns in that same comparison.

On the linked Xtensa ELF, Call expiry shrank from 2,452 to 997 code bytes and its entry frame from 800 to 176 bytes. The upload function shrank from 4,553 to 3,883 code bytes, while its entry frame grew from 1,984 to 2,048 bytes. The service dispatch future poll still reserves 11,088 bytes on entry. These figures describe individual function entry frames. The on-device task low-water readings below cover the active call chain.

The first upload fragment needs an owned canonical operation key. Follow-up packets still encode a candidate for comparison. Matching now compares its occupied prefix and leaves the stored key in place once NUM0’s condition is known. The measured reduction concerns slot scans, comparisons and binding replacement. It does not eliminate construction of every candidate. [Upload matching and dispatch](../../src/storage/engine.rs) and [request identity and fragment validation](../../src/storage/block.rs)

### The preceding and retained Apps under the same local workload

Eight further reversed pairs used the same crate path, compiler, harness and CPU affinity for both sources. Every GET verified the handler input, returned payload and reply count. The deferred cases issued a handle and completed it through the public API.

| Calls | Operation | Prior ns | Retained ns | Faster pairs |
|---:|---|---:|---:|---:|
| 4 | sync-get | 399 | 395 | 7/8 |
| 4 | async-get | 670 | 655 | 6/8 |
| 4 | sync-defer | 643 | 625 | 8/8 |
| 4 | async-defer | 969 | 898 | 8/8 |
| 32 | sync-get | 505 | 462 | 8/8 |
| 32 | async-get | 818 | 744 | 8/8 |
| 32 | sync-defer | 747 | 687 | 8/8 |
| 32 | async-defer | 1136 | 1009 | 8/8 |

In the final matched local comparison, all four operations at 32 Calls improved in all eight pairs. The final four-Call synchronous GET also improved in seven pairs, at 399 to 395 ns. The slower intermediate four-Call result above did not carry into the final comparison.

## Windows UDP reads

The profiler-off windows used the same single-thread Tokio release server and startup-allocated bounded pools, verifying native client, credentials, capacities and one outstanding confirmable request. The 64-byte body used one exchange and the 4 KiB body used four. Every row below compares the original project control (left) to the final checkout (right). The six capacity rounds rotated the original, preceding checkout and final builds through all six orders. Four paced rounds alternated the original and final builds. Capacity here means the fastest serial reads the client and server completed with one request outstanding. A concurrent-client saturation ceiling is a different measurement.

### At capacity

| Workload | Server CPU µs/read | Client CPU µs/read | Median µs | p99 µs | Verified reads/s |
|---|---:|---:|---:|---:|---:|
| 64 B unprotected | 17.6 → 16.6 | 17.6 → 16.6 | 16.9 → 15.2 | 25.9 → 26.1 | 56,431 → 61,939 |
| 4 KiB unprotected | 72.9 → 66.4 | 72.9 → 66.4 | 70.8 → 63.7 | 103.5 → 91.3 | 13,700 → 15,108 |
| 64 B OSCORE | 19.5 → 18.1 | 19.5 → 18.1 | 18.3 → 17.5 | 30.8 → 29.6 | 51,604 → 53,916 |
| 4 KiB OSCORE | 95.1 → 85.9 | 96.4 → 89.8 | 95.9 → 87.2 | 128.2 → 123.7 | 10,299 → 11,139 |

### At 800 offered reads/s

| Workload | Server CPU µs/read | Client CPU µs/read | Median µs | p99 µs | Verified reads/s |
|---|---:|---:|---:|---:|---:|
| 64 B unprotected | 46.4 → 26.9 | 56.2 → 24.4 | 39.2 → 41.4 | 79.8 → 77.0 | 800 → 800 |
| 4 KiB unprotected | 148.9 → 134.3 | 163.6 → 144.0 | 107.1 → 100.0 | 327.4 → 313.0 | 800 → 800 |
| 64 B OSCORE | 36.6 → 34.2 | 56.2 → 46.4 | 47.9 → 42.9 | 87.6 → 84.5 | 800 → 800 |
| 4 KiB OSCORE | 161.1 → 129.4 | 190.4 → 170.9 | 141.5 → 133.8 | 324.8 → 262.0 | 800 → 800 |

At capacity the final build used less charged server CPU than the original in all six pairs for both unprotected sizes and for 4 KiB OSCORE. It did so in four of six pairs for 64-byte OSCORE. At 800/s, one four-second final unprotected window charged zero server CPU ticks even though it served all 3,200 reads and recorded nonzero process cycles. For that workload the median charged CPU reduction was 42%, but median server cycles fell only 5.4% and median latency rose from 39.2 to 41.4 µs. The paced CPU magnitude is sensitive to Windows accounting.

### Memory and comparison with the preceding checkout

- Original at capacity. Server working set 6.14–6.28 MiB and private memory 1.22–1.24 MiB. Client working set 5.02–5.12 MiB and private memory 0.92–0.92 MiB.
- Preceding checkout at capacity. Server working set 6.01–6.14 MiB and private memory 1.17–1.19 MiB. Client working set 5.02–5.12 MiB and private memory 0.92–0.93 MiB.
- Final checkout at capacity. Server working set 6.04–6.17 MiB and private memory 1.18–1.20 MiB. Client working set 5.02–5.12 MiB and private memory 0.92–0.93 MiB.

- 64 B unprotected. Relative to the preceding checkout, server CPU went from 16.6 to 16.6 µs/read and throughput from 61,941 to 61,939/s. The changed build was faster in 4 of six paired windows.
- 4 KiB unprotected. Relative to the preceding checkout, server CPU went from 66.4 to 66.4 µs/read and throughput from 14,985 to 15,108/s. The changed build was faster in 5 of six paired windows.
- 64 B OSCORE. Relative to the preceding checkout, server CPU went from 18.6 to 18.1 µs/read and throughput from 52,944 to 53,916/s. The changed build was faster in 5 of six paired windows.
- 4 KiB OSCORE. Relative to the preceding checkout, server CPU went from 89.8 to 85.9 µs/read and throughput from 11,061 to 11,139/s. The changed build was faster in 3 of six paired windows.

The earlier three-build rotation isolates the first two batches. Capacity medians below use verified reads per second.

- 64 B unprotected. Preceding checkout 61,504, Call/deferred/admission 60,809, with option changes 60,726.
- 4 KiB unprotected. Preceding checkout 14,687, Call/deferred/admission 14,912, with option changes 14,909.
- 64 B OSCORE. Preceding checkout 53,027, Call/deferred/admission 51,266, with option changes 53,289.
- 4 KiB OSCORE. Preceding checkout 11,008, Call/deferred/admission 10,815, with option changes 10,675.

Charged process CPU has a 15.625 ms accounting tick on this host. One tick is 4.88 µs/read over a 3,200-read paced window, 0.98 µs/read over a 16,000-read capacity window and 1.30 µs/read over a 12,000-read capacity window. Both processes share the same host. A client CPU change with identical client code can reflect wakeups or timing changes. Additional per-role cycles, user/kernel time, peak working set and pacing misses are in the JSON.

## Audit of the original project baseline

The historical and current releases use the same native server fixture, client executable, Tokio runtime version, Mio version, security configuration and workload. Their Cargo locks differ. The original project requires AES 0.9.3 and the current one requires 0.9.4. Each project also uses its own I/O procedural macro, with Syn 2 and Syn 3 respectively at build time. All other registry package identities match. The original-versus-retained figures therefore measure a whole-project change. All incremental batches in this checkout used the same lock.

A second six-permutation experiment rebuilt three arms from the same crate and fixture paths. **Historical** used the original project and its committed dependency selection. **AES-only** used that same original source, including its original I/O macro, with the AES pin and corresponding lock entries advanced to 0.9.4. **Retained** used this checkout. Each row shows the medians from six windows per arm. The JSON includes client CPU, latency, memory, executable identity and all 144 individual windows.

| Load | Workload | Historical server µs | AES-only server µs | Retained server µs | Historical reads/s | AES-only reads/s | Retained reads/s |
|---|---|---:|---:|---:|---:|---:|---:|
| Unpaced | 64 B unprotected | 17.09 | 17.58 | 16.11 | 56,920 | 55,525 | 61,591 |
| Unpaced | 4 KiB unprotected | 72.92 | 74.22 | 67.06 | 13,611 | 13,369 | 14,851 |
| Unpaced | 64 B OSCORE | 19.53 | 20.02 | 19.53 | 51,665 | 50,721 | 51,768 |
| Unpaced | 4 KiB OSCORE | 96.35 | 96.35 | 89.19 | 10,321 | 10,286 | 11,019 |
| 800/s | 64 B unprotected | 21.97 | 34.18 | 41.50 | 800 | 800 | 800 |
| 800/s | 4 KiB unprotected | 112.30 | 102.54 | 92.77 | 800 | 800 | 800 |
| 800/s | 64 B OSCORE | 46.39 | 43.95 | 31.74 | 800 | 800 | 800 |
| 800/s | 4 KiB OSCORE | 131.84 | 139.16 | 134.28 | 800 | 800 | 800 |

For a protected 4 KiB read, changing only AES moved median charged CPU from 96.35 to 96.35 µs. Switching from the AES-matched original to the retained project reduced it to 89.19 µs, with lower charged CPU and higher throughput in 6 of six rounds. The library pin alone does not explain the blockwise gain.

For protected 64-byte reads the second rotation measured 19.53, 20.02 and 19.53 µs across the three arms. Relative to the untouched historical project the retained throughput changed by +0.2%. The first rotation’s 7.5% small protected CPU saving did not repeat at that magnitude. Across both rotations the most consistent CPU improvement is on the four-exchange reads.

At 800/s all three servers delivered the full rate. The direction of charged CPU reversed across historical-control rotations for the small unprotected read (46.39 → 26.86 µs first, 21.97 → 41.50 µs second) and the large protected read (161.13 → 129.39 µs first, 131.84 → 134.28 µs second). These workloads show no repeatable paced CPU saving.

## Executing and waiting

At capacity the final server’s median user/kernel CPU was 4.88/11.72 µs for the small unprotected read and 21.48/44.27 µs for 4 KiB. Established OSCORE used 6.84/11.72 and 38.41/45.57 µs respectively. Windows charges kernel time while the process executes operating-system work. Time asleep waiting for a packet is separate. The medians of components need not sum to the median of their totals.

The 4 KiB baseline sets a 50% budget of 36.46 µs/server read unprotected and 47.53 µs protected. At the current cost per read, kernel execution alone uses 44.27 and 45.57 µs respectively. With an unchanged kernel cost, unprotected reads already exceed their budget and protected reads leave about 2 µs for all user-space work. This comparison uses medians from the same capacity cohort. It identifies socket and runtime work per verified exchange as a necessary part of the unprotected target.

The socket event-loop thread was sampled in fresh, process-scoped observer runs. Stack counts are inclusive wall-time observations. A separate thread-state observer sampled running, ready/standby and blocked states. Each observer had a matched profiler-off workload. Shared host trace settings stayed intact.

- Shipping across four workloads. Of 13,768 stack samples, 13.6% included the completion-port wait, 69.7% included request dispatch, 46.3% included TX send, and 2.8% included the selected AES/CCM symbols. Separate state counts were 777 blocked, 63 running and 8 ready or standby.
- Final across four workloads. Of 13,659 stack samples, 15.0% included the completion-port wait, 64.7% included request dispatch, 41.2% included TX send, and 3.0% included the selected AES/CCM symbols. Separate state counts were 767 blocked, 66 running and 9 ready or standby.

In the final quiet-state capture, 767 of 844 points were blocked (90.9%), 66 running, nine standby and two unknown. The Windows API labeled the blocked samples EventPairLow. The independent stacks show the Tokio reactor’s completion-port wait. That state label alone cannot assign a particular wait to a socket. A standby sample is the closest observed indication of a worker ready and waiting for a core.

- Option iteration appeared in 199 preceding and 173 final stacks across the four cells. Inlining and sampling perturbation limit before/after interpretation.
- Option writing appeared in 78 preceding and 36 final stacks across the four cells. Inlining and sampling perturbation limit before/after interpretation.
- Call expiry appeared in 81 preceding and 18 final stacks across the four cells. Inlining and sampling perturbation limit before/after interpretation.
- Duplicate-history storage appeared in 136 preceding and 139 final stacks across the four cells. Inlining and sampling perturbation limit before/after interpretation.

- The stack observer shifted final p99 by +3749.2 to +12692.9 µs across workloads, relative to the median profiler-off window. Charged server CPU moved -36.6 to -12.2 µs/read and process cycles moved -12.0% to -9.4%. One trace per observer and workload limits precision.
- The state observer shifted final p99 by -2.0 to +7.0 µs across workloads, relative to the median profiler-off window. Charged server CPU moved -36.6 to +26.9 µs/read and process cycles moved -1.8% to +2.4%. One trace per observer and workload limits precision.

## Waveshare ESP32-S3 and libcoap

The Coaptic Constrained service registers six handlers in eight route slots plus built-in discovery. It retains two 1,152-byte datagrams per direction, one 4,096-byte body per direction, four Calls, four duplicate rows, two Observe rows, four security request bindings, one association and zero deferred jobs. The C fixture serves static values with dynamic PDU/session storage. Both use the same 240 MHz Waveshare and 16 KiB instruction-cache setting. [Firmware and capacity definitions](ESP32-LIBCOAP-HEAD-TO-HEAD-20261123.md)

Two boots per implementation ran with the worker clock enabled, and two ran with it disabled, in reversed order. Each meter state has 16 reads per path across its two boots, so the reported nearest-rank p95 there is the largest observed latency. The native client checked every reply and byte. CPU is scheduled time of the server worker between receive return and send return. The 2,000-byte result sums both exchanges. The network and idle time before the next receive lie outside this interval.

| Workload | Server worker CPU ms | Ready ms | Blocked ms | Client median ms | Client p95 ms |
|---|---:|---:|---:|---:|---:|
| 7 bytes, Coaptic | 2.86 | 0.34 | 0.00 | 5.85 | 7.61 |
| 7 bytes, libcoap | 1.59 | 0.35 | 0.00 | 6.89 | 12.11 |
| 2,000 bytes, Coaptic | 6.53 | 0.69 | 0.00 | 17.67 | 24.34 |
| 2,000 bytes, libcoap | 3.82 | 0.54 | 0.00 | 16.16 | 25.36 |

- Coaptic ordinary firmware. Median boot rate 91.7 verified reads/s and host client CPU 1302.1 µs/mixed read. Maximum sampled host client working set 5.51 MiB and private memory 1.15 MiB. Worker stack reservation 196,608 bytes. Median sampled free device heap 68,700 bytes.
- libcoap ordinary firmware. Median boot rate 91.1 verified reads/s and host client CPU 325.5 µs/mixed read. Maximum sampled host client working set 5.52 MiB and private memory 1.14 MiB. Worker stack reservation 16,384 bytes. Median sampled free device heap 231,610 bytes.

The Rust unprotected meter’s build receipt inherited an old source-diff label. The archive it verified and linked has SHA-256 `b81a760992d92f63978ef7c57f0e9b44f37388d47dac19d08276f7ff9e646ddf`, identical to the ordinary final Rust firmware archive. The diagnostic image and observed run ID were also checked. Both the stale field and the archive-derived source identity are preserved in the JSON.

The ordinary seven-byte client medians were 5.85 ms for Coaptic and 6.89 ms for libcoap. The ordinary 2,000-byte medians were 17.67 and 16.16 ms. Client latency also includes Wi-Fi, host scheduling and work outside the measured worker interval. With only two ordinary boots each, the small-read latency ranking cannot substitute for the direct CPU comparison.

The Coaptic fixture reserves 192 KiB for its worker. Its minimum free stack in the ordinary runs was 89,728 bytes. Libcoap reserves 16 KiB and had at least 13,360 bytes free. These are whole firmware reservations and high-water observations for this workload. They include the adapter and its configured stack.

Coaptic reports free heap while the service runs. Libcoap reports the preceding figure immediately before cleanup. The C firmware uses Espressif libcoap 4.3.5~8. Both fixtures serve the same static application bytes. Coaptic keeps its bounded App, duplicate history and protocol ownership. Only Coaptic has the matched durable OSCORE service in this fixture. Each enabled worker meter reserved 2,560 bytes. With the meter enabled versus disabled, client median latency changed as follows.

- Coaptic. Seven bytes 5.85 to 7.19 ms. The 2,000-byte body 17.67 to 15.82 ms. One host CPU tick across 24 reads equals 651 µs/read.
- libcoap. Seven bytes 6.89 to 6.17 ms. The 2,000-byte body 16.16 to 19.67 ms. One host CPU tick across 24 reads equals 651 µs/read.

The earlier four-pair admission comparison in this batch used an independent Python wire client and 512 verified unprotected reads. Median boot medians for the 2,000-byte body were 17.49 to 16.92 ms at automatic size, 18.80 to 16.09 ms at 1,024 bytes, 62.92 to 63.97 ms at 256 bytes and 234.96 to 247.18 ms at 64 bytes. The last case needs 32 exchanges per read, against two at automatic size. Equal wire counts, zero reported drops and the crossover between sizes rule out a uniform speedup. The JSON also includes pooled medians, p95 and every boot.

## Durable OSCORE on the ESP32-S3

The same provisioned identity served both firmware builds with durable replay checks and commits active. Each built the normal protected service from the same Rust archive used for its unprotected image. Two reversed boot pairs ran three 24-read native windows per boot.

- Preceding checkout. Seven-byte median/p95 17.89/24.46 ms. The 2,000-byte median/p95 was 46.72/97.30 ms. Median verified rate across complete client windows 30.9/s. Median host client CPU 0.0 µs/read and maximum sampled working set 6.24 MiB.
  Across these boots, running device heap free was 68,592–68,612 bytes, with 196,608 bytes reserved for the worker. The minimum reported free stack was 137,260 bytes. Host client maximum sampled private memory was 1.36 MiB.
- Final. Seven-byte median/p95 18.35/63.49 ms. The 2,000-byte median/p95 was 46.36/95.16 ms. Median verified rate across complete client windows 30.9/s. Median host client CPU 651.0 µs/read and maximum sampled working set 6.18 MiB.
  Across these boots, running device heap free was 66,684–68,508 bytes, with 196,608 bytes reserved for the worker. The minimum reported free stack was 137,292 bytes. Host client maximum sampled private memory was 1.34 MiB.

The seven-byte protected p95 increased from 24.46 to 63.49 ms. Four of 48 final reads exceeded 40 ms, compared with two of 48 before. The final boot-level p95s were 66.6 and 20.4 ms. The outliers matter to a caller, and the two boot pairs give limited evidence about their cause. Host client CPU advances by 651 µs/read per tick in a 24-read window, which accounts for the zero median reading in the preceding build.


Four further metered protected boots used two reversed pairs. Their bounded task clock matched incoming and outgoing MIDs and lengths for every exchange of the same wire-verifying native client. For a complete large read, the table sums both exchanges before taking medians.

| Workload | Preceding worker CPU ms | Final worker CPU ms | Final ready ms | Final blocked ms |
|---|---:|---:|---:|---:|
| 7 bytes | 14.21 | 14.14 | 0.36 | 0.78 |
| 2,000 bytes | 34.35 | 34.78 | 0.70 | 0.74 |

- Preceding protected meter overhead and boot variation. Seven-byte client median 17.89 ms off and 17.49 ms on. Large client median 46.72 ms off and 48.38 ms on.
- Final protected meter overhead and boot variation. Seven-byte client median 18.35 ms off and 18.97 ms on. Large client median 46.36 ms off and 49.85 ms on.

The observer adds a 2,560-byte ledger. Receive-to-send includes the persistent replay checks and commits required by the configured service. A prior diagnostic attributed 289.7 of 602.3 ms of protected task CPU to storage reads and commits over 24 operations. [Earlier protected storage measurements](CHAINED-READS-20261125.md)

## Receive queue experiment

Progress had already checked whether a queued datagram belonged to a completed Call, then dispatch checked again. Only Apps configured to retain full datagram replies can have such a hold. The experiment skips both eligibility scans in the default configuration and keeps the first check when full replies are enabled. Retry and expiry between selection and dispatch can release a hold. Only ingress establishes a new one.

| Calls | Operation | Control ns | Candidate ns | Candidate faster |
|---:|---|---:|---:|---:|
| 4 | sync-get | 393 | 402 | 1/8 |
| 4 | async-get | 665 | 656 | 8/8 |
| 4 | sync-defer | 630 | 626 | 6/8 |
| 4 | async-defer | 896 | 917 | 0/8 |
| 32 | sync-get | 462 | 433 | 8/8 |
| 32 | async-get | 748 | 707 | 8/8 |
| 32 | sync-defer | 695 | 651 | 8/8 |
| 32 | async-defer | 1008 | 980 | 7/8 |

The receive-queue change stayed out of the checkout. At the device’s four-Call capacity, the synchronous GET slowed in seven of eight local pairs and async deferred completion slowed in all eight. Larger Call tables benefited. In the matched Windows release runs, 4 KiB OSCORE throughput rose 0.5% in six of six pairs with equal median charged server CPU. Unprotected 4 KiB median throughput fell 1.4% and its p99 rose from 101.8 to 107.8 µs. Both experimental binaries used the same fixed source directory, lock, fixture and release profile. The separately rebuilt control has a different text section from the retained release, so comparisons stay within the pair.

- Experimental release pair, 64 B unprotected. Throughput 60,802 to 60,993/s, server CPU 16.60 to 16.60 µs/read, client CPU 16.11 to 16.60 µs/read. Throughput favored the candidate in 4/6 pairs.
- Experimental release pair, 4 KiB unprotected. Throughput 14,770 to 14,567/s, server CPU 67.71 to 68.36 µs/read, client CPU 67.71 to 67.71 µs/read. Throughput favored the candidate in 3/6 pairs.
- Experimental release pair, 64 B OSCORE. Throughput 53,595 to 54,458/s, server CPU 18.55 to 18.55 µs/read, client CPU 18.55 to 18.55 µs/read. Throughput favored the candidate in 3/6 pairs.
- Experimental release pair, 4 KiB OSCORE. Throughput 11,025 to 11,077/s, server CPU 89.84 to 89.84 µs/read, client CPU 91.15 to 91.15 µs/read. Throughput favored the candidate in 6/6 pairs.

## Verified uploads over Waveshare Wi-Fi

The ordinary preceding and final images exchanged eight 16-byte Block1 fragments for each 128-byte POST to the existing upload endpoint, with two URI queries. The client verified the MIDs, tokens, intermediate block acknowledgements and the handler’s returned length and rolling checksum for every completed body. It alternated tagged and untagged transfers in the same seeded order on each firmware. Four boots in two reversed pairs delivered 192 bodies over 1,536 exchanges.

- Untagged. The median of the boot medians was 62.71 to 60.25 ms per verified body. The two individual boot medians were 63.20/62.22 ms before and 63.15/57.35 ms after. Pooled p95 was 78.70 to 67.89 ms.
- Request-Tag. The median of the boot medians was 65.08 to 60.88 ms per verified body. The two individual boot medians were 63.20/66.96 ms before and 63.63/58.12 ms after. Pooled p95 was 75.70 to 71.95 ms.
Nearly all of the apparent Wi-Fi gain came from the second pair. In the first, untagged uploads differed by 0.06 ms and tagged uploads took 0.43 ms longer with the change. The second pair favored the final build for both. The device confirms correct full transfers and gave a lower observed median. A repeatable 11% network gain remains unproven.

- Preceding firmware. Median mixed rate 15.36 complete uploads/s and Python client CPU 488.3 µs/upload. Peak client working set reached 26.59 MiB. The on-device upload worker ran with its meter off. Device CPU for uploads is unmeasured.
- Final firmware. Median mixed rate 16.38 complete uploads/s and Python client CPU 1139.3 µs/upload. Peak client working set reached 26.66 MiB. The on-device upload worker ran with its meter off. Device CPU for uploads is unmeasured.

## Cost of the observers

The first Windows thread-state observer used a 4.75-second lifetime and raced the end of a four-second process window. That attempt produced no usable state series. The private replacement uses a 3.4-second bound for both builds, samples the same target thread and uses the same jitter. All 16 completed observer windows passed the wire verifier. The final protected 4 KiB stack capture pushed p99 into milliseconds. All claimed speedups use the profiler-off controls.

## Correctness and open targets

A refused 16-byte downshift could previously change the current block size before failing payload validation. The preceding executable returned `SzxMismatch` on a valid retry at 64 bytes. The final executable preserved the transfer and accepted that retry at offset 64. Coverage includes payload-length and capacity refusal, retries at the old size, NUM0 arriving late, duplicate and tag matching, pool reuse, Call expiry and cancellation, and protected dispatch. The final tree passed 1,719 all-feature workspace tests, 1,052 no-default core tests, both strict Clippy configurations, and Rust 1.89 no-default library checks with and without OSCORE. The ESP32-S3 firmware linked the no-std release archive and the board’s normal firmware was restored and independently identified after each hardware campaign.

The transfer identity tests exercise a changed Token and exact operation options. The App tests check both Block1 and Q-Block1 query and Accept propagation on every block. Refusal tests check that an oversized query consumes neither network I/O nor a body slot. [Block ownership tests](../../src/storage/tests.rs) and [App path and refusal tests](../../src/app/tests.rs)

The 50% server CPU-time target remains open separately for unprotected and established OSCORE reads against the original project. The local parser and upload improvements, individual tail latencies, and device-versus-C results use different denominators.

- **Unprotected target open.** At one outstanding request and unpaced capacity, median charged server CPU fell 5.6% for 64 bytes and 8.9% for 4 KiB. Each percentage uses the original project’s corresponding workload.
- **Established OSCORE target open.** At one outstanding request and unpaced capacity, median charged server CPU fell 7.5% for 64 bytes and 9.6% for 4 KiB. Each percentage uses the original project’s corresponding workload.
