# Security profile acceptance ledger

This ledger assesses the portable implementation and direct tests in the same
source revision against the selected roles below. **Implemented** means code and
identified direct tests exist. It does not mean independent interoperability,
physical qualification, all-RFC certification, or closure of unexamined errata.
Executed evidence must name the exact core, companion, toolchain and firmware.

The adopted deployment is local authenticated commissioning, durable identity
and trust, then fresh pairwise EDHOC/OSCORE after restart. Ordinary durable flash
is the default. Physical rollback resistance and production operator-token
custody/UV require separately selected platform proof. Application authorization
and durable effects are project contracts; they are not ACE certification.

## EDHOC initiator and responder

The profile implements both roles, method 3, suites 2 and 3, pinned CCS credentials
by `kid`, sequential CoAP transport and required message 4. The following is the
finite clause/role inventory for that profile; optional methods and credential
types are not implicitly required by RFC 9528 section 8.

- **E1 — RFC 9528 sections 3.5, 5 and 8; both roles; implemented.** Method 3
  and both mandatory suites are selected explicitly. A supported preferred
  suite cannot be skipped, and unauthenticated errors cannot change local
  policy. [Profile](../../src/provisioning/profile.rs) and
  [profile tests](../../src/provisioning/profile_tests.rs) cover both suites,
  transcript rewriting, truncated/corrupt MACs and confirmation ordering.
- **E2 — RFC 9528 sections 3.5.2, 8 and 9.8; both roles; implemented.** The
  selected CCS/kid representation is one credential type with a one-byte
  reference. [Identity](../../src/provisioning/identity.rs) validates canonical
  P-256 public credentials; [transitions](../../src/provisioning/mod.rs) verify
  possession before authorization. Full credential principals and current
  [trust policy](../../src/provisioning/trust.rs), not the short reference,
  establish identity. [Trust tests](../../src/provisioning/trust_tests.rs) cover
  key/kid substitution, revocation, regrant and local/peer binding. X.509,
  CWT/C509 discovery and other authentication methods are outside this profile.
- **E3 — RFC 9528 sections 4.1.2, 4.2.1, 8 and appendix A.1; both roles;
  implemented.** [Exporter](../../src/provisioning/exporter.rs) accepts borrowed
  context and caller-owned output, with full-width unsigned labels and the
  HKDF output bound. [Tests](../../src/provisioning/exporter/tests.rs) include
  RFC 9529 vectors, independent HMAC fixtures and refusal/recovery. Exporter
  access follows authenticated, authorized session acceptance. Label-purpose
  uniqueness and destruction of application-derived keys remain caller duties.
- **E4 — RFC 9528 sections 3.8 and 8; both roles; implemented.** Complete EAD
  sequences preserve exact transcript bytes within a 64-byte aggregate bound.
  Padding is consumed, unsupported elective items ignored, and unprocessed
  critical items refused. Authenticated-stage handlers receive verified peers;
  message 1 remains unauthenticated. See [EAD ledger](RFC9528-EAD.md),
  [direct tests](../../src/provisioning/ead_tests.rs) and
  [transport tests](../../src/provisioning/ead_transport_tests.rs). EAD hooks
  do not adopt an enrollment protocol by themselves.
- **E5 — RFC 9528 sections 5, 6, 8 and appendix A.2; both transport roles;
  implemented.** Required errors 1/2, selected error 3, reserved-zero abort,
  outbound errors and received errors have bounded ownership and exact retries.
  [Initiator errors](../../src/provisioning/initiator_error_tests.rs),
  [responder errors](../../src/provisioning/coap_error_tests.rs),
  [registry errors](../../src/provisioning/cloud_error_tests.rs) and
  [final errors](../../src/provisioning/final_error_tests.rs) cover failure,
  receipts, no error loops, handoff isolation and fresh authentication.
  Unknown future codes or encodings beyond the stated receiver limits have no
  support claim. The older [error ledger](RFC9528-ERROR-RECEPTION.md) records an
  earlier slice: its outbound/equal-ID pending statements are superseded here.
- **E6 — RFC 9528 sections 3.3, 3.9 and 5; both roles; implemented within
  declared bounds.** Messages use 192 bytes, connection identifiers 0–7 raw
  bytes, EAD 64 bytes; larger inputs refuse. Preferred definite CBOR, complete
  message consumption and curve-point validation precede unsafe cryptographic
  assumptions. See [connection IDs](../../src/provisioning/connection_id.rs),
  [CID tests](../../src/provisioning/conn_id_tests.rs),
  [crypto tests](../../src/provisioning/crypto.rs) and
  [malformed-container tests](../../src/provisioning/tests.rs). These finite
  bounds are application-profile limits, not claims to parse arbitrary CCS.
