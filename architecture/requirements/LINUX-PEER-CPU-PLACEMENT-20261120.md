# Linux peer CPU placement control

Placing the server and client on distinct guest cores leaves the
allocated Service comparison ineligible. All 28,800 reads verify,
and three pair medians and the pooled median exceed the latency guard.
Keep inherited placement for the existing benchmark.

## Matched control

Use the original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` for both
labels with the explicit historical AES 0.9.3 lock. The diagnostic
fixture is `04f40ae8f931a86afc77410a845a3e6ef224c3a6`. The runner
requires exact hash equality for both server copies and both client
copies. It uses ordinary Mio, allocated Service, 16 KiB, unprotected
communication, four independently grouped CON Calls and 50 aggregate
starts/s. Each of the eight rotated 72-second windows verifies 3,600
measured reads and 20 warmups. The 60,000 outbound budget and 3,000 ms
operation deadline stay in force. This uses one Linux 7.0.0-1017-azure
guest with an Intel Xeon 6975P-C and four assigned logical CPUs.
[Hosted run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44710858859)

The baseline inherits CPUs 0–3. The alternate pins the owned
single-thread server to CPU 0 and the client to CPU 1, which have
distinct guest core IDs. Before/after masks and thread counts pass
for every window. The guest topology does not establish physical
exclusivity. There are zero missed starts, matched rate and traffic,
and resolved process cleanup. The primary CPU comparison remains
disabled for this diagnostic.

## Results

- Separate minus inherited median deltas are −59.015, +32.795,
  +95.911 and +30.874 µs. Two pairs also exceed the p99 guard, by
  +21.532 and +73.013 µs. Pooled median rises 26.527 µs and pooled
  p99 rises 8.948 µs. One of four pairs passes both latency guards.
- The primary server tick pairs are 38/31, 34/32, 38/38 and 40/37.
  All clear the existing 20-tick floor. The additional process clock
  moves from 105.86 to 98.05 µs per verified operation on the server
  and 137.86 to 141.22 µs on the client. Server pair changes span
  −17.8% to +0.8%. These readings retain the placement effect and
  observed variation. They supply no original-to-current CPU ratio.

The numeric receipt binds the workload, clocks, process observations,
exact masks and raw hashes. The separate clock audit retains both
processes' alternate CPU totals and the ordered latency summaries.
[Placement receipt](LINUX-PEER-CPU-PLACEMENT-20261120.json) ·
[Process-clock audit](LINUX-PEER-CPU-PLACEMENT-CLOCK-20261120.json)

The [experimental fixture branch](https://github.com/jeffglousher/coaptic-validation/pull/277)
is retained with the closed draft. The earlier
[identical-binary run](LINUX-ALLOCATED-SAME-BINARY-20261120.md) also
fails the latency control, with inherited placement on both labels.
Further qualifying comparisons require a repeatable execution
environment that can sustain the unchanged guards. Keep both #650
criteria and the independent device prerequisites open.
