# Windows fixed Service peer comparison with four endpoints

Four independent endpoints verify 43,200 complete 16 KiB reads in 54
Windows loopback windows. The last eligible matched offered rates are
400/s unprotected and 200/s with established OSCORE. Every operation
completes at the next rungs. Pacing and four-way overlap fail there.

## Matched complete operations

Each absolute start group releases four CON reads. Every endpoint admits
one operation at a time and verifies full content, metadata and, for
OSCORE, authentication. Protected workers share one sender context and
serialize sequence reservation and dispatch. Eight warmups per endpoint
follow a verified bootstrap and precede each 800-operation window.
Two repeats reverse the peer order at each rate.

Coaptic uses a fixed Service App for both Mio and Tokio async. It has
32 routes with one populated, four Calls, eight deferred rows, one
association, four security request bindings, 32 datagrams of 1,472
bytes in each direction, 64 dedup and 32 Observe entries, and four
16 KiB bodies in each direction. The fixture also owns its 16 KiB
representation on the heap. The libcoap fixture uses its own allocation
policy. The same client admission bounds, content and established
security apply to both implementations.

Coaptic and libcoap use the frozen binaries from the preceding Windows
peer screen. The private driver extends its two-endpoint schedule to
four workers. The [preceding receipt](WINDOWS-PAIRED-PEERS-20261119.json)
identifies those release inputs. The
[numeric receipt](WINDOWS-SERVICE-PEERS-20261120.json) binds the repeated
windows, launchers, driver revisions, wire checks and binaries.

Eligibility requires all 200 reads per endpoint, four distinct ports,
at least 180 groups with overlapping client I/O intervals, zero missed
group periods and achieved rate within 0.5% of the offer and matched
peer. The intervals begin on first send return and end on final
verified receive return. Their intersection establishes overlap at the
client boundary. All 200 groups overlap in every eligible window.

## Latency and offered load

- At 50/s, both Coaptic runtimes report 17.6–19.8% lower established-OSCORE
  median latency than libcoap. Their unprotected p99 is 9.2–18.6% lower;
  Tokio's second unprotected median is 0.7% higher.
- The established-OSCORE windows at 100 and 200/s pass. Across all
  eligible protected rates, Coaptic's median is 15.2–21.3% lower and
  its p99 is 6.6–30.7% lower. The unprotected comparisons have mixed
  tails, including a 4.9% higher Tokio p99 in the second 400/s window.
- At protected 400/s, windows achieve 226–293 reads/s and miss
  788–793 start periods. Only 7–19 of 200 groups overlap. At unprotected
  800/s, all windows average 798.9–800.4 reads/s, while one libcoap
  repeat and one repeat of each Coaptic runtime miss the eligibility
  gate. Misses in those three windows are 22, 1 and 359, respectively.
  Keep the measured brackets as driver-and-peer pacing boundaries.

## Wire and resource accounting

Two separate short checks each verify 192 reads with packet accounting.
In the shape check, all six cells use 16 data requests per complete read.
Every unprotected peer and both protected Coaptic arms receive one
piggybacked ACK per block. The protected libcoap arm receives one empty
ACK and one authenticated separate CON per block and sends an ACK for
that CON. The protected peer latency includes these response exchanges.
Attributing its difference to server execution alone would require
matched response shape or separate attribution.

The client process uses 0.21–0.30 CPU seconds per wall second in the
protected 50/s windows and 1.17–1.27 at the ineligible 400/s rung. The
high unprotected rung ranges from 0.97 to 1.56. The numeric receipt keeps
client CPU time, server process-tree CPU time and memory snapshots
separate. Server snapshots span driver setup through exit and provide
current process memory at the recorded boundaries. The measured
workload provides no whole-memory, isolated scheduler, cycle or
server-saturation result.

The four-endpoint load boundary and protected application-level speed
comparison extend the peer evidence. Original #650 CPU-time targets
and physical prerequisites remain open.
