# Libcoap speed investigation and retained negative evidence

The follow-up to accepted `a85c51e689607051e36e450567fe051f8f85dc2b` has
not yet qualified another library implementation. The accepted packet-lifetime
batch remains intact. Eight complete-App CPU integrations and the completed
native layout screens below remain unadopted. Passing source checks, smaller
frames and removed copies do not override their failed runtime bounds.

The [portable evidence and offline verification commands](https://github.com/jeffglousher/coaptic-validation/tree/47ca9e024c365a795eb22259a67d30191bdbac05/tools/benchmark/evidence/esp-libcoap-speed-20261109)
retain all completed cohorts below, source patches, fixture hashes and frozen
limits. Verification reproduces aggregate ratios and promotion failures;
hardware execution and excluded private witnesses remain producer attestations.

## Complete-App CPU comparisons

Each integration uses the same pinned accepted baseline, 28 workloads and 64
paired process rounds, with actual synchronous/asynchronous execution,
unprotected/OSCORE operation and fixed/allocated storage. Each cohort measures
14,680,064 App operations in 3,584 timed processes, plus 56 qualification
processes. Together these eight cohorts contain 117,440,512 timed operations;
this is a finite CPU-fixture count, not hardware or reliability qualification.

Ratios below are candidate CPU cost divided by the pinned Rust baseline.

| Integration | Sync ratio | Async ratio | Failed workload bounds |
| --- | ---: | ---: | ---: |
| Body inspection and terminal snapshot | 0.99775 | 1.00355 | 4 |
| Terminal snapshot only | 1.00751 | 1.00517 | 9 |
| Broad poll boundaries | 0.99844 | 1.09187 | 14 |
| Cold poll boundaries | 0.99988 | 1.00551 | 5 |
| Compact expiry reasons | 0.99709 | 1.00288 | 7 |
| Packet views and path filling | 0.99474 | 0.99335 | 2 |
| Borrowed handler inputs | 0.99211 | 0.99021 | 2 |
| In-place request context | 0.98558 | 0.98820 | 1 |

The final request-context integration improves aggregate CPU cost by 1.44%
sync and 1.18% async. Its retained, fixed, unprotected sync workload has point
ratio 1.01594 and one-sided 95% upper bound 1.03137, above the frozen 1.02
preservation limit. It remains unmerged. The selected complete Constrained
future stays 13,448 bytes; its local native-frame and emitted-copy improvements
do not establish a peak-memory reduction.

These are Rust-to-Rust CPU comparisons. They are not libcoap comparisons and
must not be substituted for socket throughput or device latency.

## Host peer comparison

The November 7 comparison uses accepted `a85c51e`, seven implementations,
four body sizes, three concurrency levels and three rounds: 252 cells and
564,480 verified completions, with zero operation failures. Geometric goodput
ratios versus the pinned libcoap implementation across that declared mix are:

| Implementation | Goodput / libcoap |
| --- | ---: |
| Coaptic spin | 1.49001 |
| Coaptic Mio | 1.29620 |
| Coaptic async | 1.11966 |
| coap-rs | 0.94648 |
| libcoap | 1.00000 |
| go-coap | 0.52647 |
| aiocoap | 0.09631 |

Spin occupies a busy core while idle. At 64 bytes and concurrency one,
Coaptic async and libcoap goodput are nearly tied and async latency is worse.
Aggregate throughput is not an every-cell latency or CPU-per-operation claim.
Sampled process private memory after load is separate from library owners,
requested pools and whole-system peak memory. Different service capabilities,
runtimes and allocation strategies remain explicit comparison limitations.

## ESP32-S3 Wi-Fi comparisons

Device timings include the Windows Ethernet initiator, access point and
ESP32-S3 Wi-Fi server. They measure complete, body-verified unprotected GET
and Block2 operations, not isolated parsing, library CPU or MAC retries.
Both actual Rust server modes and both fixed Windows client modes run.

The earlier native-hot and pipeline screens use core
`c0151713fc187a4a2b8ed3cba1eb0b12df597d00` and the existing runtime integration
`32f8fcac22fc16b1957c35f4f145df6be3a72532`. Their host-expanded core source is
byte-identical to accepted `a85c51e`; this permits reuse of the named accepted
host results, not an invented new host measurement. Later source changes need
their own source and runtime evidence.

Rust uses Constrained capacities: eight routes, seven used including discovery;
four Calls; zero deferred slots; two 1,152-byte datagrams in each direction;
one 4,096-byte body slot in each direction; four dedup entries; two Observe
entries; eight-byte tokens. Original workers remain 96 KiB sync and 192 KiB
async. The C controls match the declared static GET/Block2 workload with
dynamic PDU/session allocation and a 16 KiB worker. This is not equivalent
whole-library feature, capacity, security or memory support.

The earlier complete native-hot screen retains 1,536 reads across 16 boots.
Selected sync and async layouts have strict strongest-C geometric latency
ratios 1.04694 and 1.16078. Their own-Rust baseline improvements did not
qualify the C-speed objective.

The later pipeline screen retains exactly 1,152 reads, 12 boots and 48 client
processes: six images in two opposite complete boot orders. Each of six
client/body cells has 32 observations nested in two device boots. Both strong
C controls and prior Rust controls are contemporaneous; older timing vectors
are not pooled into this screen.

| New layout | Latency / strongest C | Latency / own Rust control | Static RAM change / own control |
| --- | ---: | ---: | ---: |
| Sync O3/thin, larger IRAM pipeline, cache32 | 0.91852 | 0.93912 | +4,608 B |
| Async Os/fat, larger IRAM pipeline, cache16 | 1.55056 | 1.10943 | -2,048 B |

Sync passes both boot-level aggregate comparisons against each C control but
fails the all-cell gate: async-client identity and test ratios are 1.02457
and 1.01985. Async fails the six-cell and boot-level gates. Neither layout
is adopted. Cache size and code placement changed together in the async
configuration; this does not isolate a cache-miss cause.

All planned reads completed. Observed client failures, repeated CoAP sends,
unexpected API disconnects, connection refusals, Rust drops and wait errors
were zero. Reported RSSI was -45 to -42 dBm. Boot variation remains material;
finite samples do not establish long-duration or RF stability. Normal firmware
was restored and its identity verified after device work.

Physical static RAM counts union instruction/data aliases and count cache
reservation once. Reserved workers, sampled working-stack use, heap observations,
selected futures and local function frames are separate observations. None
justifies lowering task reservations or claiming whole-system peak savings.

## Exact-H integration, 9 November UTC

The combined idle-Observe fast path and cache32 placement uses core
`de9230aa22dab0c67604ea4b040a1c1daa59a41e` and runtime 32f8. It is a distinct
source from the preceding annotation-only c015 profiles. Exact-H Observe tests
(113), the protected Xtensa check and both original-stack startup qualifications
pass. Each startup qualification verifies 48 reads, excluded from performance.

The fresh host cohort retains 180 cells and 403,200 verified operations with
zero failures: H Mio/async, accepted a85 Mio/async and pinned libcoap, using the
same twelve cases in three rounds. Geometric goodput/C ratios are 1.30214 Mio
and 1.12069 async; every round aggregate is above C. Against contemporaneous
a85, goodput ratios are 0.97949 and 1.01798. These mixed throughput point
estimates establish no isolated CPU gain or per-cell preservation interval.

The separate Wi-Fi cohort retains all 1,152 reads, twelve boots and 48 client
processes, using both actual Rust modes, both Windows clients and opposite boot
orders. All profiles retain cache32 and their original workers and capacities.

| H server | Latency / strongest C | Latency / own Rust control | Static SRAM change / own control |
| --- | ---: | ---: | ---: |
| Sync O3/thin | 1.06576 | 1.16421 | -3,584 B |
| Async Os/fat | 1.47413 | 0.94793 | -256 B |

Both H profiles fail their frozen C-speed gates and remain unadopted. Sync saves
memory but has 16.42% higher pooled latency than its Rust control. Async has
5.21% lower latency than its Rust control and saves 256 bytes, but is still
47.41% above the strict C reference. Boot variation is material; the older sync
aggregate lead is not a result for this source or a replacement for this failure.
All operation, resource and finite stability guards pass, and exact normal
firmware restoration is verified. These facts do not establish RF or
long-duration stability, isolated task CPU or whole-system peak memory.

The [fresh portable package](https://github.com/jeffglousher/coaptic-validation/tree/021b691ed34167e488bde6915561613c98ae9455/tools/benchmark/evidence/esp-idle-observe-20261109)
retains every host/device latency, qualification separately, source patches,
hashes, limits and offline regrading. Seventeen counterexamples check integrity,
privacy and gates, including exported CPU/cache/task/settling policy. Numeric
guards are reproducible; source-to-binary and physical identity/restoration
witnesses remain private producer attestations. Only instruction-cache settings
are frozen per image; data-cache defaults are reconstructed, not individually
attested. No H source/profile adoption follows from the verifier passing.

Companion [#258](https://github.com/jeffglousher/coaptic-validation/pull/258)
delivers the existing ESP runtime's task-local notification handling and a
test-only future-deadline regression. One atomic word joins notification and
parking; registered child I/O/deadlines still receive nonblocking probes during
local work. It adds four bytes of shared state. The production blob matches
runtime 32f8 used by the native cohorts; the later test does not change images.
Focused network/OSCORE and pairing driver checks pass 11/11 and 12/12 against
accepted a85 core. Existing unchanged-production Clippy/Xtensa evidence is reused.
This delivery claims no isolated complete-App CPU gain for the runtime change.

## Common MCU processing diagnostic, 9 November UTC

The [completed common-boundary package](https://github.com/jeffglousher/coaptic-validation/tree/a50f202ed2ed289e391ec2cc358498954b55f6db/tools/benchmark/evidence/mcu-processing-span-20261109)
retains 192 verified reads, 256 fully matched packet spans and all four profiles.
There is one boot per profile and two fixed Windows clients. Each profile has
16 complete operations per path; each large operation sums its two associated
Block2 spans before the median is taken. Median receive-return to send-entry
wall times, in microseconds for identity / seven-byte test / large body, are:

- H sync O3/thin: 570 / 405 / 1,232.5.
- H async Os/fat: 1,335 / 1,255 / 2,984.
- C pinned: 481 / 497.5 / 1,419.
- C floating: 498 / 487.5 / 1,353.

All 256 spans contain zero native select calls. Two async delay calls total
1,139 us out of 92,358 us of async processing-span wall; all other delay totals
are zero. Receive-call and send-call wall lie outside this processing boundary.
The spans include preemption and RX observer work; paced post-send logging can
perturb later cache/work. Residual wall is not task CPU. Different code-generation
profiles and task policies prevent isolating language or async overhead. Do not
subtract these medians from unrelated Wi-Fi latencies to invent a transport cost.
The ESP32's Wi-Fi, AP and Windows path remain inside the separate complete-read
measurements. This diagnostic contributes no speed-promotion samples.

These images use exact H core de9230 and runtime 32f8, not the next c015-based
candidate. Each diagnostic image attests actual 32 KiB I/D caches, eight ways,
32-byte lines, a 240 MHz CPU and a 1,000 Hz tick. VFS select support is enabled only for H async;
no select calls were observed inside the consumed-RX spans. Rust's actual worker
core is unavailable in API output; compiled policy is floating. C pinned and
floating both report startup core1, with compiled policies core1 and floating
respectively. Startup observations do not establish continuous task placement.

Original capacities and 96/192/16 KiB worker reservations remain. H async's
200,704-byte largest internal block before task creation exactly meets the
inclusive floor of its 196,608-byte worker plus 4,096-byte margin. All running free/largest/regional-low-water
floors pass. The observer context is 2,512 bytes inside the worker stack, with
28 global bytes inside static memory; aligned static deltas are 24 bytes for
sync/C and 32 for async. These nested observations do not establish minimum
stacks, whole-system peak RAM or permission to reduce the original reservations.

The V1 incomplete publication and original V2 C archive-regeneration refusal
remain failed. A separate guarded controls inventory supplies the completed V2
images. Normal firmware is restored and verified. The portable verifier
reconstructs every operation and twelve medians, with 25 focused counterexamples;
completed codec, actual-source C++ mock and controller/target checks are reused.
Full packet/body/firmware/identity/API joins remain private producer attestations,
not an independent firmware or packet rebuild kit. The H speed gates above still
fail, and H remains unadopted.

## Target CPU integration, 9 November UTC

The [completed portable screen](https://github.com/jeffglousher/coaptic-validation/tree/098d86b7106bc6a1ffdf6e610a51b6d578792f54/tools/benchmark/evidence/esp-target-cpu-20261109)
retains all 1,152 complete reads, 48 fresh client processes and twelve boots.
Both actual Rust modes use c015 core and runtime 32f8, O3/thin LTO, selected
IRAM placement and an explicit compiled core1 request. The two contemporaneous
C controls and earlier Rust controls run in opposite complete boot orders.
These are complete-read Wi-Fi latencies, not task CPU measurements or a new
host comparison.

Sync's strict strongest-C geometric ratio is 0.85375. Its three sync-client
identity/test/large ratios are 1.04390, 1.01262 and 1.01595, so it fails the
all-cell gate despite the aggregate lead. Its own-Rust ratio is 0.98407 with
mixed individual cells. Async's strict C ratio is 1.48337; all six cells and
both boot comparisons fail. Its own Os/fat ratio is 1.14253. Both candidates
remain rejected and must not be retested unchanged.

Async physical net static addition falls from 9,984 to 8,960 bytes, and observed
lifetime stack use falls from 112,056 to 88,564 bytes. Its reserved worker stays
196,608 bytes while its image grows 255,984 bytes to 1,439,088. Sync retains
60,928 bytes of net static addition and its 98,304-byte worker; its image grows
304 bytes. C workers remain 16,384 bytes. These observations do not release a
stack reservation or establish minimum RAM, peak use or an isolated CPU cause.
C running-heap values remain unavailable; terminal cleanup is a separate phase.

All planned reads and finite resource/stability guards pass, with no observed
repeated request sends, Rust drops, wait errors, API refusal or unexpected
disconnect. RSSI ranges from -54 to -34 dBm; the final pinned-C boot reports
-54 dBm and the other boots -34 to -41 dBm. Preserve that uneven RF observation
and the per-boot mapping. Operational completion does not establish controlled
radio conditions or long-duration stability.

Compiled core1 is not an observed Rust task-core result: the one-shot startup
records are absent from these captures. C records startup core1, including its
floating control. The exact worker-plus-4,096-byte pre-task contiguous margin
is unqualified in these speed-screen images; running heap guards do not
replace it. New candidate I/D cache and SDK settings are attested per image;
missing data-cache/tick facts in older controls remain unqualified. Source,
binary, physical identity and API joins remain private producer attestations.
The public verifier reconstructs all cell/tail/boot results, ten comparisons,
both original failed decisions and the memory ledger. It reuses 21 focused
counterexamples and 39 controller checks. Normal firmware is restored and
verified; no source/profile adoption follows from offline arithmetic passing.

## Continue from measured costs

Preserve the exact sources, methods, failures and portable measurements. CPU
exports may use exact per-process cycle sums/counts and all eligibility fields
to reproduce aggregate ratios and workload bounds; that does not reconstruct
per-operation CPU tails. Full original CPU vectors remain retained separately.
Whitelisted peer-latency vectors retain every observation. Firmware, ELF,
credentials, private configuration and raw device API captures are not public
evidence artifacts.

The idle-Observe and subsequent target CPU integrations above are rejected; do
not repeat them unchanged. The common-boundary result supports investigating
the large async processing cost, which persists for tiny requests. The timer
prototype remains untimed and on hold. Next identify actual copy owners and
executed paths before another source or layout integration. Preserve original
workers, capacities, security and restoration guards.

The local empty-control extraction is checked and untimed, not adopted. It
reduces the selected Os/fat dispatch frame from 13,200 to 9,808 bytes, while
adding a separate 4,768-byte helper frame; the selected root future remains
9,896 bytes. Selected poll/dispatch/helper/drop code including literals grows
1,731 bytes. Host regressions and strict lint pass, but these static changes
do not establish runtime or peak-memory gains and do not justify promotion.
The source and its exact archive comparison remain retained as a local experiment.

The full objective still requires reproducible host and ESP32 gains in both
modes, preservation of supported behavior and capacities, independent
confirmation, reviewed delivery and verified normal-firmware restoration.
Runtime adapter development is complete for this batch. Embassy firmware
migration, new platforms and unrelated tooling remain deferred.
