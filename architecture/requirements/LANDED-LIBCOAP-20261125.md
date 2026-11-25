# Coaptic and libcoap on Windows and the Waveshare

Coaptic on main at `32fdf57` serves the same unprotected bytes with fewer microseconds of client latency on Windows and more server CPU on both platforms. The ESP32-S3 incurs an additional 1.27–1.38 ms of worker CPU for each measured small response or Block2 exchange. That nearly fixed cost makes the receive-to-send path on the constrained device the next focus. [Landed implementation](https://github.com/jeffglousher/coaptic/pull/799)

## Windows at one outstanding confirmable read

Each pair is **Coaptic async / libcoap**. The same native client checked status, content format, the exact ETag and every body byte. The table uses the median of six profiler-off process windows per implementation and workload. Each round reversed the peer order from the previous round. Every cell started a fresh server and client on loopback. Timing started after association setup, a verified bootstrap read and 32 warmup reads.

### Serial capacity

| Workload | Server CPU µs/read | Client CPU µs/read | Median µs | p99 µs | Verified reads/s |
|---|---:|---:|---:|---:|---:|
| 64 B unprotected | 17.58 / 13.67 | 18.07 / 19.53 | 16.4 / 17.6 | 37.2 / 40.0 | 55,062 / 52,793 |
| 4 KiB unprotected | 74.35 / 68.80 | 75.46 / 85.45 | 68.6 / 78.7 | 159.4 / 168.9 | 13,016 / 11,676 |
| 64 B OSCORE | 21.00 / 57.62 | 22.46 / 62.99 | 17.6 / 59.3 | 57.5 / 115.2 | 44,858 / 15,767 |
| 4 KiB OSCORE | 88.78 / 244.14 | 91.00 / 265.23 | 85.5 / 252.1 | 162.6 / 427.0 | 11,086 / 3,776 |

### 800 offered reads per second

| Workload | Server CPU µs/read | Client CPU µs/read | Median µs | p99 µs | Verified reads/s |
|---|---:|---:|---:|---:|---:|
| 64 B unprotected | 65.92 / 39.06 | 65.92 / 73.24 | 50.8 / 52.3 | 90.2 / 91.7 | 800 / 800 |
| 4 KiB unprotected | 124.51 / 87.89 | 134.28 / 134.28 | 114.6 / 130.8 | 222.8 / 239.8 | 800 / 800 |
| 64 B OSCORE | 34.18 / 63.48 | 39.06 / 100.10 | 56.9 / 108.6 | 101.2 / 173.2 | 800 / 800 |
| 4 KiB OSCORE | 107.42 / 305.18 | 114.75 / 363.77 | 138.3 / 331.0 | 255.7 / 588.7 | 800 / 800 |

At capacity Coaptic used more server CPU in all six unprotected pairs at both sizes. It also used more process cycles in all twelve. The client finished sooner in five of six blockwise pairs and all six small-read pairs by median latency. The verifying client spins while a Call is live. Server and client CPU overlap in wall time. At 800/s all four combinations delivered the offered rate. Windows charges process time in coarse ticks, and pacing can change how a wakeup is charged. The public observations include the cycles and missed dispatch periods for every window.

### Memory at capacity

| Workload | Server working set MiB | Server private MiB | Client working set MiB | Client private MiB |
|---|---:|---:|---:|---:|
| 64 B unprotected | 6.04 / 5.39 | 1.17 / 0.93 | 5.02 / 5.02 | 0.92 / 0.92 |
| 4 KiB unprotected | 6.07 / 5.43 | 1.19 / 0.97 | 4.99 / 4.98 | 0.87 / 0.86 |
| 64 B OSCORE | 6.13 / 5.55 | 1.17 / 0.97 | 5.11 / 5.12 | 0.92 / 0.92 |
| 4 KiB OSCORE | 6.17 / 5.58 | 1.19 / 0.99 | 5.07 / 5.09 | 0.87 / 0.87 |

Memory is an end-of-window process reading. Both peers served one live association at a time. Coaptic used one route, four Calls, zero deferred jobs, eight duplicate rows, four security request bindings and four datagram slots per direction. The C service allocates its PDU and session storage dynamically. This compares complete service behavior at the offered load. Maximum resource capacity and allocator behavior differ.

### Why the protected ranking reverses

Coaptic piggybacked the protected representation on the request acknowledgement. Libcoap acknowledged the request, sent a separate confirmable response and received an acknowledgement for that response. Per 4 KiB read, the native client sent and received four datagrams with Coaptic, eight with libcoap. With either peer, an unprotected read required four in each direction. An independent authenticating client classified all packet roles across 256 further verified reads. Libcoap deliberately uses the separate response to retransmit it after loss. Coaptic supports recovery through request retry and duplicate replay. The timings include those wire and scheduling policies as well as cryptography. [Loss and recovery experiments](WINDOWS-PROTECTED-RESPONSE-RECOVERY-20261120.md)

### What the runtime control adds

The same Rust executable also supports synchronous App dispatch behind a Tokio socket-readiness loop. A second capacity experiment rotated all six orders of that mode, native async dispatch and libcoap. Both Rust modes use the same bounded service, representation, current-thread runtime, socket and scratch sizes. The readiness loop drains bounded bursts and uses an immediate send attempt. The async path supports waiting for send readiness and racing a send deadline. This is a diagnostic choice of service loop.

| Unprotected read | Native async µs CPU | Readiness loop µs CPU | libcoap µs CPU | Native async reads/s | Readiness loop reads/s | libcoap reads/s |
|---|---:|---:|---:|---:|---:|---:|
| 64 B unprotected | 16.60 | 14.65 | 13.67 | 62,040 | 66,790 | 55,316 |
| 4 KiB unprotected | 67.69 | 59.93 | 56.60 | 14,783 | 16,327 | 13,366 |

Changing the Rust dispatch mode lowered server CPU, lowered cycles and raised throughput in all six rounds at both sizes. The libcoap server still used fewer cycles than either Rust mode in every round. Host runtime overhead explains part of the Windows gap. The device comparison uses its own FreeRTOS driver.

## ESP32-S3 on the exact landed packet path

The landed library and examples reproduce the measured source diff `510df7f0`. The Rust task-clock image linked the same archive as the ordinary final firmware, verified by SHA-256. Its build receipt also contains a stale source label, retained in the earlier evidence. The reanalysis used the existing Waveshare observations, with two ordinary boots and two metered boots of each implementation. It matched every measured receive and send by message ID and wire length and reproduced every whole-read CPU total. No new firmware installation was needed. [Image identities, complete read results and restoration](PACKET-WORK-20261125.md)

| Exchange | Request / response wire B | Coaptic CPU ms | libcoap CPU ms | Extra CPU ms | Coaptic / libcoap ready ms |
|---|---:|---:|---:|---:|---:|
| 7-byte representation | 17 / 20 | 2.857 | 1.587 | 1.269 | 0.344 / 0.347 |
| 2,000 bytes, first block | 18 / 1040 | 3.151 | 1.873 | 1.278 | 0.347 / 0.230 |
| 2,000 bytes, follow-up | 20 / 992 | 3.308 | 1.931 | 1.378 | 0.344 / 0.339 |

Worker CPU covers receive return to send return. Median blocked time in those intervals was zero. Each row has 16 observations per peer across two boots. The Rust excess appeared on both boots and at both block positions. For a complete large read the workers used 6.53 / 3.82 ms, and ordinary client median latency was 17.67 / 16.16 ms. For seven bytes ordinary latency was 5.85 / 6.89 ms. Coaptic and libcoap served 91.7 / 91.1 mixed reads/s and reserved 192 / 16 KiB of worker stack. Host client CPU was 1,302 / 326 µs per mixed read, subject to a 651 µs/read clock tick in a 24-read window.

The ESP32 C fixture supports the unprotected head-to-head. It lacks an equivalent durable OSCORE endpoint. On Coaptic alone the established durable service used 14.14 ms of worker CPU for the small read and 34.78 ms for the large read. Windows protected ratios cannot be transferred to the device.

## Next focus

Measure and reduce the fixed cost from a successful receive to socket send entry on the ESP async server. The small reply and both blocks each carry roughly the same extra CPU, even though the response wire size ranges from 20 to 1,040 bytes. The older phase split attributed 2.82 ms of excess large-read CPU to work before send and 0.12 ms to the send call. The latest per-block records localize the recurring cost to both exchanges. Neither dataset identifies a single Rust function. [Earlier stage measurements](CHAINED-READS-20261125.md)

Use bounded task-clock stamps at dispatch entry, handler entry and return, encoded reply ready, and socket send entry. Match the intervals back to individual packets, check the observer cost with ordinary firmware, and rank the excess on the small GET and both large blocks. Preserve the same routes, capacities and duplicate replay. Review ingress, request admission, route and option work, then response construction and retention inside the largest interval. Advance a code change when that interval and whole-read worker CPU fall in reversed boot pairs with refusal, retry and cancellation coverage intact. The original 50% Windows server CPU targets stay open separately for unprotected and established OSCORE.

## Reproduction and scope

The primary Windows cohort verified 706,560 reads across 96 process windows. The runtime control verified 414,720 more in 36 windows. An earlier unprotected rotation with the prior verifier added 412,800 reads across 48 windows. Its individual measurements are included separately. Together these are 1,534,080 native reads. Four short pilots preceded the accepted rotation. The independent wire classifier used separate processes and supplied packet counts only.

The first C pilot failed the native verifier because the original handler selected the representation length as its ETag. The matched handler supplies the integer encoding of the same seven ASCII bytes `fixture`. It links the existing release libcoap 4.3.5b archive at `851533c3` with mbedTLS. The previous native verifier also refused libcoap’s initial authenticated Echo challenge. The common client now allows at most one such challenge before warmup, keeps the same OSCORE context, verifies the complete response, and runs its original measured loop. The independent classifier first completes a readiness GET and observes that challenge there. Neither client sends credentials or challenge values to the evidence file.

The JSON contains the exact companion fixture patches, release binary hashes, per-window CPU, latency, memory and cycles, independent wire counts, and the checked ESP packet intervals. A separate Windows server and client run on the same machine, so these serial rates capture both processes. The board pools, radio, crypto persistence and worker clocks use different boundaries. [Complete numerical evidence](LANDED-LIBCOAP-20261125.json)
