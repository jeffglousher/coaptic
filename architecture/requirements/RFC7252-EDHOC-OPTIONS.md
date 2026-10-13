# RFC 7252 Options in the EDHOC Bootstrap Transport

## Implementation boundary

This assessment covers `CoapProvisioner`'s sequential, fixed-storage unicast
bootstrap profile: Confirmable POST requests, piggybacked or separate responses,
and the supported initial Echo challenge. RFC 9668 supplies the EDHOC transport
mapping; this ledger assesses its base CoAP option handling, not complete EDHOC
or CoAP compliance. The parent [IETF RFC plan](../IETF-RFC-PLAN.md) retains the
partial EDHOC assessment and the separate UDP-origin assessment.

Option recognition is specific to the selected message role: request Uri-Path
and Content-Format, response Content-Format, and initial-challenge Echo. Other
elective options, invalid elective lengths and supernumerary elective occurrences
are ignored. Unsupported critical requests at the EDHOC resource receive 4.02;
matching separate Confirmable responses receive Reset. The
complete datagram, including ignored option bytes, remains the identity of
retained duplicate replies. No normalization changes cache ownership, timers,
entropy or credentials. One fixed pending control record and twelve bytes of
send scratch bound refusal response work. Ingest does not overwrite a pending
control; the caller drains it with `flush` or `poll`. After sending, the same
slot reserves the refused MID and original response until its own exchange
lifetime expires. Changed tokens/bodies cannot replace that owner. Fresh valid
traffic cannot bypass refusal ownership by changing CON to NON; such collisions
are discarded without acknowledging a NON message. Changing request/response
code class under the same peer-owned MID also cannot cross owners; those
collisions are discarded without replaying the wrong control kind. Fresh valid
traffic uses a distinct MID; a new refusal owner is not admitted while the slot
is reserved. Replays do not renew expiry or handshake deadlines. Completed
controllers serve retained owners only. Recovery completion and retirement
caches preserve refusal ownership without retaining handshake cryptographic
state; cache residency covers the later of its original and refusal deadlines.

## Requirement assessment

| Requirement identity | Applicable requirement | Implementation and direct proof | Status |
| --- | --- | --- | --- |
| RFC7252-5.4-reserved | Reserved option zero is treated as unrecognized. | [Option filtering](../../src/provisioning/coap.rs); all new elective fixtures include zero. | Direct proof established for this transport. |
| RFC7252-5.4.1-elective | Unrecognized elective options are silently ignored on reception. | Zero-allocation filtered iterator used by request, response and Echo matching. `elective_options_complete_authenticated_sequential_bootstrap` adds options before/between/after required metadata, repeated/empty values and a large option number to all four handshake datagrams; it verifies authenticated principals and matching directional keys/IV across piggybacked, separate Confirmable and Non-confirmable replies, including exact ACK destination/MID and completion after required sends. `elective_options_echo_preserves_entropy_payload_and_deadline` checks exact message 1, reserved retry MID/token, unchanged wait deadline and duplicate challenge behavior. | Direct success proof established; independent option-extension peer proof pending. |
| PROFILE-duplicate-ownership | Ignored bytes do not become a different owner or renew a cached exchange. This is a transport contract, not an additional RFC mandate. | `elective_options_preserve_exact_duplicate_ownership_and_send_recovery` refuses same-MID removal or mutation of ignored options, replays the exact cached response, retains bytes after failed/short sends and completes a fresh valid transition. | Direct refusal/failure/recovery proof established. |
| RFC7252-5.4-role | Options undefined for the method/response are treated as unrecognized. | Role-specific option recognition; `bootstrap_options_elective_roles_and_repetitions_complete_authentication` includes Observe/Max-Age on POST and unrelated elective response metadata, then completes authentication. | Direct proof established for the selected profile; not a general application-option implementation. |
| RFC7252-5.4.1-critical-request | Unsupported critical Confirmable requests receive 4.02. | `bootstrap_options_critical_request_control_is_bounded_and_recovers` verifies endpoint, ACK, code, MID, token, empty payload and fixed slot; rejects overwrite, retains exact controls after failed/short sends, then authenticates. `bootstrap_options_critical_refusal_preserves_accepted_mid_cache` prevents critical-option mutation from crossing accepted MID ownership and proves fresh refusal followed by cached/fresh recovery. | Direct wire/refusal/recovery proof established at this resource; independent peer proof pending. |
| RFC7252-5.4.1-critical-response | Unsupported critical responses are rejected, with Reset for separate Confirmable responses. | `bootstrap_options_critical_response_reset_has_exact_ownership_and_recovery` checks endpoint/token isolation, ACK/NON rejection without response, exact four-byte Reset MID/endpoint, failed/short-send retention, unchanged deadlines and valid authenticated recovery. | Direct wire/refusal/recovery proof established; independent peer proof pending. |
| RFC7252-5.4.3-length | Invalid request-option lengths are treated as unrecognized. | Content-Format length at most two; Echo challenge length one through forty. `bootstrap_options_invalid_first_format_cannot_be_repaired_by_repeat` and `bootstrap_options_echo_repetition_keeps_first_occurrence_and_deadline` prove invalid-first refusal and fresh valid recovery. Response metadata uses the same checked Content-Format boundary. | Direct proof established for recognized profile fields; broader option semantics remain outside this profile. |
| RFC7252-5.4.5-repeat | Supernumerary non-repeatable occurrences are treated as unrecognized. | Iterator remembers the preceding option number, including invalid first occurrences; repeatable Uri-Path remains intact. Role/repetition and Echo tests above retain the first recognized value, ignore extras and preserve authentication/deadlines. | Direct proof established for recognized profile fields. |
| PROFILE-control-bound | One control slot has finite lifetime and cannot renew handshake work. | `bootstrap_options_control_expiry_and_clock_overflow_preserve_bounds` proves unsent expiry without output/handshake mutation, fresh authentication and checked deadline-overflow refusal before crypto. | Direct bounded failure/recovery proof established; sustained hostile-traffic fairness remains unqualified. |
| RFC7252-4.5-refusal-owner | Refused exchanges preserve peer MID ownership and original duplicate controls during their lifetime. | Request-control test proves changed token/body refusal, same original response and fresh-MID recovery. `bootstrap_options_recovery_handoff_preserves_refusal_mid_and_failed_send` proves handoff replay, exact failed/short-send retention and original expiry without renewal. `bootstrap_options_recovery_retirement_keeps_later_refusal_expiry` proves quarantine beyond an earlier handshake-cache deadline, no replacement candidate and exact expiry release. | Direct ownership/recovery proof established for the selected controller and transferred cache. |

