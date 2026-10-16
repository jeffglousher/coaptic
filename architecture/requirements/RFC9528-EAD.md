# Bounded EDHOC external authorization data

This assessment covers method 3, suites 2/3, pinned CCS credentials and mandatory
message 4. `Ead` owns at most 64 encoded bytes, including all item labels and
value headers. Registered labels through 65535 and their critical negative
forms are representable; larger labels are outside this profile's registry
boundary. Definite preferred CBOR is required by the EDHOC encoding profile.

- **RFC9528-3.8-sequence:** `Ead::push` and `Ead::from_encoded` support multiple
  ordered items, absent versus empty values, repeated labels and exact encoded
  byte retention. Received sequences are not normalized before MAC/transcript
  processing. The pinned credential/CID/EAD bound is checked at compile time
  against the private KDF context buffer and its length encoding.
- **RFC9528-3.8-criticality:** the existing handshake entry points ignore
  unsupported elective items and refuse unsupported critical items. Low-level
  `*_with_ead` transitions supply stage-specific application handlers; false
  means unprocessed and aborts a critical item. Handlers for messages 2, 3 and 4
  run only after authentication and receive the verified full principal.
  Message 1 is explicitly unauthenticated. Handler state remains provisional
  until full session acceptance; no hook alone authorizes effects or trust.
- **RFC9528-3.8.1-padding:** label 0, with absent, empty or caller-generated
  random value, is supported in every message. Padding never reaches receiving
  application handlers. Multiple padding items up to the full aggregate bound
  are accepted and discarded after authenticated message processing.
- **RFC9528-3.8-error / RFC9528-6:** default sequential `CoapProvisioner`
  processing sends error 1 and aborts the candidate for an unprocessed critical
  item. First/third-message response errors and second/fourth-message request
  errors retain the existing exact-byte retry and ownership rules. Refusal
  exposes no initiator session. Registry responders use the same EAD processor.
- **RFC9528-8-EAD:** the public low-level profile can send and process bounded
  EAD in all four messages. Default sequential transport supports padding,
  elective ignore and unknown-critical refusal. Application-specific handlers
  are exposed by the low-level transitions; no ELA/ACE or other registered
  security-application profile is adopted by providing these hooks.

Direct tests are in [ead_tests.rs](../../src/provisioning/ead_tests.rs) and
[ead_transport_tests.rs](../../src/provisioning/ead_transport_tests.rs). They
cover registered label/header boundaries, optional values, malformed/truncated
and nonpreferred input, full-capacity refusal without mutation, all-message
padding/elective/critical behavior, both suites, pinned and registry paths,
handler authentication ordering, tampering, exact CoAP error retries and fresh
valid handshakes. The PR records executed source/tool identities and results.
Independent security-stack EAD interoperability remains unqualified.

Original requirements: [RFC 9528](../../knowledge/rfcs/rfc9528.txt) sections 3.8,
3.8.1, 6, 8 and 10.5. [#365](https://github.com/jeffglousher/coaptic/issues/365)
retains the remaining credential/CBOR/CID/context assessment, application-profile
requirements and independent proof. This focused increment does not establish
whole-RFC conformance or complete update/errata closure.
