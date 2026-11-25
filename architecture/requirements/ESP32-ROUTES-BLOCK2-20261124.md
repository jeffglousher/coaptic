# Which route and block-size costs matter on the ESP32-S3?

**The checkout keeps one change.** A route removal now uses the slot the App already found before checking that protocol work had drained. Removing and re-registering the last of eight routes took 175 ns instead of 266 ns on Windows, with the change faster in all twelve pairs. The hash and shared-route dispatch experiments remain separate: both improved isolated host measurements, and neither produced a consistent improvement across the device read workloads.

On November 24 the same Waveshare served 1,152 verified unprotected reads and 36 additional identity reads. Each group of three boots included baseline async Coaptic (B), a heapless route-index prototype (H), and Espressif libcoap 4.3.5~8 (C). The six groups used every possible boot order. The firmware for this experiment predates the separate known-slot change below. The normal firmware identity was verified after restoration. [Per-boot measurements and firmware identities](ESP32-ROUTES-BLOCK2-20261124.json)

## Physical reads

With automatic block selection, libcoap finished both bodies sooner in all six paired rounds. The baseline Coaptic server took 44% longer for seven bytes and 47% longer for 2,000 bytes by the median paired ratio. The index improved the automatic large read by 3% in five pairs, split the small read three to three, and lost five of six comparisons with 64-byte blocks on the large body. That combination does not support enabling the hash for this device. The size of the remaining gap also matters: changing the default large request to 64-byte blocks raised the baseline median from 17.68 to 245.07 ms. Each additional exchange pays for work on both peers and another trip across Wi-Fi.

The Windows client issued one confirmable read at a time. Each boot ran eight randomized cycles of all eight body/Block2 combinations after identity verification and at least 33 seconds after upload. Each group shared a seed across its three boots. The client checked ACK, Content, token, MID, contiguous block offsets, block sizes, terminal marker and every body byte. The benchmark did not retry. Time covers request construction through the final verification, including client scheduling and Wi-Fi. The throughput column in the JSON is completed reads per second of active sequential measurement time. Capacity, client CPU and server CPU were unmeasured in this trial. Host builds overlapped parts of the device campaign. Boot-level comparisons account for order and expose run variation.

Both Coaptic images used six registered handlers in eight slots plus built-in discovery, two 1,152-byte datagram slots, one 4,096-byte body per direction, four duplicate entries, two Observe slots, four Calls, four security request bindings, one association and zero deferred jobs. OSCORE support was compiled in. The C image uses dynamically allocated sessions and PDUs, a static handler body and a 30-second unprotected service window. Its memory and security capabilities differ. [Firmware and service configuration](ESP32-LIBCOAP-HEAD-TO-HEAD-20261123.md)

Pooled median / p95 in milliseconds, with 48 reads per cell. The final column counts rounds in which the index improved the within-boot median relative to baseline.

| Body | Requested Block2 | Exchanges | Baseline async | Hash async | libcoap | Hash faster |
|---|---|---:|---:|---:|---:|---:|
| 7 B | automatic | 1 | 6.02 / 11.64 | 5.96 / 10.85 | 3.86 / 9.13 | 3/6 |
| 7 B | 1024 B | 1 | 6.54 / 12.76 | 6.16 / 11.91 | 4.05 / 7.35 | 4/6 |
| 7 B | 256 B | 1 | 6.30 / 11.56 | 6.49 / 13.57 | 4.00 / 8.51 | 3/6 |
| 7 B | 64 B | 1 | 6.42 / 9.01 | 6.54 / 11.16 | 4.18 / 8.79 | 2/6 |
| 2,000 B | automatic | 2 | 17.68 / 27.03 | 17.24 / 24.11 | 12.46 / 17.92 | 5/6 |
| 2,000 B | 1024 B | 2 | 16.77 / 22.27 | 17.60 / 25.41 | 12.85 / 20.72 | 3/6 |
| 2,000 B | 256 B | 8 | 64.17 / 72.60 | 62.70 / 84.87 | 45.97 / 69.54 | 3/6 |
| 2,000 B | 64 B | 32 | 245.07 / 278.85 | 247.01 / 306.68 | 171.05 / 227.67 | 1/6 |

