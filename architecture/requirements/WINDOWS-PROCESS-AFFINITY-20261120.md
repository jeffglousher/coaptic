# Windows process-affinity screen

Do not adopt the single-CPU server configuration. All 24 matched pairs
miss the pacing gate, and all 24 have a higher p99 with the server
restricted to one logical CPU. Process cycle changes are unqualified.

## Intervention and workload

Use the unchanged frozen server and client from the
[Windows runtime screen](WINDOWS-RUNTIME-SCREEN-20261119.md). Their build
receipt identifies core `73093d875e089f10d43530e5ebef9895a75bf4e3` and
companion `76be9a093c4c3282062bc1be509d1c2a7ca78d01`. The gate wrapper
comes from companion `78630ac16aef90ed782c85c614aed41e410808cb` and
adds affinity only for the owned child processes. The control preserves
the same executable, workload and host-control capacities as its
matched single-CPU arm. Both keep the client on logical CPU 4.

The Win32 topology reports eight physical cores with two logical CPUs
each. The control allows the server on fourteen logical CPUs, excluding
both logical CPUs of the client's physical core. The single-CPU arm allows
only CPU 10, on a different physical core. The harness checks masks at
startup and both CPU gates. There is no change to system scheduling,
tracing or power policy. These masks show where each process may run.
The test does not observe migration or actual runnable delays.
[Microsoft affinity API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setprocessaffinitymask)

Two reversed passes cover Mio, Tokio ready with sync App polling, and
Tokio async App polling. Each of 48 windows uses C1 CON reads, 64 or
4,096 bytes, 800 offered starts/s, and 3,600 verified reads after 32
warmups. Each window uses fresh fixture contexts for established OSCORE.
An initial bounded preflight verified 160 reads but the harness failed
at its Git provenance call. It was corrected to scope child hooks by
executable path. A separate 24-window functional preflight then verified
3,840 reads and all affinity checks. The frozen build source label in
the first private worksheet was corrected using the measured binaries'
original build receipt and matching executable digests.

## Result

All 172,800 operations verify. The control misses 5,049 pacing periods
and the single-CPU arm misses 10,487, with 86,400 operations offered to
each arm. No pair meets the 0.1% missed-period and matched-rate gate.
Single-CPU p99 increases by 0.79–10.09 ms across pairs. Server process
cycles vary from 36.0% fewer to 4.5% more, while client cycles rise by
29.3–583.6%. The separate server process CPU-time readings span
4.3–186.6 µs per operation. These are descriptive observations from
ineligible windows. The [numeric receipt](WINDOWS-PROCESS-AFFINITY-20261120.json)
keeps the input hashes, pair decisions, latency, process counters and
memory snapshots. No CPU budget, peak memory or throughput ceiling is
established.

The fixed-client control itself misses the pacing target. These results
do not locate the cause of the earlier unpinned socket tail, or justify
constraining CPU placement in deployment. The original #650 CPU-time
targets and peer and device scope remain open.
