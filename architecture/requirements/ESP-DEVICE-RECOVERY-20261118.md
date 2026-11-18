# Installed ESP32-S3 recovery controls

This finite follow-up closes the selected missing controls on production core
`b8750885f8bd10d2f5dd73c92807df00c7f2f0a8`: known-size NON Q-Block2 suffix loss,
observable duplicate handler effects, explicit Observe pool refusal/reuse and
per-device Wi-Fi interruption/reconnect. All four sync/async × unprotected/durable
OSCORE cells passed. The [numeric receipt](ESP-DEVICE-RECOVERY-20261118.json)
contains image/ELF/archive identities, operation spans, recovery grades and normal
restoration captures. This is finite qualification, not certification.

## Source and fixed capacities

The source checkout was documentation head `a844277`; every recorded Rust source
hash equals production `b875088`. The fixture service and platform control match
companion `0e45cec`; later host selectors and grading were added separately. The
Q-Block2 and later host executable was built at companion `3cdb069`, SHA-256
`7facd8314719ccaffb2f25d340a5e2889001f19787985f42f9170dc87b2dccc5`.
The earlier async OSCORE Wi-Fi/duplicate/Observe trial used companion `0e45cec`
and client SHA-256
`18ad2f61a0e3fb98029d6994831ec0865b9a8a42e38d9e18cbe2a55f03bfbae4`.
The numeric receipt binds each native client or independent script to its role.
Receipt fields recording a later suite HEAD at execution/build completion do not
mean that unchanged binaries were rebuilt at that HEAD. Independent review
checked code equivalence, four OTA/ELF hashes, Rust source hashes and runtime
symbols. The companion default CI pin now advances explicitly to this accepted
production core; older CI results are not relabeled.

Distinct archives select the actual Rust service implementation:

- Sync: `network,oscore,standalone`, SHA-256
  `a5a3ced46868da56e55ee8103aa9afc96fa4e3c723a3d9ec944b08290f7b67ec`.
- Async: `async-network,oscore,standalone`, SHA-256
  `5216175976eff4d6182c64fd5e5b27b5c8058e2b9b3f820a940d9538900f9db4`.

Generated `COAPTIC_ASYNC_NETWORK` separately selects C++ setup/stack and must
match the archive. Security and interruption setters were checked before image
acceptance. The existing shared core junction was preserved; process-local
source selection and restored manifest overrides selected the current checkout.
Compiler versions remain Rust ESP 1.97, host 1.99.1, ESPHome 2026.9.1/IDF 5.5.5.

The Constrained service keeps two 1,152-byte RX/TX datagrams, one 4,096-byte body
per direction, four duplicate entries, two Observe slots, four Calls, four
security bindings, one association and no deferred jobs. The eight-route table
is now full. Worker priority one and 98,304/196,608-byte sync/async stack
reservations remain. No pool growth or core protocol change was required.

## Executed controls

Each cell reads the volatile echo-entry counter, receives an echo response,
resends the exact request and verifies a fresh counter delta of one. Protected
selection also requires the matching cached ciphertext reply. This counts
handler entries, including size refusals; it is not a durable actuator receipt.

Each cell fills both Observe slots, verifies refusal of a third distinct
registration, cancels one relation and verifies reuse. Protected subscriptions
use private elective cache-key options to prevent client coalescing. Notification
ACKs and bounded cancellation/backoff are handled before accepting the result;
protected cleanup reports zero unresolved relations. Other pools were not
exhausted by this installed-device campaign.

Each cell receives a known-size 2,000-byte NON Q-Block2 body with stable ETag,
drops the first final 976-byte block after block zero/Size2=2000, requires an
outbound NUM1 suffix repair, then verifies the complete body and a fresh small
read. Protected loss selection authenticates inner metadata using read-only
wire graders. The App and exclusively recovered durable record retain security
authority. This proves host receive-loss recovery and device response reissue;
it does not prove MCU-client suffix recovery or reverse-LAN initiation.

Each cell disables only this device's Wi-Fi for a requested 1,000 ms, then
automatically re-enables it from the main loop. Measured disabled intervals are
1,003,947–1,005,998 us; subsequent connection observations are about 4.42 seconds
after enable. These are software call/observation times, not RF outage bounds.
The shared AP is untouched. One retained App completes a delayed source-bound
Call and a following fresh Call. The initial fast round trip brackets clocks;
grading adds 10 ms uncertainty and requires completion after the latest
calibrated connection observation. All delayed Calls began during the outage.
A Call already pending before disable is not established.

