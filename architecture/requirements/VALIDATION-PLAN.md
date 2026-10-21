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
| Q7 / [#333](https://github.com/jeffglousher/coaptic/issues/333) | Reserved ESP32-S3 durable local pairing and installed service; companion `tools/esphome/rust`, `tools/durable-host`. | Installed #635 proves adapter-discarded Committed delivery and fresh current-owner host-only repair, followed by protected refusal and software restart. Actual cold power/interrupted persistent transitions remain open; adapter injection is not physical wire-fault proof. Preserve original NVS and credentials. |
| Q8 / #333 / [#365](https://github.com/jeffglousher/coaptic/issues/365) | Owner transfer, backup recovery, revocation and durable effect receipts on the installed two-peer workflow. | Installed host-only/device-only/both-peer software-restart receipt recovery, revocation with Pending, exact receipt-only reconciliation, fresh work, owner A-to-B-to-A and former-owner administrative refusal pass. Installed #636 also proves one live-CON management-teardown slice with a protected retry/release control, expiry guards and durable revocation. Three-distinct-credential backup recovery, consumed-backup refusal and remaining interrupted/ambiguous transitions still need physical execution; other queue families retain their named core proof. A timeout is not authenticated network refusal. |
| Q9 / #333 | Actual entropy, key custody and storage backend. | Companion #127 closes the RF lifecycle race with a main-owner seed and bounded pure-Rust worker generator; target compilation, native tests and installed sessions pass. Companion #132 captures the selected entropy/startup and generation-2 session boundaries. Physical entropy quality and absolute transient peaks remain unmeasured limits. Ordinary durable flash is the default; hostile snapshot/physical rollback resistance is optional hardening. |
| Q10 / [#340](https://github.com/jeffglousher/coaptic/issues/340) | Whole deployment budget and useful progress under pressure. | Selected finite scope is implemented and executed: cloud startup/bound/traffic/Drop accounting, named fixed/allocated owners and disjoint physical reservations, installed #638 refusal/cooldown/recovery and companion #132 initialization/terminal observations. Use the named capacities below; absolute SP/ISR/kernel peaks and minimum deployment RAM are not claimed or added as closure gates. |
| Q11 / [#341](https://github.com/jeffglousher/coaptic/issues/341) | Controlled current-source comparison when requested. | Six-cell SmallDevice Windows quick check at core755485/suiteccb411 passes 200/200 per cell using the #110 harness; compiled-source reuse through core d0366476 / suite6f450f6d is established below. Named volatile OSCORE and unprotected fixtures retain CPU/RSS limits. Companion #125 builds the reverse MCU initiator, but its image is not installed or measured. Matched MCU/durable timing requires scoped inbound Windows permission. |
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

Receipt-only repair is merged in [core #631](https://github.com/jeffglousher/coaptic/pull/631)
and [companion #124](https://github.com/jeffglousher/coaptic-validation/pull/124).
The current installed image is core `d036647698dc38c7b351621cbcbcc61301376f91` /
companion `6f450f6d35418609a2f3eede66150156123e9b38` (tree equal #132 merge
`eb6dcfd92d356da0757d31c17ad523cbe87d95e2`), normal run
`31e69a1bce804eedb783c7ff9f218afb` with the default zero delay. Its authenticated
2,330,128-byte OTA image SHA256 is
`a5c6fa3b50740a980b20bfa4971022ed744918f1afcb429489d4079b0782ddcf`.
Original identity, owner, revision-24 echo grant, completed custody and four effects
remain intact. The finite initialization/terminal receipt below identifies the
separate delayed capture and normal restoration.

The retained custody/recovery image is core `c08de2347649cb8d3b53020c0289707786cb0e0c` /
suite `27b0e6438eff112f17646f4023abb44a41da40cc` (tree equal #127 merge
`421f666bf7bf6cafcbbdb0ff5ea026389e85c05f`), run
`802aed318cb34cdebc3830079d0f1d5c`: authenticated OTA 22.35s, image SHA256
`6cf39f75f884bd7327b628ef5731b67f76b188142653310a14a05220f49fd6d6`,
ELF SHA256 `7cc9cd1cfcf82b259cd7225f9e87d2dda299f852b49e149ee1c6062489ef3e48`.
Original identity/key remain intact; revision 24 restores the original
owner/host and echo-only grant with authenticated completed custody.

Companion #127 fixes background subscription Backoff handling. Read-only probes
now reach authenticated 4.03 after 5.03/Max-Age 1, before and after software
restart. Its entropy provider takes one 32-byte seed under the main Wi-Fi lifecycle
owner before worker creation, then uses a zeroizing, worker-owned Rust generator
with finite per-request/boot limits and permanent exhaustion refusal. Actual SDK
contract review, 80 native tests, strict Clippy, Xtensa archive/link and installed
fresh-session/management checks pass. Physical entropy quality, hostile rollback
and absolute transient peaks are not established by these checks. Companion #132's
later finite initialization/session/terminal observations are recorded below.

Four unique durable journal effects now have installed recovery evidence:
receipt 1 after host-only restart; receipt 2 after revocation with Pending and
explicit old-grant receipt-only reconciliation (also restart/retry while still
revoked); receipt 3 after both-peer restart; and receipt 4 after device-only
`esp_restart()` while host PID 29284 stays alive. Core #633's CLI fixture withholds
receipt 4 at generation 1, releases the original receipt at generation 2, then
USB Inspect and quiescence confirm durable completion before host shutdown.
The closed journal has four rows and an unchanged prior three-row prefix;
exclusive locking prevents external live hashing, so no armed/final hash equality
is claimed. Unresolved regrant and unavailable-journal checks refuse. Normal
owner A-to-B-to-A transfer and former-B USB administrative regrant refusal pass,
with unchanged revision 20 after refusal and fresh original-host 2.05 echo.
The prior B-host timeout does not prove network authentication refusal.
The [installed-current supplement](https://github.com/jeffglousher/coaptic-validation/tree/0dc0b36ea5e4bcb79e8139e60e06738c2fce8247/tools/qualification/evidence/completion-20261020/installed-current)
publishes numeric receipts, source identities and selected safe CLI observations;
private firmware, keys, NVS and raw API captures remain retained locally.

Core #633's tested CLI candidate is `c9e7ad8826becb6381e4bcfa7a1bb7341ef17b62`
(tree equal merge `ee753f974d36a41eab7266ed42e143a184e2018c`), executable SHA256
`38d32ddb2dd43884f5769c039efdbab58c19392edf430305ea7fe8f22a95ad7b`.
The selected device capture has 21 complete 12-task snapshots. Managed 29,184B
is inside future 49,560B; Runtime 104B and RNG 336B also live inside the reserved
196,608B worker stack, whose observed high-water is 177,088B. Component static
storage is 336B; the 16B compiler TLS reservation per task is inside existing
stacks. SDK heap includes dynamic stacks and runtime owners; these views must
not be added together. The numeric summary retains heap-domain overlap and
sampling limits. No PSRAM is configured.

Core [#635](https://github.com/jeffglousher/coaptic/pull/635) qualifies an actual
decoded USB Committed reply discarded before host persistence: the target advances
to read-only revision 21 while the sealed host record stays unchanged. Fresh current-owner
approval repairs only the host; duplicate recovery refuses, protected 4.03 survives
software restart and ordinary pairing restores echo at revision 22. Core
[#636](https://github.com/jeffglousher/coaptic/pull/636), candidate `6162217` / same-tree
merge `d988c979`, then proves a live encrypted CON teardown on this unchanged image.
It releases an exact protected retry as control, holds a fresh CON through its
retry, and uses the same host lock/socket for ordinary approved USB revocation.
Source-bound packet age, session and retry deadlines exclude natural expiry;
final bounded drains cover host scheduling gaps. Challenge is the first teardown,
not the later commit. Revoked revision 23 survives software restart, echo at revision 24 recovers, and
the original 4,768-byte, four-row journal is byte-identical. This is one selected queue
slice; other queue families retain matching-source core proof. The
[executed receipt](https://github.com/jeffglousher/coaptic-validation/pull/131)
names the source, binary, nine logs and measured timing bounds.

The [physical reservation ledger](https://github.com/jeffglousher/coaptic-validation/blob/e7cee494cb93ba249c81ea2c7c123ea4938b2ab0/tools/qualification/evidence/completion-20261020/fixed-physical-memory-reconciliation.json)
reconciles 540,672 bytes of physical SRAM/RTC capacity, counting aliases once. Shared
static reservation is 100,568 bytes, including 17 bytes of alignment beyond the
100,551 bytes of section content. The 3,072-byte ISR reservation is already in
`.data`. Registered heaps of 361,796 bytes include dynamic
stacks/TCBs/security/transport/runtime owners. The 4,556-byte ROM exclusion is explicitly
inferred from the exact SDK registry contract, not directly sampled. This is a
reservation ledger, not minimum deployment RAM or a measured worst-case peak.
A separate 29-snapshot admin/reset receipt has 27 complete post-reconnect
snapshots of 12 tasks: maximum nonfree heap 295,456 bytes, minimum free 66,340
bytes and worker watermark 177,084 of 196,608 bytes. The region-minimum envelope
of 306,968 bytes is not simultaneous peak use, and no
startup/terminal row was captured in that receipt. It remains unchanged and supplies
no historical performance-improvement claim.

Core [#638](https://github.com/jeffglousher/coaptic/pull/638), candidate `592d33a` /
same-tree merge `d0366476`, adds the selected installed pressure slice on the same
`c08de234` / `802aed` image. Three retained device CONs each provoke authenticated
5.03/Max-Age 1 and local Backoff, then exact protected identity 2.05 after the held
retry is released. Generation 1 remains unchanged; source-bound arrival/retry/
release/recovery windows exclude natural cancellation. Six malformed one-byte
datagrams interleave. Revision 24, the sealed host pairing and original 4,768-byte
journal remain unchanged. Companion #132's
[pressure receipt](https://github.com/jeffglousher/coaptic-validation/blob/eb6dcfd92d356da0757d31c17ad523cbe87d95e2/tools/qualification/evidence/completion-20261020/installed-pressure-592d33a.json)
retains four exact logs and finite API aggregates. This is retained-TX admission
and reuse, not installed NSTART refusal, rate fairness or every-queue saturation.

Companion [#132](https://github.com/jeffglousher/coaptic-validation/pull/132),
candidate `6f450f6d35418609a2f3eede66150156123e9b38` / same-tree merge
`eb6dcfd92d356da0757d31c17ad523cbe87d95e2`, completes the finite initialization/
terminal slice at core `d036647698dc38c7b351621cbcbcc61301376f91`. Native 89-test,
configuration, strict Clippy, target Rust and full firmware checks pass. A selected
60,000ms delay permits capture of run `ac4d17ad6ed04eada0e5c4fef7121a05` before
platform setup, entropy seeding and worker creation. Worker phase watermarks are
127,936 bytes before owner construction and 177,092 bytes after construction,
generation-2 session establishment and authenticated completion. The final
pre-deletion watermark is 177,092 of 196,608 bytes; the main producer also emits a
complete 12-task terminal snapshot during `App.safe_reboot` for the restoring OTA.
Setup/running/terminal heap rows are valid. Worker-boundary heap JSON lines were
truncated and are excluded from complete heap-boundary claims.

The [initialization/terminal receipt](https://github.com/jeffglousher/coaptic-validation/blob/bdceac3ad5ff023d00f1588a4bc296a7505b9771/tools/qualification/evidence/completion-20261020/init-terminal-d036647.json)
binds both runs and their exact artifacts. The installed EnrolledDevice App uses
Constrained RX3/TX2×1,152B, dedup4/Observe2, eight routes, four Calls, zero deferred,
four security bindings and one association, with blockwise off. The normal boot
has running snapshots and fresh protected work; its early startup/session records
were outside the capture. Managed 29,184B is nested in future
49,560B / pairing frame 49,760B; Runtime 104B is inside Driver 10,464B; RNG 336B is
inside its 816B outer frame. Component 336B is in BSS, while TLS raw 8B / reserved
16B per task is inside existing stacks. The 540,672B physical partition,
361,796B registered heap and 100,551B shared static content plus 17B padding retain
their disjoint accounting. These are layout/reservation and sampled observations,
not absolute SP/ISR/parking-path peaks, kernel attribution or minimum required RAM.
They complete the selected finite #340 scope; the unmeasured limits remain
explicit in its report.

Cold power/interrupted commits, three-distinct-credential backup recovery and
consumed-backup refusal remain #333/#365 work. On October 21 the operator deferred
touch-dependent qualification, including slot 84's third-credential schedule;
earlier touch validation is retained without claiming that backup schedule ran.
Continue independent work. #341's matched inbound MCU timing requires scoped
Windows firewall permission. #130 coordinates the remaining physical prerequisites
and deferred qualification.

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
after binding/traffic is 699,590B Windows / 662,351B Linux. Cumulative successful
Rust requests are 225 allocations / 7 reallocations on Windows and 212 / 1 on
Linux at binding and after traffic. Startup, bound, traffic and pending-future
Drop have whole-process requested-heap/process snapshots; executor work outside
App polls remains outside the zero scoped-App counts. OS/process totals overlap
component views; the 4MiB fixture stack reservation is not measured peak use.
Individual stack peaks and native/kernel attribution remain unmeasured limits.
No live growth API is added; future expansion must be explicitly selected and
disabled by default. Earlier sync/native-async #119 evidence retains its own
sources and limits.

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

The SmallDevice comparison and finite cloud fixture retain applicable compiled
source evidence through core `d0366476` / suite `6f450f6d`:
its only library-source changes from measured `755485` are two commissioning
files under `src/provisioning`, excluded by the benchmark's explicit
`std,alloc,oscore` selection with EDHOC off. The benchmark disables defaults;
the cloud default only adds OSCORE. Core manifest, lockfile, build/configuration
and macro crate match. The complete companion `tools/benchmark` tree matches
measured `ccb411`; cloud `crates/peer-coaptic`, root manifest and lockfile match
measured Linux `ea242dc`.
The [reuse receipt](https://github.com/jeffglousher/coaptic-validation/blob/bdceac3ad5ff023d00f1588a4bc296a7505b9771/tools/qualification/evidence/completion-20261020/host-source-reuse-d036647.json)
records these exact comparisons. This is compiled-source equivalence, not a new timing or host-environment sample;
it supplies no EDHOC/commissioning, durable-mode or MCU timing result.

## Execution order and prerequisites

1. Preserve the delivered #486/#520/#521 fixes and completed Q1/Q5/Q6 slices.
   Implement concrete fixture or integration gaps in coherent batches, then commit
   a clean source snapshot before provenance-enforcing execution.
2. Preserve completed finite Q2 directions and their retained limitations. The
   selected device's authenticated-history prerequisite, installed receipt
   recovery/reconciliation and read-only refusal defect are resolved. Finish the
   remaining physical transitions and matched MCU measurement prerequisite. Selected
   finite pressure and initialization/terminal resource slices are complete.
   Reuse proof when its exact source and required boundary still match. Keep
   optional DTLS fixtures and their reviewed RFC conflicts separate from the MVP.
3. Execute remaining Q7/Q8 transitions on a source-matched installed image; retain
   the completed finite Q9/Q10 resource evidence. Hardware touch is only
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
