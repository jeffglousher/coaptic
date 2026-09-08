# Security Policy

Report vulnerabilities through GitHub private security advisories:

https://github.com/jeffglousher/coaptic/security/advisories/new

Do not open a public issue for a security report.

Pairwise OSCORE (RFC 8613) is the optional `oscore` feature. You own the Master Secret, Sender/Recipient IDs, and replay window (`oscore::SecurityContext`). This slice is AES-CCM-16-64-128 only; group OSCORE, other ciphers, and first-party DTLS are backlog (harness `DatagramIo` only). Alternative networks (6LoWPAN, LoRaWAN, …) are the same future / backlog.

When a context is attached, App is fail-closed: a non-empty message without an OSCORE option is rejected (unprotected 4.01 on a request; a token-matching plain 2.xx, plaintext Observe notification, or unprotected Block1/Block2 completion does not complete a Call). Empty ACK/RST (RFC 7252 reliability) stay unprotected. AEAD / replay / OSCORE-option processing failures are a silent drop — no unprotected 4.00 that would distinguish decrypt.

Block1 / Block2 / Size1 / Size2 are RFC 8613 Figure 5 Dual (E+U). App uses the Inner field (fragment the CoAP message, then protect each datagram). They are not copied to Outer. Incoming Outer Block is not assembled as an application body.

Max-Age and No-Response are Figure 5 Dual. The application Max-Age and No-Response values stay Inner (RFC 8613 §4.1.3.1 / §4.1.3.6). Observe responses add Outer Max-Age 0 so OSCORE-unaware proxies do not cache 2.05 Content. ETag is Figure 5 Class E only. An injected Outer ETag, Outer Max-Age, or Outer No-Response is discarded on unprotect and does not become an application field.

Timed proof under load: `cargo run -p coaptic-plugtest --features oscore --bin dogfood -- --oscore` (CI smokes `--iterations 2` and `--compare` against `crates/coaptic-plugtest/baselines/dogfood-oscore.json`). The harness fails if protect/unprotect, protected Observe notify, or protected Block1/Block2 stays cold, or a plain completion sneaks through.
