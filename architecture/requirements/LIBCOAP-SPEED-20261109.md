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

## Continue from measured costs

Preserve the exact sources, methods, failures and portable measurements. CPU
exports may use exact per-process cycle sums/counts and all eligibility fields
to reproduce aggregate ratios and workload bounds; that does not reconstruct
per-operation CPU tails. Full original CPU vectors remain retained separately.
Whitelisted peer-latency vectors retain every observation. Firmware, ELF,
credentials, private configuration and raw device API captures are not public
evidence artifacts.

The combined cache32 placement and idle-Observe integration above is rejected;
do not repeat it unchanged. Next compare MCU socket receive-return to send-entry
wall spans at the same boundary in Rust and C, retaining packet identity,
failures and coverage. Such spans include instrumentation and preemption; they
are not isolated task CPU. Historical Rust-only traces do not supply the missing
C comparison. Use the common-boundary result before another ownership or layout
change, preserving the original workers, capacities and restoration guards.

The full objective still requires reproducible host and ESP32 gains in both
modes, preservation of supported behavior and capacities, independent
confirmation, reviewed delivery and verified normal-firmware restoration.
Runtime adapter development is complete for this batch. Embassy firmware
migration, new platforms and unrelated tooling remain deferred.
