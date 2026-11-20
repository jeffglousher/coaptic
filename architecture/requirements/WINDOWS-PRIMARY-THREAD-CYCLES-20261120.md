# Windows server primary-thread cycle attribution

In all 24 measured windows with thread snapshots, the server's
primary-thread cycle delta equals the bracketed server process delta.
The gated observations identify which thread receives the cycle charge.
The relative costs of App work, runtime code and syscalls on that
thread remain unresolved.

## Source and sampling

Build the ordinary Windows server from accepted core
285b7fbbf4d25581297ad5df34a963649ded3909 and companion
7636b2279efb9338735ac060c21452a3a963d77e with their verified native
Syn 3 lock. The same server executable and frozen native client are used
for both arms. The [numeric receipt](WINDOWS-PRIMARY-THREAD-CYCLES-20261120.json)
preserves the checked source tree, Cargo lock, build fixture and executable
digests. Rust is 1.99.1 x86-64 MSVC, thin LTO, one codegen unit.

The wrapper records the primary thread ID returned for its owned
server by CreateProcess. Each of the two existing CPU gates brackets
QueryThreadCycleTime with a pair of QueryProcessCycleTime reads. It
checks the owning PID and thread creation timestamp. Only the owned
server thread has an extra query in measured snapshots. Both arms keep
the same client and server process counters. Separate calibration
preflights verify 2,880 complete reads. Full thread inventories through
psutil and Toolhelp add roughly 0.1 seconds per gate. Primary-only
snapshots take 41.3–73.8 µs per measured gate. No host policy changes.
[Microsoft thread counter contract](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-querythreadcycletime)

## Complete operations and limitations

Each window completes 3,600 reads with 32 warmups, C1 CON, 800 starts/s,
and the common host-control configuration. Two reversed passes cover
Mio, Tokio readiness with sync App polling, and Tokio async App polling,
each with 64 B and 4 KiB and both unprotected and established OSCORE.
All 172,800 reads verify in 48 windows. The startup log preserves two
port-in-use refusals before the client ran. Each resumes on a fresh
loopback port. The second execution preserves the four earlier measured
windows in their planned order.

There are 121 missed pacing periods. Sixteen of 24 process-only versus
snapshot pairs pass the paced control. Two also meet the
absolute probe bounds of 2 µs median, 10 µs p99 and 5% server process
cycles. Across pairs, the snapshot arm changes process cycles by
−12.8% to +6.2% on the server and −11.4% to +4.8% on the client.
Host variation and sampling overhead remain combined in these deltas.
No runtime comparison has both of its snapshot-control pairs pass all
probe bounds.

In each of the 24 sampled windows, the lower and upper observed
process-cycle deltas both match the primary-thread delta. Any work
outside the owned process or outside those gates remains unmeasured.
Cycle counts include user and kernel work charged to the thread. They
cannot identify Winsock syscall counts or elapsed CPU time. The probe
supports no new runtime selection or original #650 CPU-time claim.
