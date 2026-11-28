# How much it costs to install an authenticated association

A server that had just authenticated a device used to check its association table twice while installing that device. Each admission check looked separately for a primary context, an identity conflict and a vacant row. Coaptic now borrows each row once, keeps the first vacancy, and continues checking occupied rows for conflicts. The exclusive borrow stays valid while the authenticated context moves into the chosen slot. The generation advances and the durable checkpoint becomes invalid only when insertion succeeds.

At 32 configured slots, two real EDHOC handoffs went from **130 and 132 row visits to 32 each**. The instrumented test counted visits. Separate release builds with counters off measured time. The baseline is the project at `75a603c`.

## Authenticated installation

The benchmark completed 17,600 fresh EDHOC handshakes and verified 795,520 successful or refused installation attempts. Each success consumed a fresh authenticated candidate. Each refusal checked the error and retained candidate. Timing starts after cryptographic authentication and includes authorization, admission and the handoff to App storage. Setup and recycling took place outside the clock interval.

Times below are median nanoseconds per installation or refusal across ten alternating pairs. Lower is better. The percentage compares the two builds within each pair. Rounded column medians can therefore give a slightly different percentage.

| Slots | Result | Original ns | Borrowed ns | Paired change | Faster pairs |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | First vacancy | 131.2 | 114.3 | -13.7% | 8/10 |
| 1 | Capacity refusal | 16.4 | 13.2 | -20.1% | 10/10 |
| 1 | Owner conflict | 16.3 | 13.1 | -18.9% | 10/10 |
| 1 | Unauthorized | 12.3 | 10.3 | -15.4% | 10/10 |
| 32 | First vacancy | 414.1 | 166.7 | -60.9% | 10/10 |
| 32 | Last vacancy | 397.1 | 164.3 | -57.3% | 10/10 |
| 32 | Capacity refusal | 122.1 | 62.8 | -48.0% | 10/10 |
| 32 | Owner conflict | 31.8 | 15.5 | -51.8% | 10/10 |
| 32 | Recipient ID conflict | 128.1 | 72.0 | -43.9% | 10/10 |
| 32 | Unauthorized | 12.1 | 10.3 | -15.6% | 10/10 |

At 32 slots, successful installation took less than half its previous time in both vacancy layouts. At one slot, eight of ten successful pairs improved. A second cohort using the same installation machine code in the preliminary source layout found 57.0% and 53.7% less time for first and last vacancies at 32 slots, again in ten of ten pairs. Unauthorized requests return before reaching the changed loop. Their roughly two-nanosecond shift shows the scale of unrelated differences on a short path.

## Direct App installation

A separate caller already has a security context and calls the public App installation method directly. Three independent runs each verified 4,608,000 operations. The first two used the preliminary borrowed implementation and reversed the starting build order in the second run. The last used the final source. All three direct-install executables for the borrowed method have identical machine code in the text section. Each interval covered 32 calls with the same fixed capacity, features and counter-free release profile. Timer controls were about one nanosecond per operation.

| Slots | Result | Earlier paired changes | Final original / borrowed ns | Final change | Faster pairs across all runs |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | First vacancy | -3.5%, +2.9% | 16.2 / 16.6 | +2.1% | 14/30 |
| 1 | Capacity refusal | -8.7%, -8.3% | 10.8 / 9.9 | -7.7% | 28/30 |
| 1 | Owner conflict | -8.1%, -6.1% | 10.5 / 9.8 | -6.5% | 26/30 |
| 1 | Recipient ID conflict | -7.0%, -6.6% | 11.0 / 10.3 | -6.2% | 30/30 |
| 32 | First vacancy | -29.9%, -29.1% | 186.9 / 131.2 | -30.3% | 30/30 |
| 32 | Last vacancy | -30.5%, -32.3% | 190.2 / 131.8 | -32.5% | 29/30 |
| 32 | Capacity refusal | -36.5%, -37.1% | 182.1 / 114.8 | -36.7% | 30/30 |
| 32 | Owner conflict | -56.0%, -55.4% | 28.3 / 11.8 | -58.5% | 30/30 |
| 32 | Recipient ID conflict | -32.4%, -35.9% | 189.1 / 119.7 | -37.5% | 30/30 |

