# Windows CPU profile of complete reads

## Workload and measurement

Each window used the same frozen native client on IPv4 loopback, one outstanding
CON Call, 32 warmups and 4,800 measured reads at 800 offered starts/s. The client
checked status, Content-Format, ETag and the complete representation. A 64-byte
read exchanged one datagram in each direction. A 4,096-byte Block2 read
exchanged four. OSCORE windows used a fresh fixture secret and measured reads
on an established context.

The control was checkout `f8ffae43`. Companion `8215e251` supplied the native
server fixture and measurement gates. The release build used MSVC x86-64,
Rust 1.99.1, thin LTO, one codegen unit, debug level 1 and the companion Syn 3
lock. Cargo metadata resolved both core crates to the checkout. Both App paths
used the Tokio 1.53.3 current-thread runtime and Mio 1.2.4 on Windows 11 Pro
26200, Ryzen 7 5800X3D. The client used its spin policy in every arm.

The allocated host-control App configured one route, four Calls, zero deferred
entries, one association, four security request bindings, eight dedup entries,
four Observe entries and four RX and four TX datagrams of 1,472 bytes each.
One RX and two TX body slots held 1,024 bytes apiece for small reads and 4,096
bytes apiece for blockwise reads.

The [numeric receipt](WINDOWS-READ-PROFILE-20261120.json) records each binary,
verified count, pacing misses, both processes' CPU time and cycles per verified
read, latency, achieved rate, and closing working set and private bytes. CPU
covers the post-warmup run through the client's measured signal, before JSON
formatting. Windows process time advances in coarse ticks. Memory observations
cover the processes at the closing gate. They do not measure peak application
storage. The 800/s workload measures cost at the offered rate.

## Stack capture

A separate sampler opened the owned server process and its primary thread.
At jittered intervals during a bounded 3.8-second window it suspended the
thread, collected its x64 context, walked up to 20 frames with DbgHelp and
resumed the thread. It resolved addresses against the matching PDB. The
histograms count wall-clock stack observations. A syscall leaf can include
kernel execution, suspension or waiting. Stop-and-walk sampling perturbed
scheduling substantially. Across the four baseline cells, p99 was 84–363 µs
with sampling off and 2.87–6.69 ms with sampling on. The first sampler build
failed to load a DLL. A statically linked sampler supplied all retained stack
captures. One async comparison server exited before readiness. A fresh window
completed that pair. Both failed attempts are excluded from the verified totals.
The matched profiler-off windows determine the
performance decision. No kernel logger, system sampling interval or host
scheduling policy was changed.

The baseline sync-App stacks included `Driver::turn → SelectorInner::select2
→ GetQueuedCompletionStatusEx`, `Engine::send_tx → try_send_to → sendto` and
`try_recv_from → recvfrom`. Coaptic frames included `request_options`,
`NoResponse::from_message` and, in OSCORE, `mixes_block_and_qblock`.
Protected blockwise reads also sampled AES encryption, CCM's CBC-MAC, and
`protect_response → seal_message_staged → encode_plaintext`.

### Executing versus waiting

A separate baseline diagnostic timed synchronous App bursts and the entire
await of `ready_wait`. It read the current server thread's cycle counter
alongside the monotonic wall clock. Two snapshots, after readiness waits 128 and
2,176, bounded the counters. The server stayed on one Tokio thread. Receive
and send calls are nested in the App burst. The empty probe measured the cost
of an otherwise empty pair of readings. All values below are per successful
send, so one blockwise operation accounts for four sends.

- At 64 bytes the readiness path spanned 1,217–1,219 µs and accrued
  62,946–64,820 thread cycles. App bursts spanned 29.0–30.8 µs and accrued
  96,804–102,785 cycles. Within those bursts, sending used 55,614–56,879
  cycles and receiving used 17,770–18,041.
- At 4,096 bytes the readiness path spanned 288.8–292.3 µs per send and
  accrued 36,734–41,082 cycles. App bursts spanned 18.5–22.4 µs and
  accrued 60,999–74,843 cycles. Sending used 34,338–36,581 cycles and
  receiving used 11,233–11,773.

