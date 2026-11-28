# Following one read through the Windows server

The unit here is a fully verified application read. For a 64-byte separate response that means four datagrams. The client sends a GET, the server sends an empty ACK and a confirmable Content response, and the client acknowledges the Content response. A 4,096-byte read repeats the exchange for four blocks. The server receives two datagrams per block. Counting only the first GET would omit half of its ingress dispatch. Both security modes use that same exchange.

## Following the packets

1. **Sleep and reserve.** The driver calls `wait_async`. It checks retained work and the earliest deadline, reserves one existing RX slot, and polls the local wake, timer and receive in that order. A pending receive yields the worker. Cancellation drops the reservation. After ingress, `receive_retained_async` returns immediately; the protocol poll consumes the occupied slot. It issues no second socket receive. [Wait order](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/wake.rs#L66) and [reservation lifetime](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/storage/async_io.rs#L154).
2. **Receive and stage.** Tokio receives into reusable scratch with one guard byte. It copies the accepted prefix into the RX slot only after checking truncation. Dispatch copies the used bytes from that slot into parsing scratch. This makes two user-space staging copies of the incoming datagram. The first copy preserves an untouched destination on oversize refusal. The second permits mutable engine access while the parsed request is borrowed. Neither copies the requested response body on a GET. [UDP adapter](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/tools/coaptic-tokio/src/lib.rs#L118) and [dispatch and `copy_rx`](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/mod.rs#L3470).
3. **Select the RX owner.** `next_unpinned_rx` tests occupancy, pinning and inbox ownership in rotating order. The original dispatcher then asked the inbox about the selected slot a second time. Between those two statements, retry and expiry can relinquish a held reply. Only dispatch can assign a new one. With four Calls the repeated search can visit four rows on the GET and four more on the final ACK. The filter at original line 2796 repeats the eligibility predicate from line 2747. That is up to eight row checks per small read or 32 per four-block read. These are source-level upper bounds. Fixed and allocated fairness tests exercise a held reply beside an earlier queued owner, saturation refusal, retry and slot reuse. [Rotating scan](../../src/storage/progress.rs), [inbox lookup](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/client.rs#L599) and [fairness regressions](../../tests/rx_fairness.rs).
4. **Decode and resolve the peer.** The decoder validates framing, token length and option boundaries. A fresh GET checks retained duplicate state before application dispatch. A protected request resolves an association from its outer identity, then verifies the ciphertext. Inner options, URI segments and query values become application input after authentication. An empty ACK has neither token nor protected recipient ID. Its MID and physical peer must resolve to one retained owner across associations. [Dispatch](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/mod.rs#L3660) and [association selection](../../src/app/associations.rs).
5. **Track replay.** In the original protected ingress, `dispatch_rx` selected the context before and after `oscore::inbound` solely to compare two replay snapshots. `inbound` also selected the mutable context to authenticate. That adds two context lookups to a protected small read and eight to a four-block read. Volatile mode discarded the comparison. In durable mode an authenticated request can advance replay before a malformed inner message returns an error. The trial compares on the already-borrowed context in durable mode, including error returns. For an admitted response, the trial marks the checkpoint dirty where the replay window advances. The ordinary response-binding and Observe notification counters live outside that window. [Admission](../../src/app/oscore.rs), [authentication ordering](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/oscore/protect.rs#L221) and [durability regressions](../../src/oscore/replay_tests.rs).
6. **Route, encode and send.** The request-option pass records borrowed URI path and query spans. The router and handler consume those spans. The timed fixture uses one resource. Tests cover multiple segments, query ordering, empty values, query overflow, refusal and subsequent reuse. A separate reply remembers the request’s empty ACK for duplicates, sends it, then retains the encoded response CON for retransmission. Block2 adds range selection, body-capacity checks and encoding for each block. In the explicit C handler fixture, libcoap registers an asynchronous reply and uses its block helpers to emit the selected range. Its stock protected server also has a different temporary encryption allocation. [Response dispatch](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/mod.rs#L5616) and [matched C fixtures](MATCHED-ACK-20261125.md).
7. **Accept the final ACK.** The control path resolves a unique association, clears the accepted pending CON and releases the TX slot. Exchange, Observe and deferred records can outlive their original packet slot. In the original, while considering a matching pending row, `control_route` looks up the pending MID and endpoint again to exclude an unsent reservation. This is a concrete remaining source of repeated work. Reusing a slot number across exchanges, or bypassing the ambiguity check, can settle the wrong Call. [Control routing](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/mod.rs#L5289), [pending ownership](../../src/storage/engine.rs) and [previous full-dispatch measurements](EMPTY-ACK-CONTROLS-20261126.md).

The original protected-response path offers another concrete lead. The outer option pass finds OSCORE first. Admission then calls `parsed.oscore()` again, and the cryptographic parser obtains and validates the header. On a response, admission reads the header again to apply the Partial IV rules. A managed association additionally checks for duplicate options and uses the outer KID and context to select a candidate. This evidence establishes repeated traversal. Route selection, authentication and replay acceptance occupy distinct trust boundaries, so a reused header would have to retain its original lifetime and duplicate-option outcome. The timing results below do not assign a cost to these walks. [Original association selection](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/associations.rs#L241) and [original response admission](https://github.com/jeffglousher/coaptic/blob/5199ef2b27620fd2a1c538274d695479eba61226/src/app/oscore.rs#L294).

## Why a read produced a write event

Tokio 1.53.3 registers a Windows UDP socket with both read and write interest. Mio 1.2.4's selector clears a readiness bit after delivering its edge. When `recv_from` returns `WouldBlock`, `IoSourceState::do_io` re-registers the original interest mask. That restores write interest even if the last write edge was already delivered and there is no waiting sender. A writable loopback socket can report that edge again, after which the remaining read interest must still be serviced. The driver can do this work during a real application wait. A wall stack inside the completion-port wait does not charge that time as server execution.

A bounded loopback probe drained the initial writable edge before each of 20 empty receives. Stock Mio produced 20 further writable edges. A private variant rearmed receive and error interest while preserving the current write arm under the socket lock. It produced zero further writable edges. Both variants delivered 20 of 20 requested writer wakeups, read data after explicit read-only registration and reused the socket. The backpressure signal in this particular test was injected through Mio’s public `try_io` path. The data and readiness events used real Windows sockets. Timeouts bounded all waits.

Earlier direct process-local Mio counters measured 2.00 AFD readiness submissions per small read at capacity and 3.64 at 800/s. At both loads there were exactly two successful receives, two successful sends and zero sends that would block. The earlier wall samples locate waiting and syscall paths; they do not establish CPU duration per syscall. The direct driver counts, real writable-edge probe and end-to-end timing answer separate questions. [Earlier profile, counter overhead and stack attribution](TOKIO-DEADLINE-WAKE-20261126.md).

## Matched Windows results

Each of two profiler-off cohorts ran 144 windows and verified 1,059,840 reads. Both used one outstanding Call and 32 warmups on Windows loopback. The paced windows verified 3,200 reads each, and the capacity windows verified 16,000 small or 7,040 blockwise reads. Each cell used all six orders of its three executables. Every reported operation checked the response and payload. Every window had exactly two sends and two receives per block at the client. The process gate saw just the frozen native client and the selected release server. No operations failed, were cancelled or went unattempted. The separate wire check verified 256 reads across four Rust builds.

The first cohort compared the current baseline, inbox-only removal and private Mio trial. The second compared the identical baseline binary, the integrated inbox/replay change on stock Mio and stock libcoap. Coaptic kept four 1,472-byte RX and TX slots per direction, eight duplicate entries, four Observe entries, four Calls, one route, zero deferred jobs, one RX and two TX body slots. Protected reads used one established context and four request bindings. Libcoap allocated PDU storage dynamically. The unprotected C fixture used its separate-response block handler. The protected C fixture used its stock encryption allocation. Historical experiments isolated the cost of that allocation. [C allocation control](MATCHED-ACK-20261125.md)

### Process CPU

Entries are microseconds of charged CPU per verified read, reported separately for the server and the client. Each entry is the median of six windows. A 15.625 ms CPU accounting tick corresponds to 4.88 µs/read in a paced window. Process cycles provide a second measurement for small differences. The receipt includes user and kernel CPU separately.

| Load | Security | Body B | Base server | Changed server | C server | Base client | Changed client | C client |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 800/s | Unprotected | 64 | 56.2 | 46.4 | 39.1 | 58.6 | 56.2 | 61.0 |
| 800/s | Unprotected | 4096 | 163.6 | 148.9 | 134.3 | 170.9 | 161.1 | 197.8 |
| 800/s | OSCORE | 64 | 75.7 | 39.1 | 83.0 | 92.8 | 63.5 | 119.6 |
| 800/s | OSCORE | 4096 | 217.3 | 183.1 | 266.1 | 231.9 | 183.1 | 310.1 |
| Capacity | Unprotected | 64 | 24.4 | 23.4 | 22.5 | 24.4 | 23.9 | 29.3 |
| Capacity | Unprotected | 4096 | 99.9 | 97.7 | 95.4 | 99.9 | 97.7 | 113.2 |
| Capacity | OSCORE | 64 | 27.3 | 27.3 | 52.2 | 27.3 | 27.3 | 57.6 |
| Capacity | OSCORE | 4096 | 119.9 | 119.9 | 223.1 | 121.0 | 121.0 | 250.8 |

### Latency and throughput

Latency columns contain p50 / p99 for a complete read in microseconds. Rate columns contain verified reads per second. The paced rate is offered load. The capacity columns show how quickly each peer served its next request with one Call outstanding.

| Load | Security | Body B | Base p50 / p99 | Changed p50 / p99 | C p50 / p99 | Base rate | Changed rate | C rate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 800/s | Unprotected | 64 | 60.8 / 129.2 | 61.9 / 127.8 | 61.8 / 120.9 | 800 | 800 | 800 |
| 800/s | Unprotected | 4096 | 144.3 / 357.1 | 141.9 / 357.9 | 159.3 / 341.6 | 800 | 800 | 800 |
| 800/s | OSCORE | 64 | 68.3 / 138.8 | 66.2 / 138.6 | 99.3 / 182.6 | 800 | 800 | 800 |
| 800/s | OSCORE | 4096 | 166.1 / 397.8 | 166.4 / 382.9 | 307.4 / 557.9 | 800 | 800 | 800 |
| Capacity | Unprotected | 64 | 22.4 / 47.1 | 22.5 / 47.1 | 27.6 / 53.1 | 41,593 | 41,584 | 34,664 |
| Capacity | Unprotected | 4096 | 95.0 / 180.3 | 93.8 / 174.6 | 108.0 / 205.7 | 10,071 | 10,215 | 8,900 |
| Capacity | OSCORE | 64 | 27.2 / 54.8 | 26.0 / 54.4 | 55.5 / 104.7 | 36,135 | 36,809 | 17,150 |
| Capacity | OSCORE | 4096 | 116.0 / 215.8 | 115.4 / 223.7 | 240.8 / 420.7 | 8,312 | 8,259 | 3,985 |

### Memory

Each entry gives private MiB / working-set MiB at the end gate. The receipt also retains peak working set and the measured Rust component sizes. Component owners nest and cannot be added together.

| Load | Security | Body B | Base server | Changed server | C server | Base client | Changed client | C client |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 800/s | Unprotected | 64 | 1.17 / 6.15 | 1.16 / 6.11 | 0.94 / 5.40 | 0.83 / 5.00 | 0.83 / 5.00 | 0.83 / 5.00 |
| 800/s | Unprotected | 4096 | 1.18 / 6.22 | 1.19 / 6.18 | 0.96 / 5.43 | 0.84 / 5.03 | 0.85 / 5.03 | 0.84 / 5.03 |
| 800/s | OSCORE | 64 | 1.17 / 6.27 | 1.16 / 6.21 | 0.96 / 5.56 | 0.83 / 5.09 | 0.83 / 5.09 | 0.82 / 5.09 |
| 800/s | OSCORE | 4096 | 1.19 / 6.33 | 1.19 / 6.29 | 0.99 / 5.58 | 0.84 / 5.12 | 0.84 / 5.12 | 0.84 / 5.12 |
| Capacity | Unprotected | 64 | 1.16 / 6.15 | 1.16 / 6.11 | 0.94 / 5.39 | 0.92 / 5.10 | 0.92 / 5.10 | 0.92 / 5.09 |
| Capacity | Unprotected | 4096 | 1.18 / 6.21 | 1.18 / 6.18 | 0.96 / 5.43 | 0.87 / 5.05 | 0.86 / 5.05 | 0.86 / 5.05 |
| Capacity | OSCORE | 64 | 1.16 / 6.26 | 1.17 / 6.22 | 0.96 / 5.56 | 0.92 / 5.18 | 0.92 / 5.18 | 0.92 / 5.18 |
| Capacity | OSCORE | 4096 | 1.19 / 6.33 | 1.19 / 6.29 | 0.98 / 5.58 | 0.87 / 5.14 | 0.87 / 5.14 | 0.87 / 5.14 |

### What each source change bought

Percentages below are the median of the six paired changes from the identical baseline in that cohort. Server cycles and throughput use opposite directions for improvement. The integrated column contains both Coaptic edits; it cannot isolate the replay edit by itself.

| Load | Security | Body B | Inbox-only server cycles | Mio trial server cycles | Mio trial rate | Integrated server cycles | Integrated server CPU | Integrated rate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 800/s | Unprotected | 64 | -0.5% | -11.1% | +0.0% | +0.4% | -38.2% | +0.0% |
| 800/s | Unprotected | 4096 | -1.1% | -1.4% | -0.0% | -0.5% | -7.2% | -0.0% |
| 800/s | OSCORE | 64 | -0.0% | -10.6% | -0.0% | -2.1% | -46.5% | -0.0% |
| 800/s | OSCORE | 4096 | -0.7% | -0.9% | -0.0% | +0.4% | -19.1% | -0.0% |
| Capacity | Unprotected | 64 | -0.8% | +10.4% | -12.8% | -1.4% | -3.9% | +0.5% |
| Capacity | Unprotected | 4096 | +4.0% | +10.1% | -10.9% | -2.1% | -3.2% | +2.3% |
| Capacity | OSCORE | 64 | -3.6% | +4.7% | -7.2% | -1.3% | +0.1% | +1.6% |
| Capacity | OSCORE | 4096 | +0.7% | +8.5% | -11.0% | +0.1% | -1.8% | -0.4% |

The private Mio trial reduced process-wide other I/O from 3.81 to 2.12 operations per paced small read and from 2.09 to 1.93 at capacity in 76,800 additional verified reads. These Windows counts include all process handles. A controller query took a median 3.5 µs and p99 7.2 µs over 257 calls. Only two extra queries per process were made in each diagnostic window. Both main cohorts ran without them. The small paced trial saved 11.1% unprotected and 10.6% protected server cycles, with reductions in all six pairs in each mode. At capacity every paired throughput comparison favored stock Mio in all four cells. For 4 KiB, median paired server CPU rose 10.9% unprotected and 13.0% protected. The driver variant was rejected.

## Execution and waiting

A fresh interpreter attached a sampler to one server worker only after all profiler-off windows finished. Each workload also ran with sampling disabled and with a separate thread-state observer. No shared host tracing settings changed. The before trace used the previously frozen baseline executable with matching private symbols. Production source and fixture match the current baseline; its CodeView identity differs from the performance binary. Both CodeView identities used for stack sampling were verified against their PDBs. Sample counts below are inclusive. The worker was suspended briefly to acquire each wall stack, so timing decisions use the independent profiler-off windows.

| Workload | Build | Wall stacks | Completion-port wait | Socket send | Socket receive | Security | Thread-state observations | Waiting | Off / sampled p99 µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Unprotected 64 B | Before | 3248 | 973 | 1277 | 197 | 0 | 191 | 184 | 123.9 / 4593.9 |
| OSCORE 4096 B | Before | 3312 | 135 | 1345 | 337 | 320 | 189 | 168 | 389.1 / 11684.3 |
| Unprotected 64 B | Changed | 3308 | 927 | 1228 | 242 | 0 | 184 | 174 | 122.6 / 5677.4 |
| OSCORE 4096 B | Changed | 3436 | 243 | 1049 | 381 | 317 | 192 | 168 | 382.9 / 13786.0 |

The most frequent leaf in all four wall traces was Windows `NtDeviceIoControlFile`. In the changed small-read trace it appeared as the leaf 1,396 times in 3,308 stacks. Socket send appeared in 1,228 stacks and the completion-port wait in 927. A separate worker-state run found 174 of 184 observations waiting, nine running and one on standby. For the changed protected blockwise workload, 168 of 192 observations were waiting and 24 running. The symbolized security paths appeared in 317 of 3,436 stacks. The worker-state runs distinguish executing from waiting, but they cannot divide charged kernel CPU between individual protocol functions. Neither inbox lookup nor replay bookkeeping accumulated a separately resolved sample. Inlining and the sampler’s large p99 perturbation preclude a per-function microsecond claim.

## Decision

The integrated Coaptic change removes the repeated RX eligibility search and scopes replay tracking to the security context that changes. It also closes a cancellation gap. The previous client marked durable replay state after returning from the whole continuation. A peer report can consume a replay sequence and suspend while sending the next block. Dropping that future skips the old post-return check. Marking the state dirty at sequence acceptance lets the next durable poll persist it before touching further ingress. The regression failed against the original source at the refused checkpoint and passed on the change. It exercises an authenticated report, pending resend, cancellation, refused persistence, retry, RX/TX/body cleanup and an unrelated live Call.

At unprotected 4 KiB capacity, the integrated build used less server CPU and fewer server cycles in five of six paired windows, and fewer client cycles in all six. Median paired server CPU fell 3.2%, throughput rose 2.3% and p99 fell 6.4%. For protected 4 KiB at capacity, baseline and changed CPU medians were both 119.85 µs on the server and 120.96 µs on the client. Throughput moved by −0.4% in the median pair and p99 by +2.0%. Across modes and sizes the paced CPU-time differences were much larger than the corresponding cycle differences. Those CPU-time percentages are unsuitable as estimates of the speed of the removed checks. The CPU, throughput, latency and memory medians were independently reconstructed from all 288 raw windows. Both Rust builds measured 15,928 inline App bytes including the adapter and 11,848 bytes for the separate borrowing loop future. The integrated timing measures both source changes together.

Stock C remained cheaper in unprotected server CPU at capacity. On a 4 KiB read the changed Rust server used 97.66 versus 95.44 µs, while its client used 97.66 versus 113.19 µs. Coaptic completed 10,215 reads/s versus 8,900. Under established OSCORE the changed Rust server used 119.85 versus 223.06 µs and completed 8,259 versus 3,985 reads/s. Stock libcoap’s known encryption allocation contributes to that protected comparison. The same verifier, response bytes, block count and two empty ACKs apply throughout.

The integrated checkout passed 1,726 all-feature workspace tests, formatting, and strict Clippy with all features and with defaults disabled. The private driver experiment also passed all 12 Tokio adapter tests. Independent no-default, alloc, OSCORE, alloc+OSCORE and std test runs passed 1,057, 1,173, 1,281, 1,417 and 1,071 checks respectively. The original-source cancellation failure was reproduced with an explicit manifest. One earlier negative-control invocation resolved to the changed checkout and was excluded from the original-source evidence.

The ESP32-S3 did not answer either recorded management port or its recorded mDNS service and Windows exposed zero serial ports. The last paired physical measurement remains 2.857 versus 1.587 ms of worker CPU for a seven-byte read on Coaptic and C. The excess was 1.278 ms on the first large block and 1.378 ms on the follow-up. Windows Mio does not run on this device. The device question is where the relatively fixed per-datagram excess falls between poll entry, handler, encoding and send. No new firmware was installed. [Physical comparison](LANDED-LIBCOAP-20261125.md)

Against the original project baseline the last source-matched capacity changes were −9.4% small and −8.2% blockwise in unprotected mode, and −1.7% and −6.1% under established OSCORE. This study’s before build is the later checkout at `5199ef2`. The two 50% targets remain open independently. [Historical comparison](EMPTY-ACK-CONTROLS-20261126.md) and [per-window evidence, executable hashes and trial patches](WINDOWS-READ-WALK-20261128.json).
