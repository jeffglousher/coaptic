# Selected-profile qualification plan

This is the finite execution plan for the direct UDP endpoint milestone. The
[IETF plan](../IETF-RFC-PLAN.md) selects roles; the linked profile ledgers map
requirements to source. This document identifies proof still needed. It does not
turn an unexecuted campaign into a result or reopen delivered implementations.

## Evidence contract

Every result records clean core and companion commits, lockfiles, compiler,
features, target/runner, configured resource capacities and actual peer/backend.
Device execution also binds the built archive, image hash and installation
receipt to the observed firmware. A newer CI pin does not update old evidence.
Reports retain failures, unsupported directions, deadlines, capture bounds and
fresh-recovery results. Transport acceptance is not remote receipt; ACK is not an
application commit. Independent security proof requires different implementations.

Use existing compatible build lanes with debug information and incremental output
disabled where appropriate. Do not create one target per campaign. Give generated
artifacts an owner, size budget and release condition. No campaign requires hosted
CI; execute a coherent batch once and repeat only to address a concrete failure.

## Owned execution slices

| ID / owner | Selected boundary and existing executable | Completion evidence and remaining boundary |
| --- | --- | --- |
| Q1 / [#199](https://github.com/jeffglousher/coaptic/issues/199) | Literal supported UDP TD directions: companion `crates/coaptic-plugtest`, original `tests/plugtest/td-coap4` fixtures. | Completed selected inventory in companion #90: 52 required IDs, 133 passing direct-UDP directions, exact conditional/readback grading and 87 refusal mutations. All 52 same-Coaptic scenarios pass. The 23 original mixed-peer Observe fixture skips retain Q2 dispositions; they are not literal PASS. DTLS02 is a reviewed alert conflict, never literal PASS. |
| Q2 / [#200](https://github.com/jeffglousher/coaptic/issues/200) | Independent UDP/OSCORE directions: process inventory, secondary `independent-profiles.json` and protected fixtures. | Completed finite scope in companion #116: selected Observe/Q lifecycle, protected extended-token boundary and native-EDHOC option envelopes. Exact source pairs and original failures remain retained; the secondary 18 selectors do not alter the 181-case evaluator. Unsupported capabilities and the intermittent Windows fixture exit stay explicit. |
| Q3 / #200 | Scoped/routed IPv6 and network errors using actual interfaces/routes. | Completed selected six cases in companion #100-102: four scoped/routed IPv6 roles, wrong-zone refusal/fresh recovery, actual host-socket ICMPv6 PTB/PMTU recovery, and lost-reply/malformed traffic with exact retransmission and fresh work. Owned topology cleanup passes. This is not automatic App PMTU adaptation. |
| Q4 / #200 | Non-idempotent effects against alternative servers. | Companion #98 characterizes both retained alternative-server fixtures: duplicate CON POST applies twice, distinct fresh POST and GET succeed. This is an explicit fixture limitation, not successful suppression or a whole-library verdict. Installed one-effect/lost-receipt recovery remains Q8. |
| Q5 / [#202](https://github.com/jeffglousher/coaptic/issues/202) | Host parser/state/resource campaigns: companion `tools/qualification/{contracts,host,seeded,fuzz}.py` and `contracts.json`; core integration tests. | Selected finite inventory executed: 52 named protocol groups, bounded parser/CBOR mutations, fixed/allocated storage churn, replay models and compound lifecycle schedules. Four evidence-oracle refusal tests reject absent, ignored, failed or stale test records. See the source-bound result below; coverage percentages and totals are not whole-RFC proof. |
| Q6 / #202 | Diagnostic host/export adapter consuming #521. | Delivered in companion #88: synchronized caller-owned recorder, sampled maxima, explicit reset/drop behavior and bounded CBOR v1 export. Nine focused host/service tests cover isolation and unchanged durability/replay outcomes; strict Clippy passes. No mandatory logger, exporter thread or allocation in core. Overhead remains unmeasured. |
| Q7 / [#333](https://github.com/jeffglousher/coaptic/issues/333) | Reserved ESP32-S3 durable local pairing and installed service; companion `tools/esphome/rust`, `tools/durable-host`. | Source-matched cold power cycle, interrupted/ambiguous commits and lost administrative replies; same identity/owner/permissions or explicit fail-closed repair. Preserve original NVS and credentials. Software restart and an archive build are separate evidence. |
| Q8 / #333 / [#365](https://github.com/jeffglousher/coaptic/issues/365) | Owner transfer, backup recovery, revocation and durable effect receipts on the installed two-peer workflow. | Execute old-owner refusal, affected queued-work cancellation, consumed-backup refusal, same-operation receipt after host/device restart and lost reply, then fresh authorized work. Recovery from a revoked tombstone with an unreconciled old-grant journal must refuse; explicit reconciliation is required and must not discard or relabel effects. |
| Q9 / #333 | Actual entropy, key custody and storage backend. | RF-provider lifecycle closure remains an active implementation follow-up; the readiness guard is not complete entropy qualification. Name failure behavior, persistence/commit guarantees, custody and initialization peaks. Ordinary durable flash is the default; hostile snapshot/physical rollback resistance is optional hardening. |
| Q10 / [#340](https://github.com/jeffglousher/coaptic/issues/340) / [#341](https://github.com/jeffglousher/coaptic/issues/341) | Whole deployment budget and useful progress under pressure. | Begin with `SmallDevice` / `Constrained`; name route, Call, deferred and security capacities. Distinguish whole-App layout, initialization peak, actual stack/heap and transport/security reservations without double counting. Execute admitted-peer progress/refusal/recovery with measured memory placement. |
| Q11 / #341 | Controlled current-source comparison when requested. | Current six-cell SmallDevice Windows quick check at core755485/suiteccb411 passes 200/200 per cell using the #110 harness. Named volatile OSCORE and unprotected fixtures retain CPU/RSS limits. Companion #125 builds the reverse MCU initiator, but its image is not installed or measured. Matched MCU/durable timing and complete peaks remain open. |
| Q12 / [#522](https://github.com/jeffglousher/coaptic/issues/522) | Observation-only Rust eBPF ingress pilot on a named Linux hook. | Complete in run39254465762 at exact core244809e / companion7d34275: actual kernel acceptance, differential cycles, counters, byte preservation, lifecycle/process recovery and capability refusal. Named verifier/object/resource metrics are retained below. No kernel authentication, admission authority or measured performance benefit is implied. |

The companion process manifest is an executable case inventory, not an RFC
requirement ledger. DTLS qualification fixtures remain distinct from the active
MVP. New supported features need explicit requirement/test rows; older inventory
counts cannot establish that the current feature set is qualified.

## Completed literal and host slices

Q1 is delivered by [companion #90](https://github.com/jeffglousher/coaptic-validation/pull/90),
merged as `7e6107c03d41f9624beb4241b26f31ff06e762de`. CORE20–23 now execute
their full conditional workflows rather than shortened substitutes. Mandatory
format fields and endpoint/control ownership have refusal checks. The full run
at companion `8dedf8c` / core `a508757` passed 22 of 23 tests; its sole failure
was an overly strict LINK09 format grader. The corrected six affected tests
passed at `d868d32` / core `ede6ee8`; final reason-only `e5ca104` passed strict
Clippy. Original failure and rerun records are retained with #199. Timed Observe
and supplemental DTLS scenarios were not unnecessarily repeated. CORE15/16 use
the permitted request-loss branch, not every listed loss alternative.

Q5/Q6 use core `73ec43c0bc8db0e204eacb221b42d140459f6d36` and companion
[host integration #88](https://github.com/jeffglousher/coaptic-validation/pull/88),
merged as `434e1d88092b71b231f7bd4552fbc9d2a9ea53ea`. The retained
[protocol inventory result](https://github.com/jeffglousher/coaptic-validation/blob/434e1d88092b71b231f7bd4552fbc9d2a9ea53ea/tools/qualification/evidence/core-profiles-73ec43c.json)
binds named tests to hashes of their original logs. It includes corrected fixture
reruns and retains the earlier failing records. This is a completed batch with
focused reruns, not a claim that the original full command passed unchanged.

The executed core campaigns include 50,000 datagram mutations, 50,000 CBOR
mutations, 6,000 fixed/allocated storage operations, 100,000 replay-model steps,
512 authenticated corruption/recovery cases, unprotected and OSCORE lifecycle
schedules, and durable replay restart/storage-error cases. Problem-details tests
cover required fields, extensions and malformed, indefinite and nested inputs.
These finite schedules establish their named refusal/recovery contracts. They do
not establish independent peer, physical device or exhaustive concurrency proof.
Earlier 32-bit and s390x results retain their earlier sources; neither was rerun
for this snapshot. Q2 and Q7–Q12 retain their separate owners; completed Q3/Q4
slices are recorded below.

Q2 has an executed partial slice in [companion #91](https://github.com/jeffglousher/coaptic-validation/pull/91),
merged as `a9f7d5d814b368e43f7549fece7b6f88946e8d81`. Fifteen supported
independent metadata, URI, extended-token and Observe cases pass against their
recorded aiocoap/libcoap implementations. Libcoap's nonzero initial Observe is
unsupported, with a separate refusal/recovery control. Initial-Valid failures
exposed the format-binding and GET continuation defects corrected in core #612;
the retained successful captures identify their exact intermediate sources.
The final async cancellation regression passes independently of those captures.
Host fixture stack and response-buffer allowances are not device RAM results.
The six selected GET/FETCH and CON Q-Observe directions now pass at their
recorded source pairs. Core #627 fixes ACK-driven CON Q windows and duplicate
retention without additional state. Companion #109-114 correct native fixture
selection, stateless FETCH, valid NON pacing, held producer admission and strict
cancellation/recovery grading. Both Q directions complete three 704-byte epochs,
missing-block recovery, cancellation and fresh work. Companion #116 retains
[18 exact selectors](https://github.com/jeffglousher/coaptic-validation/blob/cdc5b81dede6d8a80145443dce07fced883e0a5b/tools/interop/independent-profiles.json)
in a secondary inventory: seven profile directions plus eleven bootstrap
option directions, separate from the unchanged 181-case process evaluator.
Original failures remain unchanged. The reverse-only diagnostic pass did not
explain an earlier Windows server exit; [companion #115](https://github.com/jeffglousher/coaptic-validation/issues/115)
retains that host-fixture limitation. No adapter error policy was changed, and
this finite proof is not process stability qualification. The independent native
protected-token success/capacity/tamper/fresh-recovery row passes at core 7bb32c9 /
companion f6151265 in run39254030000; its six Observe fixture failures remain
retained. Companion #108 separately passes eleven native-EDHOC/CoAP-envelope
option directions at core ce7e817 / fixture 8f88d76. This is independent native
EDHOC cryptography with a Python transport envelope, not a full native CoAP
stack. The [bootstrap option ledger](RFC7252-EDHOC-OPTIONS.md) links exact
sources/capture. Existing method/option and protected receipts retain their
source-specific validity; no full option/security cross-product is required.

Q2 also includes the finite independent EDHOC slice in
[companion #95](https://github.com/jeffglousher/coaptic-validation/pull/95):
24 raw sessions across both selected suites/roles, ordered EAD, private exporter
contexts and six UDP error directions with fresh protected recovery. The
[evidence index](https://github.com/jeffglousher/coaptic-validation/blob/574c5b5c034e4ee0511eacce6a1a47aeaf4ae177/tools/qualification/evidence/independent-edhoc/index.json)
identifies native adapters, unsupported full-u64 labels/padding and callback
timing limits. These are finite profile results, not whole-RFC certification.

Q3 passes at exact core `edfd1eb1669e8c164fc318e47f531d1c7e41e7a5` /
companion `d5e93eb4f4b8b91bf51878ce6ec76d1dbc629e0f` for four addressing roles
([run39246185107](https://github.com/jeffglousher/coaptic-validation/actions/runs/39246185107)),
and core `567899121ace814ebfb417b55705adbcb1299215` /
companion `d290314823343b0f6caf8f0b7a5cc0d0c6e39569` for actual PMTU and
compound traffic
([run39247160176](https://github.com/jeffglousher/coaptic-validation/actions/runs/39247160176)).
Both have complete owned-network cleanup; the initial fixture/cleanup failure
is retained. Q4's exact counter-fixture sources and limitation receipts are in
[companion #98](https://github.com/jeffglousher/coaptic-validation/pull/98).

Q7/Q8 retain the [published baseline](https://github.com/jeffglousher/coaptic-validation/tree/d6b84dbfaea6b0f16d9583e040853adc225d6393/tools/qualification/evidence/completion-20261020)
at core `75548536d6ea417dc3f4a74c7fc9fc4e03ab5953` / companion
`ccb41104a138876f59527cfb2414f7bfa102e663` (tree equal merged #123 `971f4d7`).
Independently reviewed NVS revision/page history and controlled-workflow custody
resolved first use; absence and source chronology alone remain insufficient.
Migration `ca1ce3422cf4baaa128f897b4ae3f537` created the verified one-time claim
and authenticated 1,214-byte custody record. Permit-disabled normal run
`012e92f1a2aa3724e65a7f4aa41b1c1b` preserved identity/key/owner/echo grant/revision 8.
The bundle retains Inspect/echo, raw numeric API records, boot-scoped task margins,
exact ELF layouts and overlapping heap-domain limits; private NVS remains sealed.

Receipt-only repair is now merged in [core #631](https://github.com/jeffglousher/coaptic/pull/631)
and [companion #124](https://github.com/jeffglousher/coaptic-validation/pull/124).
The installed recovery image is core `68664c2a5004d9b370113af3c6cec60b1d571fd8` /
suite `1990fcfcf75516c172dbd713f6bf9a819943fc6c` (tree equal #125 merge
`7e47d12284a06fcd385fac24e497d7574ba604f2`), run
`407105ebaaac4bd4da558ccd78bac5f4`: authenticated OTA 22.36s, unchanged identity
and revision 8 echo grant, fresh CLI protected 45-byte echo from authenticated
5.03/Max-Age 1 to same-session 2.05. ELF SHA256
`59218eb016f7428ce307f8f27a8a324a1001a1076229e568530eeff0bca9f4be`
independently confirms managed 29,184B inside future 49,560B; Runtime 104B also
lives on the reserved 196,608B worker stack. These are nested layouts, not a sum.
The new-source finite capture has 22 complete 12-task snapshots and observed
worker high-water 176,248B; it excludes the older run's pre-OTA records.

Installed queued-work receipt reconciliation, revocation/old-owner/consumed-backup
refusal, lost replies, one/both-peer restart recovery and fresh work remain Q8
proof obligations. A new read-only revision 9 persists across software restart,
but its echo-refusal probe reaches a 30s deadline after initial 5.03; liveness is
an active defect, not a passing refusal result. The native async driver classifies
background Observe/GET cooldown Backoff as fatal; its focused fix/regression is
active until target qualification. Echo-only restoration at revision 10 again
passes fresh-session 5.03 to 2.05. Actual power cuts/interrupted management
persistence remain unexecuted. RF-provider lifecycle
closure remains active in Q9; the readiness guard does not qualify complete
entropy or production custody. The original host journal is initialized, with
no installed effect/receipt recovery claimed from the echo observation.

Q10 adds finite native-async pressure proof at core
`75548536d6ea417dc3f4a74c7fc9fc4e03ab5953`: Windows implementation
`4a426bec054d78955e34e4bc57045a4576f8f3c3` has an exact
[receipt](https://github.com/jeffglousher/coaptic-validation/blob/7ac82e297d15c1ab0c8e087a630ed78f7292652f/tools/qualification/evidence/cloud-startup/native-pressure-7554853.json);
Linux suite `ea242dc5bdf430c779ec7c2086d336fba93955e2` passes
[run39421315970](https://github.com/jeffglousher/coaptic-validation/actions/runs/39421315970).
Both unprotected/ephemeral-OSCORE modes complete seventeen 2,000-byte replies,
one small reply, local Call saturation/cancellation recovery, actual 5.03/Max-Age 1
under 31/32 held server TX slots, cooldown, 16 malformed datagrams and fresh full
recovery. Scoped App allocation/reallocation requests stay zero. Pools are
startup-sized Service RX32/TX32×1,472B, four 16,384B bodies/direction, two routes,
four Calls, zero deferred, four security bindings and one association.
Requested pool layout is 282,640B/App; whole-process requested live Rust heap
after binding/traffic is 699,590B Windows / 662,351B Linux without successful
allocation-count growth in that interval. OS/process totals overlap component
views; the 4MiB fixture stack reservation is not measured peak use. Startup,
executor work between App polls and pending-future Drop are outside scoped App
counts. Individual stack peaks, native/kernel attribution, initialization/rekey
overlap and installed pressure proof remain open. No live growth API is added;
future expansion must be explicitly selected and disabled by default. Earlier
sync/native-async #119 evidence retains its own sources and limits.

Q11 reruns the existing SmallDevice harness at core
`75548536d6ea417dc3f4a74c7fc9fc4e03ab5953` / companion
`ccb41104a138876f59527cfb2414f7bfa102e663`; the
[published raw receipt](https://github.com/jeffglousher/coaptic-validation/blob/d6b84dbfaea6b0f16d9583e040853adc225d6393/tools/qualification/evidence/completion-20261020/smalldevice-windows-755485.json)
retains every latency and tool/source/lock identity. All six cells pass 20 warmups
and 200 complete 32-byte GET/CON operations at concurrency 1. Median/p95/max
microseconds and verified operations/second are:

- Coaptic SmallDevice unprotected: 53.3/67.4/95.7; 18,101/s.
- Coaptic SmallDevice volatile OSCORE: 163.3/203.0/321.8; 5,994/s.
- aiocoap 0.4.16 unprotected: 247.5/315.9/357.7; 3,861/s.
- aiocoap 0.4.16 volatile OSCORE: 497.3/594.4/714.3; 1,948/s.
- coap-rs 0.28.1 unprotected: 84.5/111.4/180.1; 10,921/s.
- coap-lite 0.13.3 codec endpoint unprotected: 58.5/78.8/105.0; 15,775/s.

This shared Windows/Ryzen 5800X3D sample overlapped ESPHome compilation; it is
neither a quiet-host ranking nor a historical improvement claim. Whole fixed
SmallDevice App is 13,128B including adapter/scratch, four routes/one populated,
one Call, zero deferred, one association/four bindings, RX2/TX2×1,152B and no
blockwise bodies. CPU windows include setup/warmup/spin; process RSS overlaps
App/runtime/stack. Durable commits and MCU timing are not measured by these cells.
[Companion #125](https://github.com/jeffglousher/coaptic-validation/pull/125)
builds the synchronous reverse MCU 24-GET fixture at core 68664c2/suite 1990fcf,
run `69ec4c6764cfc5455b7aaa3e41e17db8`, but this separate image is not installed.
Its body-enabled Constrained configuration is not the paired/SmallDevice profile.
The four-role matched MCU timing matrix still needs actual complete captures.
Scoped Windows inbound permission for the exact host executable/reserved device
requires human UAC; the user is unavailable and the firewall remains unchanged.

Q12 is complete at core `244809ebcd852cfeaf912bd2f5ba5956b6495bfc` /
companion `7d34275be771ee1b689c9005c2eb2745b74586e7`,
[run39254465762](https://github.com/jeffglousher/coaptic-validation/actions/runs/39254465762).
The actual kernel accepts the object. Three differential cycles (two
instrumented, one ordinary) each pass 4,556 BPF executions plus 1,456 exact
short-frame kernel refusals: 6,012 corpus vectors per cycle. Multi-CPU/saturation
accounting and live protected/unprotected unchanged bytes pass. Both loaders pass SIGKILL child recovery and parent isolation;
actual all-capability and BPF/SYS_ADMIN refusals leave identical before/after
interface inventories. Kernel 6.17.0-1022-azure/image 20261012.334.1 reports 352+0B
stack, 344 translated instructions (2,752 B), 1,816 B JIT and 4,096 B program memlock.
The five 8-byte per-CPU map's 464 B reported memlock is runner accounting. The ELF
SHA256 598ebc622ea76e0b23d818ba5c4a7b705403933c88e6f90a628bb160cd917ec2
matches independent Windows emission. Retained logs, original failures and the
[artifact](https://github.com/jeffglousher/coaptic-validation/actions/runs/39254465762/artifacts/12187792308)
are linked from closed #522. AF_PACKET receipts do not establish authenticated
application delivery, native-driver support or a performance benefit. WSL2
installation remains separate and requires administrator elevation.

A separate finite detached cost probe passes in
[run39359240571](https://github.com/jeffglousher/coaptic-validation/actions/runs/39359240571)
at core `cdabea60b43db0a9ec110276b9122d45c85eab73` / companion
`f09f168eb4897f483cbd3345d41b070bf75f6d6c`, using unmodified Aya.
Across five batches of 10,000 invocations, median kernel-reported average times
are 158 ns for another UDP port, 160 ns for a 32-byte unprotected payload,
170 ns for opaque OSCORE syntax and 429 ns for 126 CoAP options; the minimal
PASS control is 28 ns in each case. Expected counters match all 400,800
control/classifier invocations, including warmups, and every returned frame
matches its input. Neither program
is attached to a network interface in this probe. These are medians of kernel
batch averages, not individual-packet percentiles, NIC throughput or application
latency. The current classifier adds observation cost; no acceleration is shown.

## Execution order and prerequisites

1. Preserve the delivered #486/#520/#521 fixes and completed Q1/Q5/Q6 slices.
   Implement concrete fixture or integration gaps in coherent batches, then commit
   a clean source snapshot before provenance-enforcing execution.
2. Preserve completed finite Q2 directions and their retained limitations. The
   selected device's authenticated-history prerequisite is resolved; finish actual
   installed Q8 effects/reconciliation and the active read-only refusal defect.
   Reuse proof when its exact source and required boundary still match. Keep
   optional DTLS fixtures and their reviewed RFC conflicts separate from the MVP.
3. Execute Q7–Q10 on a source-matched installed image. Hardware touch is only
   needed by the selected administrative approval policy, not ordinary traffic.
   An interrupted commit must be recoverable without regenerating identity keys.
4. Preserve completed Q3 and Q12 source-bound evidence. A separate WSL2 lane
   requires installed virtualization support and an actual distribution;
   installation alone does not establish BPF support or a passing verifier result.
5. Preserve the completed Q11 short host report. Widen comparisons or deployment
   peak measurements only for a concrete implementation question or requested
   qualification batch. Long stability campaigns remain outside this batch.

Publication is separately owned by [#132](https://github.com/jeffglousher/coaptic/issues/132).
It needs a concrete reviewed candidate, package/license inspection, requested
release validation and explicit publication authorization. This development plan
does not authorize a tag, registry upload or external outreach.

## Closure rules

Close delivered implementation issues with merged source and relevant focused
proof. A mixed issue should identify its remaining qualification boundary instead
of calling delivered code absent. Close unsupported role-selection work with the
operator's explicit scope disposition, never as an implementation success.
Qualification issues remain open until their selected evidence is executed and
reviewed, or the operator explicitly changes their scope. Project status follows
the same truth; a green hosted run does not override a retained counterexample.
