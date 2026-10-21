use std::ffi::OsStr;

pub fn command(command: &OsStr) -> Option<&'static str> {
    if command == "inspect" {
        Some(
            "Usage: coaptic-enroll inspect [--port PORT] [--host NAME]

Show local credential-record presence, host names and serial ports.
  --host NAME  Verify this host's stored identity and pairing details.
  --port PORT  Query only this device, with a 10-second response limit.

Inspection does not sign or change pairing. Local inventory does not contact
devices. A failed selected target is reported while other results remain visible.
Credential-record presence is not a live key test.",
        )
    } else if command == "serial-pair" {
        Some(
            "Usage: coaptic-enroll serial-pair PORT HOST HOST_ADDRESS [--read-only | --echo-only] APPROVAL

Approve one exact device grant and persist its reciprocal host pairing only after
the device confirms its commit. Stop the host service to release its pairing lock.
APPROVAL: --piv-touch or --development-no-touch; omit for Windows WebAuthn.

Qualification only: --qualify-lost-commissioning-reply
  Discard the actual decoded USB Committed reply before host pairing persistence.
  The command fails deliberately; it does not retry the update. Inspect both
  records, then use serial-recover with fresh current-owner approval. The selected
  grant must differ from the host's stored permissions/enabled state for recovery.
  This injects receive loss in the host adapter, not a physical USB disconnect.
  The flag is accepted once and only by serial-pair. Other errors stay unchanged.",
        )
    } else if command == "peer" {
        Some(
            "Usage: coaptic-enroll peer HOST LOCAL_ADDRESS PEER_ADDRESS [MODE]

Restore the named host's pairing and communicate over EDHOC/OSCORE.
Addresses are numeric IP:port pairs; use [IPv6]:port for IPv6.
Every mode uses the same deterministic role: the greater credential fingerprint
initiates. A responding host needs OS firewall permission for incoming UDP.

Persistent service (also the default when MODE is omitted):
  --connect       Serve authorized requests and observe the peer's identity.
                  Keep running until stopped; fresh sessions use stored pairing.

One-shot checks (30-second deadline):
  --probe         Read and verify the peer's identity.
  --probe-echo    Send and verify a protected echo.
  --probe-denied  Verify that the peer refuses an echo with 4.03 Forbidden.

Qualification-only fault injection (approved telemetry grant required):
  --qualify-lost-receipt  Exit 75 after the first new durable effect is synced,
                         before sending its receipt. Preserve the journal and
                         restart normally to recover the same effect/receipt.
  --qualify-device-restart
                         Keep this same host process alive for at most90 seconds.
                         Withhold the first new durable receipt using5.03/Max-Age1.
                         After the armed marker, restart only the ESP32. A later
                         protected session releases only that original receipt;
                         all other new effects refuse before journal commit.
                         Record the physical action and unchanged host PID, then
                         inspect durable completion and quiesce before stopping.
                         A fresh session alone does not prove a device restart.

Shared integration check (separate handshake and operation deadlines):
  --probe-shared  Exercise concurrent shared connection handles.

Installed admission qualification (45-second finite window):
  --qualify-pressure
                  Current echo-only outbound-initiating host required. Lose one
                  echo ACK as control, then hold each of three fresh target CONs.
                  Require authenticated5.03/Max-Age1 for an identity request while
                  the target TX is held, verify local cooldown, release its exact
                  echo retry, then verify protected identity2.05 in the same session.
                  Interleave six fixed malformed1B datagrams; no packet queue,
                  telemetry, journal or management changes. Source/time guards
                  refuse missed windows and session changes. Target source/profile
                  matching needs the installation receipt, not network attestation.
                  Selected retained-TX reuse only; no all-queue, installed NSTART
                  refusal, rate fairness or whole-system memory-peak claim.

Queued-CON qualification (ends with device and host revoked):
  --qualify-queued-revoke PORT --development-no-touch
                  Current echo-only, outbound-initiating host and Complete device
                  custody required. Lose one echo reply, deliver its exact retry,
                  then hold another real CON through an exact retry. Discard the
                  host session before normal fresh current-owner approval and USB
                  revocation, keeping the same pairing lock and UDP socket.
                  Observe old ciphertext through its next would-send deadline.
                  No telemetry effect, touch, packet queue or pressure claim.
                  A missed timing window refuses qualification; inspect before
                  recovery. Restore service with ordinary approved serial-pair.
                  Firmware source matching is an external installation receipt;
                  this does not attest firmware or qualify power loss/all queues.

Identity and echo probes retry authenticated 4.29/5.03 responses at most three
attempts, respecting Max-Age within the deadline. Durable telemetry has its own
receipt policy. The service holds the pairing lock; stop it before grant changes.
Serial inspection/quiescence can run while this host service remains alive.
No operator signature or key touch is needed for ordinary communication.",
        )
    } else if command == "serial-reconcile" {
        Some(
            "Usage: coaptic-enroll serial-reconcile PORT HOST PRIOR_PAIRING.cbor APPROVAL

Reconcile an already committed telemetry receipt after revocation. Serial-revoke
tries to retain the exact public device checkpoint in the host directory. If
storage fails, it warns and prints public CBOR without delaying revocation. Keep
that checkpoint and the original receipt journal; missing history is not reset.

Requires the same device/store, current owner and original service/permissions,
a disabled current policy, its older enabled checkpoint, authenticated retained
operation metadata and the exact committed journal row. Current-owner approval
binds the fresh device challenge, prior checkpoint and complete receipt.

APPROVAL: --piv-touch or --development-no-touch; omit for Windows WebAuthn.
Use --approval-directory DIRECTORY for a separately registered current owner.
Pairing and journal locks remain held. This never applies a new effect or restores
permissions. Inspect after an uncertain reply; retry only the same retained repair.",
        )
    } else if command == "serial-quiesce" {
        Some(
            "Usage: coaptic-enroll serial-quiesce PORT

Privileged USB only. Drain existing durable work for at most 30 seconds while
the host service keeps serving receipts. Refusal preserves unresolved work.
On success, hold new production for 60 seconds; an administrative challenge extends
the hold through its own approval deadline. Expiry resumes the existing policy.
After success, stop the host service before pairing to release its store locks.
No grant changes and no host-store lock are taken by this command.",
        )
    } else {
        None
    }
}
