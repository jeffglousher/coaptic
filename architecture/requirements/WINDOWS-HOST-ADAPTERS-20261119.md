# Windows native async host adapters

The existing standalone async-io adapter uses 5.9–9.5% fewer server cycles
than the Tokio fixture with 64-byte reads in this Windows screen. The
blockwise cells miss the advancement gates. Keep the existing runtime
selection policy.

## Matched runtime screen

One release bench binary binds the accepted standalone async-io adapter
or the Tokio fixture adapter to the same native async App loop. Both
paths use the same 32-pass yield policy, App capacities, user handler,
receive scratch length, offered load, protection and frozen verifying
client. The clocks, I/O and scheduler drivers belong to their respective
runtimes. The resulting measurements include all those differences.

The source uses the accepted `05fd7fb` core runtime and companion
`9019751` fixture. The private resolution preserves every native Syn 3
lock package identity. It adds the existing async-host adapter packages,
including async-io 2.6.0, polling 3.11.0 and futures-lite 2.6.1, matching
their accepted adapter lock identities. The [numeric receipt](WINDOWS-HOST-ADAPTERS-20261119.json)
binds the source inputs, lockfiles, binary and raw windows.

All 104,960 reads verify in the 16 reversed Windows loopback windows.
Each offers 800 operations/s with host-control resources, C1 CON and
64 or 4,096-byte bodies in both security modes. Use the same guard as
the previous runtime screens. Three of the eight pairs advance in the
finite screen. In the larger unprotected cells, async-io p99 rises by
30.9–37.0 µs. In larger protected cells, it raises p50 by 15.2–25.9 µs
and p99 by 16.1–141.7 µs. Both protected blockwise windows use more
server and client cycles.

Process samples include all threads in each runtime process, including
reactor workers. Nested inline memory values describe their recorded
owners. Full worker stacks, runtime heap and kernel costs remain
unmeasured. Process CPU time and cycles are retained independently and
do not qualify an original-baseline CPU-time gain.

## Portable receive refusal

Both public async UDP adapters now classify a Windows receive larger
than the guard buffer as `InvalidData`, the same result as a returned
oversized length. Winsock consumes the datagram and reports `WSAEMSGSIZE`.
The copy to the App buffer occurs only after receiving a valid length.
See [the Winsock receive contract](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-recvfrom).

Before the change, both focused Windows tests reproduced the different
error class for a datagram larger than the scratch. Afterward, the tests
cover capacity-plus-one and larger refusal, unchanged destination bytes
or released RX ownership, and verified reuse on the same sockets.
All eight Tokio and six async-io adapter tests pass with strict
all-target Clippy. These checks cover the adapter fix. The runtime
screen used the preceding adapter sources.

The synchronous standard UDP transport subsequently adopted the same
error class for the raw socket and reusable scratch adapter. Before the
change, four Windows tests failed at the expected error class. Afterward,
all 17 focused tests pass across the fixed and allocated host builds
and strict all-target Clippy passes. IPv4 and IPv6 exact-capacity
receives, one-byte and larger oversize refusal, untouched destination bytes,
and same-socket recovery are covered. Allocation-sensitive receive
checks report zero allocations. The Engine releases the RX slot and
the same unprotected App completes a new 2,000-byte Block2 read after
the refusal. The Windows Mio performance screen uses its own earlier
fixture adapter.
