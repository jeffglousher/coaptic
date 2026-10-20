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
