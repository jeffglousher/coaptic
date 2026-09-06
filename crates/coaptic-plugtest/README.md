# coaptic-plugtest

Workspace test crate. Not published. The `coaptic` library stays `no_std` with zero runtime Cargo dependencies.

```bash
cargo test -p coaptic-plugtest
cargo test -p coaptic-plugtest --features dtls
```

- [`Peer`](src/peer.rs) — start/stop server, client request, local UDP addr. Backends: `coaptic`, `coap-rs`. Add a library by implementing the trait.
- Pcap writer + golden JSON grader (`expectations/catalog.json`). MID / Token / ports / time are wildcards.
- DTLS: feature `dtls` uses webrtc-dtls (same stack as coap-rs) as a **harness** `DatagramIo` adapter. The `coaptic` library stays zero-dep. Mixed pairs (`coap-rs→coaptic`, `coaptic→coap-rs`, `coaptic→coaptic`) run handshake + GET `/secure` with coaptic as SUT.
- `TD_6LoWPAN_*` stay skipped (`not planned`).

Tracking: [#56](https://github.com/jeffglousher/coaptic/issues/56).
