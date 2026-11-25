# Coaptic and libcoap on the same ESP32-S3

On November 23, 2026, the commissioned Waveshare served the same unprotected confirmable GETs from three firmware images. The native-client campaign ran all six permutations of libcoap, synchronous Coaptic and asynchronous Coaptic. Each image occupied each position twice. Each boot ran four fresh client processes in balanced sync/async order, eight cycles per process. Each cycle verified the 32-byte identity, seven-byte value and complete 2,000-byte binary body. Every image used the same 16 KiB instruction-cache setting. The 240 MHz CPU, Wi-Fi access point, client host, block size and application partition stayed fixed. Timing began at least 36 seconds after each upload and after the device identified its test image.

The two Coaptic images are the qualified Constrained firmware from core `48e513e` and companion `1de100cc`. The only core source differences from the checkout under comparison concern the std UDP adapter and its tests. The C fixture is Espressif’s libcoap package `4.3.5~8`, companion `2ff7dbd`. These are installed-image comparisons. Both implementations serve static representations. Only the three read routes are exercised. The C fixture allocates PDUs and sessions dynamically, replies from a borrowed static representation, and checks the peer address. Coaptic retains its bounded routes, duplicate entries, Observe rows, body slots and four Calls. The C fixture accepts unprotected traffic for a 30-second service window. The C benchmark has no matching durable OSCORE endpoint.

**The clearest latency gap is the async device's seven-byte GET.** With the async Windows client it took 7.03 ms on Coaptic async, 5.82 ms on Coaptic sync and 5.32 ms on C. The async device took longer than C in every boot rotation with both native client modes. The sync device was much closer, and changing the host client reversed the blockwise ordering. Treat short-value, identity and blockwise reads as separate workloads.

Both Coaptic archives included OSCORE support and ran these reads in unprotected mode. Each used six of eight route slots plus built-in discovery, two 1,152-byte datagram slots, one 4,096-byte body per direction, four duplicate entries, two Observe slots, four Calls, four security request bindings, one association and zero deferred jobs. Those reserved capabilities count toward the measured footprint even though a successful GET exercises only a subset. The comparison tests deployed firmware choices. It cannot isolate the memory cost of the programming language. [Firmware build and capacity records](ESP-WIFI-RELIABILITY-20261117.md)

## Latency from sending to verifying the body

The table gives pooled median / p95 in milliseconds over 96 reads per cell. “Async” and “sync” in the client column identify the Windows initiator. All clocks include client work, scheduling, radio transit, device processing and response verification. The six boots are the repeated hardware units.

| Windows client | Body | libcoap (ms) | Coaptic sync (ms) | Coaptic async (ms) |
|---|---|---|---|---|
| async | identity | 5.34 / 9.48 | 4.79 / 8.43 | 6.09 / 9.99 |
| async | test | 5.32 / 9.03 | 5.82 / 10.72 | 7.03 / 10.79 |
| async | large | 13.88 / 22.11 | 14.13 / 19.07 | 16.32 / 24.33 |
| sync | identity | 5.63 / 10.29 | 6.10 / 9.15 | 6.61 / 10.50 |
| sync | test | 6.09 / 10.35 | 6.15 / 12.15 | 7.59 / 12.19 |
| sync | large | 15.29 / 22.29 | 15.23 / 21.29 | 18.27 / 22.85 |

## How sensitive is the ranking?

Each percentage is the median of six within-round differences computed as (Coaptic median / C median − 1) × 100. Positive means Coaptic took longer. The range drops each round in turn and recomputes the median of the other five. The warm column pools all reads after omitting the first three reads of each fresh process. The large headline sample count does not turn shared radio conditions or six boots into independent requests. The drop-one range is a sensitivity check, not a confidence interval.

| Windows client | Route | Firmware | Paired difference | Rounds slower than C | Drop-one range | Warm pooled difference |
|---|---|---|---|---|---|---|
| sync | identity | Coaptic sync | +24.4% | 5/6 | +16.5% to +32.3% | +8.7% |
| sync | identity | Coaptic async | +29.2% | 5/6 | +25.7% to +32.7% | +10.8% |
| sync | test | Coaptic sync | +8.4% | 6/6 | +8.3% to +8.6% | +0.4% |
| sync | test | Coaptic async | +22.4% | 6/6 | +21.2% to +23.7% | +24.6% |
| sync | large | Coaptic sync | -2.7% | 2/6 | -2.8% to -2.6% | -0.0% |
| sync | large | Coaptic async | +22.5% | 6/6 | +17.8% to +27.3% | +20.0% |
| async | identity | Coaptic sync | -9.5% | 0/6 | -9.9% to -9.2% | -10.3% |
| async | identity | Coaptic async | +18.5% | 6/6 | +16.8% to +20.3% | +11.6% |
| async | test | Coaptic sync | +7.1% | 5/6 | +6.6% to +7.6% | +7.9% |
| async | test | Coaptic async | +33.5% | 6/6 | +32.3% to +34.7% | +30.4% |
| async | large | Coaptic sync | +4.0% | 4/6 | +2.8% to +5.3% | +1.1% |
| async | large | Coaptic async | +21.2% | 6/6 | +19.9% to +22.4% | +16.9% |

