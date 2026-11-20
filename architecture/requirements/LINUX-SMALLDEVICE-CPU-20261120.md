# Original-source Linux SmallDevice CPU comparison

All 480,000 measured reads verify. Both security modes exceed the
original p99 guard, so both CPU-time ratios stay withheld.

## Matched inputs

The [manual Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44693744776)
compares the original b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 and accepted
753beb0269de31f806ff9e42f9b50412c6eb86e4 at companion
78630ac16aef90ed782c85c614aed41e410808cb. Both use the same ordinary Mio
fixture, release flags and baseline-built native client. The original
AES 0.9.3 lock and accepted AES 0.9.4/Syn 3 lock are verified against the
committed inputs. Profiling is off.

Each 30-second window offers 30,000 C1 CON reads at 1,000 starts/s,
with 20 additional warmups. The 32-byte workload uses body-free
SmallDevice/Constrained with four route slots and one GET route, one
Call, zero deferred rows, one association, four request bindings,
two 1,152-byte datagrams in each direction, four dedup rows and two
Observe rows. Both unprotected and established OSCORE have four
rotated pairs. The runner reports Linux 7.0.0-1014-aws, glibc 2.39,
an Intel Xeon 6975P-C with four assigned logical CPUs and 100 Hz
process accounting.

## Outcome

Every window completes its 30,000 reads with matched datagram counts.
There are 15 missed periods across the campaign. All individual
windows satisfy the missed-period, rate, cleanup and 20-server-tick
gates. Server time accrues 32–51 ticks per window. Achieved rates are
1,000.0249–1,000.0307 reads/s. The raw CPU times and process memory
snapshots do not establish cycles, peak memory or throughput capacity.

- Unprotected p99 increases by 27.4 µs and 22.5 µs in zero-based
  repeats 0 and 2. Pooled p99 increases by 10.249 µs.
- Established OSCORE p99 increases by 156.9 µs and 84.9 µs in repeats
  2 and 3. Pooled p99 increases by 43.0 µs.

Both protection modes pass the median guard. The p99 ceiling is
+10 µs per pair and pooled. The [numeric receipt](LINUX-SMALLDEVICE-CPU-20261120.json)
binds both executable and native-lock inputs plus every raw window.
Independent calculations agree with the runner's guards. The raw
artifact remains separate from the [Linux fixed Service C4 result](LINUX-SERVICE-C4-CPU-20261120.md)
and from actual-device SmallDevice tests. Both #650 targets remain open.
