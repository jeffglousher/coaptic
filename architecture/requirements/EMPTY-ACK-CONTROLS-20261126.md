# Empty ACKs on Windows, including the original Coaptic baseline

The [first matched study](MATCHED-ACK-20261125.md) compared established OSCORE directly with stock libcoap. This follow-up measures separate unprotected responses on both libraries, checks a proposed Rust change and compares current Coaptic with the original project on the same four-datagram exchange.

## Unprotected libcoap and Coaptic

The stock C service uses libcoap’s retained large-response API, which can answer follow-up Block2 GETs before the resource handler. Simply deferring the first response produced a separate first block and three piggybacked follow-ups. The opt-in C handler uses libcoap’s public block-selection functions on every request. Its separate mode registers and triggers a native asynchronous response. Both empty ACKs and the tracked response CON travel through the library. A piggyback arm uses exactly the same block handler. A third C arm measures the unchanged stock behavior in the same executable. The two Coaptic arms use its existing response option.

Ten alternating Latin-square rounds per body size and load balanced the position of all five arms. Each timed read verified the full body and each block’s wire shape. The same native client and bounded Coaptic service ran on Windows loopback with one outstanding Call, a 32-read warmup and the same spin setting. Private bytes and working set are end-of-window per-process readings. See the JSON for raw-window digests, per-role cycles, peak working set and individual pairs. The live loss test saw the exact 1,049-byte response retransmitted after a withheld final ACK, then verified refusal of out-of-range block 4 and reuse for a complete read. [Reference fixture and reproduction commands](https://github.com/jeffglousher/coaptic-validation/pull/280)

### At capacity

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | libcoap stock piggyback | 12.70 | 17.58 | 17.4 / 30.2 | 56,483 | 0.93 / 0.92 | 5.40 / 5.05 |
| 64 | libcoap handler piggyback | 12.21 | 17.58 | 16.8 / 27.2 | 57,500 | 0.93 / 0.92 | 5.38 / 5.05 |
| 64 | libcoap handler separate | 21.48 | 29.30 | 28.8 / 46.9 | 34,425 | 0.93 / 0.92 | 5.39 / 5.08 |
| 64 | Coaptic piggyback | 16.60 | 16.60 | 15.6 / 24.1 | 61,573 | 1.18 / 0.92 | 6.09 / 5.06 |
| 64 | Coaptic separate | 24.41 | 24.41 | 22.9 / 37.0 | 41,864 | 1.18 / 0.92 | 6.14 / 5.08 |
| 4,096 | libcoap stock piggyback | 55.49 | 73.24 | 71.3 / 108.3 | 13,683 | 0.96 / 0.86 | 5.43 / 5.02 |
| 4,096 | libcoap handler piggyback | 57.71 | 71.02 | 69.7 / 104.0 | 13,966 | 0.95 / 0.86 | 5.41 / 5.02 |
| 4,096 | libcoap handler separate | 95.44 | 113.19 | 109.5 / 170.2 | 8,926 | 0.96 / 0.86 | 5.43 / 5.05 |
| 4,096 | Coaptic piggyback | 67.69 | 67.69 | 65.8 / 108.6 | 14,633 | 1.19 / 0.86 | 6.12 / 5.02 |
| 4,096 | Coaptic separate | 99.88 | 99.88 | 97.7 / 144.2 | 9,955 | 1.20 / 0.87 | 6.20 / 5.05 |

### At 800 offered reads/s

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | libcoap stock piggyback | 29.30 | 61.04 | 40.7 / 74.7 | 800 | 0.93 / 0.84 | 5.40 / 4.98 |
| 64 | libcoap handler piggyback | 26.86 | 43.95 | 40.0 / 75.0 | 800 | 0.93 / 0.83 | 5.38 / 4.97 |
| 64 | libcoap handler separate | 31.74 | 73.24 | 57.4 / 105.1 | 800 | 0.94 / 0.83 | 5.39 / 5.00 |
| 64 | Coaptic piggyback | 36.62 | 51.27 | 43.1 / 77.8 | 800 | 1.18 / 0.83 | 6.09 / 4.97 |
| 64 | Coaptic separate | 68.36 | 90.33 | 60.1 / 107.8 | 800 | 1.19 / 0.83 | 6.15 / 4.99 |
| 4,096 | libcoap stock piggyback | 80.57 | 126.95 | 114.7 / 207.9 | 800 | 0.96 / 0.84 | 5.43 / 5.00 |
| 4,096 | libcoap handler piggyback | 100.10 | 141.60 | 115.0 / 211.8 | 800 | 0.95 / 0.85 | 5.41 / 5.01 |
| 4,096 | libcoap handler separate | 134.28 | 168.46 | 154.4 / 298.6 | 800 | 0.96 / 0.84 | 5.43 / 5.03 |
| 4,096 | Coaptic piggyback | 114.75 | 131.84 | 110.1 / 267.1 | 800 | 1.19 / 0.85 | 6.12 / 5.00 |
| 4,096 | Coaptic separate | 141.60 | 161.13 | 144.9 / 344.1 | 800 | 1.21 / 0.84 | 6.21 / 5.03 |

### Cost of both acknowledgements

Two paced 64-byte C windows charged zero server CPU despite completing all 3,200 operations and recording nonzero cycles. Both raw measurements are included. Percentage changes with a zero denominator are undefined.

For 64 bytes, switching to a separate response added 9.28 µs of server CPU per read on libcoap’s matched handlers and 7.81 µs on Coaptic. Directly against separate libcoap, Coaptic’s median paired server CPU difference was +11.2% and its median paired throughput difference was +21.7%. Coaptic used less server CPU in 1 of ten pairs.

For 4,096 bytes, switching to a separate response added 37.73 µs of server CPU per read on libcoap’s matched handlers and 32.18 µs on Coaptic. Directly against separate libcoap, Coaptic’s median paired server CPU difference was +7.0% and its median paired throughput difference was +12.4%. Coaptic used less server CPU in 1 of ten pairs.

### Where the server thread spends time

Coaptic had 736 socket-send stacks among 3,047 wall samples on paced separate 4 KiB reads. The independent scheduler capture observed 18 running and 193 waiting. Full stack collection moved p99 from 344 µs to 7.42 ms. libcoap had 1,543 socket-send stacks among 4,501 wall samples on paced separate 4 KiB reads. The independent scheduler capture observed 16 running and 190 waiting. Full stack collection moved p99 from 299 µs to 8.04 ms. The process CPU figures come from profiler-off runs. A suspended thread's stack shows where it was observed, including time blocked in Windows. The separate state samples cannot assign exclusive execution time to an individual frame.

### Kernel and user CPU

Windows process accounting distinguishes time executing in user code from kernel execution on behalf of the process. Socket-readiness and scheduler waits consume wall time. For small unprotected separate reads, pooling ten capacity windows gave Coaptic **6.25 µs user + 17.77 µs kernel** per verified read, against libcoap **10.06 µs user + 11.43 µs kernel**. Coaptic used more kernel CPU in all ten pairs. For 4 KiB separate reads the corresponding values were 32.85 + 67.69 µs and 32.40 + 62.59 µs. The small default piggyback comparison also charged more kernel time to Coaptic in all ten pairs. These pooled arithmetic means add by construction. The tables above use medians of windows.

A separate 96-window experiment alternated process I/O counter snapshots on and off across 1,105,920 verified reads. For 64-byte piggyback reads, Windows counted 2.010 other I/O operations per read for Coaptic and 1.000 for the explicit C handler. With separate responses the counts were 2.071 and 2.000. At 4 KiB, piggyback counts were 8.019 and 4.000 per read, and separate counts were 8.190 and 8.000. These process-wide totals include operations such as control requests and do not identify individual Winsock calls. The small separate CPU gap survived with nearly equal operation counts. A Python counter query cost 3.4 µs median and 4.1 µs p99 in 257 isolated calls. Each endpoint was queried twice per window. Median paired server cycle shifts between observation on and off ranged from −0.81% to +1.35%. Small and large separate Coaptic p99 shifted about 19–20% across these short windows, so performance decisions use the profiler-off study. [Windows process I/O counters](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-io_counters)

## Current Coaptic against the original project

The original library revision is b8f44fa. The current library is b3a786e. The companion server source, release profile, Tokio/Mio runtime, service capacities, authentication settings, verifying client and separate response mode match. The original manifest pins AES 0.9.3 and its build-time macro uses Syn 2. The current manifest pins AES 0.9.4 and its macro uses Syn 3. These dependencies follow their respective library revisions. All other registry versions in the two native locks match. The capacity cohort balanced the six possible orders of the original, current and a second build of the same current source. Six paced pairs alternated current and original. Debug symbols verified checksums for 55 linked core source files against Git in each of the three release executables. The independently rebuilt current executable had the same code section length and a different linked layout. It measures build-to-build variation on unchanged source.

### At capacity

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | Unprotected original | 26.37 | 26.37 | 24.9 / 51.1 | 37,910 | 1.23 / 0.92 | 6.22 / 5.08 |
| 64 | Unprotected current | 23.44 | 23.93 | 22.9 / 37.7 | 41,926 | 1.18 / 0.92 | 6.13 / 5.08 |
| 4,096 | Unprotected original | 106.53 | 106.53 | 105.1 / 145.3 | 9,305 | 1.24 / 0.86 | 6.29 / 5.04 |
| 4,096 | Unprotected current | 98.77 | 98.77 | 96.3 / 131.2 | 10,178 | 1.20 / 0.86 | 6.20 / 5.04 |
| 64 | Established OSCORE original | 28.32 | 27.83 | 26.6 / 50.8 | 35,573 | 1.22 / 0.92 | 6.31 / 5.17 |
| 64 | Established OSCORE current | 27.34 | 27.83 | 27.8 / 36.9 | 36,278 | 1.18 / 0.92 | 6.23 / 5.17 |
| 4,096 | Established OSCORE original | 127.62 | 126.51 | 121.2 / 210.6 | 7,886 | 1.24 / 0.86 | 6.39 / 5.13 |
| 4,096 | Established OSCORE current | 119.85 | 122.07 | 117.8 / 179.3 | 8,215 | 1.21 / 0.87 | 6.31 / 5.13 |

### At 800 offered reads/s

| Body | Server | Server CPU µs | Client CPU µs | Median / p99 µs | Reads/s | Server / client private MiB | Server / client working set MiB |
|---|---|---:|---:|---:|---:|---:|---:|
| 64 | Unprotected original | 80.57 | 80.57 | 61.4 / 107.8 | 800 | 1.24 / 0.83 | 6.23 / 5.00 |
| 64 | Unprotected current | 34.18 | 29.30 | 59.7 / 105.3 | 800 | 1.19 / 0.83 | 6.14 / 4.99 |
| 4,096 | Unprotected original | 153.81 | 173.34 | 151.3 / 347.2 | 800 | 1.25 / 0.84 | 6.30 / 5.03 |
| 4,096 | Unprotected current | 139.16 | 139.16 | 142.1 / 337.9 | 800 | 1.21 / 0.84 | 6.21 / 5.02 |
| 64 | Established OSCORE original | 73.24 | 80.57 | 65.5 / 117.0 | 800 | 1.24 / 0.82 | 6.32 / 5.08 |
| 64 | Established OSCORE current | 61.04 | 78.12 | 64.5 / 113.8 | 800 | 1.19 / 0.82 | 6.25 / 5.08 |
| 4,096 | Established OSCORE original | 197.75 | 209.96 | 171.6 / 377.3 | 800 | 1.25 / 0.84 | 6.39 / 5.11 |
| 4,096 | Established OSCORE current | 183.11 | 209.96 | 167.6 / 373.6 | 800 | 1.21 / 0.84 | 6.31 / 5.11 |

For unprotected reads, median paired server CPU differences for current versus original at capacity were -9.4% and -8.2% at 64 B and 4 KiB. At 800/s the corresponding differences were -56.7% and -14.2%.

For established OSCORE reads, median paired server CPU differences for current versus original at capacity were -1.7% and -6.1% at 64 B and 4 KiB. At 800/s the corresponding differences were -4.0% and -5.8%.

The small unprotected paced server CPU result was 56.7% lower, while measured process cycles fell 4.58% and median latency fell 4.08%. At capacity the same workload used 9.4% less charged server CPU. An unchanged-source rebuild raised server cycles by 2.67% on all six unprotected blockwise capacity pairs. These controls put changes of a few percent in context. Protected small-read median latency rose 4.30% against the historical original in all six capacity pairs even as throughput improved.

The open project target is a 50% reduction in server CPU time per verified operation in each security mode. The paired cycle counters and repeated runs qualify changes in Windows’ coarsely charged process CPU. Individual outliers cannot establish either target.

## Sharing one encoded ACK between transport and duplicate replay

The private Rust trial constructed the four control bytes once, kept the duplicate-record ordering and passed those bytes to the send path. It preserved short-slot refusal and reuse. All 1,257 all-feature library tests and 749 no-default tests passed. Six paired rotations completed 1,105,920 verified reads. Small unprotected separate responses gained 2.58% in median paired server cycles. Blockwise unprotected separate cycles moved only −0.07% and p99 latency rose 16.02%. Protected separate server cycles differed by −0.18% at 64 bytes and +0.27% at 4 KiB, with three of six pairs improving for each. Small protected median latency rose 8.26%. The release code section grew by 83,008 bytes. This version was rejected.

## Skipping empty ownership tables on a final ACK

A second private trial used existing Exchange and Observe occupancy to skip empty tables. With a single pending CON and no other owners, an instrumented storage backend counted reads in three selected owner-table walks falling from 11 to 5 on Constrained, 35 to 11 on Gateway and 131 to 35 on Service. The probe omitted the post-match client exchange lookup and the additional RST cleanup walks. Its counts cover the selected path only. A custom Observe backend whose row storage is independent of its pool also needs a conservative full-scan default. Receipt of an ACK or RST still required an eligible owner in the tested built-in backends. The changes passed unsent-reservation, detached-exchange, namespace-collision and reuse tests.

The end-to-end experiment used four or 32 TX and Observe slots, with every other capacity fixed between builds. Both servers passed 256 independent wire reads in total. Six alternating pairs for each security, body, response mode and capacity completed another 2,211,840 verified reads. At four slots, separate 64-byte unprotected reads used 3.04% fewer server cycles and completed 3.47% more reads/s, with improved cycles and throughput in all six pairs. Small protected server CPU increased 3.45%. At 32 slots, separate protected 4 KiB server cycles rose 0.83% in all six pairs and paired throughput fell 0.69%. Both unprotected blockwise and protected blockwise separate server cycles also rose at the larger capacity. The expected additional saving when increasing capacity did not repeat. Increasing TX capacity also lengthened the pending-CON scan in both builds. This specific capacity interaction lacked a same-source rebuild control. This change was rejected. The JSON includes all raw-window digests and the paired capacity interaction relative to piggyback controls.

In the unprotected paced Coaptic trace, 12 of 3,047 wall stacks contained control_route, while 736 contained socket send and 1,015 contained the completion-port wait. Stack samples can miss inlined work. Along with charged kernel time at capacity, these observations put the next Windows investigation at the transport boundary. Count actual receive submissions, readiness completions and sends per verified datagram in each adapter. On ESP32-S3 the available evidence still points to a fixed worker-CPU excess per datagram. The staged poll and send-entry probes need a physical run to locate that cost.

## Reproducing the protected C allocation control

The [primary profile](MATCHED-ACK-20261125.md) led to a one-allocation counterfactual in libcoap revision 851533c3. In coap_oscore_new_pdu_encrypted_lkd the experiment checked that plain_pdu->used_size fit within OSCORE_CRYPTO_BUFFER_SIZE minus AES_CCM_TAG, then passed plain_pdu->used_size + AES_CCM_TAG as the allocation size. The original ciphertext bound and encryption call stayed intact. At 4 KiB this reduced four transient 8,388,880-byte requests to the space required for four 1,049-byte ciphertexts. This is a comparison with a private experimental peer. The stock peer and all Coaptic deployment defaults retain their original behavior.

The unprotected five-arm rotation verified 1,472,000 reads. The original-project and rebuild controls verified 983,040. [Window-level metrics and binary identities](EMPTY-ACK-CONTROLS-20261126.json)

## Following the final ACK through dispatch

The earlier counter stopped after the pending match. Following a separate response's final ACK through async dispatch found another exchange lookup in the Q-Block continuation. A private Rust trial passed that lookup's result to the cancellation guard and continuation, and let the fixed and allocated backends skip empty Exchange and Observe tables. Custom storage retained the full scan by default.

With one sent CON and both typed tables empty, the selected pending, Exchange and Observe accesses during complete ACK dispatch fell from 17 to 7 on Constrained, 53 to 13 on Gateway, and 197 to 37 on Service. Reset counts fell from 23 to 7, 71 to 13, and 263 to 37. On Service, the rest of the progress pass still read 96 Observe rows. Putting a live Exchange and Observe row in the last slots reduced the Service ACK's exchange reads from 96 to 64. The corresponding unrelated reset still made the full scans. The custom-storage control counted 96 Exchange and 128 Observe reads for an empty Service reset.

The state checks covered superseded notifications and deferred owners as well as ordinary reads. Two instrumented companion executables saw zero typed Exchange and Observe rows before and after all 3,000 ingress progress passes. Across those passes 1,000 final ACKs released their matching pending CON. Both the independent wire verifier and the frozen native timing client exercised the two security modes, body sizes, response modes and table sizes. The timed binaries recorded configuration and verified packets. Occupancy was measured in the separate calibration processes.

Six permutations of the original, an independent rebuild of the same source, and the trial produced 288 profiler-off capacity windows and 3,317,760 verified reads. Each permutation covered piggyback and separate responses, 64 B and 4 KiB, both security modes and 4 or 32 TX and Observe slots. The PDBs matched the linked core sources. There was no compiler overlap. The [numeric receipt](EMPTY-ACK-CONTROLS-20261126.json) includes the build identities, individual pairs, per-process CPU and memory, capacity interaction and the occupancy calibrations.

The unprotected 4 KiB separate read improved against both originals in all six server-cycle pairs at both capacities. At four slots, the trial used 3.10% fewer cycles against the first build and 3.54% fewer against the rebuild. Server CPU went from 99.88 to 96.55 µs/read and client CPU from 100.99 to 97.66. Median/p99 latency went from 98.0/175.1 to 95.35/138.45 µs and capacity rose from 9,876 to 10,206 reads/s. The server and client working sets stayed at 6.21 and 5.05 MiB.

The 32-slot protected small read regressed against both originals. Median latency rose from 25.35 to 27.45 µs, with the trial slower in all six pairs against each build. Capacity fell from 37,185 to 36,669 reads/s. Server cycles rose 1.47% against the first build and 1.73% against the rebuild. Removing empty-table visits saved measurable work on large unprotected reads. The protected regression ruled out this implementation as a general optimization.

The reset tests also found a reliability defect in the original two-slot App. After accepting an empty ACK it could reuse the original request's datagram slot to serve a separate response. A reset for that response could incorrectly fail the still-outstanding Call. The merged correction matches control and retry failures by Message ID and routed endpoint, and rolls back a refused continuation by the key returned at insertion. The original Call now receives its own reply even if both directions chose the same token. [Call lifetime fix and regression tests](https://github.com/jeffglousher/coaptic/pull/807)

## What wakes the Windows I/O driver

In the 4 KiB separate-response traces, all 103 unprotected and all 127 protected wall samples containing `PostQueuedCompletionStatus` came through the Tokio deadline timer polling `Sleep` and waking the I/O driver. Of these, 91 and 100 respectively occurred inside the async datagram send. The Mio socket-readiness update appeared in another 91 and 155 stacks respectively. These inclusive stack counts locate two concrete paths. The traces had 3,047 and 3,081 wall samples, including 1,015 and 240 in the completion-port wait. An earlier scoped probe observed one progress deadline timer per successful send. Separating timer registration from socket readiness is the next Windows transport experiment. It must preserve refusal when a deadline and a send are ready together. [Earlier deadline-poll measurements](WINDOWS-TOKIO-TIMER-POLLS-20261120.md)

## ESP32-S3 availability

A read-only check reached the board's management registry. All nine Waveshare ESPHome sensors were unavailable, including uptime and address. The address history returned only its previously recorded address and unavailability, with the last transition to unavailable on November 25 at 13:53 UTC. Windows also reported zero serial ports. A physical separate-response comparison still requires the board to reconnect.