The section 5.4.1 SHOULD diagnostic payload is omitted for this bounded bootstrap
profile: the fixed 4.02 header/token identifies refusal without reflecting
unauthenticated option bytes or reserving another body. This is an explicit
SHOULD disposition, not a claim that diagnostic payloads are forbidden.
Cancelled/failed recovery candidates retain ownership as quarantine without
replies. Their duplicate-response SHOULD disposition follows the explicit
retirement policy: termination stops reply work while preventing a stale MID
from becoming a replacement handshake. Successful completion caches retain
their original replies and refusal controls normally.

Named tests are in [coap_tests.rs](../../src/provisioning/coap_tests.rs). Their
references and implementation resolve in the same Git revision as this ledger.
The reviewed PR retains executed candidate/toolchain identities and results.
Both no-default `edhoc` and `alloc,edhoc` configurations are tested; this
controller uses fixed storage in either configuration.
Reserved option zero is an intentional receiver robustness fixture; compliant
senders are prohibited from including it.

## Remaining applicable work

- Complete the clause/dependency/errata inventory and independent review of
  every applicable base CoAP requirement beyond the option boundary assessed
  here. Other resources, proxy roles and application option semantics require
  their own assessments; this is not whole-RFC compliance.
- Qualify sustained hostile-traffic fairness and admission across peers. One
  bounded refusal slot does not establish general availability or an
  independently bounded multi-association runtime.
- Qualify independent EDHOC peers carrying elective extensions, including
  separate responses, exact duplicate ownership and fresh recovery.
- Assess `CoapRecovery`'s initial admission dispatcher before a candidate exists.
  It currently routes only supported message-1 metadata; this ledger's 4.02
  processing proof applies to `CoapProvisioner` and active recovery candidates,
  not that initial dispatcher or every application adapter.
- Complete the RFC 9528/9668 requirement inventory and remaining error/profile,
  suite, EAD and installed-policy work owned by
  [#365](https://github.com/jeffglousher/coaptic/issues/365). This option increment
  does not implement combined requests or broaden the authentication profile.

Normative sources: retained [RFC 7252](../../knowledge/rfcs/rfc7252.txt)
sections 4.2, 5.4, 5.4.1, 5.4.3 and 5.4.5, and [RFC 9668](../../knowledge/rfcs/rfc9668.txt)
section 2. Source review and direct tests are separate from independent peer
proof and full applicable-requirement compliance.