### Choosing the first block

For the 2,000-byte body the two 1,024-byte cases returned the same two slices, 1,024 and 976 bytes. The first explicit request added two CoAP bytes in the Rust fixture. A separate counted Constrained App test exercised the real send path. Automatic selection tried to acquire a TX slot twice and encountered one whole-response size failure. The explicit 1,024-byte request acquired TX once and had zero failed encodes. Both forms acquired once with zero failures for a small body. The failed ordinary encoder writes framing before detecting insufficient space. It does not copy 2,000 bytes into a discarded datagram. These are call counts from a test build, separate from all clocks.

The client can already opt in through <code>.block2(BlockValue::from_size(0, false, 1024)?)</code>. This expresses a maximum block payload. The server may downshift, decides which bounded slot to use and signals further blocks with M. CoAP, UDP and OSCORE framing add bytes. For automatic selection, a candidate design is to reuse the encoder’s token, option and protection layout to select a fitting range before encoding, with one TX admission and identical error/replay ordering. Adding a caller-visible pool-slot selector would expose storage details without changing wire behavior. [Call builder](../../src/app/client.rs) and [RFC 7959, sections 2.3 and 2.9](../../knowledge/rfcs/rfc7959.txt)

### Device memory

- Baseline Coaptic had 68,796 bytes of median free heap across six boots (68,764–68,808).
- Indexed Coaptic had 68,774 bytes of median free heap across six boots (68,760–68,800).
- libcoap had 233,174 bytes of median free heap across six boots (226,252–235,184).

Coaptic reports the last running, all-8-bit heap sample. The C fixture reports heap immediately before cleanup. Their worker stack reservations were 196,608 and 16,384 bytes respectively. The index added 32 bytes to an eight-slot Site and 2,256 bytes to the application image, from 1,641,568 to 1,643,824. Heap snapshots include SDK and network state. They move by more than the 32-byte Site change.

## Windows lookup and mutation costs

The full fixed-capacity Site kept the original stable route slots and exact segment equality. The index added a 16-bit head per bucket and a 16-bit link per route, totaling four bytes per capacity slot. It hashed the borrowed UTF-8 bytes and segment boundaries with FNV-1a; a collision requires comparing each route in the chain. Removal unlinks and reuses a stable slot. A one-core Windows release probe ran one million queries per cell, rotating four algorithms, discarding warmup and retaining eight rounds. The table reports median ns/query on a 75% hit, 25% miss mix of single-segment keys. Eight-route keys used short endpoint names. The larger fixtures used numbered resources. The C lookup called HASH_FIND from libcoap’s own uthash header through a DLL using an already contiguous key. Neither side timed URI construction or a handler in this microbenchmark.

| Routes | Linear | Hash | Sorted directory | libcoap hash |
|---:|---:|---:|---:|
| 8 | 41.26 | 28.31 | 31.21 | 20.47 |
| 32 | 198.41 | 34.56 | 61.57 | 22.66 |
| 128 | 669.76 | 33.90 | 83.24 | 26.60 |

At eight routes the first registered route took 10.52 ns by scan and 16.25 ns through the index. At 128 routes the last match took 1,048 and 27 ns respectively. The C hash still won the mixed lookup at all three sizes. Neither the host microbenchmark nor the unconstrained C memory policy establishes a device win. The software prototype passed 1,249 all-feature library tests including collision, full-table refusal, replacement, removal and reuse.

## First attempt at reusing a known slot

An Observe interest stores an exact local route identity encoded as the stable slot plus one. The previous snapshot-source lookup scanned occupied slots and reconstructed each identity for comparison. The changed code decodes that value once, checks its range and looks up the occupied entry. NONE, an unoccupied slot and a route without a snapshot still have no source. App route removal uses the slot it found before the busy check. The configuration path still returns unknown before busy, holds an exclusive App borrow and frees the same stable slot after protocol work drains. Neither change allocates memory.

