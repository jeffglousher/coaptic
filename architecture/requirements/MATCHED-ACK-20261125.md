# Matched empty-ACK reads against libcoap

With both empty ACKs counted, Coaptic used less server CPU than stock libcoap for protected reads at Windows capacity. Sizing a single libcoap allocation to the ciphertext actually produced eliminated most of that gap. The Rust borrowing trials removed a full-body copy and still slowed other reads. Both were rejected.

Windows used the same release client, loopback socket workload, warmup, credential derivation, full-body verification and single outstanding CON Call for each peer. Six order-balanced rounds covered 1,067,520 verified reads. The Coaptic server used four RX and four TX datagrams of 1,472 bytes, eight duplicate entries, four Observe entries, one RX body, two TX bodies, one installed route, four Calls and zero deferred replies. The native C fixture used the same service and dynamic libcoap storage. The 64-byte representation fit one block. The 4,096-byte representation took four. Every measured window completed without failure, cancellation or skipped work.

The default unprotected peers both piggyback. Stock libcoap separates each protected response. Opting Coaptic into separate CON delivery makes both protected peers send the request, the empty ACK for that request, the authenticated response and the empty ACK for that response. That is four UDP datagrams per block and sixteen per 4 KiB read. Each control ACK is four bytes. Both protected fixtures used AES-CCM-16-64-128 and a 32-entry replay window. A fresh libcoap context required one authenticated Echo challenge before warmup. The same client verified both peers’ fixture ETag and contents. Libcoap also sent optional Size2=4096, three encoded bytes per large block.

## Windows at capacity

Each entry is the median of six per-process windows. Latencies include complete body verification. Server and client CPU are charged microseconds per complete read. Capacity is closed loop. Working set and private bytes are sampled at the end of the same process window. The JSON includes every repeat, cycles, peak working set and distribution tails.

| Security | Bytes | Server and response | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---:|---|---:|---:|---:|---:|---:|---:|
| Unprotected | 64 | libcoap piggyback | 12.21 | 18.07 | 17.8 / 28.3 | 54,319 | 0.93 / 0.92 | 5.38 / 5.05 |
| Unprotected | 64 | Coaptic piggyback | 16.11 | 16.11 | 15.9 / 27.3 | 60,509 | 1.18 / 0.92 | 6.10 / 5.06 |
| Unprotected | 4,096 | libcoap piggyback | 59.93 | 76.57 | 73.3 / 124.0 | 13,040 | 0.97 / 0.86 | 5.43 / 5.02 |
| Unprotected | 4,096 | Coaptic piggyback | 68.80 | 68.80 | 66.9 / 103.5 | 14,526 | 1.19 / 0.86 | 6.12 / 5.02 |
| OSCORE | 64 | libcoap separate | 50.78 | 58.59 | 56.8 / 99.5 | 16,811 | 0.97 / 0.92 | 5.55 / 5.17 |
| OSCORE | 64 | Coaptic separate | 28.32 | 28.32 | 28.4 / 48.8 | 35,061 | 1.18 / 0.92 | 6.23 / 5.17 |
| OSCORE | 64 | Coaptic piggyback | 18.07 | 18.07 | 17.4 / 26.4 | 55,313 | 1.18 / 0.92 | 6.20 / 5.15 |
| OSCORE | 4,096 | libcoap separate | 229.71 | 257.46 | 249.4 / 360.1 | 3,873 | 0.98 / 0.86 | 5.57 / 5.13 |
| OSCORE | 4,096 | Coaptic separate | 122.07 | 122.07 | 118.2 / 215.8 | 8,177 | 1.20 / 0.87 | 6.30 / 5.13 |
| OSCORE | 4,096 | Coaptic piggyback | 85.45 | 86.56 | 86.3 / 123.6 | 11,394 | 1.20 / 0.86 | 6.24 / 5.11 |

The empty ACKs account for part of Coaptic’s protected advantage in the earlier comparison. Matching them increased Coaptic server CPU from 18.07 to 28.32 µs for the small read and from 85.45 to 122.07 µs for the large one. The stock C server took 50.78 and 229.71 µs. This adds 10.25 µs per small read and 36.62 µs per large read in this configuration, including protocol bookkeeping and both additional sends. In unprotected mode Coaptic consumed more server CPU despite completing more reads per second. Client work and overlap affect the complete-read rate.

## Windows at 800 offered reads per second

