# Windows Mio direct receive screen

The direct receive variant reduces the enclosing App by 2,048 bytes
for this benchmark, with the same configured pools. It fails the
advancement gates. Keep ordinary Mio and its guarded receive.

## Candidate and preservation

The Windows-only extension of the explicit direct mode receives once
into the Engine's reserved RX slot. Winsock refuses oversized datagrams
with `WSAEMSGSIZE`. The adapter returns `InvalidData`, and the Engine
releases the slot without installing a packet or dispatching its prefix.
An exhausted Engine returns before the socket receive. Readiness,
working capacities and immediate send behavior are shared with the
guarded control. The production adapters retain their existing behavior.
[Winsock receive contract](https://learn.microsoft.com/en-us/windows/win32/api/winsock/nf-winsock-recvfrom)

Run accepted core `d03b7fbc3242244884c75ff87203e4e4ef69a499` and candidate
fixture `5829564df878c679c1aa1edecc8df753c9cbadd0` with the verified Syn 3
lock. Both arms use the same candidate executable, changing only the
Mio receive mode at startup. The frozen native client verifies both.
All 64 native release tests pass, including registered IPv4/IPv6
readiness, oversize refusal, full RX refusal before consumption and
authenticated recovery. Strict release Clippy passes. A separate
preflight completes 1,280 verified reads.

## Complete operation comparison

Use the host-control profile, C1 CON, 800 offered starts/s, 64 B and
4 KiB, unprotected and established OSCORE. Each of 32 windows verifies
3,600 measured reads after 32 warmups, with four rotated pairs per
security/payload stratum. All 115,200 operations verify and send the
expected datagrams. There are 45 missed periods. One pair fails the
pacing gate. Six fail the latency gate. The direct arm changes p99 by
−73.2 to +70.5 µs and server process cycles by −5.75% to +5.25% across
pairs. Client process cycles change by −5.36% to +5.26%.

The direct variant removes the 2,049-byte scratch array. After
alignment, the measured enclosing App is 13,984 B instead of 16,032 B.
This is a nested object-size observation. The
[numeric receipt](WINDOWS-DIRECT-MIO-20261120.json) preserves separate
CPU time, cycles, latency, process snapshots and retained owners.
Only one pair meets the pacing, latency and 5% server-cycle advance
gates together. Retain the exact private candidate and raw evidence.
The original #650 CPU-time targets, throughput and the device scope
stay open.
