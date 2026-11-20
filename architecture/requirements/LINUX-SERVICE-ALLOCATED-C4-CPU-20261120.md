# Original-source Linux allocated Service C4 comparison

All 24,000 reads verify and every latency guard passes. Seven windows
fall below the existing 20-tick CPU accounting floor. Keep both
CPU-time ratios withheld.

## Matched inputs and workload

The [manual Linux run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44696916380)
compares original b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57 with accepted
753beb0269de31f806ff9e42f9b50412c6eb86e4 against companion
8174011233e44f11d9479a420d6ed8041e6915d2. Both labels use ordinary Mio,
the same baseline-built native client, and explicit committed AES
0.9.3 or AES 0.9.4/Syn 3 lock inputs. Profiling is off. The platform
reports Linux 7.0.0-1017-azure, glibc 2.39, Intel Xeon Platinum 8573C,
four assigned logical CPUs and 100 Hz process accounting.

The allocated Service profile preserves the fixed Service dimensions,
with bounded startup allocation and 16 KiB representations. Four
independent endpoints each start one CON operation per 80 ms group,
retaining NSTART one. Each label runs four 30-second rotated pairs
per security mode at 50 aggregate starts/s, with 20 warmups in each
window. Each timed window completes 1,500 reads, with peak four live
Calls, 375 completions per endpoint, and 1,125 completions while
another Call in the group is live. There are no missed periods, and
each window sends and receives 24,000 measured datagrams.

## Guard outcomes

- Unprotected latency passes all four pairs. Pooled median and p99
  fall by 82.7 and 48.4 µs. The four current windows accrue 13, 16, 17
  and 15 server ticks. Two baseline windows also miss the 20-tick
  floor, with 19 ticks each.
- Established OSCORE latency also passes all pairs. Pooled median
  and p99 fall by 74.0 and 105.5 µs. Current repeat 1 accrues 19 ticks,
  withholding the stratum's CPU ratio.
- The startup server App owner including the adapter is 34,736 B
  at baseline and 34,424 B at current. The separately requested App
  heap is 282,640 B and 282,512 B, with 16,384 B of representation
  backing in either label. The receipt retains process memory
  snapshots separately from configured owners.

The [numeric receipt](LINUX-SERVICE-ALLOCATED-C4-CPU-20261120.json)
binds exact labels, locks, raw window hashes and accounting. Current
and original source comparisons within this run are matched. The
[fixed Service C4 run](LINUX-SERVICE-C4-CPU-20261120.md) is on a
different hosted machine. The two runs do not isolate allocation
mode or measure peak memory, device results or throughput capacity.
An additional longer grouped window could improve accounting
resolution at this offered load while retaining the failed windows.
Both original #650 targets stay open.
