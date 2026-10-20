# EDHOC Error Reception in the Sequential CoAP Profile

## Implementation boundary

This assessment covers inbound server error responses while `CoapProvisioner`
awaits message 2 or message 4, and their routing through `CoapRecovery` and
`ManagedConnection`. The selected method-3/suite-2/reference-credential profile
requires message 4. Peer errors are unauthenticated and cannot select a suite,
revoke trust, install a context or start recovery. The caller explicitly decides
whether to start another handshake.

`PeerError` borrows the retained response buffer. Diagnostic text is validated
UTF-8, at most 255 bytes; advertised suites are at most nine signed eight-bit
identifiers. Other encodings, malformed metadata and unsupported extensions are
ignored under the selected bounded receiver policy. No allocation, diagnostic
truncation, automatic logging or new datagram buffer is required.
Reserved code zero is an exception: once identified in a matching error
response, it aborts even if the following information is malformed or outside
these decoder bounds. Mandatory termination does not depend on that field.

## Requirement assessment

| Requirement identity | Implementation and direct proof | Status |
| --- | --- | --- |
| RFC9528-6.1-reserved-zero | Reserved-success errors abort without sending an EDHOC error, independent of information-field parsing. Parser fixtures include integer 256, missing/truncated information and trailing bytes; `peer_error_responses_abort_each_stage_after_exact_ack_and_retain_diagnostics` covers both waits and ACK/CON/NON response forms. | Direct mandatory-zero/fail-closed proof; full CBOR error grammar remains unqualified. |
| RFC9528-6-receiver-policy | Recognized applicable errors terminate the candidate, drop its handshake state and stop handshake retransmission. The transport ACK is drained before `PollError::PeerAbort` is returned. | Adopted receiver abort policy; no identity or recovery authority is inferred. |
| RFC9528-6.2-diagnostic | `PeerError::diagnostic` exposes the complete supported diagnostic by reference; Debug omits its contents. `peer_error_diagnostic_capacity_and_every_truncation_are_checked` proves the 255-byte boundary and truncated-input refusal. | Direct proof within declared capacity. Logging is caller policy. |
| RFC9528-6.3-suite-error | Code 2 is accepted only while awaiting message 2. The bounded iterator exposes advisory signed suite identifiers. Empty/singleton/excess lists, unsupported numeric ranges and wrong-stage errors are ignored. | Reception implemented; sender-side negotiation and capability expansion remain open. |
| RFC9528-6.4-credential-error | Code 3 is accepted while awaiting message 4, where the selected message 3 carried a credential reference. Invalid information type and inappropriate message-1 replies are ignored. | Direct selected-role proof; no credential discovery or transfer is implemented. |
| RFC9528-A.2.3-response | Matching error-class response, token/ACK MID and recognized response Content-Format are required. Role-specific option processing retains critical-option priority. Wrong endpoint/token/MID/format and malformed error-response input preserve the valid candidate. | Direct success/refusal and subsequent authenticated recovery proof. |
| RFC7252-4.2-error-ACK | Recognized separate CON errors retain exact empty ACK bytes through failed/short sends. Complete response bytes own duplicates and MID collisions until original expiry. | Direct control/failure/expiry proof. |
| PROFILE-recovery-owner | `peer_error_recovery_preserves_live_owner_ack_cache_and_fresh_authentication` preserves the old active ID/principal, error diagnostics and ACK cache, refuses changed bytes and authenticates a caller-started fresh session. | Direct proof; no automatic recovery. |
| PROFILE-managed-routing | `peer_error_managed_routing_preserves_authorized_app_and_allows_explicit_recovery` proves demultiplexing, unchanged trust, authorized protected application traffic, full-cache refusal and caller-started recovery after cache expiry. | Direct fixed-storage managed-service proof. |

Tests resolve in [peer_error.rs](../../src/provisioning/peer_error.rs),
[coap_tests.rs](../../src/provisioning/coap_tests.rs),
[recovery_tests.rs](../../src/provisioning/recovery_tests.rs) and
[managed_tests.rs](../../src/provisioning/managed_tests.rs). The reviewed PR records
executed candidate/tool identities and results; independent peer qualification
is separate.

## Established context mapping and remaining work

`Session::derive` already maps local/peer connection IDs to OSCORE recipient/
sender IDs. `extended_connection_ids_authenticate_and_derive_full_oscore_identifiers`
and `peer_selected_connection_ids_derive_correct_oscore_directions` directly
check complete bounded identifiers, directional keys and protected delivery.
These existing tests do not establish all RFC 9668 reservation requirements or
the mandatory error reply for equal initiator/responder IDs.

The [security profile](SECURITY-PROFILES.md) and
[bootstrap ledger](RFC7252-EDHOC-OPTIONS.md) retain the later selected
suite/EAD/exporter/error and independent-envelope results. This receiver slice
alone does not supply that broader proof.
[#365](https://github.com/jeffglousher/coaptic/issues/365) retains remaining
installed authorization/effect qualification. Unknown future error codes and encodings outside this receiver's stated
bounds have no support claim. Cancelled or locally failed candidates retain the
previous quarantine policy; a recognized peer abort instead retains transport
ACKs and diagnostics until its cache expires. This consumes one existing cache
slot and cannot evict another owner. Sustained availability remains unqualified;
independent error results retain their separately recorded sources and limits.

The [IETF RFC plan](../IETF-RFC-PLAN.md) records L3 for the selected EDHOC profile;
this is not whole-RFC qualification. Normative sources
are original [RFC 9528](../../knowledge/rfcs/rfc9528.txt) sections 6 and A.2.3,
[RFC 9668](../../knowledge/rfcs/rfc9668.txt) section 4 and
[RFC 7252](../../knowledge/rfcs/rfc7252.txt) sections 4.2 and 4.5.