For the sync Windows client, Coaptic sync's seven-byte median was 8.4% above C when comparing matched boot medians but only 0.9% above C when pooling individual reads. Omitting the first cycle reduced the pooled gap to 0.4%. Pooling changes the weight of the latency distributions. For the async Windows client, the same read showed a 7.1% paired gap and a 9.4% pooled gap. A single percentage would overstate the precision here.

The identity route is a particularly poor proxy for whole-library speed. It runs first in every new process and has different handlers: the Coaptic fixture loads its 32-byte ID and constructs a copied response; the C fixture borrows a static representation. On this route Coaptic sync beat C in all six rotations with the async client but in only one with the sync client. The unprotected identity string establishes which image answered; it does not authenticate the peer.

## Delivered rate and process costs

One outstanding request at a time, mixed identity/small/blockwise reads. Each rate is the median of the 12 process windows per client/firmware combination, counting from first dispatch to last verified completion. Host CPU covers the whole Windows client process, divided by its verified reads including startup and exit costs. A Windows 15.625 ms accounting tick divided over 288 reads changes a combined cell by 54.3 microseconds/read. Working set and private bytes are the largest samples observed on that client, with 25 ms sampling. These figures measure the client process; no paired target-side scheduled CPU meter was present on these ordinary images.

| Client | Device | Verified reads/s | Client CPU (µs/read) | Client WS / private (MiB) |
|---|---|---|---|---|
| sync | libcoap | 109.5 | 379.8 | 5.44 / 0.93 |
| sync | Coaptic sync | 99.9 | 379.8 | 4.96 / 0.89 |
| sync | Coaptic async | 89.5 | 434.0 | 5.10 / 0.93 |
| async | libcoap | 123.1 | 542.5 | 5.91 / 1.17 |
| async | Coaptic sync | 115.1 | 379.8 | 5.45 / 1.15 |
| async | Coaptic async | 94.3 | 759.5 | 5.54 / 1.19 |

## Device memory

Bytes below come from the running firmware as a whole. Each free-heap figure is the median of six final in-service all-8-bit samples for Coaptic or six just-before-cleanup samples for libcoap. These sampling points are close to the end of service but not synchronized peak-memory measurements. Stack reservations already affect heap headroom. A free-stack watermark cannot establish a safe minimum for untested paths. OTA image size includes the platform and enabled features.

| Device | Free heap (KiB) | Worker reservation (KiB) | Observed unused stack (KiB) | App image (KiB) |
|---|---|---|---|---|
| libcoap | 226.7 | 16 | 13.0 | 866.0 |
| Coaptic sync | 163.7 | 96 | 80.2 | 1189.5 |
| Coaptic async | 67.3 | 192 | 88.9 | 1602.4 |

The C image left roughly 63 KiB more heap free than Coaptic sync and 159 KiB more than Coaptic async. Its worker stack reservation was 16 KiB versus 96 and 192 KiB. These are important deployment differences. The C firmware exposes three timed resources; Coaptic includes additional routes and bounded facilities. Removing the stack reservations on paper would neither reproduce an actual build nor establish its safe stack size.

## Independent client and overlapping reads

aiocoap ran in a separate six-boot forward/reverse campaign on the same firmware. Each boot verified 16 cycles of identity/small/blockwise reads. Packet capture was local to the client process. Complete-operation timers included aiocoap overhead and response verification. Packet decoding occurred after the single-request timings. Each median below covers 32 reads.

| Route | libcoap (ms) | Coaptic sync (ms) | Coaptic async (ms) |
|---|---|---|---|
| identity | 6.15 | 5.05 | 6.24 |
| test | 5.51 | 5.35 | 6.24 |
| large | 16.24 | 15.00 | 17.99 |

