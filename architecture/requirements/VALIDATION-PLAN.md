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

| ID / owner | Selected boundary and existing executable | Completion evidence still required |
| --- | --- | --- |
| Q1 / [#199](https://github.com/jeffglousher/coaptic/issues/199) | Literal supported UDP TD directions: companion `crates/coaptic-plugtest`, original `tests/plugtest/td-coap4` fixtures. | Completed selected inventory in companion #90: 52 required IDs, 133 passing direct-UDP directions, exact conditional/readback grading and 87 refusal mutations. All 52 same-Coaptic scenarios pass. The 23 mixed-peer Observe fixture skips remain Q2; DTLS02 is a reviewed alert conflict, never literal PASS. |
| Q2 / [#200](https://github.com/jeffglousher/coaptic/issues/200) | Independent UDP/OSCORE directions: companion `tools/interop/capabilities.json`, process runner and protected fixtures. | Source-matched discovery, extended tokens, selected Observe/transfers and ownership/error directions against actual independent implementations. Record each unsupported peer capability instead of substituting a different scenario or shared crypto backend. |
| Q3 / #200 | Scoped/routed IPv6 and network errors using actual interfaces/routes. | Completed selected six cases in companion #100-102: four scoped/routed IPv6 roles, wrong-zone refusal/fresh recovery, actual host-socket ICMPv6 PTB/PMTU recovery, and lost-reply/malformed traffic with exact retransmission and fresh work. Owned topology cleanup passes. This is not automatic App PMTU adaptation. |
| Q4 / #200 | Non-idempotent effects against alternative servers. | Companion #98 characterizes both retained alternative-server fixtures: duplicate CON POST applies twice, distinct fresh POST and GET succeed. This is an explicit fixture limitation, not successful suppression or a whole-library verdict. Installed one-effect/lost-receipt recovery remains Q8. |
| Q5 / [#202](https://github.com/jeffglousher/coaptic/issues/202) | Host parser/state/resource campaigns: companion `tools/qualification/{contracts,host,seeded,fuzz}.py` and `contracts.json`; core integration tests. | Selected finite inventory executed: 52 named protocol groups, bounded parser/CBOR mutations, fixed/allocated storage churn, replay models and compound lifecycle schedules. Four evidence-oracle refusal tests reject absent, ignored, failed or stale test records. See the source-bound result below; coverage percentages and totals are not whole-RFC proof. |
| Q6 / #202 | Diagnostic host/export adapter consuming #521. | Delivered in companion #88: synchronized caller-owned recorder, sampled maxima, explicit reset/drop behavior and bounded CBOR v1 export. Nine focused host/service tests cover isolation and unchanged durability/replay outcomes; strict Clippy passes. No mandatory logger, exporter thread or allocation in core. Overhead remains unmeasured. |
| Q7 / [#333](https://github.com/jeffglousher/coaptic/issues/333) | Reserved ESP32-S3 durable local pairing and installed service; companion `tools/esphome/rust`, `tools/durable-host`. | Source-matched cold power cycle, interrupted/ambiguous commits and lost administrative replies; same identity/owner/permissions or explicit fail-closed repair. Preserve original NVS and credentials. Software restart and an archive build are separate evidence. |
| Q8 / #333 / [#365](https://github.com/jeffglousher/coaptic/issues/365) | Owner transfer, backup recovery, revocation and durable effect receipts on the installed two-peer workflow. | Execute old-owner refusal, affected queued-work cancellation, consumed-backup refusal, same-operation receipt after host/device restart and lost reply, then fresh authorized work. Recovery from a revoked tombstone with an unreconciled old-grant journal must refuse; explicit reconciliation is required and must not discard or relabel effects. |
| Q9 / #333 | Actual entropy, key custody and storage backend. | Name platform entropy failure behavior, persistence/commit guarantees, custody and initialization peaks. Ordinary durable flash is the default; hostile snapshot/physical rollback resistance is optional hardening and has no implied ordinary-NVS claim. |
| Q10 / [#340](https://github.com/jeffglousher/coaptic/issues/340) / [#341](https://github.com/jeffglousher/coaptic/issues/341) | Whole deployment budget and useful progress under pressure. | Begin with `SmallDevice` / `Constrained`; name route, Call, deferred and security capacities. Distinguish whole-App layout, initialization peak, actual stack/heap and transport/security reservations without double counting. Execute admitted-peer progress/refusal/recovery with measured memory placement. |
| Q11 / #341 | Controlled current-source comparison when requested. | Equivalent complete operations, body, concurrency, security, durability and initiator directions; throughput/latency/failures plus measured system requirements. Competing-library/protected gaps stay explicit. No speedup or current-performance claim follows from an older isolated checkpoint result. |
| Q12 / [#522](https://github.com/jeffglousher/coaptic/issues/522) | Observation-only Rust eBPF ingress pilot on a named Linux hook. | Actual object/compiler/kernel/hook identity and verifier result; bounded stack/instructions/maps/work, shared CoAP parsing, conservative pass on unsupported packets, differential cases and concurrent attach/detach/reload recovery. Windows execution/cross-compilation alone cannot satisfy this slice. No kernel authentication or admission authority is implied. |

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
Selected GET/FETCH and CON Q-Observe directions, one protected extended-token
capacity/refusal boundary and four native-EDHOC/CoAP-envelope option-extension
profiles remain Q2. Existing method/option and protected receipts retain their
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

Q7/Q8 have current installed echo-only proof: core `775b42c` / companion
`0b597ea`, run `ab15edb5b3bc49078e5c2751d53c9e69`, plus host liveness fix
#619. Two fresh host processes establish authenticated sessions and protected
echoes without a device reset or unnecessary rekey; the second accepts Observe.
Identity and pairing are preserved. This does not qualify telemetry effects,
revocation, physical power interruption or production custody. The installed
telemetry integration is delivered by core #623 / companion #103: exact permission
restoration, authenticated Pending transitions, bounded producer drain/hold and
an explicit AfterSync lost-receipt boundary. Twenty focused pairing tests include
real UDP restart/drain proof; actual installed telemetry execution remains open.
Missing/unverifiable history never means first use.

Q12 now has actual kernel acceptance at core
`244809ebcd852cfeaf912bd2f5ba5956b6495bfc`, companion `d290314`,
[run39249773527](https://github.com/jeffglousher/coaptic-validation/actions/runs/39249773527).
Both instrumented cycles pass 4,556 differential cases, multi-CPU counters,
protected/unprotected unchanged bytes and live attach/detach/reload. The kernel
reports 352+0 bytes stack and 344 translated instructions. A 20-second child
startup deadline expired while matching successful verifier loads took about
20.3 seconds; terminated-child recovery, ordinary loader and capability refusal
were not reached. Companion #105 extends the bounded startup budget; exact
[run39252498484](https://github.com/jeffglousher/coaptic-validation/actions/runs/39252498484)
executes those stages against the same core. Earlier failed
verifier objects/logs are preserved. WSL2 installation is separate and still
requires administrator elevation while the operator is away.

## Execution order and prerequisites

1. Preserve the delivered #486/#520/#521 fixes and completed Q1/Q5/Q6 slices.
   Implement concrete fixture or integration gaps in coherent batches, then commit
   a clean source snapshot before provenance-enforcing execution.
2. Complete the finite remaining Q2 integration directions and installed Q8
   effect ownership.
   Reuse proof when its exact source and required boundary still match. Keep
   optional DTLS fixtures and their reviewed RFC conflicts separate from the MVP.
3. Execute Q7–Q10 on a source-matched installed image. Hardware touch is only
   needed by the selected administrative approval policy, not ordinary traffic.
   An interrupted commit must be recoverable without regenerating identity keys.
4. Preserve completed Q3; finish the remaining Q12 lifecycle stages on the hosted
   Linux lane. A WSL2
   lane requires installed virtualization support and an actual distribution;
   installation alone does not establish BPF support or a passing verifier result.
5. Execute Q11 as a short controlled report, then widen only for a measured
   implementation question. Long stability campaigns remain outside this batch.

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