The same endpoints paced absolute starts with the native client in spin mode. Windows charges CPU in coarse increments. Process cycles and paired longer windows help interpret differences. The end-of-window gate follows the client’s final ACK send. The server could process that last datagram after the CPU sample. All preceding exchanges and warmup are accounted for at their respective gates. The elapsed wall time includes idle intervals. Throughput near the offered rate establishes that the service kept up.

| Security | Bytes | Server and response | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---:|---|---:|---:|---:|---:|---:|---:|
| Unprotected | 64 | libcoap piggyback | 19.53 | 39.06 | 41.0 / 72.7 | 800 | 0.94 / 0.83 | 5.38 / 4.97 |
| Unprotected | 64 | Coaptic piggyback | 61.04 | 68.36 | 44.5 / 77.7 | 800 | 1.18 / 0.83 | 6.09 / 4.97 |
| Unprotected | 4,096 | libcoap piggyback | 90.33 | 129.39 | 119.7 / 212.3 | 800 | 0.96 / 0.84 | 5.43 / 5.00 |
| Unprotected | 4,096 | Coaptic piggyback | 124.51 | 136.72 | 112.2 / 307.9 | 800 | 1.20 / 0.84 | 6.13 / 5.00 |
| OSCORE | 64 | libcoap separate | 70.80 | 90.33 | 101.8 / 164.1 | 800 | 0.97 / 0.83 | 5.55 / 5.08 |
| OSCORE | 64 | Coaptic separate | 78.12 | 85.45 | 67.8 / 115.7 | 800 | 1.19 / 0.83 | 6.24 / 5.08 |
| OSCORE | 64 | Coaptic piggyback | 29.30 | 61.04 | 50.5 / 84.5 | 800 | 1.18 / 0.82 | 6.20 / 5.05 |
| OSCORE | 4,096 | libcoap separate | 292.97 | 339.36 | 324.6 / 574.0 | 800 | 0.99 / 0.84 | 5.57 / 5.11 |
| OSCORE | 4,096 | Coaptic separate | 185.55 | 200.20 | 170.9 / 377.5 | 800 | 1.20 / 0.84 | 6.31 / 5.11 |
| OSCORE | 4,096 | Coaptic piggyback | 126.95 | 161.13 | 132.8 / 304.6 | 800 | 1.20 / 0.83 | 6.23 / 5.09 |

## Process-scoped profile

The stack tracer suspended only the server thread in a separate paced 4 KiB OSCORE run. A second run sampled the same thread’s scheduler state. Inclusive stacks overlap. Neither a socket stack nor a reserved stack frame measures active CPU by itself. Profiler-off Windows process CPU and cycles supply the execution totals.

Coaptic’s separate path included 687 empty-ACK send stacks and 797 classic block-issue stacks among 3,081 wall samples. The most frequent path crossed Tokio, Mio and Winsock sendto. The scheduler sampler recorded 174 waits, 38 running observations and one unknown. The traced p99 rose from 378 µs to 9.76 ms. The borrow trial removed the old classic-issue stack and retained the socket sends. Its sampler recorded 178 waits, 27 running, two standby and one unknown.

Libcoap showed 635 select stacks, 842 WSASendMsg stacks and 635 NtFreeVirtualMemory stacks among 3,755 observations. Scheduler states counted 163 waits, 44 running and four standby. The traced p99 rose from 574 µs to 12.80 ms. Relinking the same C objects with a map produced identical executable .text. The allocation and free call sites account for 918 of 3,755 inclusive wall samples. The sampled free return address maps to coap_oscore_new_pdu_encrypted_lkd. The binary requests 8,388,880 bytes of ciphertext scratch and passes plaintext length plus eight as the encryption output limit. Per-process working-set snapshots do not capture the size of this transient reservation.

## Why stock libcoap spends more CPU on OSCORE

The trace located a temporary encryption buffer in libcoap. On this Windows build the code requests **8,388,880 bytes for each protected response**. The encryption call on the next lines receives the actual allowable output length. That output was 84 bytes for the small reply and 1,049 bytes for each large block. The four-block read therefore requested 33,555,520 bytes of temporary storage to produce 4,196 ciphertext bytes. These figures describe allocation requests. They do not measure resident memory or bytes copied.

An isolated C build changed only the requested scratch size to plaintext length plus the same authentication tag, with a guard retaining the existing upper bound. The unmodified fixture and a clean stock relink had byte-identical executable code. All authenticated wire checks passed on the right-sized build. Both peers retained the original representation, optional Size2, wire lengths, authenticated Echo setup and replay policy. The stock benchmark and upstream libcoap were left intact.

