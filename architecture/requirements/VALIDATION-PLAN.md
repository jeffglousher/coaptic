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
| Q1 / [#199](https://github.com/jeffglousher/coaptic/issues/199) | Literal supported UDP TD directions: companion `crates/coaptic-plugtest`, original `tests/plugtest/td-coap4` fixtures. | Enumerate each required scenario/direction once; retain complete endpoint/content/control capture, refusal and recovery. Distinguish engine self-tests from actual peer execution. A reviewed fixture/RFC conflict is a disposition, not literal PASS. |
| Q2 / [#200](https://github.com/jeffglousher/coaptic/issues/200) | Independent UDP/OSCORE directions: companion `tools/interop/capabilities.json`, process runner and protected fixtures. | Source-matched discovery, extended tokens, selected Observe/transfers and ownership/error directions against actual independent implementations. Record each unsupported peer capability instead of substituting a different scenario or shared crypto backend. |
| Q3 / #200 | Scoped/routed IPv6 and network errors using actual interfaces/routes. | Nonzero zone identity, PMTU/ICMP and exact endpoint matching; compound loss/malformed traffic followed by a valid operation. `::1` proves neither routing nor nonzero zone handling. Requires a named usable network topology. |
| Q4 / #200 | Non-idempotent effects against alternative servers. | Duplicate/lost-reply schedules bind one effect to its exact operation and owner; report actual peer behavior, bounded refusal and fresh recovery. A successful repeated GET does not qualify it. |
| Q5 / [#202](https://github.com/jeffglousher/coaptic/issues/202) | Host parser/state/resource campaigns: companion `tools/qualification/{contracts,host,seeded,fuzz}.py` and `contracts.json`; core integration tests. | Each selected requirement has a named bounded input/state schedule, configuration and success/refusal/recovery oracle. Check cancellation, restart, churn and cardinality with owner isolation. Mutation-check the oracle. Coverage percentages and test totals are diagnostic only. |
| Q6 / #202 | Diagnostic host/export adapter consuming #521. | Caller-owned snapshots, fixed reason cardinality, sampled high-water semantics, reset/drop and concurrent collection through explicitly synchronized ownership. No mandatory logger, exporter thread or allocation in core. Measure overhead separately only when making cost claims. |
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

## Execution order and prerequisites

1. Finish verified code defects and diagnostics (#486/#520/#521), with local
   success/refusal/recovery and unchanged whole-object ceilings. Commit a clean
   source snapshot before running provenance-enforcing companion fixtures.
2. Run the finite supported UDP/OSCORE and parser/resource slices affected by that
   snapshot. Reuse proof when its exact source and required boundary still match.
   Keep optional DTLS fixtures separate; fixing a retained GCM finding remains
   necessary maintenance even though DTLS endpoint support is outside the MVP.
3. Execute Q7–Q10 on a source-matched installed image. Hardware touch is only
   needed by the selected administrative approval policy, not ordinary traffic.
   An interrupted commit must be recoverable without regenerating identity keys.
4. Execute Q3 and Q12 when their network/Linux prerequisites are available. A WSL2
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
