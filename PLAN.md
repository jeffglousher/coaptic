# Project plan

Updated 2026-11-19 after measuring accepted core `c172c44` and companion
`9019751`. Production Rust remains `b875088`. Companion
[#266](https://github.com/jeffglousher/coaptic-validation/pull/266) delivers the
Windows process-cycle diagnostic.
Current effort: **performance**, with reliability as an acceptance condition.
This is the current status and work order. Detailed results live in linked
[evidence and history](architecture/requirements/PROJECT-HISTORY-20261117.md).

## Current position

The supported direct UDP App implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, commissioning, durable ownership and
application receipts. Software recovery coverage has improved. Selected current-source installed-device recovery cells are qualified. Broader
qualification and faster-than-libcoap/CPU goals remain unfinished. A completed batch is not project completion.

## Preceding software batch

- [#760](https://github.com/jeffglousher/coaptic/pull/760): same-App recovery across
  finite datagram blackouts, including identical retries, duplicate suppression,
  capacity refusal/recovery, cancellation and expiry, in sync/async and
  unprotected/OSCORE cells. This is software injection, not physical Wi-Fi proof.
- [#762](https://github.com/jeffglousher/coaptic/pull/762): bounded custom elective
  request options and the runnable sync/async `application_options` example,
  covering payload, Accept/Content-Format, deadlines and useful errors.
- [#763](https://github.com/jeffglousher/coaptic/pull/763): known-size NON Q-Block2
  trailing-loss recovery. The integrated core passes 1,711 all-feature tests,
  743 no-default library tests and 91 source-selected companion checks, plus
  strict lint, MSRV, rustdoc and an Xtensa OSCORE library check. Original
  capacities and whole-App ceilings remain; native body sidecar stays 688 bytes.
  See the [source-matched receipt](https://github.com/jeffglousher/coaptic/pull/763#issuecomment-6954242320).

No new performance improvement or firmware execution was established by that
batch. Hosted checks on #763 have since settled without failures. The recovery batch explicitly advances companion ordinary CI to accepted core
`b875088`; older CI proof is not relabeled.

## Latest device qualification

The source-matched [ESP32-S3 functional matrix](architecture/requirements/ESP-WIFI-RELIABILITY-20261117.md)
passed sync/async device service in unprotected and durable OSCORE modes:
96 complete timed reads, two nine-case independent unprotected campaigns and two
nine-case protected campaigns. Normal authorized firmware was restored and
verified after each cell. No core protocol fix or performance gain was established.
Two mismatched fixture assemblies were rejected and retained before corrected
sync qualification. All advisory checks on #765 settled without failure.

## Latest recovery batch

All four sync/async and unprotected/durable OSCORE device cells passed observable
duplicate effects, explicit Observe refusal/reuse, known-size NON Q-Block2 suffix
loss/repair and same-App Call retry/reconnect during a bounded per-device Wi-Fi
interruption. Normal firmware was restored and verified after every trial.
Q-Block2 loss is on host receive; MCU-client recovery and other pool exhaustion
remain separate. Calls started during the outage; pre-existing-at-disable Calls
are not established. The [source-matched recovery record](architecture/requirements/ESP-DEVICE-RECOVERY-20261118.md)
retains failures, exact scope, image/archive identities and independent review.

One bounded async OSCORE CPU diagnostic completed 24 authenticated reads and 32
matched exchanges, with 350 complete metadata rows and protected traffic resource
observations. It separates scheduled CPU, blocked and ready-unscheduled time;
readiness/send and available post-send tails have explicit boundaries. It is not
whole-App CPU, a normal-firmware peak-memory claim or a performance gain. No core
protocol fix or new optimization was required.

Implementation and evidence reviews are complete. No device/build job remains
active; normal firmware restoration is verified. Detailed custody and private
evidence locations belong in the batch handoff.

## Active performance and reliability effort

This is a continuation checkpoint. After verifying current state and custody,
execute the next unblocked batch in the same run; completed qualification and
advisory-check settlement alone are not reasons to end. Follow the
[continuation and handoff procedure](DEVELOPMENT.md#finish-a-batch-or-hand-off)
for actual stop or transfer conditions.

1. The [current Windows screen](architecture/requirements/WINDOWS-RUNTIME-SCREEN-20261119.md)
   covers both runtime and security modes, complete small and blockwise reads,
   process time and cycles, and an independent libcoap screen. High-rate pacing
   missed periods. Lower-rate cycles are more repeatable than process CPU time
   in the pilot. Two independent endpoints also complete overlapping CON Calls
   in all selected cells. The Tokio path remains a priority after the
   same-runtime sync control. Existing staged receive does not earn adoption.
2. The [Windows phase attribution](architecture/requirements/WINDOWS-UDP-PHASES-20261119.md)
   records 282,080 verified reads and a further 256,000-read matched Windows
   reactor screen. A smaller completion dequeue bound reduces small-read
   server cycles. Four blockwise pairs miss the advancement gate, so retain
   the default bound. The [64-entry follow-up](architecture/requirements/WINDOWS-IOCP-BOUND-20261119.md)
   establishes a Windows API argument effect and rejects the 64-entry
   candidate on complete operations. Companion
   [#268](https://github.com/jeffglousher/coaptic-validation/pull/268)
   proves same-process oversized receive recovery in 12 Windows cells.
   Resetting the async burst budget after a pending wait also misses the
   gate. Retain the current Tokio scheduling policy.
3. The [matched peer screen](architecture/requirements/WINDOWS-PACED-PEERS-20261119.md)
   establishes a protected median latency advantage in these Windows cells
   and a 600/s eligible rung for protected blockwise C1. The 800/s rung
   misses the pacing guard. The [paired-start follow-up](architecture/requirements/WINDOWS-PAIRED-PEERS-20261119.md)
   verifies overlapping 4,096-byte operations across two endpoints and
   passes the protected 800/s rung. The 1,000/s rung misses pace in the
   libcoap arms. Keep the original #650 baseline, both 50% targets and
   physical prerequisites.
4. The [Tokio loopback tests](tools/coaptic-tokio/README.md) exercise body
   refusal and reuse, Call refusal and established OSCORE binding refusal.
   Keep #130 open for the remaining runtime/security cells and device
   qualification. Reuse settled recovery evidence where it applies.
5. The [standalone async-io screen](architecture/requirements/WINDOWS-HOST-ADAPTERS-20261119.md)
   passes selected small-read cycle pairs and misses the blockwise gates.
   Both adapters now report oversized receives as InvalidData on Windows
   and prove same-socket reuse.
6. The [common Mio screen](architecture/requirements/WINDOWS-ORIGINAL-MIO-20261119.md)
   compares current core with the original #650 source. Both protected
   4,096-byte pairs reduce server cycles, with inconsistent CPU-time
   ratios. The unprotected 4,096-byte pairs fail the p99 guard. Keep
   the original matrix and both targets open. Check whether the
   synchronous standard UDP transport can expose the same oversized
   refusal as the async host adapters while preserving App progress.

## Blocked and deferred work

- [#333](https://github.com/jeffglousher/coaptic/issues/333): physical cold-power and
  interrupted persistent-write recovery requires physical power control.
- [#365](https://github.com/jeffglousher/coaptic/issues/365): installed three-credential
  backup-owner recovery requires the operator's deferred credential touch.
- [#341](https://github.com/jeffglousher/coaptic/issues/341): reverse MCU-to-host LAN
  cells require scoped inbound UDP access. Host-to-MCU proof does not complete them.
- [#650](https://github.com/jeffglousher/coaptic/issues/650): CPU reduction and the
  broader peer-speed objective remain open. No new candidate qualified in the last batch.
- [#647](https://github.com/jeffglousher/coaptic/issues/647): volatile plaintext and
  zeroization policy is future hardening research, not an established remote defect.
- Runtime adapter development is complete for the preceding optimization batch.
  Embassy firmware migration, new platforms and unrelated tooling remain deferred.
  Publication, tags and outreach need separate authorization under
  [#132](https://github.com/jeffglousher/coaptic/issues/132).

## Retained decisions and evidence

The accepted packet-lifetime batch measured 2.91% lower complete-App CPU cost sync
and 3.28% async against its pinned baseline, with smaller selected futures and App
owners. These are [source-specific historical results](architecture/requirements/PACKET-LIVE-RANGES-20261106.md),
not fresh device or current-main measurements.

Quiet-dispatch, timer-lifetime, cache-size and target-layout integrations failed
preservation or speed gates. Keep the
[failed sources, raw vectors and limits](architecture/requirements/ESP-TIMER-LIFETIMES-20261117.md)
and [earlier experiment history](architecture/requirements/PROJECT-HISTORY-20261117.md).
Do not rerun unchanged failures or use host results from rejected code as accepted
current-source performance. The rejected extra Q-Block2 mode field also exceeded
the whole-App ceiling; its smaller receive-state representation is the accepted fix.

[Protocol scope](architecture/IETF-RFC-PLAN.md), the
[qualification checklist](architecture/requirements/VALIDATION-PLAN.md), and
[runtime/comparison policy](architecture/ASYNC-RUNTIMES.md) define supported
behavior and valid claims. Keep this page short and current; detailed acceptance
belongs in issues and execution history in evidence documents.
