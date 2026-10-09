# Security Policy

Report vulnerabilities through GitHub private security advisories:

https://github.com/jeffglousher/coaptic/security/advisories/new

Do not open a public issue for a security report.

## Supported security boundary

Pairwise OSCORE is enabled by default and uses caller-owned
`oscore::SecurityContext` with AES-CCM-16-64-128. Ordinary App construction
requires a provisioned context; unprotected operation requires an explicit
`.allow_plaintext()`. Disabling default Cargo features does not silently permit
plaintext. See rustdoc for context lifetime and protocol behavior.
Group OSCORE, other ciphers and library-owned DTLS are not supported. DTLS in
the [test peers](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/interop/README.md)
does not add a library DTLS API.

When a context is attached, App is fail-closed: a non-empty message without an OSCORE option is rejected (unprotected 4.01 on a request; a token-matching plain 2.xx, plaintext Observe notification, or unprotected Block1/Block2 completion does not complete a Call). Empty ACK/RST (RFC 7252 reliability) stay unprotected. AEAD / replay / OSCORE-option processing failures are a silent drop â€” no unprotected 4.00 that would distinguish decrypt.

Optional EDHOC provisioning supports pinned P-256 method 3 / suite 2 with
explicit confirmation before handing off fresh OSCORE keys. Applications own
trusted credentials, secure entropy, authorization, and key custody. Reusing
an OSCORE key across restarts requires durable sender-sequence reservations
and replay-state handling as documented by the context and checkpoint APIs.
Message protection alone does not establish the security of a device or service.

## Security validation

The [App harness](https://github.com/jeffglousher/coaptic-validation/blob/main/crates/coaptic-plugtest/README.md)
exercises protected requests,
Observe notifications and Inner Block1/Block2, and rejects plaintext completion.
These harness exchanges are Coaptic-to-Coaptic.

The [independent process suite](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/interop/README.md)
includes libcoap OSCORE interoperability and wrong-key refusal, Coaptic
replay-state scenarios, and PSK DTLS through test adapters. Its
[case list and qualification gaps](https://github.com/jeffglousher/coaptic-validation/blob/main/tools/interop/capabilities.json)
define the tested scope; enabled cases must retain passing evidence in the run
report. Individual scenarios do not establish full protocol conformance.

Production readiness and end-to-end device security qualification remain in
progress. Hardware reports in [#202](https://github.com/jeffglousher/coaptic/issues/202)
and [#333](https://github.com/jeffglousher/coaptic/issues/333) distinguish
protected loopback tests from plaintext Wi-Fi exchanges. Network OSCORE,
durable flash state, and power-loss recovery are not yet qualified. Test
timings and coverage counters are not security or performance guarantees.