Normal authorized firmware was restored and its live identity verified after
every OTA trial. No erase, provisioning, re-pairing, credential reset, NVS rollback
or security-history replacement occurred. Legitimate durable security advancement
was retained. Passing ordinary/blockwise evidence from the preceding
[96-read qualification](ESP-WIFI-RELIABILITY-20261117.md) was reused.

## One bounded CPU diagnostic

One instrumented async OSCORE image completed eight identity, eight small
(`test`) and eight 2,000-byte Block2 reads: 32 matched device exchanges. All 350
metadata rows were retained without overflow. For every adjacent pair,
wall time equals scheduled task CPU + blocked + ready-but-unscheduled time
with zero observed accounting error.

The diagnostic copies the exact IDF kernel into owned build inputs; the installed
SDK remains unchanged. The linked archive has exactly one `tasks.c` object.
A read-only snapshot includes the current running slice under the kernel lock;
blocked-to-Ready and context-switch observations distinguish blocked waiting from
ready-but-unscheduled time. The worker is unpinned, so snapshots use its current
core. U64 ESP timer runtime counters and the specific non-SMP two-core kernel
are mandatory. CPU includes attributed interrupt time and flash stalls; ready
time includes yields and preemption. Instrumentation overhead is included.

For all eight small reads, observed readiness-to-final-send-return scheduled CPU
averages 14,161.75 us, versus 15,035.875 us wall. The eight large reads average
37,018.875 us CPU and 43,229.75 us wall across both blocks. Host complete-read
means are 18.5958 ms small and 50.193925 ms large. These are different boundaries;
the difference is not isolated radio latency. Large device wall spans include
waiting for the next block request. Readiness means worker-observed select
return, excluding earlier socket enqueue delay; queued receives get a separately
named receive-return boundary.

The final-send boundary excludes post-send cleanup. Where a subsequent select
entry is retained, the extended processing span averages 15,543.143 us CPU for
seven small reads and 38,715.286 us for seven large reads. Other tails lack that
boundary and are not estimated. None of these figures is silently treated as
whole-App CPU. Nested durable-state spans retain 64 reads and 32 commits, totaling
92,609 and 123,507 us scheduled CPU respectively; they are already included in
the enclosing spans and must not be added again.

A resource snapshot inside the measured traffic interval has RX21/TX20, 40,932
bytes free internal heap, largest block 30,720 bytes and worker free-stack
watermark 62,320 bytes. Sampling begins at its recorded timestamp; heap queries
are sequential and may be preempted. The 33,068-byte global minimum includes
startup. Later observer activity lowered it further, outside the traffic window.
These observations include a 28,672-byte fixed diagnostic ledger, runtime
statistics and diagnostic code; they do not establish normal-firmware traffic
peaks or minimum stack requirements. No performance gain, sync CPU result or
unprotected CPU result is claimed. Historical timer/cache candidates remain
rejected.

## Failures, validation and remaining work

All partial attempts remain private and source-matched. Two new protected
assemblies omitted required generated security/interruption setters and were
rejected; the flashed attempt was restored. A later protected Observe attempt
exposed transient notification/TX pacing in the fixture. Bounded ACK draining
and cancellation/backoff handling corrected it. One unprotected runner selected
a Python environment without aiocoap; its valid Wi-Fi proof was retained and
only missing controls were repeated. The earlier mismatched async-archive/sync-
C++ attempts remain rejected fixture failures. None is relabeled as a passing
cell or a core protocol defect.

Relevant local checks pass: async 48 library + 17 driver tests, sync 30 + 17,
strict selected Clippy and formatting, correlation/CPU negative replays, checked
macro-lock graph conservation, and locked metadata acceptance for eight manifests.
Independent read-only reviews checked source, grading boundaries, raw device
receipts, archive/runtime matching and restoration. Raw firmware/configs,
credentials, failures and evidence remain outside the public repositories.

Physical power/interrupted-write #333 needs specific power control; credential
recovery #365 needs the operator's deferred touch; reverse-LAN #341 needs scoped
inbound UDP. They remain unexecuted. Other pool campaigns, broader CPU reduction
and peer-speed goals remain open. Publication, tags and outreach still require
separate #132 authorization.