The 32-slot cases improved in at least 29 of 30 pairs each. At one slot, direct successful installation moved in both directions. Established reads and full handshakes have separate measurement boundaries.

## Why removing loops alone failed

The first single-pass implementation read the embedded association by value. In source-matched Windows x64 assembly, each occupied-row owner comparison stored 32 bytes onto the stack and reloaded them. A 32-slot first-vacancy install rose from 184.3 to 373.9 ns. Last vacancy, full and late recipient conflict also took about twice as long, each slower in ten of ten pairs. That version was rejected.

A smaller intermediate implementation reused the staged preflight and kept the original direct admission algorithm. It cut the authenticated first- and last-vacancy times by 40.9% and 40.1%, respectively, in ten of ten pairs. Direct public calls showed little movement.

The retained single-pass implementation changes the row access to `entry.association.as_ref()`. Its release assembly compares the owner without that spill and reload. The frame still reserves 552 bytes below saved registers, the same as the by-value trial and above the original 408. The retained function spans 2,048 bytes versus 1,549 for by-value and 3,218 for the original. The measured comparisons distinguish work done on each occupied row from function size or stack reservation alone.

## Correctness

The staged-session regression presents an unauthorized candidate to a full table, then a valid candidate to the same full table. It checks both refusal reasons and candidate preservation, revokes the installed association, retries successfully and verifies the old handle stays inactive. The two-owner integration case covers protected blockwise operation and reuse with a hole. Checkout builds passed 1,727 workspace tests, 753 no-default library tests, 969 allocating OSCORE library tests and Clippy with warnings denied. These suites overlap.

[Raw original and trial windows, fixture sources, binary hashes and checks](MANAGED-INSTALL-20261128.json). [Verified UDP reads, process CPU, latency and memory](THROUGHPUT-AND-LOOKUP-20261128.md).

## Why the placement of the new implementation mattered

The first borrowed implementation also moved four packet-path functions in the Windows benchmark executable. Adding the new implementation before the association lookup implementations changed compiler-generated symbol ordinals and their linker order. Although managed installation is outside the timed read path, this binary's unprotected 4 KiB read rate fell 1.8% against the previous release in five of six pairs. An independent eight-pair run again measured a fall, 1.2% in the median pair with six pairs slower. The initial 40.6% increase in paced blockwise server CPU and 17.1% increase in small-read capacity p99 did not recur. In the repeat they changed −2.3% and −0.6%. All 192 windows, including unfavorable outcomes, are retained in the receipt.

Moving the new implementation below the existing implementations preserved the previous packet-path symbols. The rebuilt established-read server matches the previous release in all 7,851,745 executable text bytes and all 172,944 bytes of unwind tables. All 81 differing read-only data bytes belong to source locations, CodeView identifiers or debug directory timestamps. Meanwhile, the complete executable text of the two installation benchmarks matches the faster borrowed-implementation builds, 162,123 bytes for direct admission and 457,822 for staged EDHOC. This removes the suspected layout effect without giving up the measured install improvement. Exact function ordering depends on the toolchain and build.

The cold installation benchmark and the established-read server serve different purposes. Faster admission helps when adding an authenticated device. Ordinary GETs on an established context perform zero managed installations.

## Final Windows read comparison

The previous release, the final source and stock libcoap ran in all six orders for each workload. The same Windows client allowed one outstanding confirmable read. A 4 KiB read required four blocks. Every block carried a request, its empty ACK, the separate confirmable response and the client's empty ACK. All 144 profiler-off windows completed and verified 1,059,840 reads. The independent audit reconstructed throughput, latency, time charged to each process, cycle counts and memory from the raw windows. Higher reads/s and lower CPU and latency are better. Each slash below separates final Coaptic from stock C.