The release timings below use twelve alternating pairs after warmup. Each source sample performed 500,000 checked lookups. Each integrated sample verified 12,000 actual unprotected Observe registration replies through App::poll with a fixed in-memory transport, one endpoint and a fresh MID per request. The update benchmark used 40,000 complete operations per cell with a full table, alternating three implementations. Source time is ns/lookup. Registration and route removal plus reuse are ns/operation.

| Routes / position | Source, baseline → direct | Registration, baseline → direct | Faster registrations | App remove + reuse, baseline → direct | Indexed prototype remove + reuse |
|---|---:|---:|---:|---:|---:|
| 8 / 1 | 2.3 → 0.7 | 1104.3 → 1093.1 | 6/12 | 129.6 → 114.9 | 125.7 |
| 8 / 8 | 2.7 → 0.7 | 1243.6 → 1208.3 | 11/12 | 267.2 → 174.1 | 125.2 |
| 32 / 32 | 1.8 → 0.7 | 1498.9 → 1544.5 | 1/12 | 949.9 → 611.7 | 138.6 |
| 128 / 128 | 34.0 → 0.7 | 2838.4 → 2883.7 | 4/12 | 3571.8 → 2264.2 | 149.6 |

All mutation cells, including replacement, first and last routes, are in the JSON. The physical GET table above measures a different change. Known-slot reuse was exercised by the Windows Observe and configuration workloads. The software regression suite covers occupied and missing sources, routes with holes, busy refusal, pinned snapshots, expiry, cancellation, duplicate handling and reuse.


## Ordinary Windows UDP reads on the first attempt

These runs check for a spillover effect on ordinary reads from the changed router. They used the historical current-thread Tokio fixture, release with symbols, a frozen client, 800 offered operations/s, one outstanding confirmable exchange, 32 warmup reads and 3,200 measured reads per cell. Each operation verified status, Content-Format, ETag and every payload byte. For 4 KiB responses the client also verified all four exchanges. All six permutations of original project (P), pre-change checkout (B) and the changed checkout (D) ran once. Each arm therefore occupied every position twice. The pre-change executable was built from source identical to the checkout before these edits. B and D share the same fixture source and dependency lock; P uses its original resolved lock. The unchanged security context was established before protected measurement. The 230,400 reads were timed with the profiler off.

Each row shows the median of six runs. CPU is process user plus kernel time divided by verified reads. Latencies are the medians of the runs’ p50 and p99. Working sets are end-of-window samples and include the runtime. The JSON separately records private bytes, lifetime peak working sets, user/kernel split, cycles and missed pacing periods.

| Mode | Body | Build | Server CPU, µs/read | Client CPU, µs/read | p50 / p99, ms | Reads/s | Server / client working set, MiB |
|---|---|---|---:|---:|---:|---:|---:|
| unprotected | 64 | P | 39.06 | 53.71 | 0.043 / 0.081 | 800.2 | 6.14 / 4.93 |
| unprotected | 64 | B | 26.86 | 34.18 | 0.042 / 0.080 | 800.2 | 6.06 / 4.93 |
| unprotected | 64 | D | 31.74 | 53.71 | 0.041 / 0.080 | 800.2 | 6.06 / 4.93 |
| unprotected | 4096 | P | 109.86 | 126.95 | 0.113 / 0.260 | 800.1 | 6.18 / 4.98 |
| unprotected | 4096 | B | 119.63 | 126.95 | 0.113 / 0.315 | 800.2 | 6.11 / 4.98 |
| unprotected | 4096 | D | 87.89 | 104.98 | 0.110 / 0.311 | 800.1 | 6.10 / 4.98 |
| oscore | 64 | P | 29.30 | 43.95 | 0.047 / 0.088 | 800.2 | 6.24 / 5.01 |
| oscore | 64 | B | 12.21 | 24.41 | 0.044 / 0.086 | 800.1 | 6.15 / 5.01 |
| oscore | 64 | D | 48.83 | 80.57 | 0.047 / 0.087 | 800.2 | 6.15 / 5.01 |
| oscore | 4096 | P | 129.39 | 166.02 | 0.140 / 0.286 | 800.1 | 6.28 / 5.06 |
| oscore | 4096 | B | 139.16 | 175.78 | 0.138 / 0.273 | 800.1 | 6.20 / 5.06 |
| oscore | 4096 | D | 139.16 | 168.46 | 0.137 / 0.262 | 800.1 | 6.19 / 5.06 |

