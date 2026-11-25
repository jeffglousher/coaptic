# Windows and ESP32-S3 read validation

On November 22 the Windows runner completed 281,600 verified reads in 64 profiler-off windows. Four pairs per workload balanced the build order. The physical ESP32-S3 was unavailable on its recorded LAN address and encrypted API, and its mDNS name did not resolve. The device figures below were regraded from earlier physical runs.

## Windows

The foreground comparison used the old one-millisecond sleep and the shipped socket-readiness policy, with otherwise identical standard-thread server builds. The client offered 500 starts/s for 64 B and 125/s for 4 KiB. The original-project comparison used the same historical current-thread Tokio fixture and 800 starts/s on each side, comparing original `b8f44fa` with the executable for `f8ffae43`. The current checkout has identical production library sources to the latter. The two historical arms used their project-resolved dependency locks. Each release build had symbols. Within each comparison the client executable, pool capacities, security setup, 32-read warmup and one-outstanding-CON workload were fixed. Every read checked status, Content-Format, ETag and the full expected body.

Values are medians across four runs. CPU is user plus kernel time per verified read. Latency begins at dispatch. All figures read original / changed.

| Comparison and workload | Server CPU (us/read) | Client CPU (us/read) | p50 (us) | p99 (us) | Verified reads/s |
|---|---:|---:|---:|---:|---:|
| Foreground unprotected 64 B | 21.7 / 28.2 | 1,095.9 / 43.4 | 1,515.4 / 48.5 | 1,590.2 / 76.0 | 500.1 / 500.1 |
| Foreground unprotected 4096 B | 104.2 / 91.1 | 5,631.5 / 136.7 | 5,626.9 / 151.7 | 6,253.8 / 225.3 | 125.0 / 125.1 |
| Foreground oscore 64 B | 28.2 / 41.2 | 1,108.9 / 69.4 | 1,528.1 / 54.7 | 1,600.5 / 85.1 | 500.0 / 500.1 |
| Foreground oscore 4096 B | 136.7 / 130.2 | 5,618.5 / 221.4 | 5,656.3 / 188.2 | 6,376.9 / 308.3 | 125.0 / 125.1 |
| Project unprotected 64 B | 40.3 / 52.5 | 41.5 / 68.4 | 51.8 / 46.5 | 77.3 / 75.7 | 800.1 / 800.1 |
| Project unprotected 4096 B | 129.4 / 136.7 | 147.7 / 148.9 | 142.3 / 142.4 | 263.6 / 256.6 | 800.1 / 800.1 |
| Project oscore 64 B | 46.4 / 29.3 | 58.6 / 41.5 | 55.1 / 52.4 | 84.8 / 85.3 | 800.1 / 800.1 |
| Project oscore 4096 B | 181.9 / 129.4 | 212.4 / 153.8 | 196.0 / 186.4 | 296.4 / 272.4 | 800.1 / 800.1 |

The [per-window CSV](WINDOWS-ESP-READ-VALIDATION-20261122.csv) includes separate server and client CPU time, cycles, closing working set and private bytes (MiB), process-lifetime peak working set (MiB), raw-window and executable hashes, dispatch lateness and verified counts. CPU time is quantized. Scheduled-start mean latency adds mean dispatch delay to mean service time. Aggregate start lateness supports that mean only. The two 50% project CPU-time targets remain open.

Across the 16 foreground pairs, the readiness server lowered median latency, p99, client CPU and server cycles every time. Much of the client saving comes from spending fewer cycles spinning for a reply. At these matched rates, the foreground server's blockwise CPU medians fell 12.5% unprotected and 4.8% with OSCORE. Individual CPU measurements moved in both directions. Closing memory was similar.

Against the original project, OSCORE blockwise reads used less server CPU in four of four pairs, with a 28.9% lower median. The client used 153.8 versus 212.4 microseconds per verified read. Unprotected small reads used more server CPU in three pairs and tied in one, with a 30.3% higher median. These results concern distinct server loops and baselines. The historical fixture's saturated throughput was not measured in this run.

## ESP32-S3

The November 17 firmware matrix verified eight identity, eight 7-byte and eight 2,000-byte reads per cell. The 2,000-byte read used two exchanges. Median small / blockwise latency was 10.05 / 23.03 ms for sync unprotected, 8.20 / 22.20 ms for async unprotected, 15.79 / 41.24 ms for sync established OSCORE and 17.66 / 45.20 ms for async established OSCORE. These short cohorts establish end-to-end timing for the tested reads. Saturated throughput and client process CPU were unavailable. [Physical workload and capacities](ESP-WIFI-RELIABILITY-20261117.md).

On the November 18 instrumented async protected build, 24 reads matched 32 device exchanges and 350 ledger rows. For small reads the worker used 14.16 ms of CPU in a 15.04 ms device interval, and the host observed 18.60 ms end to end. For blockwise reads the worker used 37.02 ms across 43.23 ms, and the host observed 50.19 ms. Device timing starts at observed readiness, with one receive-return fallback, and ends at the final send return. The traffic sample showed 40,932 bytes free internal heap and a 62,320-byte free-stack watermark. The diagnostic ledger used 28,672 bytes. [Device timing boundaries and memory](ESP-DEVICE-RECOVERY-20261118.md).

The November 9 unprotected development comparison also reproduced all 1,152 reads across 12 boots. The integrated sync Rust and pinned libcoap firmwares took 3.51 / 3.05 ms on small reads and 10.47 / 10.36 ms on 2,000-byte reads with the same async client. These were 32 KiB instruction-cache development builds. The later 16 KiB firmware matrix is a separate experiment.
