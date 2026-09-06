---
name: plugtest
description: Use when adding or checking CoAP interoperability against this repo's plugtest harness. Not for generic unit tests.
license: MIT OR Apache-2.0
---

# Plugtest

Tracking: <https://github.com/jeffglousher/coaptic/issues/49>. Protocol copies: `knowledge/rfcs/`.

In-repo harness: `cargo test --test block_sweep` and `cargo test --test plugtest` (inventory prints RUN vs SKIP). Multi-impl + pcap: `cargo test -p coaptic-plugtest` and `cargo test -p coaptic-plugtest --features dtls` (tracking #56).

- TD identifiers come from `tests/plugtest/td-coap4/*.yml`. Do not invent TD identifiers.
- Map each TD to the local RFC copies in `knowledge/rfcs/`. Open the `.txt`, not a rewrite.
- DTLS TDs run in `crates/coaptic-plugtest` (`--features dtls`). The in-memory Engine pair still skips them (no sockets).
- `6lowpan` / `TD_6LoWPAN_*` is **not planned**.
