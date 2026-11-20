# Windows Tokio timer-poll attribution

The scoped probe records one deadline future started during App wait and
one during progress per successful server send. Each progress future
returns pending on one poll. App wait averages about two polls per future.
All observed polls return pending. The elapsed spans inside the timer
polls are small, and window-level latency changes limit the timing
comparison. Keep the current deadline policy.

## Measurement boundary

The release probe binds to core `c9ee00a14f8d31d4d56da9c7971e8970c11234a1`
and companion `5d5567622aa96b5f0dec9c94abe006b8b19c6517`. It wraps the
existing Tokio `AppTimer` only inside the async UDP fixture loop. The
explicit build overlay verifies both local core dependencies and the
committed Syn 3 lock. Build with Rust 1.99.1 for x86-64 Windows MSVC,
thin LTO and one codegen unit. The recorded platform is the same Windows
26200 and Ryzen 7 5800X3D used in the preceding phase measurements.

Use the frozen client and one server executable for off, count-only
and timed arms. Two serial passes reverse cell and arm order. Each
window has 32 warmups and 6,560 C1 CON reads at 800 starts/s. Both
protections and 64/4,096 B are represented under the same host-control
capacities. The client verifies status, body length, content, format
and ETag. All 157,440 reads verify across 24 windows. The client misses
61 start periods in total, with at most six in any window. Achieved
rates span 800.018–800.113 reads/s. Cross-check the
[numeric receipt](WINDOWS-TOKIO-TIMER-POLLS-20261120.json) for the raw
window and snapshot hashes, build and script identities, and both
processes' counters.

The fixed counters report two cumulative snapshots after sends 2,048 and
6,144. Subtract these for the 4,096-send interior interval. Each timed
poll uses a monotonic wall clock around the existing timer future's
poll, including first-poll execution. Timer destruction after the race,
kernel time, idle time between polls and scheduler work fall outside
this boundary. The preceding [phase probe](WINDOWS-UDP-PHASES-20261119.md)
keeps transport and App intervals separately.

## Observations

- In every measured interval, the progress timer starts and polls
  exactly 4,096 times. The wait timer starts exactly 4,096 times and
  polls 8,183–8,192 times. Each protection and body size shows the same
  counts within that range.
- Summed active timer-poll wall time per send is 0.199–0.329 µs in
  App wait and 0.636–0.896 µs in progress. The sample measures a send,
  which corresponds to one complete short read or one of four block
  exchanges for the 4,096 B read.
- Count-only server cycle deltas relative to off span −4.26% to
  +5.14%. Timed deltas span −3.42% to +3.23%. The perturbation guard
  allows ±5% server cycles, at most 0.1% missed starts, achieved rate
  within 0.5%, and absolute median/p99 changes within 2/10 µs. One of
  eight count-only and four of eight timed pairs pass every guard.
  An absolute tail change in either direction affects attribution.
- Measured server CPU time spans 23.82–185.79 µs per complete read
  across the two sizes and all arms. Process cycle counts are tracked
  separately. The receipt includes before/after process memory and
  the instrumented object sizes. These samples describe the measured
  windows and configured owners. Peak memory and sustained capacity
  require separate evidence.

The counts identify repeated deadline polling in the measured path.
Total scheduler cost remains unresolved. Keep the current timer policy
and the [earlier timer lifetime results](ESP-TIMER-LIFETIMES-20261117.md).
Both #650 targets and the broader peer and device qualification stay open.