| Load | Security | Body B | Verified reads/s | Server CPU µs/read | Client CPU µs/read | Median latency µs | p99 µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Capacity | unprotected | 64 | 40,275.5 / 33,574.3 | 24.41 / 23.93 | 24.90 / 29.79 | 22.4 / 28.1 | 49.8 / 62.5 |
| Capacity | unprotected | 4096 | 9,882.0 / 8,623.1 | 100.99 / 100.99 | 100.99 / 116.52 | 94.9 / 110.0 | 183.8 / 222.8 |
| Capacity | oscore | 64 | 36,514.3 / 16,855.1 | 26.37 / 52.25 | 27.83 / 59.57 | 26.5 / 56.2 | 55.5 / 106.6 |
| Capacity | oscore | 4096 | 8,054.1 / 3,884.1 | 120.96 / 237.48 | 123.18 / 257.46 | 115.8 / 246.6 | 238.0 / 441.3 |
| 800/s | unprotected | 64 | 800.1 / 800.2 | 70.80 / 46.39 | 85.45 / 83.01 | 61.0 / 61.5 | 122.7 / 126.6 |
| 800/s | unprotected | 4096 | 800.1 / 800.1 | 141.60 / 126.95 | 153.81 / 166.02 | 143.8 / 162.2 | 369.8 / 361.8 |
| 800/s | oscore | 64 | 800.2 / 800.2 | 68.36 / 97.66 | 80.57 / 124.51 | 67.3 / 100.8 | 145.9 / 190.2 |
| 800/s | oscore | 4096 | 800.1 / 800.1 | 180.66 / 288.09 | 197.75 / 327.15 | 166.4 / 313.3 | 418.2 / 580.2 |

Across the four capacity workloads Coaptic completed more verified reads in 24 of 24 paired comparisons. Paced runs set the requested rate to 800/s. A speed claim for authenticated installation applies only when installing a context. The established request path never calls this method. The comparison uses stock C, including its protected response allocation. [Right-sized libcoap control](MATCHED-ACK-20261125.md).

Closing private memory and resident working set in MiB are shown per process. These snapshots came from the same windows.

| Load | Security | Body B | Server private, Coaptic / C | Server resident, Coaptic / C | Client private, Coaptic / C | Client resident, Coaptic / C |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Capacity | unprotected | 64 | 1.16 / 0.93 | 6.15 / 5.39 | 0.93 / 0.93 | 5.09 / 5.09 |
| Capacity | unprotected | 4096 | 1.18 / 0.96 | 6.21 / 5.43 | 0.86 / 0.88 | 5.05 / 5.06 |
| Capacity | oscore | 64 | 1.16 / 0.97 | 6.25 / 5.56 | 0.92 / 0.93 | 5.18 / 5.18 |
| Capacity | oscore | 4096 | 1.18 / 0.98 | 6.32 / 5.58 | 0.87 / 0.87 | 5.14 / 5.14 |
| 800/s | unprotected | 64 | 1.16 / 0.93 | 6.14 / 5.39 | 0.84 / 0.83 | 5.00 / 5.00 |
| 800/s | unprotected | 4096 | 1.18 / 0.96 | 6.21 / 5.43 | 0.84 / 0.84 | 5.03 / 5.03 |
| 800/s | oscore | 64 | 1.17 / 0.96 | 6.25 / 5.55 | 0.82 / 0.84 | 5.08 / 5.09 |
| 800/s | oscore | 4096 | 1.18 / 0.98 | 6.32 / 5.58 | 0.84 / 0.85 | 5.12 / 5.12 |

The two Coaptic executables have identical benchmark-path machine code. Their observed paired capacity rate differences, final relative to prior, were -4.5%, +0.9%, +3.5%, +0.9% in table order. These results illustrate the variation between runs at unchanged executable code and supply no established-read credit to the cold installation change. Both original-project 50% server CPU targets remain open.

The same-code controls also show why short Windows process CPU windows need care. On 64-byte unprotected reads at 800/s, paired server CPU moved +58.1% from the previous binary while paired server cycles moved +0.4%. At capacity on that body, p99 moved +15.1%. The Windows CPU clock advances in 15.625 ms increments and a paced cell here contains 3,200 reads, or 4.88 µs/read per clock tick. The data establish that two identical packet-path instruction streams can produce materially different process measurements on this host. They do not distinguish scheduling, clock accounting and other host effects. For unprotected 4 KiB at capacity, the two peers both charged 100.99 µs of server CPU per read in the final cohort. Coaptic still reduced client CPU from 116.52 to 100.99 µs/read and median latency from 110.0 to 94.9 µs. An advantage in completed reads/s does not require an advantage in server CPU.