| Body | C build | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client private MiB |
|---|---|---:|---:|---:|---:|---:|
| 64 | stock | 55.18 | 61.04 | 58.2 / 99.8 | 16,453 | 0.97 / 0.92 |
| 64 | right-sized | 27.83 | 36.13 | 34.9 / 63.0 | 27,690 | 0.96 / 0.92 |
| 4,096 | stock | 239.70 | 265.23 | 254.7 / 394.0 | 3,745 | 0.98 / 0.87 |
| 4,096 | right-sized | 128.73 | 154.25 | 145.8 / 246.5 | 6,526 | 0.98 / 0.86 |

All six pairs improved at both sizes on server CPU, client CPU, process cycles, latency and throughput. Median paired server CPU fell 48.5% small and 47.0% large. Each peer used the same single spinning client. A single-job ESP firmware build ran during this separate cohort. The JSON retains every pair. This result isolates a Windows libcoap allocation cost on established OSCORE. The physical C firmware in the ESP comparison serves an unprotected endpoint.

### Direct comparison after sizing the C scratch buffer

A separate 414,720-read rotation put all three servers in all six possible execution orders. Every arm used the four-datagram exchange and the same client executable. The first round overlapped the end of an offline SDK build. The five subsequent rounds gave the same interpretation.

| Body | Server | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | Coaptic separate | 28.32 | 28.32 | 28.45 / 50.35 | 35,568 | 1.17 / 0.92 | 6.23 / 5.17 |
| 64 | Stock libcoap | 55.66 | 60.55 | 58.50 / 98.05 | 16,507 | 0.97 / 0.92 | 5.55 / 5.17 |
| 64 | libcoap scratch experiment | 29.79 | 35.64 | 34.60 / 59.70 | 28,153 | 0.97 / 0.93 | 5.55 / 5.17 |
| 4,096 | Coaptic separate | 122.07 | 122.07 | 118.95 / 194.45 | 8,193 | 1.20 / 0.87 | 6.31 / 5.13 |
| 4,096 | Stock libcoap | 240.81 | 260.79 | 255.70 / 371.30 | 3,810 | 0.99 / 0.86 | 5.58 / 5.13 |
| 4,096 | libcoap scratch experiment | 117.63 | 148.70 | 143.45 / 219.55 | 6,759 | 0.98 / 0.87 | 5.57 / 5.13 |

Coaptic took less server CPU than the experimental C peer in four of six small-read pairs and two of six blockwise pairs. Server cycles likewise gave no consistent winner. Coaptic completed more verified reads per second in every pair, with median paired advantages of 24.5% small and 20.7% blockwise. Median latency and spinning-client CPU both fell in every pair. C used less steady server memory. The apparent roughly twofold stock server-CPU advantage largely disappears when the C peer’s temporary allocation is sized to its actual work.

## Rust changes tested and rejected

The separate-response size fallback stages the complete body on each independently requested classic block. A 4,096-byte read stages 16 KiB across four requests. The first trial admitted against the same bounded pool, borrowed the selected slice and retained the encoded response CON for retransmission. A separate diagnostics test used a 2,000-byte representation and counted the snapshot falling from 2,000 bytes to zero. All Windows blockwise timing rows below used 4,096 bytes. The inline App and loop future retained their original sizes of 15,928 and 11,816 bytes.

At capacity the first trial reduced unprotected separate server CPU from 102.10 to 97.66 µs. In eight longer 800/s pairs, server CPU rose by a median paired 0.61%, with four pairs improving. Server cycles fell 1.73%. On the ordinary protected piggyback control, median latency rose 1.89% and p99 rose 5.05% at capacity. This version changed a shared helper and was rejected.

A second trial moved borrowing entirely into the separate-response size fallback. It retained the existing pending CON, its sampled retry interval and ciphertext. Six alternating pairs covered each of eight capacity and three paced configurations, a further 1,297,920 verified reads. For the 4 KiB separate response, the medians by build were:

