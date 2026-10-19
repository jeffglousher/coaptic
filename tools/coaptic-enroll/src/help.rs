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

Persistent service (also the default when MODE is omitted):
  --connect       Serve authorized requests and observe the peer's identity.
                  Keep running until stopped; fresh sessions use stored pairing.

One-shot checks (30-second deadline):
  --probe         Read and verify the peer's identity.
  --probe-echo    Send and verify a protected echo.
  --probe-denied  Verify that the peer refuses an echo with 4.03 Forbidden.

Shared integration check (separate handshake and operation deadlines):
  --probe-shared  Exercise concurrent shared connection handles.

Identity and echo probes retry authenticated 4.29/5.03 responses at most three
attempts, respecting Max-Age within the deadline. Durable telemetry has its own
receipt policy. The service holds the pairing lock; stop it before management.
No operator signature or key touch is needed for ordinary communication.",
        )
    } else {
        None
    }
}
