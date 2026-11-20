# Original-source Windows Mio screen

The original #650 source and current core complete 157,440 verified
reads in one Windows Mio fixture. Established OSCORE at 4,096 bytes
uses 4.1% and 9.1% fewer server cycles across the two pairs. Its CPU-time
results move in opposite directions. Both unprotected 4,096-byte
pairs miss the p99 guard. Both original CPU-time targets remain open.

## Sources and matched workload

The original core is `b8f44fa16b3e16f4c9c6cc5f1205d2aee9cb0b57`.
The other arm uses accepted `da486b0f88b12a1b72ada12901da7d2af4eb000c`
with core Rust and its Cargo files identical to `b875088`. Both releases
compile the historical companion fixture `eea0374b60c93098a7f0dcf24a999e49d753d624`
on the same Windows 10.0.26200 AMD Ryzen 7 5800X3D host using Rust
1.99.1. The selected Mio 1.2.4 source, server fixture and settings are
common. The baseline uses the historical committed native lock.
The current arm retains every package identity from that lock except
the core-required AES change from 0.9.3 to 0.9.4, checked against the
current committed Syn 3 lock. Cargo metadata verifies the explicit
core and `coaptic-io` paths. The archived fixture matches the committed
Rust files after Windows line-ending normalization.

Run C1 CON on IPv4 loopback at 800 offered starts/s, with 64, 1,024
and 4,096-byte representations, both unprotected and established OSCORE.
Each of two rotated pairs per cell uses 6,560 measured operations and
32 warmups per window. The same frozen native client verifies every
complete body and its response contract. Within every pair, sent and
received datagram counts match. The historical Default/AllocMemory
fixture configures one route, four Calls, zero deferred rows, one
association and four request bindings. It uses four RX and four TX
1,472-byte datagrams, eight dedup and four Observe entries, one RX and
two TX bodies rounded up to 1,024 bytes, a 2,049-byte receive scratch
and room for eight Mio events.

The companion `9019751` diagnostic samples both child processes at the
same CPU gates. A private wrapper skips its startup resource line
requirement because the common historical server predates that line.
The bounds in this receipt come from the fixture source. Global host
sampling policy is unchanged. The [numeric receipt](WINDOWS-ORIGINAL-MIO-20261119.json)
binds the source, binary, manifest resolution, harness, preflight and
raw window hashes.

## Observations

- **Throughput and pacing.** All 24 measured windows finish without
  operation failure, cancellation or dropped starts, at 799.96–800.12
  verified reads/s. The client records 82 missed periods. Five pairs
  exceed the screen's limit of six in a window. This paced workload
  provides no saturation estimate.
- **Latency.** Median improves in each pair and all six pooled cells.
  Four pairs exceed a +10 µs p99 allowance. Unprotected p99 rises 23.1
  and 44.4 µs for 4,096 bytes, with a 49.4 µs pooled increase. The
  corresponding OSCORE p99 falls 19.0 and 80.7 µs, with a 59.9 µs
  pooled decrease. A pooled pass leaves a failed pair ineligible.
- **Process cycles.** Current/original server ratios range from 0.909
  to 1.003. Client ratios range from 0.930 to 1.021. These counts cover
  child process threads and are reported independently of CPU time.
- **Process CPU time.** Observed user and system deltas occur in 15.625 ms
  steps. Eight pairs have at least one server window below twenty
  observed quanta. This diagnostic floor is separate from Linux
  accounting ticks in the original criteria. The protected 4,096-byte
  pair ratios are 1.103 and 0.544 for server CPU time. Raw ratios across
  all pairs are 0.544–2.714. The short-window time data support no
  repeatable CPU-time reduction.
- **Memory.** The before/after child process samples retain RSS, lifetime
  working-set peak and private bytes. Final server RSS ranges from
  5,967,872 to 6,103,040 bytes for original and 5,898,240 to 6,070,272
  bytes for current. These overlapping ranges include the processes'
  loaded code and OS accounting. Whole-memory ownership and kernel
  socket costs remain unmeasured.

Only the established OSCORE 4,096-byte cell passes all of this screen's
pair and pooled guards. The two-pair 800/s Windows screen is separate
from the original acceptance matrix and the historical 4,000/s cohort.
Keep both 50% server CPU-time targets, the broader peer goal and the
physical prerequisites open.