With aiocoap issuing one read at a time, Coaptic sync beat C on both body sizes in both rounds. The small-read median changed from a C lead under both native clients to a 0.16 ms Coaptic sync lead. The decoded packet types, codes, lengths, token lengths, options and block sizes matched across all three images on each route. Every large response delivered 1,024 bytes then 976 bytes. This rules out a skipped block or a shorter response as the explanation. The two rounds give little precision on the size of the advantage.

Two independent source sockets then started two reads of the same route together. The overlap column counts groups with both transmissions observed before the first received datagram. Group elapsed time ends when both client read attempts finish. The rate divides successful reads by summed group wall time and excludes gaps between groups. Two endpoints test one step beyond the serial workload. No saturation sweep was performed.

| Device | Route | Verified / attempted | Groups overlapping | Both complete (median ms) | Verified reads/group-second |
|---|---|---|---|---|---|
| libcoap | test | 32 / 32 | 16 / 16 | 7.50 | 244.6 |
| libcoap | large | 32 / 32 | 16 / 16 | 18.75 | 105.0 |
| Coaptic sync | test | 32 / 32 | 16 / 16 | 9.08 | 200.0 |
| Coaptic sync | large | 32 / 32 | 16 / 16 | 22.02 | 89.8 |
| Coaptic async | test | 32 / 32 | 16 / 16 | 11.13 | 172.2 |
| Coaptic async | large | 32 / 32 | 16 / 16 | 27.44 | 72.4 |

Starting two reads together changed the result again. C finished each pair sooner than either Coaptic image at the median for both routes in both rounds. On the large body, the median time to finish both reads was 18.75 ms on C, 22.02 ms on Coaptic sync and 27.44 ms on Coaptic async. All 192 reads succeeded. Both requests left the client before the first reply in all 96 pairs. The capture establishes overlapping demand at the host. It cannot tell whether the ESP32 executed the handlers concurrently.

## What the timing can localize

The two exchanges needed for a large body are observable at the host. Each number below is a median for the async native client. An exchange runs from the host send call starting to receive returning, so includes radio, scheduler and endpoint work. The middle interval isolates the time the client took to issue the next block request. Separate medians need not sum to the complete-read median.

| Device | First exchange (ms) | Host turnaround (ms) | Second exchange (ms) |
|---|---|---|---|
| libcoap | 6.635 | 0.015 | 6.580 |
| Coaptic sync | 7.020 | 0.015 | 7.055 |
| Coaptic async | 8.211 | 0.015 | 8.328 |

An older instrumented development campaign measured the narrower device interval from socket receive return to send entry. Its 192 verified unprotected reads and 256 datagrams were regraded against the original trace; the result file reproduced byte for byte. “Large” below sums its two device intervals before taking the median. These images have a different instruction-cache/code-placement profile from the 16 KiB images above. The spans include preemption and omit work outside their boundaries. They cannot be subtracted from today’s Wi-Fi latency or used as charged CPU time.

| Earlier development image | 7 B device interval (µs) | 2,000 B device interval (µs) |
|---|---|---|
| H sync / O3 thin | 405.0 | 1232.5 |
| H async / Os fat | 1255.0 | 2984.0 |
| C / pinned | 497.5 | 1419.0 |
| C / floating | 487.5 | 1353.0 |

The same C application binary also ran in an earlier 16 KiB-cache campaign. Rechecking 288 verified reads against its recorded hashes gave async-client medians of 5.15 ms small and 13.50 ms large. Its three large-read boot medians ranged from 11.85 to 15.47 ms. That variation is larger than today's 0.25 ms pooled gap between C and sync Coaptic on the async client's large read. Conversely, Coaptic async's short-read deficit repeated with both native clients across all six rounds. A matched on-device trace of this packet path is the best-supported next optimization experiment. A change to the blockwise encoder alone would miss the gap already present on a one-datagram read.

All six rotations used one board, one LAN and one host on one day. Signal samples ranged from −61 to −32 dBm. The 1,728 native-client reads completed with 2,304 request datagrams and 2,304 response datagrams. The firmware recorded zero C peer refusals and zero Coaptic application drops in these runs. Wi-Fi link-layer retries and time spent by the radio were unmeasured. The two campaigns together verified 2,208 reads across 24 test boots. Saturation throughput, energy use and comparative ESP32 CPU cost remain unmeasured.

## Measurement record

[Numeric results, boot-level observations, wire shapes and restoration status](ESP32-LIBCOAP-HEAD-TO-HEAD-20261123.json). The [previous physical OSCORE measurements](ESP32-WIFI-READS-20261123.md) use Coaptic on both ends. Both original-project 50% server CPU-time goals remain open.