- **E7 — RFC 9528 sections 5.5 and 9.8; both roles; implemented.** The profile
  requires message 4. Fresh entropy is requested per attempt with bounded
  scalar rejection; retransmission reuses exact saved messages. Secret-bearing
  state, ECDH/KDF scratch and AES round keys erase on completion, failure or
  cancellation. [State erasure proof](../../src/provisioning/lakers/shared/mod.rs)
  covers all retained secret states; [crypto proof](../../src/provisioning/crypto.rs)
  checks AES drop support. Compiler-created copies/registers are not guaranteed
  erased. The older [exporter ledger](RFC9528-EXPORTER.md) predates this erasure fix.

## EDHOC to OSCORE mapping and lifetime owners

- **M1 — RFC 9668 sections 2 and 4; client/initiator and server/responder;
  implemented.** `Session::derive` maps local C_I/C_R to local OSCORE Recipient
  ID and peer CID to Sender ID, with no ID Context. Full byte strings are
  retained. [CoAP tests](../../src/provisioning/coap_tests.rs) exercise extended
  IDs and message-3 routing; [handshake tests](../../src/provisioning/tests.rs)
  verify directional keys and protected delivery.
- **M2 — RFC 9668 section 4.1; namespace owner; implemented with explicit caller
  contract.** Low-level APIs require a caller-reserved local CID for every live
  handshake/context/cache. They cannot inspect another independently owned App.
  [Managed connections](../../src/provisioning/managed.rs),
  [recovery](../../src/provisioning/recovery.rs) and the
  [registry service](../../src/provisioning/cloud.rs) own their reservation
  lifetimes. Their tests cover active-session retention, collisions, expiry
  and refusal without eviction. Custom composition must enforce the same
  namespace reservation across its own contexts.
- **M3 — RFC 9668 section 4.1.3; initiator; implemented.** Equal C_I/C_R aborts
  and sends error 1. `second_message_failures_route_errors_including_equal_connection_ids`
  in [initiator error tests](../../src/provisioning/initiator_error_tests.rs)
  proves error routing; the responder refuses equal IDs before key agreement.
- **M4 — RFC 9668 sections 3, 5 and 6; explicit exclusion.** Combined EDHOC +
  OSCORE requests are not selected: mandatory message 4 is inconsistent with
  that profile. No combined-flow or complete EDHOC web-link discovery claim is
  inferred from implementing sequential transport. Custom discovery metadata
  must truthfully advertise the selected profile.

## Pairwise OSCORE sender and recipient

- **O1 — RFC 8613 sections 3–6 and appendix C; both roles; implemented.**
  AES-CCM-16-64-128, HKDF-SHA-256, context/nonce/AAD construction and compressed
  option/inner-message processing live in [OSCORE](../../src/oscore/mod.rs).
  [Tests](../../src/oscore/tests.rs) include published key derivation and
  request/response vectors, malformed metadata, tampering and wrong contexts.
  This is pairwise OSCORE, not generic COSE or Group OSCORE.
- **O2 — RFC 8613 sections 7–8 and appendix B; context/storage owner;
  implemented with explicit persistence contract.** Sender reservation and
  recipient replay checkpoints prevent reuse under retained keys. The App
  checkpoint barrier precedes dispatch, effects and acknowledgments.
  [Replay tests](../../src/oscore/replay_tests.rs) cover ambiguous commit,
  consumed authenticated replay, restart, deferred/blockwise work and exact
  cached responses. Uncertain recovery refuses; a fresh handshake establishes
  new keys. Caller stores must meet the documented durability/freshness rules.
- **O3 — RFC 8613 sections 4, 7 and 8; App client/server; implemented.**
  Contexts retain bounded request bindings; overload does not evict another
  live owner. Authentication failure does not expose an unprotected decrypt
  oracle. Observe, blockwise, selected extended Tokens and asynchronous paths
  retain correlation and replay ownership. See
  [OSCORE integration tests](../../src/oscore/tests.rs),
  [Q checkpoint tests](../../src/oscore/checkpoint_q_tests.rs) and
  [extended-token tests](../../tests/extended_token_app.rs).
