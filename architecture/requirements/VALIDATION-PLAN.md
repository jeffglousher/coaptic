# Qualification checklist

[PLAN.md](../../PLAN.md) sets priorities.
[Protocol scope](../IETF-RFC-PLAN.md) and the requirement ledgers define the
behavior being qualified. This page lists remaining execution and where to find
completed proof.

## Remaining runs

### Cold power and interrupted persistence — #333

Needs physical control of the reserved ESP32's power.

- Capture the installed image, identity, owner, permissions, custody and unresolved
  application receipts before the run.
- Exercise cold power and the selected interrupted persistent transitions.
- Verify exact recovery or explicit fail-closed repair. Preserve identity, NVS
  and unresolved effects; never replace missing history with an empty journal.
- Restore the normal image and authorized state and retain the execution receipt.

Software restart, a discarded USB reply and a simulated transport loss do not
substitute for physical power interruption.
Owner: [#333](https://github.com/jeffglousher/coaptic/issues/333).

### Installed backup-owner recovery — #365

Needs the operator's deferred credential touch action.

- Use three distinct owner, backup and destination credentials.
- Recover on the installed device, then refuse reuse of the consumed backup.
- Verify former-owner refusal, retained effects and fresh authorized operation.

Existing touch-required registration/signing and software P-256 proofs do not
complete this installed schedule. Ordinary reconnects still require no touch.
Owner: [#365](https://github.com/jeffglousher/coaptic/issues/365).

### MCU-to-host LAN execution — #341

Needs scoped inbound UDP permission on the Windows host.

- Run the unprotected and protected MCU-to-host cells with equivalent workloads.
- Retain complete-response timing and measured resource views.
- Restore the original firmware and temporary network configuration.

Host-to-MCU cells have run. Reverse drivers and target/native builds are delivered;
those builds are not reverse runtime results.
Owner: [#341](https://github.com/jeffglousher/coaptic/issues/341).

These three prerequisites remain open. They do not block independent software
work. Extra boards, sustained availability campaigns, absolute ISR/stack/kernel
peaks and minimum possible RAM are not additional gates for this finite milestone.

## What a useful result contains

Identify the core and companion revisions, compiler, lockfiles, target, features,
capacities, security/durability settings and actual peer/backend. Device results
also identify the installed image and its installation receipt.

Keep original failures, unsupported directions, deadlines and fresh recovery.
A result applies to its recorded source and configuration. Reuse needs a matching
source/boundary; a new CI pin is not new execution. An ACK is not an application
commit, and shared crypto backends are not independent cryptographic proof.

For memory, distinguish component layout, stack reservations, heap requests,
runtime/socket storage and observed process use. Nested views overlap. Sampled
watermarks and finite regional minima do not establish a simultaneous worst-case
peak. Preserve the source and limits with each number.

## Completed proof

Use the linked owners and receipts for details instead of copying execution
history into this plan:

- [Literal scenarios #199](https://github.com/jeffglousher/coaptic/issues/199),
  [independent peers #200](https://github.com/jeffglousher/coaptic/issues/200) and
  [host/parser/resource checks #202](https://github.com/jeffglousher/coaptic/issues/202)
  retain the finite inventories, exact sources and peer limitations.
  [Companion #115](https://github.com/jeffglousher/coaptic-validation/issues/115)
  retains an unexplained earlier server exit; no process-stability claim follows.
- [Deployment accounting #340](https://github.com/jeffglousher/coaptic/issues/340)
  is complete for its selected startup/traffic/Drop, initialization, terminal and
  installed-pressure checks. It does not claim minimum RAM or absolute peaks.
- [Installed-device receipts](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/qualification/evidence/completion-20261020)
  retain identity/custody, owner transfer/back, software-restart receipt recovery,
  revocation, commissioning-reply repair and selected live-CON/pressure proof.
  Original firmware, keys, NVS and raw private captures remain preserved.
- [Current protected/unprotected comparison](https://github.com/jeffglousher/coaptic-validation/tree/98b54fe8fd4c9b76301d2d22a533af40ecade744/tools/benchmark/evidence/comprehensive-windows-20261022)
  retains the full matrix, all failures, exact sources, resource accounting and
  independent socket-packet grades. OSCORE copy cleanup is delivered; the
  before/after probe does not establish a large speedup. Native UDP, matched
  Python UDP/OSCORE, test-only DTLS adapters and HTTP/3 remain separate strata.
  The patched Go control is distinct from original timing observations.
- [Host-to-MCU evidence](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/qualification/evidence/matched-host-mcu-20261021)
  records the executed direction and unchanged-image restoration.
- [Kernel observation #522](https://github.com/jeffglousher/coaptic/issues/522)
  is complete for the actual Linux hook/lifecycle slice. It makes no
  authentication or acceleration claim; local WSL elevation is not a missing
  prerequisite for the retained hosted-Linux proof.

Close an implementation issue when the code and relevant proof are delivered.
Keep required qualification open until executed or explicitly removed from scope.
Publication remains a separate decision under
[#132](https://github.com/jeffglousher/coaptic/issues/132).
