# Windows request options, duplicate lookup and thread scheduling

Both source changes were reverted. The duplicate lookup reduced unprotected blockwise server cycles in four fresh pairs, but server CPU time increased in two. In an identical-binary control, a protected blockwise pair appeared to halve server CPU time with only a 0.9% cycle difference.

We tested two independent source changes in the synchronous App on the current-thread Tokio runtime. Each release used the same MSVC optimization settings, debug symbols, companion fixture, frozen client and bounded host-control capacities as the [earlier Windows stack profile](WINDOWS-READ-PROFILE-20261120.md). The source baseline is `a41180e`. Its measured core is identical to `f8ffae4`. Both controls and candidates used the fixture with phase counters disabled. The client verified status, ETag, Content-Format and all representation bytes. A 64 B read required one datagram in each direction and a 4 KiB read required four. OSCORE contexts were already established.

Each profiler-off comparison used three adjacent pairs of 12,000 reads per cell at 800 offered starts/s. The middle pass reversed arm and cell order. Each separate stack profile used one on/off pair of 4,800 reads per cell. A wall-stack observation can include a blocked or preempted thread. The duplicate counts below include the enclosing replay, pin eviction and reply storage paths. Both candidate patches and the per-window CPU time, cycles, throughput, latency and memory are in the [receipt](WINDOWS-OPTIONS-DEDUP-PROFILE-20261120.json).

## Changes and profiler-off server CPU results

The option candidate replaces up to four outer Block and Q-Block presence searches with one ordered traversal. It runs after the outer duplicate check and before opening an OSCORE message. The allocated duplicate-table candidate reads the existing optional row directly. Successful raw acquisition and release clear the payload, and typed insertion writes a row only into an occupied slot. Raw occupied slots without a payload still yield an empty lookup. Both candidates passed the 1,711-test all-feature suite individually, including finite-blackout Call recovery, cancellation, expiry, duplicate replay and capacity refusal/reuse.

| Case | Outer scan CPU changes | Dedup lookup CPU changes | Pairs passing pace, latency and CPU floor, outer / dedup |
|---|---|---|---:|
| Unprotected 64 B | -19.1%, +136.4%, +260.0% | -32.5%, +30.3%, -55.6% | 0 / 0 |
| Unprotected 4 KiB | +46.2%, -23.3%, -36.7% | -17.3%, -0.9%, -3.2% | 0 / 3 |
| Established OSCORE 64 B | -10.6%, -13.9%, +15.4% | -60.0%, +46.7%, -8.9% | 0 / 1 |
| Established OSCORE 4 KiB | +71.2%, -4.3%, +189.1% | +27.3%, +10.3%, -24.3% | 0 / 1 |

The protected blockwise outer-check samples fell from 19/3,438 to 5/2,976, but the three profiler-off server cycle changes were +5.1%, +2.4% and +5.8%. Unprotected traffic skips the modified function. The unprotected pairs therefore measure between-build variation unrelated to that scan. The duplicate lookup reduced unprotected blockwise CPU in all three pairs, with two server cycle increases. Protected blockwise CPU rose in two pairs. Neither change qualified across the workload, and both were reverted.

On the baseline the stack observer changed whole-process server cycles by -5.7% to +5.3%. Baseline p99 latency ranged from 121 to 363 µs without sampling and 3.43 to 7.21 ms during sampling. The sampler targeted the first thread enumerated by the fixture, and the captured App stacks identify the serving thread on the named paths. These inclusive counts do not isolate cycles spent checking occupancy.

## Retained control at 800 reads/s

Medians use the six profiler-off control windows per cell. CPU is per verified read. Memory is the closing working set in MiB, with private bytes retained in the receipt.

| Case | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client MiB |
|---|---:|---:|---:|---:|---:|
| Unprotected 64 B | 47.5 | 65.1 | 53.1 / 153.6 | 800.04 | 5.91 / 5.00 |
| Unprotected 4 KiB | 121.1 | 151.7 | 114.2 / 334.6 | 800.05 | 5.94 / 5.04 |
| Established OSCORE 64 B | 51.4 | 71.6 | 56.8 / 176.6 | 800.05 | 6.01 / 5.08 |
| Established OSCORE 4 KiB | 94.4 | 107.4 | 144.2 / 382.2 | 800.04 | 6.04 / 5.12 |