The project target requires at least 50% lower server CPU per verified operation against P. Each security mode is evaluated independently. The following are changes in total measured server CPU over equal numbers of verified reads across all six pairs; negative values mean less CPU.

- unprotected/64: D against P -20.0% (3/6 lower); D against B +22.2% (2/6 lower).
- unprotected/4096: D against P -21.7% (4/6 lower); D against B -21.7% (4/6 lower).
- oscore/64: D against P +23.9% (2/6 lower); D against B +111.1% (1/6 lower).
- oscore/4096: D against P +5.0% (2/6 lower); D against B +2.5% (3/6 lower).

The ordinary GETs do not request Observe or mutate routes. They provide a regression and baseline check. Direction changes here cannot establish a direct causal benefit from the known-slot optimization. Two pre-change unprotected 64-byte runs recorded zero server CPU ticks despite completing all reads. All observations are retained. Percentages use the ratio of total CPU time across six equal-count windows. Individual percentage ratios would be undefined for those two pairs. [Prior CPU sampling and wait boundaries](WINDOWS-FOREGROUND-READS-20261122.md) and [prior original-project controls](WINDOWS-ESP-READ-VALIDATION-20261122.md).


## Matching a route once

The small protected control illustrates the limits of the Windows CPU clock. The direct-lookup build used 111% more reported server CPU than the pre-change build across six equal-count windows. Their median server cycle counts were 128,723 and 128,609 cycles per read, a 0.09% difference. The frozen client's CPU reading moved substantially too. The two counters and the latency measurements do not establish a doubling of the work performed by the server.

The first attempt made resolving a stored identity faster in all twelve host pairs, but complete registration at 32 routes slowed down in eleven pairs. Tracing the next caller showed that registration and dispatch each searched for the same URI. The revised server resolves the authenticated or unprotected request path once, then shares a borrow of the method router and its stable slot identity with Observe planning and handler dispatch. A custom dispatch function bypasses the site lookup when it has no Observe or DELETE work. Lookup of a retained Observe identity uses a checked array index. Route removal retains its separate busy check and frees the slot already found. The public API, capacities and discovery ordering are unchanged.

The three-arm release rerun rotated the original, direct-lookup-only and shared-match binaries through all orders twice after warmup. Each row is a complete verified registration, in nanoseconds. The last column counts comparisons with a faster shared-match binary than the original.

| Routes / position | Original | Direct lookup only | Shared match | Paired change | Faster rounds |
|---|---:|---:|---:|---:|---:|
| 8 / 1 | 1,096 | 1,099 | 1,075 | -1.4% | 12/12 |
| 8 / 8 | 1,232 | 1,230 | 1,155 | -5.9% | 12/12 |
| 32 / 32 | 1,487 | 1,534 | 1,279 | -14.0% | 12/12 |
| 128 / 128 | 2,866 | 2,842 | 1,945 | -31.2% | 11/12 |

A source recount also corrected the fixture description in the earlier comparison. Six handlers occupy six of the eight slots. Built-in discovery serves a seventh path by a flag on the Site. This changes the occupancy label only. No measurements were rescaled.

