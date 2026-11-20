# Windows matched-rate UDP peer measurements

Coaptic's Windows loopback median latency is 20.7–25.7% lower than libcoap's
in the tested established-OSCORE cells at 400 offered reads/s. In the larger
protected workload, 600 reads/s is the last tested eligible common rung.
All arms complete at the 800/s rung, but the offered cadence fails in three
windows. This locates a measurement boundary for the tested client and peers.
Saturation of each server requires separate evidence.

## Scope and source

Mio and Tokio async run the same frozen Coaptic server from the
[preceding Windows screen](WINDOWS-RUNTIME-SCREEN-20261119.md), using
SmallDevice with blockwise enabled. This has four routes with one populated,
one Call, zero deferred entries, four security request bindings, one
association, two datagrams per direction at 1,152 bytes, four dedup and two
Observe entries, and one 4,096-byte body per direction. Libcoap uses its
existing fixture and allocation strategy for the same GETs. Both endpoints
are compared for the configured complete-read service.

The independent Python driver has the same bytes in both passes. The
[companion change](https://github.com/jeffglousher/coaptic-validation/pull/269)
adds opt-in absolute start times and reports starts at least one period late.
Both security modes use one outstanding CON operation and a common driver.
The OSCORE state uses fresh fixture credentials and an established context.
The driver verifies the complete representation, response code, Content-Format,
and stable ETag across blocks where advertised. Bootstrap and 16 warmups
precede each measured window. Each window measures 1,600 reads.

The [numeric receipt](WINDOWS-PACED-PEERS-20261119.json) preserves the exact
driver and binary hashes, resource snapshots, sample counts and pacing
results. It retains the server resource window from the campaign separately
from client latency. The process-tree CPU readings include startup and warmup.
These runs do not establish CPU cost per operation for #650.

## Measured conditions

- At 400/s, two reversed passes across both protections, both body sizes and
  all three peers produce 38,400 verified operations across 24 windows. No
  warmup or request fails, and there are zero missed start periods. The
  established-OSCORE median advantage spans both runtimes and sizes. The
  Tokio async 64-byte OSCORE p99 varies relative to libcoap across repeats.
  Unprotected median differences are small at 64 bytes. At 4,096 bytes,
  Coaptic medians are 2.4–9.6% lower, while p99 ordering varies.
- At 600/s, only the 4,096-byte established-OSCORE workload is exercised.
  Six windows verify 9,600 reads with zero missed periods. Coaptic shows
  19.2–25.1% lower median latency in both runtimes, with lower p99 in every
  pair. Treat the observations as fixed-load latency and verified throughput
  in this particular Windows loopback setup.
- At 800/s, the same six-window configuration verifies all 9,600 reads
  and all achieved rates stay within 0.5% of offered. The guard permits
  one missed period per window. Tokio async misses zero in both, Mio
  misses three and zero, and libcoap misses 141 and 599. Retain all windows
  and withhold the matched-load comparison at this rung.

The next comparison needs a controlled two-endpoint start shape and evidence
of overlapping verified completions. Preserve one outstanding Call per
endpoint and shared OSCORE nonce ownership. Keep the broader peer-speed goal,
the original CPU-time gates and physical qualification open.
