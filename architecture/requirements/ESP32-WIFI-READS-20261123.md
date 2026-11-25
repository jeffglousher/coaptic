# Physical ESP32-S3 Wi-Fi read performance

On November 23, 2026, the commissioned Waveshare returned at its recorded address. Its encrypted management connection verified the expected device identity and normal firmware. Four qualification configurations completed 768 verified reads across two boots each, plus two 24-read scheduled-CPU diagnostic trials. The normal application image and live identity were verified after every trial. OSCORE used the existing credentials and durable sender/replay history.

## End-to-end latency

Each row contains 64 complete reads. On each boot four fresh client processes ran eight cycles of identity, small value and large value. The second pass reversed configuration order. The small value took one exchange and the large value two. Latency runs from the client starting a read through checking its returned value. p95 is the nearest-rank 95th percentile. Differences between boots describe repeatability within this session.

| Device | Security | Route | Bytes | Median (ms) | p95 (ms) | Maximum (ms) | Boot 1 / 2 medians (ms) |
|---|---|---|---:|---:|---:|---:|---:|
| sync | unprotected | identity | 32 | 6.55 | 11.19 | 26.03 | 6.39 / 6.60 |
| sync | unprotected | test | 7 | 6.09 | 9.02 | 11.01 | 6.01 / 6.24 |
| sync | unprotected | large | 2000 | 16.06 | 22.26 | 24.60 | 14.48 / 17.73 |
| async | unprotected | identity | 32 | 6.33 | 10.75 | 14.34 | 6.90 / 5.96 |
| async | unprotected | test | 7 | 7.25 | 11.02 | 12.61 | 7.48 / 7.11 |
| async | unprotected | large | 2000 | 16.95 | 23.17 | 25.95 | 16.86 / 17.08 |
| sync | oscore | identity | 32 | 19.38 | 22.69 | 43.49 | 18.79 / 19.57 |
| sync | oscore | test | 7 | 16.94 | 20.24 | 23.64 | 16.29 / 17.65 |
| sync | oscore | large | 2000 | 42.78 | 50.73 | 53.36 | 42.17 / 43.50 |
| async | oscore | identity | 32 | 19.93 | 24.66 | 405.89 | 19.52 / 20.30 |
| async | oscore | test | 7 | 17.26 | 21.15 | 28.42 | 17.03 / 17.29 |
| async | oscore | large | 2000 | 44.83 | 50.71 | 56.45 | 44.57 / 45.78 |

## Delivered rate, client CPU and memory

One async OSCORE identity read took 405.89 ms, the first request in its boot's first batch. The client spent 405.61 ms awaiting the single response. The trace does not locate the cause inside the network or the device. This observation is included in the latency table.

Throughput is the median of eight batches of 24 sequential mixed reads, timed from first dispatch to final verification. It includes the gaps between reads inside each batch. A single Wi-Fi client issued one read at a time. This is the delivered rate for that workload. Device capacity under multiple clients and 800 offered starts/s was not tested.

Windows client CPU is user plus kernel time over the entire process, including startup, security recovery where applicable, reporting and teardown, divided by verified reads across the four second-boot batches. One 15.625 ms accounting tick changes a single 24-read batch by 651 us/read. The memory columns give the largest client working set and private bytes sampled at roughly 25 ms intervals. The device column is the smallest observed running internal free heap across snapshots on the two boots. Snapshots can miss traffic peaks.

| Device / security | Verified reads/s | Client CPU (us/read) | Client working set (MiB) | Client private (MiB) | Device free heap (KiB) |
|---|---:|---:|---:|---:|---:|
| sync-unprotected | 103.2 | 651.0 | 5.44 | 1.15 | 165.9 |
| async-unprotected | 97.9 | 325.5 | 5.44 | 1.14 | 69.3 |
| sync-oscore | 37.3 | 488.3 | 5.66 | 1.29 | 165.6 |
| async-oscore | 36.4 | 325.5 | 5.66 | 1.29 | 69.0 |

## Device execution versus waiting

The ordinary sync worker reserves 96 KiB of stack and the async worker reserves 192 KiB. The extra reservation accounts for most of the difference in observed free heap. The Windows client CPU figures cover only two to four accounting ticks per configuration across the four processes. Their resolution is insufficient to rank client efficiency.

The first diagnostic trial verified all 24 reads but its management log dropped clock records and lacked the end marker. It was excluded from device CPU statistics. On the repeat, the separately instrumented async established-OSCORE build matched 24 host reads and 32 exchanges to the device worker. Times below are arithmetic means for eight reads per route. Device time starts with worker-observed readiness (or receive return when the preceding readiness event is unavailable) and ends at final send return. Scheduled CPU includes attributed interrupt and flash stalls. Device and host intervals have different boundaries. Normal images have no comparable task CPU counter.

| Route | Host elapsed (ms) | Device elapsed (ms) | Device CPU (ms) | Blocked (ms) | Runnable, awaiting CPU (ms) |
|---|---:|---:|---:|---:|---:|
| identity | 22.52 | 17.46 | 16.44 | 0.56 | 0.46 |
| test | 21.15 | 15.94 | 15.16 | 0.53 | 0.25 |
| large | 50.42 | 43.47 | 38.88 | 3.40 | 1.18 |

The diagnostic has 350 ledger entries. Durable-state reads and commits used 109.8 and 135.0 ms of scheduled CPU over the full cohort. Their spans overlap the enclosing device work. Across all 816 verified reads, the client recorded 0 sends beyond the expected exchange count.

The ordinary images are the exact [November firmware matrix](ESP-WIFI-RELIABILITY-20261117.md), built from core `48e513e`. The only library source file changed between that revision and this checkout covers the std UDP adapter and tests. The device uses the non-std Xtensa build. The [instrumented image](ESP-DEVICE-RECOVERY-20261118.md) used its own retained source and includes an additional 28,672-byte trace ledger. These are fresh measurements of the qualified images. The campaign did not compare a pair of device-side optimizations or rerun libcoap. Both original-project 50% CPU-time goals remain open.

[Numerical results, per-boot summaries, batch process costs, image hashes and restoration receipts](ESP32-WIFI-READS-20261123.json).
