# Tokio single-reader readiness screen

Keep the existing `UdpSocket::readable()` wait. The reserved read-waker
candidate verifies all 209,920 operations and misses the advancement
gate in every pair.

## Candidate and recovery

The [dispatch count experiment](WINDOWS-THREAD-DISPATCH-20261120.md)
finds fewer context switches per blockwise send in the Tokio paths
and more measured thread cycles per send. This candidate changes the
software readiness wait inside the sync-App Tokio fixture. The one
receiving task uses `poll_recv_ready` through `poll_fn`, selecting
Tokio 1.53.3's reserved read-waker slot. The fixture keeps the same
deadline calculation, timeout wrapper and guarded receive. The
startup selection lives in a private fixture build.

The release build binds accepted core `e7ffd8735b9be2df721929921ca81f3f0655ce5e`
and companion `5d5567622aa96b5f0dec9c94abe006b8b19c6517` to the committed
Syn 3 lock. The first private compile refuses an overlapping App borrow
and a missing argument in a fixture test caller. The revised build
computes the deadline before borrowing the socket and retains the
test-call entry point. All 61 native tests and strict release Clippy
pass. A focused real UDP proof cancels an idle wait in both arms,
refuses an oversized datagram and receives the next exact reply on
the same socket with unchanged pools.

## Measured operations

Use the frozen client and same MSVC release image in both arms. The
32 serial windows rotate four pairs per security and body size under
host-control capacity. Each window offers 800 C1 CON complete reads/s
with 6,560 measured and 32 warmup operations. Sizes are 64 B and 4 KiB.
The client verifies content, code, format and ETag under unprotected
and established OSCORE modes. Client and server traffic accounting
match in every pair.

- Thirteen pairs pass pacing, and nine pass both latency guards. The
  median changes span −7.8 to +7.0 µs. The p99 changes span −94.7 to
  +61.7 µs.
- Server cycle changes span −4.12% to +4.48%. Client cycle changes span
  −3.57% to +4.30%. No pair reaches the predeclared 5% server-cycle
  benefit threshold. Process CPU time and memory snapshots remain in
  the [numeric receipt](WINDOWS-TOKIO-SINGLE-READER-WAIT-20261120.json).

The rejected source and raw results remain available for review. Keep
the existing readiness and deadline policy. Both original #650 targets,
peer speed and device qualification remain open.
