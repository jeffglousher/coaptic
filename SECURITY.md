# Security policy

Report vulnerabilities through [GitHub private security advisories](https://github.com/jeffglousher/coaptic/security/advisories/new).
Do not use public issues for vulnerability reports.

## Message protection

Pairwise OSCORE is enabled by default, using a caller-owned `SecurityContext`
with AES-CCM-16-64-128. Ordinary App construction requires a provisioned context.
Unprotected operation requires `.allow_unprotected()`.
Disabling Cargo defaults does not disable this check.

An attached context rejects non-empty messages without OSCORE. Unprotected
requests receive 4.01; unprotected responses, Observe notifications and block-wise
completions cannot complete a protected Call. Empty ACK/RST remain unprotected
as required by CoAP reliability. Authentication, replay and OSCORE-option failures
are dropped without an unprotected error response.

Group OSCORE, other ciphers and library-owned DTLS are unsupported. DTLS test
adapters in the companion repository do not provide a library DTLS API.

## Identity and persistence

Optional EDHOC supports pinned P-256 method 3, suites 2 and 3, and required message 4
before releasing fresh OSCORE keys. Applications own authenticated credential
installation, entropy, owner/resource authorization and key custody. A UUID or
self-generated key alone does not authenticate a peer at first contact.

The local pairing profile persists device identity, owner, permissions and
revocation state, then establishes fresh sessions after restart. Ordinary durable
flash is the default; physical rollback protection is optional hardening with a
separate protected-anchor contract. Keep session keys volatile on this path.
If a deployment reuses OSCORE keys after restart, it must satisfy the sender
reservation and replay-checkpoint contracts before processing traffic.

An applied revocation must survive restart. A disconnected peer cannot enforce
a remote update it has not received. Durable effects need application-level
commit/receipt handling in addition to message protection.

## Evidence and remaining work

Rustdoc defines the supported API contracts. The companion
[case inventory](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/interop/capabilities.json)
records independent peers, directions and gaps. Coaptic-to-Coaptic tests establish
local behavior; they do not establish independent cryptographic interoperability.

[#365](https://github.com/jeffglousher/coaptic/issues/365) tracks operator approval,
enrollment and recovery. [#333](https://github.com/jeffglousher/coaptic/issues/333)
tracks physical persistence, key custody and source-identified device evidence.
Production touch/user verification, physical transfer/reset recovery and relevant
power-loss behavior still need qualification. Existing finite reports apply to
their recorded source and firmware, not automatically to current main.
