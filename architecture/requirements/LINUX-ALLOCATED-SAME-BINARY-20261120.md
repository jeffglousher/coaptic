# Linux allocated Service identical-binary control

The unprotected control fails the original latency guards with identical
client and server binaries in both labels. All 28,800 reads verify and
each window clears the CPU accounting floor. The control measures
variation in this hosted environment and supplies no CPU-time improvement
claim.

## Workload and outcome

Build the original core `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`
under both labels with companion `5d5567622aa96b5f0dec9c94abe006b8b19c6517`,
committed AES 0.9.3 lock and the same Rust 1.99.0 release configuration.
Require matching SHA-256 for both copies of each executable. The control
holds allocated Service, 16 KiB, ordinary Mio, four true overlapping
independent-endpoint CON Calls and 50 aggregate starts/s across four
rotated pairs. Eight 72-second windows on one AMD EPYC 7763 Linux hosted
runner each complete 3,600 measured reads and 20 warmups. The combined
client sends are 57,920 per window, within the same 60,000 budget. There
are zero missed periods, matching achieved rates and resolved cleanup.
[Hosted run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44705976120)

- Across the four pairs, candidate-labelled minus baseline medians are
  +24.497, +197.796, −50.400 and −72.942 µs. Corresponding p99 deltas
  are +18.354, +44.423, −10.269 and −71.574 µs. Two pairs exceed both
  guards. Pooled median rises 31.640 µs and pooled p99 falls 19.917 µs.
- The primary server tick pairs are 76/80, 72/89, 89/87 and 85/74.
  The alternate total process CPU clock varies from −12.6% to +22.9%
  across server pairs. Preserve the client measurements separately.

Reproduction uses the existing long workflow with the original commit
as both inputs, one unprotected partition and the same declared workload.
The [numeric receipt](LINUX-ALLOCATED-SAME-BINARY-20261120.json) records
each window, matching binary hashes and guard outcomes. The
[process-clock supplement](LINUX-ALLOCATED-SAME-BINARY-CLOCK-20261120.json)
captures the second clock and the ordered latency vectors by block.

This observed variation limits interpretation of the
[original/current comparison](LINUX-SERVICE-ALLOCATED-C4-LONG-CPU-20261120.md).
Its recorded failures remain. A controlled environment with repeatable
matched windows is needed before treating long-window deltas as a
performance qualification. Both #650 targets remain open.
