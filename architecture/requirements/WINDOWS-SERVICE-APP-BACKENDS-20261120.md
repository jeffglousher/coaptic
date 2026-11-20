# Windows Service App storage comparison

Fixed Service has a lower median server App poll wall sum in all
16 matched 16 KiB pairs. The Windows App measurement leaves the Linux
C4 latency cause unresolved.

## Matched control

Reuse both frozen `bench-core` images from the
[fixed Service probe](WINDOWS-SERVICE-APP-20261120.md). The exact
historical core is `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57` and the
accepted core is `753beb0269de31f806ff9e42f9b50412c6eb86e4`. The fixture
is companion `78630ac16aef90ed782c85c614aed41e410808cb`. The source-specific
locks and toolchain are unchanged. Each binary runs both fixed and
allocated Service on the same Windows 11 Pro 26200 host and Ryzen 7
5800X3D. The allocation strategy changes while configured capacities,
the bounded client and verification stay matched.

Run `bench-core 1500 MODE --service-fixed SIZE` and the corresponding
`--service-allocated` arm with 64 and 16,384 bytes in both protections.
The four serial passes reverse order on alternate repeats. Each window
verifies 1,500 measured plus 128 warmup reads over an in-memory C1 CON
path, including OSCORE authentication and complete body verification
where selected. All 96,000 measured reads verify. Each short read uses
one exchange and each blockwise read uses 16. Every arm retains the
same poll count per completed read.

The measured sum covers each server `App::poll` call, including the
in-memory receive/send copies. An independent clock calibration stays
unsubtracted. The client, verification and bridge are outside the timed
server calls. This is socket-free wall time. CPU time, cycles, on-wire
latency and throughput remain separate measurements.

## Observations

- At 16 KiB, current allocated minus fixed median wall time is
  +0.9–1.1 µs unprotected and +0.8–2.3 µs with established OSCORE.
  Historical pairs span +1.6–2.2 and +1.6–2.8 µs respectively.
  The current 16 KiB p99 deltas span −7.2 to +27.3 µs unprotected
  and −3.1 to +31.5 µs with OSCORE.
- At 64 B, 15 of 16 median deltas are 0 or +0.1 µs. One current
  unprotected pair has −0.7 µs. The short-read comparison stays
  inconclusive about a backend timing advantage.
- For the accepted core, the server App including the in-memory
  transport is 315,336 B fixed and 35,352 B allocated. The allocated
  arm requests 282,512 B of pool heap. The fixed arm requests zero.
  Both include the same 225,280 B of nested pool payload capacity and
  use separate representation backing. Peak stack, allocator overhead
  and whole-process memory stay unmeasured.

The [numeric receipt](WINDOWS-SERVICE-APP-BACKENDS-20261120.json) retains
the image and window hashes, configured owners, counts and timing
observations. Keep both supported backends and the separate Linux
controls. Both #650 targets and device/peer qualification remain open.
