# RFC 7252 Options in the EDHOC Bootstrap Transport

## Implementation boundary

This assessment covers `CoapProvisioner`'s sequential, fixed-storage unicast
bootstrap profile: Confirmable POST requests, piggybacked or separate responses,
and the supported initial Echo challenge. RFC 9668 supplies the EDHOC transport
mapping; this ledger assesses its base CoAP option handling, not complete EDHOC
or CoAP compliance. The parent [IETF RFC plan](../IETF-RFC-PLAN.md) retains the
partial EDHOC assessment and the separate UDP-origin assessment.

Unknown elective options are ignored when matching the supported profile. The
implementation recognizes the option set exposed by `OptionNumber::is_known`;
known options outside this transport's supported metadata remain refused. The
complete datagram, including ignored option bytes, remains the identity of
retained duplicate replies. No normalization changes cache ownership, timers,
entropy, credentials or resource limits.

## Requirement assessment

| Requirement identity | Applicable requirement | Implementation and direct proof | Status |
| --- | --- | --- | --- |
| RFC7252-5.4-reserved | Reserved option zero is treated as unrecognized. | [Option filtering](../../src/provisioning/coap.rs); all new elective fixtures include zero. | Direct proof established for this transport. |
| RFC7252-5.4.1-elective | Unrecognized elective options are silently ignored on reception. | Zero-allocation filtered iterator used by request, response and Echo matching. `elective_options_complete_authenticated_sequential_bootstrap` adds options before/between/after required metadata, repeated/empty values and a large option number to all four handshake datagrams; it verifies authenticated principals and matching directional keys/IV across piggybacked, separate Confirmable and Non-confirmable replies, including exact ACK destination/MID and completion after required sends. `elective_options_echo_preserves_entropy_payload_and_deadline` checks exact message 1, reserved retry MID/token, unchanged wait deadline and duplicate challenge behavior. | Direct success proof established; independent option-extension peer proof pending. |
| PROFILE-duplicate-ownership | Ignored bytes do not become a different owner or renew a cached exchange. This is a transport contract, not an additional RFC mandate. | `elective_options_preserve_exact_duplicate_ownership_and_send_recovery` refuses same-MID removal or mutation of ignored options, replays the exact cached response, retains bytes after failed/short sends and completes a fresh valid transition. | Direct refusal/failure/recovery proof established. |
| PROFILE-strict-boundary | Ignoring unknown elective options does not silently enable known unsupported or critical options. | `elective_options_do_not_hide_critical_or_known_unsupported_options` proves unknown critical, OSCORE and Observe refusal before entropy/authorization, then valid authenticated recovery. | Direct state-preservation proof established; not proof of the required critical-option wire reply. |

Named tests are in [coap_tests.rs](../../src/provisioning/coap_tests.rs). Their
references and implementation resolve in the same Git revision as this ledger.
The reviewed PR retains executed candidate/toolchain identities and results.
Both no-default `edhoc` and `alloc,edhoc` configurations are tested; this
controller uses fixed storage in either configuration.
Reserved option zero is an intentional receiver robustness fixture; compliant
senders are prohibited from including it.

## Remaining applicable work

- Unknown critical Confirmable requests still need the required 4.02 reply.
  Rejected separate Confirmable responses need the applicable Reset behavior;
  current state-preserving ignore does not establish those wire requirements.
- Assess option applicability, recognized elective metadata, repeated
  non-repeatable options, invalid lengths/values and method/response-specific
  dispositions across the complete bootstrap profile.
- Qualify independent EDHOC peers carrying elective extensions, including
  separate responses, exact duplicate ownership and fresh recovery.
- Complete the RFC 9528/9668 requirement inventory and remaining error/profile,
  suite, EAD and installed-policy work owned by
  [#365](https://github.com/jeffglousher/coaptic/issues/365). This option increment
  does not implement combined requests or broaden the authentication profile.

Normative sources: retained [RFC 7252](../../knowledge/rfcs/rfc7252.txt)
sections 5.4 and 5.4.1, and [RFC 9668](../../knowledge/rfcs/rfc9668.txt)
section 2. Source review and direct tests are separate from independent peer
proof and full applicable-requirement compliance.
