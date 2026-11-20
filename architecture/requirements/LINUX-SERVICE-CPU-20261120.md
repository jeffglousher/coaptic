# Original-source Linux fixed Service C1 comparison

All 24,000 complete reads verify. One pair in each protection mode fails
the original latency guards, so both server CPU-time ratios are withheld.

## Matched inputs and bounds

The [manual Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44688190914)
builds the original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and
accepted `753beb0269de31f806ff9e42f9b50412c6eb86e4` against companion
`bd2a18b627e481f4ad8c7b3a9ffde4f6e580b043`. The same ordinary Mio
fixture and original-built native client serve both labels. The
original selects the verified AES 0.9.3 and Syn 2 lock, and current
selects AES 0.9.4 and Syn 3. Both builds use the same compiler, flags
and benchmark manifest. The runner restores the canonical lock.

The Ubuntu hosted runner reports Linux 7.0.0-1014-aws, glibc 2.39,
an Intel Xeon 6975P-C with four assigned logical CPUs, and 100 Hz
process accounting. Each 30-second window offers 1,500 C1 CON reads
at 50 starts/s, with 20 additional warmups. Four rotated pairs run
in each mode, unprotected and established OSCORE, with 16 KiB
representations and the fixed Service capacities. Profiling is off.
Keep the sampled server/client memory and CPU boundaries separate
from the client latency and offered-load metrics.

The [numeric receipt](LINUX-SERVICE-CPU-20261120.json) records the
artifact, exact sources, binary and lock identities, every raw
window hash, client and server measurements, and both guard results.
The local owner retains the bounded textual evidence. The hosted
artifact also includes the release binaries.

## Outcome

Every window completes its 1,500 reads, with no missed periods, matched
datagram counts, and 26–42 server accounting ticks. Achieved rates
are 50.0320–50.0327 reads/s. Process CPU clocks and memory samples
remain available for diagnosis. This clean run does not sample hardware
cycles or establish peak memory or saturation.

- **Unprotected.** Zero-based repeat 2 raises median latency by
  24.8 µs and p99 by 76.1 µs. Pooled p99 rises by 99.9 µs.
- **Established OSCORE.** Zero-based repeat 3 raises median latency
  by 23.9 µs and p99 by 124.8 µs. The pooled guard passes.

The source-matched receipt applies the original +2 µs median and
+10 µs p99 guards for every pair and pooled stratum. The initial
short-window runner's pacing and accounting comparison omits that
latency check. Replaying its raw reports with the corrected aggregator
reproduces the guard results and withholds both ratios. Future short
reports apply those limits through companion
[#273](https://github.com/jeffglousher/coaptic-validation/pull/273).

This is the declared 30-second C1 cohort for the accepted source.
The older C1 partitions and ineligible historical cohort retain their
own observations. Both #650 targets and the remaining matrix stay open.