## Running, ready and waiting

A separate observer repeatedly refreshed the benchmark process and read the earliest-created thread’s Windows scheduler state. It sampled for up to 4.75 seconds with randomized sleeps and a fixed sample bound. Each cell had a same-binary on/off pair. These observations distinguish ready-to-run and running states from waits. Windows wait-reason labels describe kernel wait categories. They do not identify a particular socket or measure wake-to-run latency. The stack and state captures ran in separate windows. Shared tracing and scheduler settings stayed intact.

| Case | Wait | Ready + standby | Running | Other | Total | Observer query median ms | On/off server cycles change |
|---|---:|---:|---:|---:|---:|---:|---:|
| Established OSCORE 4 KiB | 205 | 1 | 26 | 0 | 232 | 5.42 | +10.7% |
| Established OSCORE 64 B | 238 | 1 | 13 | 0 | 252 | 4.97 | +3.9% |
| Unprotected 4 KiB | 253 | 2 | 29 | 0 | 284 | 3.84 | +5.1% |
| Unprotected 64 B | 254 | 1 | 12 | 0 | 267 | 4.03 | +2.1% |

Each observed process initially had four threads. Only the earliest-created thread was sampled. Process CPU and cycles cover all four. The earliest-created thread was waiting in 950 snapshots, running in 80 and on standby in five. Windows labeled all sampled waits `EventPairLow`. These are counts of sampled states rather than integrated CPU seconds. The socket calls and completion dequeue appear in the separate stack profiles. One benchmark command ended during the profile campaign. After confirming its children had exited, the runner resumed the remaining windows and the analysis validated every retained window against its executable digest and verified datagram totals.

## Identical-binary pacing control

At each rate both arms ran the identical server executable. We alternated rate order, kept the verifying client fixed and compared adjacent A/A windows. Before starting we required every 799/s server CPU ratio to fall between 0.90 and 1.10, and the median absolute log ratio to improve over 800/s in both cells. This tested whether moving away from exactly 800/s improves Windows CPU-time repeatability.

| Case | Rate | B/A CPU ratios | B/A server cycle ratios |
|---|---:|---|---|
| Unprotected 64 B | 800 | 2.083, 3.545, 1.346 | 1.012, 1.000, 1.029 |
| Unprotected 64 B | 799 | 0.833, 1.276, 0.857 | 0.968, 1.004, 0.987 |
| Established OSCORE 4 KiB | 800 | 0.670, 0.632, 1.095 | 1.051, 1.085, 1.005 |
| Established OSCORE 4 KiB | 799 | 1.302, 1.089, 0.500 | 1.024, 1.001, 0.991 |

Five identical-binary windows exceeded 12 missed periods. Three were protected 800/s windows (93, 1,443 and 1,574 misses). One unprotected window at each rate also exceeded the limit (20 at 799/s and 23 at 800/s). The protected 0.500 CPU-ratio pair met the pacing limit with 10 and nine misses.

All 1,209,600 reads, including the confirmation below, verified. The identical-binary control failed the predeclared 799/s repeatability criterion. The data do not support treating a small tick-time reduction as a server optimization on this host. Keep the unprotected and established OSCORE 50% CPU-time targets open against the original project source. The comparison in this campaign uses the current checkout as its control.

## Fresh confirmation of the blockwise result

After the allocated lookup reduced unprotected blockwise server CPU in all three initial pairs, we froze both binaries and repeated four pairs per mode. The confirmation reversed arm and cell order on alternate passes. It added 192,000 verified reads at the same 800/s and used the same latency, pacing and CPU-floor criteria.

| Blockwise case | Server CPU changes | Server cycle changes | Pairs passing pacing, latency and CPU floor | Pooled latency pass |
|---|---|---|---:|---|
| unprotected | +58.1%, +62.0%, -20.0%, -46.9% | -3.6%, -2.5%, -2.1%, -5.1% | 3 / 4 | True |
| oscore | +61.8%, +6.4%, +37.8%, -79.7% | +5.9%, -2.7%, -3.0%, -2.7% | 1 / 4 | True |

The initial unprotected CPU-time reduction did not repeat. Protected blockwise server CPU time increased in five of the seven pairs across the original comparison and confirmation. Both pooled latency comparisons passed. The evidence supports keeping the existing lookup and pool bookkeeping.