The device block-size clocks include Wi-Fi, readiness and scheduling. For a separate comparison with a worker CPU clock, the instrumented established-OSCORE firmware averaged 15.16 ms executing, 0.53 ms blocked and 0.25 ms runnable for a 7-byte read. The 2,000-byte read averaged 38.88 ms executing, 3.40 ms blocked and 1.18 ms runnable. Those clocks describe the earlier protected worker. [Worker timing and diagnostic overhead](ESP32-WIFI-READS-20261123.md).


## Installed shared-match firmware

The same Waveshare alternated original and shared-match firmware in four pairs (BE, EB, EB, BE). E was built from the changed checkout; its image was 752 bytes larger. Each boot ran 24 randomized cycles with a plain seven-byte GET, a plain 2,000-byte GET, a seven-byte GET whose handler declined Observe, and registration/deregistration on the changing four-byte ticks resource. A real notification was required between registration and deregistration. The client verified the request MID and token, reply code, every static payload byte, the notification token and Observe option, and valid monotonic tick values. A confirmable notification was acknowledged. The register and cancel latency clocks exclude the deliberate wait for that notification. Each boot included two identity reads and two ordinary tick controls.

The retained series delivered 960 timed replies, 192 separately verified notifications, 32 controls and 0 recorded capacity refusals. A separate preliminary baseline boot ended on an unexpected deregistration reply before the collector saved its code. That pilot was restored and excluded from timing. It cannot identify the reason for the reply. The retained client waits for and verifies a notification before cancelling, and records capacity refusals explicitly. Both builds used this same client. The original normal image was independently verified after the campaign.

| Operation | Original p50 / p95, ms | Shared match p50 / p95, ms | Paired change | Faster pairs |
|---|---:|---:|---:|---:|
| Ordinary 7 B | 6.01 / 10.85 | 6.11 / 11.12 | +2.2% | 1/4 |
| Ordinary 2,000 B | 17.49 / 22.96 | 17.10 / 22.70 | -1.2% | 4/4 |
| Observe declined, 7 B | 6.56 / 9.96 | 6.35 / 10.19 | -5.3% | 3/4 |
| Observe registration | 6.86 / 13.62 | 6.97 / 13.23 | +6.5% | 2/4 |
| Observe deregistration | 7.12 / 11.34 | 6.27 / 9.54 | -7.0% | 3/4 |

The client numbers include handling notifications. Overall wall throughput includes the deliberate wait for the device’s 250 ms tick, so it is specific to this workload. The server has no worker CPU clock in either profiler-off image. The per-boot JSON includes memory readings and the software drop/wait-error counters.

- B completed 19.8 timed reads/s including notification waits, used 195 µs of client process CPU per verified timed read and ended with a 26.74 MiB client working set. The server had 68,720 bytes of median free heap.
- E completed 19.8 timed reads/s including notification waits, used 65 µs of client process CPU per verified timed read and ended with a 26.65 MiB client working set. The server had 68,712 bytes of median free heap.


## Windows checks on the shared-match build

The final test used the release-with-symbols shared-match server (E), the pre-change executable (B) and the original project server (P). The fixed current-thread Tokio workload again offered 800 verified reads/s. Three rotations (PBE, BEP, EPB) put each executable in every position once. Each window verified 6,400 operations, doubling the earlier read count per window to improve CPU accounting resolution. The 230,400 measured operations include ordinary and blockwise bodies in both security modes. The current and changed server used the identical fixture source, dependency lock and runtime. The historical baseline used the same fixture and its historical core and dependency lock. The frozen client, 32-read warmup, one outstanding exchange and supplied capacities were the same for every build within each cell.

Each value is the median of three runs. Latencies are within-run percentiles. The JSON provides all individual windows, process cycles, private bytes, lifetime peak memory and the user/kernel split.

