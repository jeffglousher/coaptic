# Original-source Linux fixed Service C4 comparison

The current core uses 11.9% less server CPU time per verified operation
in the unprotected stratum. All four pairs and the pooled latency guards
pass. In established OSCORE, two pairs and the pooled median exceed the
latency guard, so the CPU-time ratio is withheld. Both original #650
50% targets remain open.

## Matched inputs and overlap

The [manual Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44690650351)
compares the original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`
with accepted `753beb0269de31f806ff9e42f9b50412c6eb86e4` using companion
`78630ac16aef90ed782c85c614aed41e410808cb`. Both arms use ordinary Mio,
fixed Service resources, the same benchmark/compiler options and the
original-built native client. The original uses the verified AES 0.9.3
lock and the current uses AES 0.9.4/Syn 3. Profiling is off. This run
uses Linux 7.0.0-1017-azure, glibc 2.39, an AMD EPYC 9V74 and four
assigned logical CPUs with 100 Hz process accounting.

Every 80 ms, four independent endpoints start one CON read apiece, for
50 aggregate offered starts/s. Each individual endpoint retains NSTART
one. Every 30-second window verifies 1,500 complete 16 KiB responses,
with 20 extra warmups. Both protection modes have four rotated pairs.
Each window records four simultaneous Calls, 375 completions per
endpoint, and 1,125 completions while another Call in the group is live.
The 16 windows verify 24,000 measured operations, report no missed
periods, and each send and receive 24,000 measured datagrams. Rates are
50.1318–50.1325 verified operations/s. No throughput ceiling is inferred.

## Results

- Unprotected server CPU is 168.3 µs per verified operation at the
  baseline and 148.3 µs at current. Server ticks across the four pairs
  are 27→22, 25→20, 25→24 and 24→23. A tick is 10 ms per window, so
  one-tick differences have limited resolution. Client CPU separately
  moves from 191.7 to 180.0 µs per verified operation. Pooled median and
  p99 fall by 60.4 and 41.8 µs, respectively.
- With established OSCORE, zero-based repeats 1 and 2 raise the median
  by 102.4 and 9.5 µs. The pooled median rises by 32.0 µs. Every p99
  guard passes. Keep the raw client and server counters and withhold the
  CPU-time ratio for this stratum.

The [numeric receipt](LINUX-SERVICE-C4-CPU-20261120.json) binds each
window to the native binaries, selected locks, raw reports and source
revision. The independent check verifies group overlap, counts and
source/build identities, and reproduces the runner's pair and pooled
guard results. The retained process memory snapshots describe the local
processes. This run reports neither hardware cycles nor peak memory.
The 11.9% comparison is one Linux fixed Service stratum. It supplies no
Windows, peer or device CPU result. The [C1 comparison](LINUX-SERVICE-CPU-20261120.md)
uses a separate runner host and retains both of its failed strata.
