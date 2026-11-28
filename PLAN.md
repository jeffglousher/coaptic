# Project plan

Updated 2026-11-28 after bringing up the native Linux comparison target.
Current effort: **performance**, with reliability as an acceptance condition.
This is the current status and work order. Detailed results live in linked
[evidence and history](architecture/requirements/PROJECT-HISTORY-20261117.md).

## Current position

The manual Home Assistant app adds native amd64 Linux to Windows and ESP32-S3.
Across 262,144 verified Linux reads, Coaptic led all four capacity pairs for
64-byte unprotected reads and both OSCORE sizes. For 4 KiB unprotected reads,
the second peer won in either order. At 800/s, the 64-byte unprotected Coaptic
server used 62.26 µs/read versus 42.72 for libcoap. Profile the paced receive
and ACK wake path before changing the core for this gap. Both original-baseline
CPU targets remain open. [Linux results and raw receipts](https://github.com/jeffglousher/coaptic-validation/tree/eb9532ce642235f6076b156417563e3860933646/tools/benchmark/evidence/ha-linux-20261128).

The [authenticated association installation comparison](architecture/requirements/MANAGED-INSTALL-20261128.md)
found repeated preflight work and a 32-byte owner copy in a single-pass trial.
Borrowing the row and keeping the checked vacancy cut the final authenticated
32-slot install time by 57–61% in ten alternating pairs per vacancy layout.
Direct 32-slot calls improved in at least 29/30 pairs per case across three runs.
Placing the new implementation after the lookup implementations preserves the
previous established-read server's complete executable code. One-slot direct
success remained inconclusive. The final Windows release completed more reads
than stock libcoap in all 24 capacity pairs. At 800/s both met the offered rate.

The [Windows and ESP32-S3 throughput comparison](architecture/requirements/THROUGHPUT-AND-LOOKUP-20261128.md)
separates capacity, an 800/s offered load and active device intervals. Coaptic
won all six Windows capacity pairs per workload in the second rotation. The
preceding unprotected 4 KiB rotation was mixed. The landed ESP32-S3 peers had
similar mixed throughput, while Coaptic consumed 1.27–1.38 ms more worker CPU
per datagram. Physical stage attribution and both original 50% server-CPU
targets remain open. Bounding the association scan while the generation is zero
raised unprotected four-block throughput by 6.2% and 4.8% in two independent
32-slot Windows cohorts. A wider association-lookup rewrite and a compact duplicate
history representation both regressed on repeat and were rejected.

The [Windows read and ACK walkthrough](architecture/requirements/WINDOWS-READ-WALK-20261128.md)
follows both received datagrams per block. The rotating RX scan already excludes
inbox-held replies. Protected ingress can track replay on its selected context,
and an admitted client replay must mark its durable checkpoint before an awaited
continuation can be cancelled. In the integrated comparison, unprotected 4 KiB
capacity reads used 97.66 µs of server CPU and 97.66 µs of client CPU, versus
99.88 µs each before. The protected 4 KiB medians held at 119.85 and 120.96 µs.
A private Mio trial suppressed all 20 redundant writable edges but cut capacity
throughput in every pair at both sizes and security modes. It was rejected.

The [shipped Tokio adapter comparison](architecture/requirements/TOKIO-DEADLINE-WAKE-20261126.md)
verified 1,497,600 Windows reads with both empty ACKs. At capacity, 4 KiB reads
used 103.08 versus 106.34 µs of unprotected server CPU for Coaptic and libcoap,
and 122.61 versus 238.72 µs under established OSCORE. At 800/s the small
unprotected Coaptic server used 34.3% more cycles in the median pair, with
higher cycles in all six. Skipping a timer poll on immediately writable sends
improved capacity in two independent cohorts. At 800/s the trial raised paired
unprotected blockwise server CPU by 16.1% and 11.6% and grew the loop future
by 24 bytes. The trial was rejected.

The [direct unprotected and historical controls](architecture/requirements/EMPTY-ACK-CONTROLS-20261126.md)
measure both empty ACKs on both peers. At capacity, separate 4 KiB reads used
99.88 µs of Coaptic server CPU and 95.44 µs for libcoap. Verified throughput
was 9,955 and 8,926 reads/s respectively. Against the original Coaptic release,
paired capacity server CPU fell 9.4% and 8.2% for unprotected small and blockwise
reads, and 1.7% and 6.1% for established OSCORE. Neither mode meets the 50% target.

The [matched acknowledgement study](architecture/requirements/MATCHED-ACK-20261125.md)
covers stock Coaptic and libcoap, small and blockwise reads, two security
modes and both process roles. The [comparison harness](https://github.com/jeffglousher/coaptic-validation/pull/278)
verifies every block and both empty ACKs when separate responses are selected.
At Windows capacity, matched protected server CPU was 28.32 versus 50.78 µs
for 64 bytes and 122.07 versus 229.71 µs for 4 KiB. A controlled C experiment
showed that libcoap’s temporary encryption-buffer allocation explains much
of that difference. In a direct three-way test the adjusted C server consumed
29.79 and 117.63 µs per read. Coaptic completed more reads/s in all six pairs.
Two Rust borrowing trials eliminated full-body staging but slowed other reads
and were rejected. Both original-project 50% targets remain open.
The ESP32-S3 separate-response fixtures are built and wire-verified on Windows.
Physical qualification awaits a connection to the board.

The complete ACK/RST dispatch check also exposed a Call ownership bug. An empty
ACK frees the outgoing packet while the Call still awaits its separate reply.
Reset and retransmit cleanup now select the exchange by Message ID and routed
peer, and failed continuation sends retire the exchange they inserted.
A separate empty-table optimization improved unprotected 4 KiB reads, but
made small protected reads slower in every scaled-capacity latency pair.
The complete-profile study above records the rejected trial and traces
deadline timer registration through the Windows completion port.

The packet-path improvements landed in [#799](https://github.com/jeffglousher/coaptic/pull/799)
at `32fdf57`. The [Windows and Waveshare peer comparison](architecture/requirements/LANDED-LIBCOAP-20261125.md)
covers 1,534,080 native Windows reads and regrades both blocks of each ESP read.
On the ESP32-S3, Coaptic used 1.27 ms more worker CPU than libcoap on a small
reply, 1.28 ms more on block zero and 1.38 ms more on the follow-up. Windows
unprotected reads also used more server CPU. The protected Windows peers used
different acknowledgement strategies.

**Next focus.** On the board, use the staged worker-clock probes at receive
return, async poll entry, handler and send to locate the common cost across
the small GET and both block positions. On Windows, follow the send and receive calls accounting for charged kernel
CPU in small unprotected reads. Narrowing receive re-registration reduced
paced other-I/O counts from 3.81 to 2.12 per read but slowed saturated
reads. Keep the stock readiness behavior while attributing the remaining gap. At capacity, Coaptic used more kernel CPU in
all ten small unprotected pairs. Small piggyback reads counted 2.01 other I/O
operations per read for Coaptic and 1.00 for the matched C handler. Check the
whole-read worker time in reversed boot pairs and retain refusal, duplicate,
retry and cancellation coverage. Both historical Windows 50% targets are open.

The landed packet-path batch was measured at diff `510df7f0`. It carries Call
expiry and admission decisions forward, borrows deferred rows, and reduces
option and upload-binding work. Six balanced rounds of verified eight-fragment
PUTs through the fixed-pool App took 6.03 to 5.38 microseconds without a
Request-Tag and 6.32 to 5.59 with one, relative to `3eb1b847`. Neither
comparison includes UDP or an executor. The preceding build refused a valid
original-size retry after an invalid smaller block. The new one accepts it.
At Windows capacity the original and current releases used 17.58 and 16.60
microseconds of server CPU per 64-byte unprotected read, and 72.92 and 66.41
at 4 KiB. Established OSCORE used 19.53 and 18.07 for the small read, and
95.05 and 85.94 at 4 KiB. Each comparison used six order-balanced rounds.
On the Waveshare, current Coaptic and libcoap workers used 2.86 and 1.59 ms
for seven bytes, and 6.53 and 3.82 ms for 2,000 bytes. Both Windows 50%
server CPU targets remain open against the original project. On the Waveshare,
192 eight-fragment uploads took 62.71 to 60.25 ms untagged and 65.08 to
60.88 ms tagged across two reversed boot pairs. Protected read worker CPU
was 14.21 to 14.14 ms small and 34.35 to 34.78 ms large. A separate receive
queue change regressed four-Call paths and stayed out. The
[packet-path report](architecture/requirements/PACKET-WORK-20261125.md)
records the rates, both process roles, stack traces, candidate decisions and
five verified restorations. A separate six-order rotation isolated the original
AES pin. The 4 KiB protected CPU saving repeated with AES matched. The small
protected saving varied from 7.5% to zero across historical-control rotations.
All-feature workspace tests passed (1,719).

The earlier chained-read controls used the working tree on `161a22569c1d` with source diff
`e412562a082fcc1f7f46b6b0c4789a7cae31892db9e9f41b972e943d490e3b05`.
Diff identities cover `git -c core.safecrlf=false diff --binary --no-ext-diff HEAD -- src examples`.
The working tree also lends completed responses to a synchronous caller and
extracts OSCORE key material once per association setup.

- Call setup carries the selected live row to send acceptance. Block follow-ups
  use the retained path and query bytes. Ordinary reply collection uses the
  saved payload boundary, and the first accepted block supplies header metadata
  for the finished representation.
- Classic and Q block assembly copy each admitted fragment directly from the
  parsed payload into its bounded body slot. Authenticated requests and block
  responses consume plaintext in dispatch scratch. Full ordinary replies retain
  plaintext in RX for the caller. Token-capacity refusals reuse the original
  validated datagram and authenticate protected requests before replying.
- Admission skips exemption lookups when packet headroom suffices. Observe
  scheduling borrows interest rows, accounts for each physical endpoint's send
  credit, and handles queued ingress before scanning deadlines. The desktop
  client and protected two-peer example wait for socket readiness or the
  earliest deadline on Windows and Unix.

Windows passed 1,716 all-feature workspace tests. Loopback returned the expected
unprotected value and both protected examples verified all 200 bytes. An idle
socket initially returned before its deadline. The wait now re-arms after an
early timeout and passed 32 consecutive idle checks. Rust 1.89 and ESP32-S3
Xtensa library checks passed, including heap-free OSCORE. The device read comparisons used the integrated server. Later Windows runs also covered
the borrowed client API and revised association setup on the exact checkout.

The [chained-read measurements](architecture/requirements/CHAINED-READS-20261125.md)
retain each process window and boot. In four position-balanced capacity rounds
the integrated server used 6–10% less CPU per verified read than the original
`b8f44fa` across 64-byte and 4 KiB unprotected and established OSCORE reads.
At 800/s the small unprotected CPU result reversed between campaigns.
Both original 50% server CPU targets remain open. On the Waveshare, unprotected
2,000-byte reads with automatic Block2 took 16.91 ms with Coaptic and 13.29 ms
with libcoap. The cached-history-deadline experiment increased latency in all
three boot pairs and was removed. Native release images, process-local traces
and same-board controls retain their separate source identities.

The exact working source diff is `3eb1b84701050813796a3f07aa283aff75a1425b7e29d6c56c012727808b59a5`.
Four original/current pairs per workload verified 658,944 reads. At capacity
the current release served 1.8% to 10.1% more reads/s across the four cells.
Small protected p99 rose and small unprotected server/client CPU rose at 800/s.
Borrowing a 4 KiB result eliminated a caller-owned destination of that size.
One extraction per association reduced complete Windows setup by 43.1%.
The revised Xtensa setup has the same 1,360-byte deepest identified frame sum
as the original three-extraction code. In one image on two Waveshare boots,
6,144 complete setups used 52.7% to 53.0% less CPU and 96 fewer stack bytes
per worker across both inputs. The protected read worker used 14.6/38.4 ms
of CPU for 7/2,000 bytes, with 48.1% of observed CPU inside durable state I/O.
The protected option-reuse build traded wins across boots and was removed.

For unprotected reads the matched Waveshare worker clocks measured 2.94/6.72 ms
of CPU for Coaptic against 1.62/3.74 ms for libcoap. Ready-to-run time was
similar. Four more boots isolate the gap before the UDP send. The two workers
used 2.079 vs 0.834 ms there for seven bytes and 4.916 vs 2.095 ms for
2,000 bytes. Inside send the differences were 0.045 and 0.120 ms. The next
server attribution boundary is ingress/dispatch versus encode/replay bookkeeping.
A role-lookup experiment reduced a fixed-slot search from 11.4 to 8.2 ns,
but 454,144 complete reads did not support a client CPU gain. It was rejected.
Per-read energy needs a physical power meter.

The preceding packet-path fixture followed a multi-segment, multi-query GET through
Block2 reassembly. Server admission and Observe planning reuse the option
validation pass. The client encodes follow-ups from the live Call's retained
query bytes. An empty TX body pool skips its slot search. A retained response
carries its slot and metadata into the sender, which checks representation
identity before reusing the bytes. The client classifies notification freshness
and format once and records ACK replay only after an admitted response succeeds.
Against `161a22569c1d`, identical verified Windows fixtures counted codec
option-header reads for each 2,000-byte transfer. The unprotected 32-block
read went from 7,530 to 4,982 and the established OSCORE read with
negotiated blocks went from 274 to 158. The counts include both endpoints
and the test's wire checks. Outgoing body slot inspections went from
384 to 0 and 20 to 0 respectively. On x64 the borrowed Request view grew
from 304 to 328 bytes and each live Call row fell from 1,136 to 1,128 bytes.
Physical ESP32-S3 performance and the two original server CPU targets remain open.

The [Windows foreground server](architecture/requirements/WINDOWS-FOREGROUND-READS-20261122.md)
now waits for socket readability or the next App deadline. In three alternating
release-build pairs it sustained 800 offered reads/s for 64 B and 4 KiB in
unprotected and established OSCORE modes. Both original Tokio server CPU-time
targets remain open. A [281,600-read follow-up](architecture/requirements/WINDOWS-ESP-READ-VALIDATION-20261122.md)
repeated the foreground latency gain in all 16 pairs. Against the original
Tokio baseline at 800/s, protected 4 KiB reads used less server CPU in all
four pairs; unprotected 64 B reads used more in three and tied in one. The
ESP32-S3 was offline on November 22. On November 23 the
[fresh physical Wi-Fi campaign](architecture/requirements/ESP32-WIFI-READS-20261123.md)
verified 816 reads. The instrumented protected worker used 15.16 ms of CPU
for the 7-byte value and 38.88 ms for the 2,000-byte value. Normal firmware
was restored and identified after every trial.
The [24-boot libcoap comparison](architecture/requirements/ESP32-LIBCOAP-HEAD-TO-HEAD-20261123.md)
verified another 2,208 unprotected reads. Coaptic sync and libcoap traded
the single-read lead with the client. Libcoap finished pairs of overlapping
reads sooner and had more free heap. Coaptic async trailed libcoap on both
body sizes in every native-client rotation. The single-datagram device path
is the next profiling target. Both security modes still need baseline CPU evidence.
The [GET source audit](architecture/requirements/ESP32-GET-SOURCE-AUDIT-20261124.md)
traced the installed async binary and tested reusing the parsed Observe flag.
Two option walks disappeared and Windows loopback improved, but 2,176 physical
reads across eight boots gave mixed latency and worse overlapping small reads.
The production path retains its original behavior.
The [route and Block2 comparison](architecture/requirements/ESP32-ROUTES-BLOCK2-20261124.md)
verified 1,152 unprotected Waveshare reads across all six orders of baseline
Coaptic, a heapless route index and libcoap. The index cut a mixed 128-route
Windows lookup from 670 to 34 ns. With six registered device handlers its gains
varied with the request, and libcoap still served both ordinary reads faster.
An explicit 1,024-byte first block eliminated one failed encode and one TX
acquisition on a large response. The board's normal image was restored.
Passing an already matched route through Observe dispatch reduced complete
Windows registration time by 31% at the last of 128 routes. On the Waveshare,
960 timed operations across eight boots showed mixed registration latency and
slower small ordinary reads in three of four pairs. That packet-path change
remains experimental. The checkout keeps the independent route-removal change:
after the App drains, it frees the slot it already found. Removing and reusing
the last of eight routes fell from 266 to 175 ns and improved in all twelve
host pairs. The final Windows build sustained 800 verified reads/s in all four
cells. Against the original project, aggregate server CPU per read was 20%
and 5.5% lower for unprotected 64 B and 4 KiB, and 18.2% and 6.0% higher for
established OSCORE. Route removal is outside the GET path. Both 50% targets
remain open.
At equal delivered rates the original foreground comparison reduced
blockwise server CPU in both modes. Small-read server CPU medians increased.
Direct ACK encoding, duplicate-slot reuse, admission recounting and Mio rearm
each regressed at least one capacity workload. Reusing validated OSCORE option
bytes reduced reconstruction time but also lowered median protected read capacity.

The supported direct UDP App implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, commissioning, durable ownership and
application receipts. Software recovery coverage has improved. Selected current-source installed-device recovery cells are qualified. Broader
qualification and faster-than-libcoap/CPU goals remain unfinished. A completed batch is not project completion.

## Preceding software batch

- [#760](https://github.com/jeffglousher/coaptic/pull/760): same-App recovery across
  finite datagram blackouts, including identical retries, duplicate suppression,
  capacity refusal/recovery, cancellation and expiry, in sync/async and
  unprotected/OSCORE cells. This is software injection, not physical Wi-Fi proof.
- [#762](https://github.com/jeffglousher/coaptic/pull/762): bounded custom elective
  request options and the runnable sync/async `application_options` example,
  covering payload, Accept/Content-Format, deadlines and useful errors.
- [#763](https://github.com/jeffglousher/coaptic/pull/763): known-size NON Q-Block2
  trailing-loss recovery. The integrated core passes 1,711 all-feature tests,
  743 no-default library tests and 91 source-selected companion checks, plus
  strict lint, MSRV, rustdoc and an Xtensa OSCORE library check. Original
  capacities and whole-App ceilings remain; native body sidecar stays 688 bytes.
  See the [source-matched receipt](https://github.com/jeffglousher/coaptic/pull/763#issuecomment-6954242320).

No new performance improvement or firmware execution was established by that
batch. Hosted checks on #763 have since settled without failures. The recovery batch explicitly advances companion ordinary CI to accepted core
`b875088`; older CI proof is not relabeled.

## Latest device qualification

The source-matched [ESP32-S3 functional matrix](architecture/requirements/ESP-WIFI-RELIABILITY-20261117.md)
passed sync/async device service in unprotected and durable OSCORE modes:
96 complete timed reads, two nine-case independent unprotected campaigns and two
nine-case protected campaigns. Normal authorized firmware was restored and
verified after each cell. No core protocol fix or performance gain was established.
Two mismatched fixture assemblies were rejected and retained before corrected
sync qualification. All advisory checks on #765 settled without failure.

## Latest recovery batch

All four sync/async and unprotected/durable OSCORE device cells passed observable
duplicate effects, explicit Observe refusal/reuse, known-size NON Q-Block2 suffix
loss/repair and same-App Call retry/reconnect during a bounded per-device Wi-Fi
interruption. Normal firmware was restored and verified after every trial.
Q-Block2 loss is on host receive; MCU-client recovery and other pool exhaustion
remain separate. Calls started during the outage; pre-existing-at-disable Calls
are not established. The [source-matched recovery record](architecture/requirements/ESP-DEVICE-RECOVERY-20261118.md)
retains failures, exact scope, image/archive identities and independent review.

One bounded async OSCORE CPU diagnostic completed 24 authenticated reads and 32
matched exchanges, with 350 complete metadata rows and protected traffic resource
observations. It separates scheduled CPU, blocked and ready-unscheduled time;
readiness/send and available post-send tails have explicit boundaries. It is not
whole-App CPU, a normal-firmware peak-memory claim or a performance gain. No core
protocol fix or new optimization was required.

Implementation and evidence reviews are complete. No device/build job remains
active; normal firmware restoration is verified. Detailed custody and private
evidence locations belong in the batch handoff.

## Active performance and reliability effort

This is a continuation checkpoint. After verifying current state and custody,
execute the next unblocked batch in the same run; completed qualification and
advisory-check settlement alone are not reasons to end. Follow the
[continuation and handoff procedure](DEVELOPMENT.md#finish-a-batch-or-hand-off)
for actual stop or transfer conditions.

1. The [current Windows screen](architecture/requirements/WINDOWS-RUNTIME-SCREEN-20261119.md)
   covers both runtime and security modes, complete small and blockwise reads,
   process time and cycles, and an independent libcoap screen. High-rate pacing
   missed periods. Lower-rate cycles are more repeatable than process CPU time
   in the pilot. Two independent endpoints also complete overlapping CON Calls
   in all selected cells. The Tokio path remains a priority after the
   same-runtime sync control. Existing staged receive does not earn adoption.
2. The [Windows phase attribution](architecture/requirements/WINDOWS-UDP-PHASES-20261119.md)
   records 282,080 verified reads and a further 256,000-read matched Windows
   reactor screen. A smaller completion dequeue bound reduces small-read
   server cycles. Four blockwise pairs miss the advancement gate, so retain
   the default bound. The [64-entry follow-up](architecture/requirements/WINDOWS-IOCP-BOUND-20261119.md)
   establishes a Windows API argument effect and rejects the 64-entry
   candidate on complete operations. Companion
   [#268](https://github.com/jeffglousher/coaptic-validation/pull/268)
   proves same-process oversized receive recovery in 12 Windows cells.
   Resetting the async burst budget after a pending wait also misses the
   gate. Retain the current Tokio scheduling policy.
3. The [matched peer screen](architecture/requirements/WINDOWS-PACED-PEERS-20261119.md)
   establishes a protected median latency advantage in these Windows cells
   and a 600/s eligible rung for protected blockwise C1. The 800/s rung
   misses the pacing guard. The [paired-start follow-up](architecture/requirements/WINDOWS-PAIRED-PEERS-20261119.md)
   verifies overlapping 4,096-byte operations across two endpoints and
   passes the protected 800/s rung. The 1,000/s rung misses pace in the
   libcoap arms. Keep the original #650 baseline, both 50% targets and
   physical prerequisites.
4. The [Tokio loopback tests](tools/coaptic-tokio/README.md) exercise body
   refusal and reuse, Call refusal and established OSCORE binding refusal.
   Keep #130 open for the remaining runtime/security cells and device
   qualification. Reuse settled recovery evidence where it applies.
5. The [standalone async-io screen](architecture/requirements/WINDOWS-HOST-ADAPTERS-20261119.md)
   passes selected small-read cycle pairs and misses the blockwise gates.
   Both adapters now report oversized receives as InvalidData on Windows
   and prove same-socket reuse.
6. The [common Mio screen](architecture/requirements/WINDOWS-ORIGINAL-MIO-20261119.md)
   compares current core with the original #650 source. Both protected
   4,096-byte pairs reduce server cycles, with inconsistent CPU-time
   ratios. The unprotected 4,096-byte pairs fail the p99 guard. Keep
   the original matrix and both targets open. The
   [tail follow-up](architecture/requirements/WINDOWS-MIO-TAIL-20261119.md)
   reproduces the clean failure. The probe changes the tail and its
   phase timings leave the cause open. Keep the current runtime settings.
7. The synchronous standard UDP transport now reports oversized
   receives as InvalidData. The same App completes a 2,000-byte Block2
   read after that refusal. Preserve the fixed and allocated host
   recovery evidence with the async adapter proofs.
8. The [fixed Service peer screen](architecture/requirements/WINDOWS-SERVICE-PEERS-20261120.md)
   verifies 16 KiB reads with four overlapping endpoints. The 400/s
   unprotected and 200/s OSCORE rungs pass. Both modes complete all
   reads at their next rungs and miss pacing or overlap. The protected
   peer comparison includes libcoap's separate responses and Coaptic's
   piggybacked responses. Companion
   [#270](https://github.com/jeffglousher/coaptic-validation/pull/270)
   makes group starts and response accounting repeatable. The
   [loss check](architecture/requirements/WINDOWS-PROTECTED-RESPONSE-RECOVERY-20261120.md)
   verifies exact retry and reply replay plus a fresh read across both
   runtimes, security modes and body sizes.
9. The [Linux C1 comparison](architecture/requirements/LINUX-SERVICE-CPU-20261120.md)
   restores the historical AES lock and verifies all 24,000 fixed Service
   reads. Both protections fail a pair's latency gate. Keep both CPU
   ratios withheld. Companion #272 adds native group starts and #273
   enforces latency guards in short runs. The [fixed Service C4 comparison](architecture/requirements/LINUX-SERVICE-C4-CPU-20261120.md)
   verifies all 24,000 reads and all four endpoints in each group.
   Unprotected server CPU time falls 11.9% with all latency guards met.
   Established OSCORE fails median latency guards and its ratio stays withheld.
10. The [Windows fixed Service App control](architecture/requirements/WINDOWS-SERVICE-APP-20261120.md)
    verifies 72,000 measured reads against those same core revisions.
    Server poll wall medians fall in all pairs. The unprotected 16 KiB
    p99 also falls in all four pairs. The clean socket tail cause stays open.
11. The [Windows affinity control](architecture/requirements/WINDOWS-PROCESS-AFFINITY-20261120.md)
    verifies all 172,800 measured reads. All pairs miss pacing, and all
    operation p99 observations rise with a single-CPU server. Keep the
    default placement.
12. The [Windows primary-thread results](architecture/requirements/WINDOWS-PRIMARY-THREAD-CYCLES-20261120.md)
    charge every sampled server cycle to the primary thread. Separate
    process-only controls limit the runtime comparison. The App, runtime
    and kernel costs within that thread stay unresolved.
13. The [Linux SmallDevice comparison](architecture/requirements/LINUX-SMALLDEVICE-CPU-20261120.md)
    verifies all 480,000 body-free 32-byte reads. Both protection modes
    fail p99 guards at 1,000 starts/s and withhold CPU-time ratios.
    Companion [#274](https://github.com/jeffglousher/coaptic-validation/pull/274)
    separately verifies four occupied 16 KiB receive bodies, cancellation
    and expiry, late replies and successful replacement Calls on the same
    endpoints in both protection modes.
14. The [empty readiness scan candidate](architecture/requirements/WINDOWS-EMPTY-READINESS-SCAN-20261120.md)
    verifies 96,000 fixed Service reads. Only one of 16 pairs meets every
    advancement gate. Preserve the rejected source and keep the accepted
    App wake path.
15. The Tokio loopback test drops the first mutating reply in each
    protection mode. It verifies the same retry and reply bytes, one
    server effect per Call, and a second completed mutation on the same
    sockets. The test drives protocol time and uses real UDP readiness.
16. The [allocated Service C4 comparison](architecture/requirements/LINUX-SERVICE-ALLOCATED-C4-CPU-20261120.md)
    verifies 24,000 reads and all pair and pooled latency guards. Seven
    windows miss the CPU accounting floor. Both ratios stay withheld.
17. The [Windows direct Mio screen](architecture/requirements/WINDOWS-DIRECT-MIO-20261120.md)
    verifies 115,200 reads, with a 2,048-byte smaller App owner. Only
    one of 16 pairs meets every advancement gate. Keep the guarded path.
18. A Tokio UDP test fills the server's only Block1 receive body slot with
    a partial upload in each protection mode. A second upload receives a
    refusal with no handler effect. The first and a fresh third upload
    each deliver the complete tagged body once and release the slot.
19. The [allocated Service long-window comparisons](architecture/requirements/LINUX-SERVICE-ALLOCATED-C4-LONG-CPU-20261120.md)
    clear the CPU accounting floor and verify all 57,600 reads. Both
    modes fail a latency guard and withhold the CPU ratios.
20. The [Windows Tokio timer-poll probe](architecture/requirements/WINDOWS-TOKIO-TIMER-POLLS-20261120.md)
    verifies 157,440 reads and counts a pending progress timer poll per
    send. Timing perturbation limits the comparison. Keep the current
    timer policy and preserve the earlier timer results.
21. The [Linux process-clock audit](architecture/requirements/LINUX-ALLOCATED-PROCESS-CLOCK-AUDIT-20261120.md)
    records lower alternate CPU time in every 30-second pair under both
    protections. The original CPU ratios stay withheld. Run a same-binary
    control at the 72-second workload to quantify repeatability.
22. The [Service storage control](architecture/requirements/WINDOWS-SERVICE-APP-BACKENDS-20261120.md)
    verifies all 96,000 socket-free reads. Fixed Service has a lower
    16 KiB poll-wall median in each pair. Keep both backends and use
    separate measurements for Linux sockets and process CPU time.
23. The [Linux identical-binary control](architecture/requirements/LINUX-ALLOCATED-SAME-BINARY-20261120.md)
    verifies 28,800 reads and clears every accounting floor. The median
    and p99 guards still fail on identical code. Preserve the original
    comparisons and establish repeatable matched conditions before
    qualifying more long-window CPU deltas.
24. The [Windows thread-dispatch probe](architecture/requirements/WINDOWS-THREAD-DISPATCH-20261120.md)
    verifies 314,880 reads. Tokio switches contexts less often for
    blockwise replies and still records more cycles per send. Check
    a single-reader readiness alternative within the Tokio sync path.
25. The [Tokio single-reader screen](architecture/requirements/WINDOWS-TOKIO-SINGLE-READER-WAIT-20261120.md)
    verifies all 209,920 reads. None of the 16 pairs meets the
    advancement gate. Keep the existing readiness wait.
26. A Tokio UDP test drops the final Block1 reply after the assembled
    body is released. Both protection modes replay the exact reply
    without a second effect, then reuse the slot for another upload.
27. The [Linux peer-placement control](architecture/requirements/LINUX-PEER-CPU-PLACEMENT-20261120.md)
    verifies 28,800 reads with stable masks. Separate guest cores still
    fail three pair medians and the pooled median. Retain the control
    and keep the existing placement. The next qualifying CPU comparison
    needs repeatable matched windows under the original latency guards.
28. The [Windows read profile](architecture/requirements/WINDOWS-READ-PROFILE-20261120.md)
    verifies 614,400 reads and measures thread cycles in App bursts, socket
    calls and the readiness path. Neither the option lookup nor the OSCORE
    sealing candidate reduces CPU time consistently. Both were reverted.
29. The [Windows async adapter profile](architecture/requirements/WINDOWS-TOKIO-ADAPTER-PROFILE-20261120.md)
    measures send and receive polls on the production Tokio adapter. Direct
    polling did not repeat a server CPU-time gain across complete reads.
    With the original project fixture, unprotected server cycles fell in all
    six pairs and CPU time varied in direction. Established OSCORE server
    CPU time fell in all three small-read pairs and rose in all three
    blockwise pairs. Both original #650 CPU-time targets remain open.
30. The [Windows option and duplicate-table profile](architecture/requirements/WINDOWS-OPTIONS-DEDUP-PROFILE-20261120.md)
    separates sampled server-thread waits from running and standby states.
    An identical binary recorded a 50% server CPU-time reduction in one
    protected blockwise pair at 799/s, with a 0.9% cycle reduction.
    Neither pacing rate established repeatable CPU-time accounting for
    both cells on this host. Keep the original targets separate and open.

## Blocked and deferred work

- [#333](https://github.com/jeffglousher/coaptic/issues/333): physical cold-power and
  interrupted persistent-write recovery requires physical power control.
- [#365](https://github.com/jeffglousher/coaptic/issues/365): installed three-credential
  backup-owner recovery requires the operator's deferred credential touch.
- [#341](https://github.com/jeffglousher/coaptic/issues/341): reverse MCU-to-host LAN
  cells require scoped inbound UDP access. Host-to-MCU proof does not complete them.
- [#650](https://github.com/jeffglousher/coaptic/issues/650): CPU reduction and the
  broader peer-speed objective remain open. No new candidate qualified in the last batch.
- [#647](https://github.com/jeffglousher/coaptic/issues/647): volatile plaintext and
  zeroization policy is future hardening research, not an established remote defect.
- Runtime adapter development is complete for the preceding optimization batch.
  Embassy firmware migration, new platforms and unrelated tooling remain deferred.
  Publication, tags and outreach need separate authorization under
  [#132](https://github.com/jeffglousher/coaptic/issues/132).

## Retained decisions and evidence

The accepted packet-lifetime batch measured 2.91% lower complete-App CPU cost sync
and 3.28% async against its pinned baseline, with smaller selected futures and App
owners. These are [source-specific historical results](architecture/requirements/PACKET-LIVE-RANGES-20261106.md),
not fresh device or current-main measurements.

Quiet-dispatch, timer-lifetime, cache-size and target-layout integrations failed
preservation or speed gates. Keep the
[failed sources, raw vectors and limits](architecture/requirements/ESP-TIMER-LIFETIMES-20261117.md)
and [earlier experiment history](architecture/requirements/PROJECT-HISTORY-20261117.md).
Do not rerun unchanged failures or use host results from rejected code as accepted
current-source performance. The rejected extra Q-Block2 mode field also exceeded
the whole-App ceiling; its smaller receive-state representation is the accepted fix.

[Protocol scope](architecture/IETF-RFC-PLAN.md), the
[qualification checklist](architecture/requirements/VALIDATION-PLAN.md), and
[runtime/comparison policy](architecture/ASYNC-RUNTIMES.md) define supported
behavior and valid claims. Keep this page short and current; detailed acceptance
belongs in issues and execution history in evidence documents.