The readiness span includes parked time, executor work, and delay until the
task resumes. The thread accumulates cycles while scheduled. The counters
cannot separate socket-blocked time from ready-but-unscheduled time in that
span. Socket sends were the largest measured component inside the bursts.
The empty probe cost 0.34–0.45 µs and 1,070–1,163 cycles per send. Across
the four same-binary on/off controls the instrumented windows used 6.4–13.1%
more whole-process server cycles per verified read. These are observed overheads
including run-to-run variation.

## Experiments

1. **Absent option lookup.** Retain the last validated option number in the
   decoded view and its OSCORE reborrow metadata. Return an empty iterator for
   a larger requested option. Across the four sync-App traces the three selected
   option paths appeared in 149 of 14,185 baseline observations and 119 of
   14,070 candidate observations. The small counts and probe disturbance limit
   attribution. In profiler-off measurements, async unprotected blockwise server
   CPU fell in all three pairs, while sync protected blockwise server cycles
   rose in all three pairs. Both protected median CPU costs increased in both
   runtimes. The candidate was rejected.
2. **Full-capacity OSCORE sealing fallback.** When direct inner encoding or
   sealing fails after receiving the full 1,280-byte inner capacity, the
   staging path repeats the same operation at the same capacity. The candidate
   returns that error directly. Smaller output buffers still take the original
   fallback, preserving error precedence. The existing differential test
   compares direct and staged results and successful bytes across options,
   token, payload and output-capacity boundaries.
   The cold fallback appeared in six of 3,479 protected blockwise baseline
   samples and zero of 3,579 candidate samples. Profiler-off server CPU deltas
   for this cell were +87.5%, −14.5% and −20.6% with the sync App, and
   −26.9%, −9.5% and +221.4% with the async App. The sync server consumed
   more cycles in all three pairs. The candidate was rejected.

## Profiler-off measurements

The two experiments each used three adjacent checkout/candidate pairs per
runtime, security mode and body size. The second pass reversed both cell and
build order. Below are medians of the six sync-App checkout windows per cell,
combining the two experiments' control arms. The numeric receipt also gives
every async window, candidate window and missed-start count.

- Unprotected 64 B used 40.7 µs server and 68.4 µs client CPU per verified
  read. Latency was 51.7 µs p50 and 122.0 µs p99 at 800.14 reads/s.
- Unprotected 4 KiB used 113.9 µs server and 148.1 µs client CPU per read.
  Latency was 116.7 µs p50 and 369.9 µs p99 at 800.11 reads/s.
- Established OSCORE 64 B used 53.7 µs server and 73.2 µs client CPU per
  read. Latency was 57.4 µs p50 and 143.4 µs p99 at 800.12 reads/s.
- Established OSCORE 4 KiB used 127.0 µs server and 166.0 µs client CPU
  per read. Latency was 147.8 µs p50 and 369.4 µs p99 at 800.11 reads/s.

Across those four cells, median server working sets were 5.88–6.03 MiB and
client working sets were 4.94–5.07 MiB. Median private bytes were
1.10–1.12 MiB for the server and 0.83–0.85 MiB for the client.

## Disposition

All 128 retained windows verified their reads, for 614,400 total. Of those,
460,800 were measured in the alternating profiler-off comparisons. The original
checkout and each candidate passed all 1,711 all-feature tests, including
protected and unprotected finite-blackout Call recovery, cancellation, expiry,
capacity refusal and reuse, duplicate replay and body limits. Both trial patches
are preserved in the numeric receipt. Production code remains at the checkout
baseline.

The larger remaining costs are socket send and the runtime readiness path.
The existing [single-reader](WINDOWS-TOKIO-SINGLE-READER-WAIT-20261120.md) and
[completion dequeue](WINDOWS-IOCP-BOUND-20261119.md) experiments already tested
two ways to reduce readiness cost. The present traces reach the Winsock/kernel
boundary without identifying removable work below it. Another server change
needs a way to isolate reducible work in one of those paths and repeatable CPU
time at matched latency. The independent unprotected and OSCORE 50% CPU-time
targets against the original project baseline `b8f44fa` remain open.
