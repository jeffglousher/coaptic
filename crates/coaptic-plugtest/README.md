# coaptic-plugtest

Workspace test crate. Not published. The `coaptic` library stays `no_std` with zero runtime Cargo dependencies.

```bash
cargo test -p coaptic-plugtest
cargo test -p coaptic-plugtest --features dtls
cargo run -p coaptic-plugtest --bin dogfood
cargo run -p coaptic-plugtest --features oscore --bin dogfood -- --oscore
cargo run -p coaptic-plugtest --bin dogfood -- --iterations 2 --compare crates/coaptic-plugtest/baselines/dogfood.json
cargo run -p coaptic-plugtest --features oscore --bin dogfood -- --oscore --iterations 2 --compare crates/coaptic-plugtest/baselines/dogfood-oscore.json
```

Timed mixed-stack dogfood (coap-rs client → coaptic server and the swap): GET/PUT/POST, Observe register, Block2/Block1. A third leg is coaptic↔coaptic Observe **notify collect** (`App::notify` → client `take_response`) so `observe_notify` is not left cold — coap-rs is a register/deregister stub. `--oscore` (feature `oscore`) adds a coaptic↔coaptic OSCORE GET/PUT/POST loop plus Observe register/notify collect and Inner Block-wise (Block2 GET /large, Block1 PUT /large-update) with mirrored caller-owned `SecurityContext`s; the run fails if protect/unprotect, protected notify, or protected Block stays cold, or a token-matching plain 2.xx / plaintext notify completes a Call. Default is 50 iterations; short CI smoke: `--iterations 2` and `--oscore --iterations 2`. Prints wall min/mean/p50/p99/max, Engine occupancy, and `app.metrics()`.

`--json PATH` writes schema `coaptic-dogfood/1` (Metrics + series timings). `--compare PATH` diffs this run against that file: **fail** if path-proving counters drop (`observe_notify`, `block1_assemble`, `block2_assemble`, …) or error counters rise; **print** wall-timing deltas (host/load specific, not a fail). CI compares the `--iterations 2` smokes to [`baselines/dogfood.json`](baselines/dogfood.json) and [`baselines/dogfood-oscore.json`](baselines/dogfood-oscore.json). Refresh those files with the same flags after an intentional Metrics change:

```bash
cargo run -p coaptic-plugtest --bin dogfood -- --iterations 2 --json crates/coaptic-plugtest/baselines/dogfood.json
cargo run -p coaptic-plugtest --features oscore --bin dogfood -- --oscore --iterations 2 --json crates/coaptic-plugtest/baselines/dogfood-oscore.json
```

Tracking: [#150](https://github.com/jeffglousher/coaptic/issues/150) / [#131](https://github.com/jeffglousher/coaptic/issues/131) / [#140](https://github.com/jeffglousher/coaptic/issues/140) / [#142](https://github.com/jeffglousher/coaptic/issues/142) / [#144](https://github.com/jeffglousher/coaptic/issues/144).

This crate is the **App SUT**. The in-crate `cargo test --test plugtest` harness is Engine↔Engine only (no sockets).

- [`Peer`](src/peer.rs) — start/stop server, client request, local UDP addr. Backends: `coaptic`, `coap-rs`. Add a library by implementing the trait.
- Pcap writer + golden JSON grader (`expectations/catalog.json`). CORE goldens assert type, token echo, and CON↔ACK MID, and omit `allow_extra`. OBS / BLOCK / LINK / DTLS keep `allow_extra` (notifications, block trains, mixed-peer extras). Ports / time are wildcards.
- DTLS: feature `dtls` uses webrtc-dtls (same stack as coap-rs) as a **harness** `DatagramIo` adapter. The `coaptic` library stays zero-dep. Mixed pairs (`coap-rs→coaptic`, `coaptic→coap-rs`, `coaptic→coaptic`) run handshake + GET `/secure` with coaptic as SUT.
- `TD_6LoWPAN_*` stay skipped (`future/backlog` — contributor opportunity).

Tracking: [#56](https://github.com/jeffglousher/coaptic/issues/56).
