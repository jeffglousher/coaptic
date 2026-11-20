# Windows server thread dispatch counts

At 4 KiB, the sampled Tokio threads switch contexts less often per
successful send than direct Mio. Mio still records fewer thread cycles
per send in these intervals. At 64 B, all three paths are close to one
switch per send. Switch frequency alone leaves the Windows runtime cost
gap unresolved.

## Matched inputs and measurement

The local release fixture uses accepted core `e7ffd8735b9be2df721929921ca81f3f0655ce5e`
and companion `5d5567622aa96b5f0dec9c94abe006b8b19c6517`, with the committed
Syn 3 lock. Cargo metadata binds both core dependencies to the intended
checkout. The instrumented source changes only the private native server
fixture. Sixty native tests and strict release Clippy pass. The unchanged
frozen client verifies complete content, status, format and ETag.

On Windows 11 Pro 26200 and Ryzen 7 5800X3D, compare ordinary Mio,
Tokio with synchronous App polling, and Tokio async App wait/progress.
Two passes reverse window order. Each off/on pair uses the same server
binary, host-control pools, C1 CON, 64 or 4,096 B and either unprotected
or established OSCORE. All 48 windows at 800 starts/s verify 6,560 reads
plus 32 warmups apiece. There are 314,880 measured reads and 122 missed
start periods, with at most eight misses in one window.

The probe enables Windows dispatch profiling on the current server thread
at send 2,048 and disables it after the second snapshot at send 6,144. It
uses flags for dispatch counts and a zero hardware-counter mask. All
snapshots use the same thread in each run and all recorded fences reach
the selected send counts. The API supplies context-switch counts, active
thread cycles, snapshot retries and a bitmap of wait reasons. The bitmap
describes reason presence. Separate before/after process counters cover
the whole window. [Windows thread profiling](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-enablethreadprofiling) ·
[Windows dispatch data](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-performance_data)

## Observations

- All 64-byte windows with the probe measure 1.001–1.007 context
  switches per successful send. For 4 KiB, Mio measures 0.903–0.976,
  Tokio sync App measures 0.274–0.451, and Tokio async App measures
  0.271–0.353. Each blockwise read takes four exchanges.
- For 4 KiB, interior server thread cycles per send span 73,864–88,168
  with Mio, 88,250–104,959 with Tokio sync App, and 94,106–110,081 with
  Tokio async App. These counters include work in the measured thread
  interval and do not identify Winsock calls or time spent descheduled.
- Enable, read and disable wall spans are 3.7–11.2, 0.2–0.8 and
  2.5–4.3 µs respectively. Whole-window process cycle changes in the
  on arms range from −6.25% to +12.99% relative to off. Eleven of 24
  matched comparisons pass every perturbation guard, including ±5%
  cycle change, pacing and absolute 2/10 µs median/p99 limits. Probe
  activity covers a smaller fraction of each 4 KiB window. These
  whole-window differences do not bound the cost during the active span.

The [numeric receipt](WINDOWS-THREAD-DISPATCH-20261120.json) retains
phase data, source and clock identities, client/server CPU time and
cycles, memory snapshots and raw hashes. Dispatch frequency, latency,
CPU time, cycles and memory stay separate. Both #650 targets remain open.
