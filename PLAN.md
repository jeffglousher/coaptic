# Project plan

Updated 2026-11-17 after core `b875088` and companion main `4190a89`.
Priorities: **reliability, performance, ease of use**, in that order.
This is the current status and work order. Detailed results live in linked
[evidence and history](architecture/requirements/PROJECT-HISTORY-20261117.md).

## Current position

The supported direct UDP App implementation is delivered: bounded fixed/allocated
storage, sync/async operation, OSCORE/EDHOC, commissioning, durable ownership and
application receipts. Software recovery coverage has improved. Current-source
installed-device qualification and the broader faster-than-libcoap/CPU goals
remain unfinished. A completed batch is not project completion.

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
batch. Hosted checks on #763 have since settled without failures. Companion
ordinary CI still pins accepted core `a6400cb`; that proof must not be silently
relabeled as qualification of newer core `b875088`.

## Latest device qualification

The source-matched [ESP32-S3 functional matrix](architecture/requirements/ESP-WIFI-RELIABILITY-20261117.md)
passed sync/async device service in unprotected and durable OSCORE modes:
96 complete timed reads, two nine-case independent unprotected campaigns and two
nine-case protected campaigns. Normal authorized firmware was restored and
verified after each cell. No core protocol fix or performance gain was established.
Two mismatched fixture assemblies were rejected and retained before corrected
sync qualification. All advisory checks on #764 settled without failure.

## Active batch

The finite existing-fixture functional batch is complete. No builds or device
changes remain active. This does not complete installed reliability qualification
or the broader performance goal; unsupported cells below remain open.

## Next batch: missing device recovery controls

Extend the existing finite fixture to select known-size NON Q-Block2 trailing
loss, count duplicate effects and force explicit pool refusal/reuse. Add a bounded
per-device Wi-Fi interruption/reconnect control with automatic recovery, preserving
the shared AP, credentials, NVS, workers and capacities. Qualify each supported
sync/async and security cell on matching Rust/C++ runtime images; restore and verify
normal firmware. Do not repeat passing ordinary/blockwise cells without a concrete
concern. Track this under [#130](https://github.com/jeffglousher/coaptic/issues/130).

The next performance assignment is a bounded measurement of current-device active
task CPU and socket-ready-to-send time for the same small/large reads, separating
preemption/waits from radio wall latency. Existing host traces cannot identify the
largest device cost. Keep this measurement distinct from a new optimization
campaign; [#341](https://github.com/jeffglousher/coaptic/issues/341) remains open.

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