| Load | Security | Build | Server CPU µs | Client CPU µs | p50 / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---|---:|---:|---:|---:|---:|---:|
| capacity | unprotected | Original | 99.88 | 99.88 | 98.5 / 138.3 | 9,948 | 1.19 / 0.86 | 6.19 / 5.05 |
| capacity | unprotected | Trial | 97.66 | 97.66 | 94.7 / 145.7 | 10,288 | 1.19 / 0.87 | 6.17 / 5.04 |
| capacity | oscore | Original | 118.74 | 119.85 | 116.3 / 160.0 | 8,439 | 1.20 / 0.86 | 6.30 / 5.13 |
| capacity | oscore | Trial | 120.96 | 122.07 | 119.8 / 172.8 | 8,193 | 1.19 / 0.86 | 6.28 / 5.13 |
| paced | unprotected | Original | 179.44 | 192.87 | 184.9 / 320.1 | 800 | 1.21 / 0.87 | 6.21 / 5.05 |
| paced | unprotected | Trial | 177.00 | 189.21 | 180.5 / 334.1 | 800 | 1.20 / 0.87 | 6.18 / 5.05 |
| paced | oscore | Original | 241.70 | 257.57 | 239.6 / 371.9 | 800 | 1.20 / 0.86 | 6.30 / 5.13 |
| paced | oscore | Trial | 235.60 | 268.55 | 236.4 / 365.6 | 800 | 1.20 / 0.86 | 6.28 / 5.13 |

The narrower trial improved unprotected capacity. Median paired server cycles fell 3.78% and latency fell 4.20%, each in all six pairs. Protected 4 KiB capacity went the other way. Median latency rose in all six pairs, by 2.03% paired. Server cycles rose 2.00% paired and throughput fell 1.94%. At 800/s protected separate latency fell 1.54% paired, while unprotected separate p99 rose 7.20% paired. This trial was also rejected. The regression test was kept. It covers bounded refusal and reuse, a full ETag-bearing final block, exact duplicate empty ACK, lost response ACK, byte-identical retransmission, and release on acknowledgement.

The narrower trial's process-scoped trace found 0 old classic-issue stacks, 627 empty-ACK send stacks and 830 socket-send stacks among 3,161 wall stacks. The separate scheduler capture counted 26 running observations and 182 waits. Stack collection moved p99 from 366 µs to 9.36 ms. Decisions use the profiler-off pairs.

A separate endpoint-store trial improved 7-byte fixed-pool reads by 6.61% and 4.64% in two independent 30-pair cohorts. The 2,000-byte reads slowed by 1.35% and 3.60%. That experiment was removed.

The retained source passed 1,720 workspace tests with all features, 749 no-default-feature library tests, workspace formatting and all-target Clippy with warnings denied. The new regression runs on the original library implementation.

## ESP32-S3 and original project targets

Across eight earlier ESP boots, 192 verified unprotected reads consisted of 256 request/response pairs with zero empty ACKs. The previous Coaptic worker used 2.86 ms for seven bytes versus libcoap’s 1.59 ms. On the first and following 2,000-byte body blocks the gaps were 1.28 and 1.38 ms. The nearly constant excess spans receive return through send return. Reviews by Grok and Codex independently recommended measuring the cooperative wake and poll entry before assigning the gap to parsing or copying. Previous pre-send phase measurements came from an earlier Rust image.

The earlier ordinary 7-byte reads had median client latency of 5.85 ms for Coaptic and 6.89 ms for libcoap. At 2,000 bytes the values were 17.67 and 16.16 ms. Mixed throughput was 91.7 and 91.1 verified reads/s. Coaptic reserved 192 KiB of worker stack versus 16 KiB for C. These are reservations, and the heap samples had different collection boundaries. The small host client CPU windows were quantized at approximately 651 µs per read per Windows tick.

The new ESP unprotected fixture can select its response shape by query. The host exercised both actual peer implementations through piggybacked and separate exchanges. Each completed 16 reads and 24 blocks. On a small separate read their UDP payloads measured 21, 4, 18 and 4 bytes. The first large block measured 24, 4, 1,047 and 4 bytes, and the last block measured 24, 4, 999 and 4. Each server retransmitted the same CON after a deliberately withheld ACK and served the next read. Both frozen public fixtures built and passed ESP32-S3 image and partition checks. The C firmware is 887,616 bytes and the Rust firmware 1,634,128 bytes. No new physical timing or probe-overhead number is available. The board did not respond at its recorded interface, serial and mDNS checks found no alternate connection, and no upload was attempted. Device libcoap OSCORE with the same durable service is still unqualified.

The 50% server-CPU targets against the original Coaptic release remain open independently for unprotected and established OSCORE. A peer comparison or a gain on one response mode cannot close either project target.

[Comparison harness](https://github.com/jeffglousher/coaptic-validation/pull/278) · [Windows process time](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes) · [Windows process cycle count](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryprocesscycletime) · [Per-window measurements and build identities](MATCHED-ACK-20261125.json) · [Previous peer comparison](LANDED-LIBCOAP-20261125.md)
