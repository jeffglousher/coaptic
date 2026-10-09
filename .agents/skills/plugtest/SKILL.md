---
name: plugtest
description: Use when adding or checking CoAP interoperability against this repo's plugtest harness. Not for generic unit tests.
license: MIT OR Apache-2.0
---

# Plugtest

Tracking: <https://github.com/jeffglousher/coaptic/issues/199>. Protocol copies: `knowledge/rfcs/`.

Rust in-repo harness: `cargo test --test block_sweep` and `cargo test --test plugtest` (Engineâ†”Engine; inventory prints RUN vs SKIP). App SUT + pcap live in [coaptic-validation](https://github.com/jeffglousher/coaptic-validation): run `cargo test -p coaptic-plugtest` and `cargo test -p coaptic-plugtest --features dtls` there after its source preparation (tracking #199).

- TD identifiers come from `tests/plugtest/td-coap4/*.yml`. Do not invent TD identifiers.
- Map each TD to the local RFC copies in `knowledge/rfcs/`. Open the `.txt`, not a rewrite.
- DTLS TDs run in the companion repository?s `crates/coaptic-plugtest` (`--features dtls`) with a harness `DatagramIo` adapter (webrtc-dtls). Mixed pairs (`coap-rsâ†’coaptic`, `coapticâ†’coap-rs`) treat coaptic as SUT. The in-memory Engine pair still skips them (no sockets). Do not add a DTLS runtime dep to the library crate.
- `6lowpan` / `TD_6LoWPAN_*` is future/backlog. Leave YAML TD keys as-is.
