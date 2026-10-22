---
name: plugtest
description: Use when adding or checking CoAP interoperability against this repo's plugtest harness. Not for generic unit tests.
license: MIT OR Apache-2.0
---

# Plugtest

Run `cargo test --test block_sweep` and `cargo test --test plugtest` for in-memory
Engine pairs. Network App/peer and DTLS fixtures live in
[coaptic-validation](https://github.com/jeffglousher/coaptic-validation):
`cargo test -p coaptic-plugtest`, with `--features dtls` for its DTLS adapter.

TD identifiers come from `tests/plugtest/td-coap4/*.yml`; do not invent them.
Use original `knowledge/rfcs/rfcNNNN.txt` for protocol behavior. Report RUN, SKIP,
failure and fixture/RFC conflicts honestly. Companion DTLS fixtures do not add
DTLS endpoint support to the library. Leave future `TD_6LoWPAN_*` keys intact.