- **O4 — enrolled service authorization/effects; project contract; implemented.**
  [Local pairing/recovery](../../src/provisioning/local_pairing.rs),
  [backup recovery tests](../../src/provisioning/local_transfer_tests.rs) and
  [receipt adapter](../../src/provisioning/receipt.rs) provide persistent identity,
  current owner/permissions, one-use backup authority and exact durable-operation
  correlation. [Receipt tests](../../src/provisioning/receipt_tests.rs) and
  [managed tests](../../src/provisioning/managed_tests.rs) prove refusal/recovery
  and installed-revocation behavior. An offline peer cannot enforce a revocation
  it has not received. HA/operator tooling is replaceable; it cannot bypass the
  core authorization or persistence contracts.

## Remaining evidence and issue disposition

[#365](https://github.com/jeffglousher/coaptic/issues/365) owns the remaining
installed authorization/effect integration. Independent selected EDHOC proof is
delivered by core #614 / companion #95:24 raw sessions plus six UDP error
directions, with exact native adapters/capability limits in the evidence index.
This does not establish whole-RFC certification or authenticate native EAD
callbacks that run before MAC verification. Installed host restart/echo now
passes with #619; exact permission restoration, authenticated Pending custody
transitions, producer drain/hold and explicit lost-receipt integration are
delivered in core #623 / companion #103 with focused host/UDP proof. Their
installed telemetry execution remains open. Missing/unverifiable journals refuse
rather than authorizing new effects; revocation must not wait for an unreachable
peer. The current installed core 7bb32c9 / companion11288b4 image passes protected
echo but Inspect reports missing enrolled `coaptic_pair/pending_v1` history.
That record is distinct from earlier static telemetry custody. Dependent grants
remain refused until authentic prior custody is restored/reconciled or an
independently reviewed legacy first-use provenance contract is established;
absence alone never authorizes initialization. Core #625 makes the custody state
visible in CLI inspection.

The finite native-EDHOC/CoAP-envelope option extension is also complete in
companion #108: eleven directions pass, including exact duplicate ownership,
critical-option silence/Reset/4.02 and fresh protected recovery. The
[bootstrap option ledger](RFC7252-EDHOC-OPTIONS.md) identifies exact sources,
capture and independent-backend boundaries. This does not convert the Python
transport envelope into a separate native CoAP stack.

[#333](https://github.com/jeffglousher/coaptic/issues/333) remains physical
qualification: actual power cuts at persistence boundaries, entropy/custody,
source-matched device/host recovery and real actuator effects are not established
by software tests. Existing restart observations remain evidence only for their
recorded firmware. Optional rollback hardware, routed/additional-MCU workloads,
Ethernet/PoE and sustained hostile-load campaigns need explicit scope decisions;
ordinary flash must not be described as physically rollback-resistant.

DTLS belongs to the companion test adapter, not the first-party core.
[#486](https://github.com/jeffglousher/coaptic/issues/486) is corrected by
[companion PR 87](https://github.com/jeffglousher/coaptic-validation/pull/87):
short GCM tags refuse before nonce allocation or subtraction, typed invalid-MAC
errors follow the actual PSK GCM fatal-20 policy, structural errors discard and
non-PSK behavior is unchanged. Eight offline codec/policy tests, CCM8/GCM live
control/wrong-key/fresh-recovery profiles and 92 capture mutations passed.
Captures bind core `73ec43c` and companion `1a1e42e`; merged companion `a48f0f2`
has the identical tree. This is finite local same-backend UDP proof, not
independent-peer or ESP32 DTLS qualification.

RFC 6347 section 4.1.2.1 allows silent discard; when sending an invalid-MAC alert
it requires fatal `bad_record_mac(20)`. This does not satisfy literal
TD_COAP_DTLS_02's prescribed `decrypt_error(51)`; the reviewed conflict remains a
separate #199 disposition rather than a relabeled PASS.

Normative sources: original [RFC 9528](../../knowledge/rfcs/rfc9528.txt),
[RFC 9668](../../knowledge/rfcs/rfc9668.txt) and
[RFC 8613](../../knowledge/rfcs/rfc8613.txt); companion retains the original
RFC 6347/5246 text and vendor provenance. Scope and remaining execution belong
in the linked issues and [IETF plan](../IETF-RFC-PLAN.md), not repeated historical
progress reports in this ledger.
