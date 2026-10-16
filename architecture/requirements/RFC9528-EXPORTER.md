# EDHOC application-key exporter

This assessment covers the SHA-256 exporter in the method-3, suites-2/3,
pinned-credential profile with mandatory message 4. It does not complete the
remaining EAD, credential/CBOR, connection-ID or independent-peer assessment in
[#365](https://github.com/jeffglousher/coaptic/issues/365).

- **RFC9528-4.1.2-KDF:** `Exporter::export` supplies preferred CBOR headers and
  borrowed context directly to HKDF-SHA-256. All unsigned 64-bit labels are
  representable; context storage belongs to the caller. Output is limited to
  8160 bytes by HKDF. An oversized request leaves the output and exporter
  unchanged. `full_width_labels_and_borrowed_contexts_match_independent_hmac_vectors`
  covers the integer/context header widths with separate .NET HMAC fixtures;
  `output_limit_refuses_without_mutation_and_valid_exports_recover` checks the
  full 255-block output digest, refusal, empty output and fresh valid calls.
- **RFC9528-4.2.1-exporter / RFC9528-8-exporter:** `Session::exporter` and
  `Session::into_parts_with_exporter` expose a usable exporter after peer
  authentication, current authorization and the mandatory message-4 transition.
  No exporter accessor exists on incomplete public handshake states; a
  compile-fail doctest checks the initiator's confirmation boundary. Pinned and
  registry responders prepare message 4 after authenticating message 3.
- **RFC9528-4.2.1-purpose:** rustdoc requires registered application labels and
  unique `(label, context)` purposes, and explains deterministic re-derivation,
  output-length binding and nonce responsibility. Purpose assignment is caller
  policy; this stateless interface cannot detect reuse across applications.
- **RFC9528-A.1-OSCORE:** OSCORE secret/salt derivation now uses the same public
  exporter implementation. `rfc9529_published_exporter_vectors` checks the
  published PRK/exported-secret/salt vectors in RFC 9529 sections 2.5/2.6 and
  3.7/3.8. `both_suites_and_responder_paths_keep_exporter_through_oscore_handoff`
  checks both suites, pinned/registry paths, both peer directions, fresh-session
  separation and unchanged OSCORE derivation after handoff.

Exporter storage retains only the 32-byte PRK_exporter in a zeroizing,
non-Clone/non-Copy owner. Debug is redacted. The public API does not expose
PRK_out or the raw exporter secret. Existing handshake-internal copies are not
claimed erased. Caller-owned authorization must govern retained exporters and
derived keys, including revocation and destruction after handoff.

Implementation and direct tests: [exporter.rs](../../src/provisioning/exporter.rs),
[exporter tests](../../src/provisioning/exporter/tests.rs), and
[session transitions](../../src/provisioning/mod.rs). Original protocol:
[RFC 9528](../../knowledge/rfcs/rfc9528.txt) sections 4.1.2, 4.2.1, 8 and A.1;
published vectors: [RFC 9529](https://www.rfc-editor.org/rfc/rfc9529.html).
Executed tool/source identities and limits belong to the change's PR evidence.
These derivation vectors do not establish independent end-to-end EDHOC
interoperability or complete updates/errata closure for the entire profile.
