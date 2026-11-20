# Windows paced peer comparisons with two endpoints

Two independent endpoints complete overlapping 4,096-byte reads at matched
paired-start demand. For established OSCORE, all three peers pass at 800
offered reads/s. The first ineligible rung is 1,000/s, where both libcoap
windows miss the cadence guard while still completing every read. Across
the eligible protected rungs, Coaptic uses 10.6–17.1% lower median latency
than libcoap in both App runtime paths.

## Workload and witness

Each five-millisecond start slot at 400/s releases one CON operation per
endpoint. Higher rungs keep the same paired shape with shorter intervals.
Both workers share one OSCORE sender context and serialize sequence reservation
and send. Tokens distinguish the workers. Each worker verifies the complete
body and admits one operation at a time. The Coaptic fixture uses Device
fixed storage, with eight route slots, four Calls, two deferred entries,
four security bindings, one association, four 1,472-byte datagrams per
direction, eight dedup and four Observe entries, and two 4,096-byte bodies per
direction. These are distinct from the C1 SmallDevice comparisons.

The client records the interval from the return of the first successful send
through the return of the final authenticated and verified datagram's receive.
Intersect these observed intervals for the same scheduled pair. This witnesses
concurrent operations through two UDP endpoints. It cannot locate concurrent
server instruction execution or kernel packet arrival. Acceptance requires
400 successful reads per endpoint in every window, distinct ports, at least
360 observed overlaps, no starts a pair period late, and achieved rate within
0.5% of offered. Bootstrap and eight warmups per endpoint precede the window.

Coaptic and libcoap use the same frozen executables as the
[C1 screen](WINDOWS-PACED-PEERS-20261119.md). The private driver adapts the
accepted companion `9019751` paced driver. An initial launch refused the
campaign's C1 readiness probe and produced six startup refusals. This launcher
was stopped, the startup proof was accommodated, and the six raw failures
remain retained. The [numeric receipt](WINDOWS-PAIRED-PEERS-20261119.json)
records both driver and binary identities, corrected windows and scope.

## Rate and latency observations

- At 400/s, 12 windows cover both protections, both Coaptic runtime paths
  and libcoap. All 9,600 reads verify, with overlap in all 4,800 pairs and
  no missed start periods. Coaptic's unprotected median is 4.2–6.1% lower
  and protected median is 12.7–17.1% lower than libcoap's.
- At 600 and 800/s in established OSCORE, 12 windows verify another
  9,600 reads. All 4,800 pairs overlap and all windows pass the pace guard.
  Coaptic median latency is 10.6–11.5% lower at 600/s and 11.4–16.9% lower
  at 800/s, with lower p99 in every eligible protected pair.
- At 1,000/s, all 4,800 reads verify and 2,393 of 2,400 pairs overlap.
  Coaptic misses zero start periods in all four windows. Libcoap misses
  11 and 81. The higher-rate driver additionally samples client process
  CPU time. Its CPU-to-elapsed ratio ranges from 0.90 to 0.96 for the
  Coaptic cells and 1.15 to 1.23 for libcoap, summed across client threads.
  The rate is pacing-ineligible and withholds a matched latency comparison.
  Both client and peer path costs affect this outcome.

These cells extend the observed concurrent workload and leave the broader
peer speed objective open. CPU-time acceptance against the original #650
baseline, larger endpoint counts, overload recovery and physical qualification
require separate evidence.