| Security | Body | Build | Server CPU, µs/read | Client CPU, µs/read | p50 / p99, ms | Reads/s | Server / client working set, MiB |
|---|---:|---|---:|---:|---:|---:|---:|
| unprotected | 64 | P | 51.27 | 83.01 | 0.048 / 0.080 | 800.1 | 6.14 / 4.95 |
| unprotected | 64 | B | 46.39 | 63.48 | 0.045 / 0.078 | 800.1 | 6.05 / 4.95 |
| unprotected | 64 | E | 51.27 | 56.15 | 0.046 / 0.079 | 800.1 | 6.07 / 4.95 |
| unprotected | 4096 | P | 114.75 | 148.93 | 0.133 / 0.244 | 800.1 | 6.18 / 5.00 |
| unprotected | 4096 | B | 131.84 | 146.48 | 0.127 / 0.258 | 800.1 | 6.10 / 5.00 |
| unprotected | 4096 | E | 122.07 | 153.81 | 0.128 / 0.260 | 800.1 | 6.11 / 5.00 |
| oscore | 64 | P | 58.59 | 58.59 | 0.054 / 0.088 | 800.1 | 6.23 / 5.04 |
| oscore | 64 | B | 17.09 | 17.09 | 0.052 / 0.086 | 800.1 | 6.15 / 5.04 |
| oscore | 64 | E | 48.83 | 63.48 | 0.052 / 0.086 | 800.1 | 6.18 / 5.03 |
| oscore | 4096 | P | 122.07 | 153.81 | 0.190 / 0.275 | 800.1 | 6.27 / 5.08 |
| oscore | 4096 | B | 163.57 | 207.52 | 0.181 / 0.271 | 800.0 | 6.20 / 5.08 |
| oscore | 4096 | E | 153.81 | 185.55 | 0.183 / 0.276 | 800.1 | 6.21 / 5.08 |

Percent changes below divide total E server CPU by the reference total across equal-count windows. The paired count tracks how often E used less CPU.

- unprotected/64 against the original project: -4.5%, lower in 2/3. Against the pre-change checkout: +56.1%, lower in 0/3.
- unprotected/4096 against the original project: -11.3%, lower in 1/3. Against the pre-change checkout: +2.9%, lower in 2/3.
- oscore/64 against the original project: -26.0%, lower in 3/3. Against the pre-change checkout: +80.0%, lower in 0/3.
- oscore/4096 against the original project: +28.8%, lower in 1/3. Against the pre-change checkout: +0.5%, lower in 2/3.

Both 50% original-project server CPU targets remain open. The row-by-row values distinguish the direct effect on Observe from ordinary GETs and from differences between historical builds.


## Change retained in the checkout

The retained Rust change is confined to route removal. The App used to walk the table to check that a route existed, verify that protocol state had drained, then walk the table again to remove it. It now keeps the slot from the first lookup and removes that slot after the same busy check. Unknown routes still return false even while busy, a busy refusal preserves the route, and the freed slot is reusable. It adds no storage or public API. The runtime GET and Observe dispatch implementations are the original ones.

An instrumented Constrained App confirmed the operation count on the last of eight routes. Removal fell from 16 exact path comparisons to eight. An ordinary GET still performed eight. The experimental shared-match dispatch also reduced Observe registration from 16 comparisons to eight, but on the six-handler device ordinary small reads were slower in three of four pairs and real registration split two to two. The host improvement is worth preserving as evidence for larger services. The shared-match dispatch and the heapless hash are excluded from the default checkout.

These timings compare the original release and the final removal-only release, in ns per complete successful App removal and re-registration. Each of twelve paired runs performed 40,000 cycles per cell after warmup and checked removal and re-registration. The full Site remained at its configured capacity. This was a local configuration operation, with a transport that fails the test if configuration sends a packet.

| Routes | Slot position | Original ns | Retained ns | Paired change | Faster pairs |
|---:|---:|---:|---:|---:|---:|
| 8 | 1 | 131.6 | 115.7 | -12.1% | 12/12 |
| 8 | 8 | 266.0 | 174.9 | -34.3% | 12/12 |
| 32 | 1 | 380.8 | 340.7 | -9.7% | 12/12 |
| 32 | 32 | 962.1 | 610.0 | -36.6% | 12/12 |
| 128 | 1 | 1,246.8 | 1,229.1 | -1.6% | 9/12 |
| 128 | 128 | 3,571.9 | 2,270.2 | -36.4% | 12/12 |

