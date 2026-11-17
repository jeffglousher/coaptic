# ESP validation and rejected timer/dispatch integrations

The current-profile ESP compatibility repair is separate from the experimental
timer and quiet-request dispatch integrations. All experimental runtime changes
below remain unadopted. Faster average work did not compensate for regressions
in supported workloads. The accepted timer API and runtime remain in use.

## Timer ownership

In the proposed scheduled-timer design, one deadline may stay armed between App waits. Its waiter must end when ingress,
local work, completion or cancellation ends that wait. The ESP driver previously
treated every armed deadline as an active waiter. The experimental integration
also polled timers without owning their cancellable wait futures. Together these
could add a native `select` between receiving a request and sending its response.

The experimental App owns the timer wait future when racing ingress. A ready send does not
arm a timer. A blocked send borrows the earlier of its own and the existing
deadline, owns its wait future, and restores the previous deadline on every exit.
The wait future drops before the borrowed-deadline guard restores registration.
The ESP driver probes only deadlines with a registered waiter. Protocol capacity,
authentication, persistent state and fairness delays retain their existing rules.

The proposal makes `AsyncTimer::armed`, `schedule` and `poll_wait` required methods. An adapter
implementing only the old `wait_until` API fails to compile. No-op defaults would
otherwise silently lose timeouts under App's separate schedule/wait operations.
Core host adapters and companion qualification callers were migrated together
in the retained experiment. This API change is not selected for main.

## Measured experimental integration

These measurements identify the fault and retain rejected work. They are not
measurements of a newly selected timer-only source.

The pre-fix diagnostic used core `136f9f674f876685cf3b31719e649cfa8ea03aa9`
and adapter `97f3d3f63138beab947318d93550bb5866dfca23`. The corrected diagnostic
used core `b98deb50240c8d0086885094a99807bf8eac4b0f` and adapter
`4c7ed3b023547c270482478d8330adee07197dc3`. Each closes 48 verified reads and
64 packet spans. The seven-byte response's median receive-return to send-entry
wall fell from 2,551 to 844 microseconds. All 64 corrected spans contain zero
native selects. Two fairness delays remain. Global timer wall includes observer
work and preemption; it is not isolated task CPU or clean network RTT.

Three separate clean ESP32 Wi-Fi continuation screens each close 192 verified
reads over four boots. Each uses both synchronous and asynchronous Windows
clients, identity32/test7/large2000 Block2 responses, pinned Rust and C images,
original workers and capacities. Each has zero repeated request sends and zero
unexpected API disconnects. The frozen gate requires geometric mean Rust/C
latency below one and every six-cell mean ratio at most 1.02.

- Initial cache16 integration: 1.40604 overall, C/Rust/Rust/C, RSSI -44 to -42 dBm.
- Timer-corrected cache16: 1.04366, Rust/C/C/Rust, RSSI -46 to -45 dBm.
- Timer-corrected cache32: 1.24687, C/Rust/Rust/C, RSSI -45 to -44 dBm.

All three fail. Their controls were recaptured in separate cohorts; do not pool
them or turn their ratio differences into an isolated causal RTT gain. The extra
16 KiB instruction-cache reservation does not earn its cost here. The C fixture
implements the matched GET workload, not full App capability or memory parity.
Wi-Fi contributes variability even without loss or disconnects. Normal firmware
was restored and its encrypted API identity verified after every trial.

## Complete-App CPU refusal

The frozen `0d31cbc` integration was compared with accepted `a85c51e` using the
retained in-memory complete-App fixture, the same pinned Rust 1.99.1 compiler,
flags and baseline binaries. Four AB/BA/BA/AB process pairs cover 28 workloads:
SmallDevice and fixed/allocated Service, unprotected and OSCORE, synchronous
poll and asynchronous wait/progress, and retained 11,264-byte Q-Block2 snapshots.
All 917,504 measured operations retain their complete cycle vectors. Capacities
and pool payload backing are checked, with inline object sizes recorded apart.

The all-workload point ratios are 0.97545 sync and 0.97608 async. Seven cell
ratios exceed the predeclared 1.02 limit; the worst is fixed Service unprotected
sync 1,024 bytes at 1.04498. Therefore the continuation gate fails. No unchanged
64-pair cohort or adoption is justified. A four-pair point is not a confidence
bound. Client preparation, verification, sockets, executors and Wi-Fi are outside
these CPU clocks. Smaller futures or a narrow GET win cannot override refusal.

## Continuation

The first isolated timer source `ec9221f51a39a1265c52436af51cf3475ba0031e`
also fails its separate 64-pair CPU preservation cohort: 14,680,064 measured
operations, sync ratio 1.00037 (upper95 1.00188), async 1.01014 (1.01163).
Four fixed-Service async cells fail, with point ratios 1.02913 to 1.04057 and
upper bounds up to 1.05287. Do not promote it as a cost-preserving repair or
repeat the unchanged source. The follow-up moves the borrowed timer state and
drop guard into the blocked-send branch, so immediately ready sends do not
query or construct them. Source `496e216d68aa47367af46adf79c7667a272f62c1`
passes its 16 affected regressions but fails a separate four-pair preservation
screen: sync 1.00081, async 0.99435, fixed Service unprotected sync 16,384 bytes
1.02809. All four formerly failing async cells have point ratios at most 1.01516.
Preserve this useful mechanism result without adopting the source, discarding
the failing sync cell, or repeating the unchanged screen.

Deliver and validate the companion's current `MemoryProfile::ResidentRxScratch`
binding against accepted core main. The existing one-shot timer API already
owns and cancels wait futures; retain that supported design. Preserve the
experimental branch, source commitments, raw failures and source-matched peer
comparisons. The faster-than-libcoap goal remains open. A future optimization
needs a distinct mechanism and evidence across supported workloads; avoid another
cache-size rerun or the unchanged rejected quiet-path integration.

## Completed host comparison and selected compatibility fix

Five fresh independently spaced loopback windows close 84,000 measured reads
without request or warmup failures. Across 18 unprotected CON/NON workloads
(64/1,024/65,536 bytes and concurrency 1/4/16), geometric mean completion-rate/C
ratios are 1.55635 Coaptic sync/spin, 1.30014 Coaptic Mio, 1.20454 Coaptic async,
1.34698 coap-lite codec fixture, 0.81251 coap-rs and 0.09741 aiocoap. The C
reference is 1.0. The codec fixture has narrower behavior. These are closed-loop
workload comparisons, not throughput ceilings or full service-capability parity.
They measure rejected core `0d31cbc` with fixture `130683f` and Rust 1.99.0,
separate from the Rust 1.99.1 CPU cohorts. Do not relabel them as selected-source
results or infer ESP32 behavior from Windows loopback.

The selected companion repair is one missing associated-type binding in its
`LoopbackConstrained` profile: inherit `ResidentRxScratch` from Constrained,
as it already does for the other scratch and pool types. Accepted production
core and runtime source remain unchanged. Companion `12fdc73` against core
`3b62f05` passes 91 network/pairing/security-state checks and the ESP32-S3 target
archive/SDK build. That exact async server image completes 48 verified reads
across synchronous and asynchronous Windows clients. Normal firmware is restored
and its encrypted API identity verified afterward. This is functional evidence,
not a new peer-speed qualification or long-term Wi-Fi stability claim.

The [portable vectors and verifier](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/esp-timer-lifetime-20261117)
retain all failed CPU cohorts and Wi-Fi screens, host request vectors and exact
source/image hashes. Private firmware/configuration and encrypted API captures
remain with their owner. The verifier recomputes points and failed guards from
the complete retained numeric vectors without rerunning an unchanged hypothesis.
