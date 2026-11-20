# Windows fixed Service App poll comparison

The current core has a lower median server App poll wall sum in every
pair. At 16 KiB, all four unprotected p99 observations also fall. The
unprotected tail increase in the clean [Windows Mio comparison](WINDOWS-MIO-TAIL-20261119.md)
does not reproduce at this socket-free boundary. This comparison does not
identify the cause of the socket result.

## Matched inputs and workload

Compare the original `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`
with accepted `753beb0269de31f806ff9e42f9b50412c6eb86e4`, the same
core inputs used in the [Linux C1 socket comparison](LINUX-SERVICE-CPU-20261120.md).
Use the unchanged `bench-core` Service fixture from companion
`78630ac16aef90ed782c85c614aed41e410808cb`. The locked release builds
use Rust/Cargo 1.99.1, LLVM 23.1.1, x86-64 MSVC, thin LTO and one
codegen unit. Cargo metadata resolves both core crates to their explicit
sources, with the committed AES 0.9.3 lock for original and the committed
AES 0.9.4/Syn 3 lock for current. The historical source export is checked
byte-for-byte against its Git archive. Both suites pass the six Service
tests covering full replies, capacity, cancellation and recovery.

Run four rotated baseline/current pairs per payload and security mode on
Windows 11 build 26200 with a Ryzen 7 5800X3D. The 48 serial windows each
verify 1,500 measured and 128 warmup C1 CON operations. At 64 B and 1 KiB,
each complete response takes one exchange. At 16 KiB, each takes sixteen
Block2 exchanges. Two real Apps use a pair of single-packet in-memory
ports. The verifier checks every ACK, MID and token, and collects and
verifies the entire body and its metadata after OSCORE authentication in
protected mode. The server keeps the fixed Service preset with four Call
slots, eight deferred rows, one association, four bindings and four
16 KiB bodies in each direction.

The clock sums every server `App::poll` wall interval for one fully
verified operation, including in-memory fixture copies, maintenance,
dispatch and server cryptography. The bridge, client work and verification
are outside these intervals. Each window retains a separate 4,096-sample
clock calibration, with median 0 ns and p99 100 ns at the observed clock
granularity. No calibration is subtracted. The native release binaries,
raw vectors and scripts fit the local 8 MiB budget. The
[numeric receipt](WINDOWS-SERVICE-APP-20261120.json) binds them to inputs,
windows, sends, calibration, sizes and pair deltas.

## Observations

- At 16 KiB unprotected, median server poll wall sum moves from
  31.9–32.3 µs to 19.2–19.3 µs. The four p99 deltas range from
  −40.5 to −11.0 µs.
- At 16 KiB with established OSCORE, the median moves from 63.5–64.5 µs
  to 49.5–51.3 µs. Three p99 deltas fall by 9.8–57.7 µs and one rises by
  6.2 µs.
- At 64 B, all eight protection/pair medians fall by 0.6–0.8 µs. At
  1 KiB, all eight medians fall by 0.6–2.5 µs. A few 1 KiB p99 values
  rise, with the largest increase 1.1 µs.
- The enclosing server App measures 315,784 B for original and
  315,336 B for current, including the in-memory transport. Both request
  zero pool heap bytes. This is an object-size observation, with no peak
  stack, process memory or socket equivalence established.

To reproduce, build the recorded fixture against each source with its
recorded selected lock in the registered lane. Freeze each executable,
then use `bench-core.exe 1500 unprotected --service-fixed 16384` and
`bench-core.exe 1500 oscore --service-fixed 16384`. Substitute `64` and
`1024` and use the receipt's order. These are unpaced App wall samples.
End-to-end latency, CPU time, cycles, throughput and both original #650
targets require separate evidence. No optimization is promoted from
this probe.