All-feature and no-default-feature regression suites and strict Clippy ran on the retained source. Tests cover unknown and busy refusal, route identity and reuse, outstanding Calls, deferred work, duplicate-retention expiry, Observe and installed security bindings. None of the packet-path timings from the experimental firmware are attributed to this local configuration change.


## Windows checks on the retained change

The final test used the release-with-symbols route-removal-only server (R), the pre-change executable (B) and the original project server (P). The fixed current-thread Tokio workload again offered 800 verified reads/s. Three rotations (PBR, BRP, RPB) put each executable in every position once. Each window verified 3,200 operations. The 115,200 measured operations include ordinary and blockwise bodies in both security modes. The current and changed server used the identical fixture source, dependency lock and runtime. The historical baseline used the same fixture and its historical core and dependency lock. The frozen client, 32-read warmup, one outstanding exchange and supplied capacities were the same for every build within each cell.

Each value is the median of three runs. Latencies are within-run percentiles. The JSON provides all individual windows, process cycles, private bytes, lifetime peak memory and the user/kernel split.

| Security | Body | Build | Server CPU, µs/read | Client CPU, µs/read | p50 / p99, ms | Reads/s | Server / client working set, MiB |
|---|---:|---|---:|---:|---:|---:|---:|
| unprotected | 64 | P | 39.06 | 68.36 | 0.039 / 0.080 | 800.2 | 6.13 / 4.93 |
| unprotected | 64 | B | 53.71 | 63.48 | 0.036 / 0.077 | 800.2 | 6.05 / 4.93 |
| unprotected | 64 | R | 34.18 | 39.06 | 0.037 / 0.080 | 800.2 | 6.06 / 4.93 |
| unprotected | 4096 | P | 117.19 | 126.95 | 0.105 / 0.287 | 800.1 | 6.18 / 4.98 |
| unprotected | 4096 | B | 112.30 | 112.30 | 0.100 / 0.307 | 800.1 | 6.10 / 4.98 |
| unprotected | 4096 | R | 107.42 | 141.60 | 0.105 / 0.303 | 800.2 | 6.09 / 4.98 |
| oscore | 64 | P | 48.83 | 58.59 | 0.041 / 0.086 | 800.2 | 6.23 / 5.01 |
| oscore | 64 | B | 48.83 | 78.12 | 0.040 / 0.084 | 800.1 | 6.14 / 5.00 |
| oscore | 64 | R | 43.95 | 43.95 | 0.045 / 0.084 | 800.2 | 6.14 / 5.01 |
| oscore | 4096 | P | 146.48 | 166.02 | 0.140 / 0.300 | 800.1 | 6.27 / 5.06 |
| oscore | 4096 | B | 146.48 | 175.78 | 0.137 / 0.279 | 800.1 | 6.19 / 5.06 |
| oscore | 4096 | R | 141.60 | 195.31 | 0.139 / 0.335 | 800.1 | 6.19 / 5.06 |

Percent changes below divide total R server CPU by the reference total across equal-count windows. The paired count tracks how often R used less CPU.

- unprotected/64 against the original project: -20.0%, lower in 1/3. Against the pre-change checkout: -40.7%, lower in 2/3.
- unprotected/4096 against the original project: -5.5%, lower in 1/3. Against the pre-change checkout: -1.4%, lower in 2/3.
- oscore/64 against the original project: +18.2%, lower in 1/3. Against the pre-change checkout: -21.2%, lower in 3/3.
- oscore/4096 against the original project: +6.0%, lower in 2/3. Against the pre-change checkout: +0.0%, lower in 2/3.

Both 50% original-project server CPU targets remain open. The row-by-row values are ordinary read controls for a change confined to administrative route removal.
