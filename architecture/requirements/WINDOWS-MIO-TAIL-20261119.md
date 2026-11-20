# Windows Mio blockwise tail follow-up

The unprotected 4,096-byte p99 increase in the preceding test reproduces
with both frozen binaries. The private probe changes the tail comparison.
The observed server phases do not identify the cause of the clean result.

## Matched workload and observation boundaries

Use the original `b8f44fa` and accepted `da486b0` cores, the historical
`eea0374b` Mio fixture and the original comparison's native lock for
each label. Build the probe against copies of the exact inputs and
confirm both core dependency paths with Cargo metadata. The clean
comparison reruns the already frozen executables. The host and verifying
client are those of the [original comparison](WINDOWS-ORIGINAL-MIO-20261119.md).

Repeat 4,096-byte C1 CON reads at 800 starts/s in both security modes.
Reverse the label, probe and security order for the second repeat.
Each window uses 6,560 verified reads and 32 warmups. The three
conditions are frozen source, probe build with timing disabled, and
probe build with timing enabled. All 24 measured windows complete
157,440 reads. Every read uses four datagram exchanges and preserves
the existing authentication and body verification. A separate preflight
verifies 3,200 reads and the probe's bounded record length.

The server records 8,192 successful datagrams by ordinal, from reply
1,025 through 9,216. These align with 2,048 complete reads inside the
client measurement. A single write of 163,840 bytes follows reply
12,288, beyond the sampled portion. The recorded stages cover App
entry to receive, guarded receive, receive return to send entry, send,
and send return to App exit. This timed build reserves the record buffer
before warmup. It uses wall time, which can include preemption, and
observes neither kernel arrival nor the time between App polls.
Client operation latency has a different boundary and can overlap a
server span. These durations cannot be subtracted to produce a CPU or
network residual. The [numeric receipt](WINDOWS-MIO-TAIL-20261119.json)
binds the sources, releases, raw timings, windows and paired controls.

## Observations

- The frozen unprotected p99 deltas are +22.5 and +79.6 µs. They
  exceed the +10 µs guard even as the current medians fall by 4.2
  and 1.5 µs. The frozen established-OSCORE pairs pass the pacing
  and latency guards, with p99 deltas of +5.8 and −39.4 µs.
- The unprotected p99 differences in the probe-disabled pairs are
  −16.7 and −2.0 µs. With recording enabled they are −19.9 and
  −7.7 µs. Relative to the respective frozen arms, the disabled
  builds shift p99 by −36.7 to +44.9 µs across the four unprotected
  windows. The additional timing can also perturb pacing. The
  first unprotected timed original window misses 23 periods.
- In the timed subset, current has a lower receive-return-to-send-entry
  p99 in both repeats and protections. The unprotected span sums fall
  from 22.2 to 20.3 µs and from 28.5 to 20.3 µs. The unprotected send
  span instead changes from 64.5 to 79.7 µs and from 84.0 to 78.5 µs.
  The direction changes across repeats and the measured App span
  does not explain the clean tail increase.
- The frozen current server performs about 3.0–6.2% fewer unprotected
  cycles and 5.9–7.3% fewer established-OSCORE cycles. Observed process
  CPU time still varies independently. The receipt keeps both processes'
  counters, memory snapshots and achieved rates. These are paced runs.

Retain the clean latency failure. Keep the runtime settings and leave
the cause open. Nomination from these phases would require a measurement
that preserves the clean tail, including the time before App entry.
Both original #650 CPU-time targets, the peer objective and the device
prerequisites remain open.
