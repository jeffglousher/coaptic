# Security Policy

Report vulnerabilities through GitHub private security advisories:

https://github.com/jeffglousher/coaptic/security/advisories/new

Do not open a public issue for a security report.

Pairwise OSCORE (RFC 8613) is the optional `oscore` feature. You own the Master Secret, Sender/Recipient IDs, and replay window (`oscore::SecurityContext`). This slice is AES-CCM-16-64-128 only; group OSCORE, other ciphers, and first-party DTLS are backlog (harness `DatagramIo` only). Alternative networks (6LoWPAN, LoRaWAN, …) are the same future / backlog.

When a context is attached, App is fail-closed: a non-empty message without an OSCORE option is rejected (unprotected 4.01 on a request; a token-matching plain 2.xx does not complete a Call). Empty ACK/RST (RFC 7252 reliability) stay unprotected. AEAD / replay / OSCORE-option processing failures are a silent drop — no unprotected 4.00 that would distinguish decrypt.
