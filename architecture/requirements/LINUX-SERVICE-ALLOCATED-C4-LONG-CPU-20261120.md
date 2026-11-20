# Linux allocated Service C4 long-window comparison

The unprotected partition verifies all 28,800 reads and clears the CPU
accounting floor in every window. Two pair medians and the pooled median
exceed the original latency guard, so the CPU-time ratio stays withheld.

## Workload and source

The [unprotected run](https://github.com/jeffglousher/coaptic-validation/actions/runs/44700489024)
compares the original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`
with `753beb0269de31f806ff9e42f9b50412c6eb86e4` using companion
`5d5567622aa96b5f0dec9c94abe006b8b19c6517`. Both labels use ordinary
Mio and the same baseline-built client. The committed AES 0.9.3 and
AES 0.9.4/Syn 3 locks bind their respective server builds. The release
builds use Rust/Cargo 1.99.0, thin LTO, one codegen unit and frame
pointers. Profiling is off. The hosted runner reports Linux
7.0.0-1017-azure, glibc 2.39, an AMD EPYC 7763, four logical CPUs
and 100 Hz process CPU accounting.

The bounded allocated Service preset serves 16 KiB complete bodies.
Four independent endpoints each start a CON Call every 80 ms for 50
aggregate starts/s. Four rotated baseline/current pairs use 72-second
windows. Each window has 20 warmups and 3,600 verified measured reads,
with 900 per endpoint and peak four live Calls. Every window observes
2,700 completions while another Call in its group is live. The measured
client sends and receives 57,600 datagrams per window. The 60,000-send
ceiling includes warmup. The operation deadline is 3,000 ms.

## Unprotected results

- CPU accounting passes. Baseline/current server ticks per pair are
  93/97, 90/81, 86/85 and 105/80 against a 20-tick floor. The receipt
  retains user and system CPU time for both processes.
- There are zero missed start periods. Achieved rate remains about
  50.055 reads/s in every window. The offered load establishes the
  comparison demand. Saturation throughput is outside this run.
- Current minus baseline pair medians are +212.713, −84.257, +36.738
  and −373.503 µs. The pooled median rises 20.893 µs, above the +2 µs
  guard. All four p99s fall and pooled p99 falls 187.802 µs. The
  runner returns failure on the comparison and retains all eight
  verified windows with resolved process cleanup.
- Both builds retain the same configured pool capacities. The inline
  App including its adapter is 34,736/34,424 B and the requested App
  heap is 282,640/282,512 B. Representation backing is 16,384 B in
  both. Separate process memory snapshots do not bound peak memory.

The [numeric receipt](LINUX-SERVICE-ALLOCATED-C4-LONG-UNPROTECTED-20261120.json)
binds the build records, binary hashes, raw windows and original guards.
Reproduce using the `performance-cpu.yml` manual workflow at the named
companion revision with the receipt's workload, `long_window=true`,
`security_mode=unprotected`, and exact core revisions. Use separate
host identities for the 30-second and 72-second campaigns. Keep the
[failed short accounting windows](LINUX-SERVICE-ALLOCATED-C4-CPU-20261120.md)
and this long latency failure. Both original #650 targets remain open.
