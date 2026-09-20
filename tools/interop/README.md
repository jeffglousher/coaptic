# Independent process interoperability

Three separately built executables communicate only over loopback UDP/DTLS:

- `peer-coaptic`: Coaptic App, webrtc-dtls 0.12 / util 0.11. No coap-rs dependency.
- `peer-coap-rs`: coap 0.28.1 with a test transport bridge to webrtc-dtls 0.12 / util 0.11. No Coaptic dependency.
- `peer-libcoap`: C fixture using libcoap 4.3.5 at `7cf7465b784baded4de183290c547d582becfd28`. CMake verifies the revision and unmodified tracked source. CI builds OpenSSL DTLS and OSCORE support; this initial matrix exercises PSK DTLS, not OSCORE.

libcoap is BSD-2-Clause; its independent source/build is outside the published library. OpenSSL is a test-machine dependency. No C FFI or DTLS dependency enters Coaptic. Lockfile pins Rust peers; the C source pin is checked by CMake. These are test fixtures, with a public, non-production PSK (`sesame`).

## Build and run

Requires stable Rust, Python 3.10+, CMake 3.20+, a C compiler and OpenSSL development headers/libraries.

```sh
cargo build --locked --release -p peer-coaptic -p peer-coap-rs
git init /tmp/libcoap
git -C /tmp/libcoap fetch --depth 1 https://github.com/obgm/libcoap.git 7cf7465b784baded4de183290c547d582becfd28
git -C /tmp/libcoap checkout --detach FETCH_HEAD
cmake -S tools/interop/libcoap -B target/libcoap-peer -DLIBCOAP_SOURCE=/tmp/libcoap -DCMAKE_BUILD_TYPE=Release -DENABLE_DTLS=ON -DDTLS_BACKEND=openssl -DENABLE_OSCORE=ON
cmake --build target/libcoap-peer --parallel 2
python3 -m unittest discover -s tools/interop -p 'test_*.py'
python3 tools/interop/run.py --coaptic target/release/peer-coaptic --coap-rs target/release/peer-coap-rs --libcoap target/libcoap-peer/peer-libcoap --iterations 100 --build-note "all peers Release; record OS/compiler here" --output target/process-interop.json
```

Windows: use MSVC (`-G "Visual Studio 17 2022" -A x64`, `--config Release`) and `.exe` paths. Without native OpenSSL, configure `-DENABLE_DTLS=OFF -DENABLE_OSCORE=OFF` and explicitly pass `--libcoap-udp-only`. The resulting report records the missing C-peer DTLS coverage; CI does not use this exception. Build directories may live on another drive.

## Coverage and interpretation

Both directions between Coaptic and each independent peer, plus Coaptic self-pair: GET exact bytes, 4.04 status, 2,000-byte Block2 assembly and repeated fresh client sessions. DTLS adds wrong-PSK refusal followed by successful service. Peer startup is bounded by 8 seconds; client deadline is bounded by its argument plus a process-kill margin. Children are always reaped. Server restarts reuse the same UDP port.

A bounded UDP relay records actual datagrams and injects one lost response, one duplicate request, or a blackhole. Lost replies must cause observable retransmission. Counter POST followed by GET must prove exactly one handler effect under duplication/retransmission. Blackhole must return a protocol timeout, not a process crash or supervisor kill. Restart checks explicitly expect fresh in-memory fixture state; they make no storage durability claim.

JSON schema `coaptic-process-interop/2` records source/dirty state, platform, executable hashes, library source/version, all case outcomes, fault traces, startup time, and min/mean/p50/p95/p99/max request timings. `request_us` includes socket/session setup, DTLS handshake and response assembly; process startup is excluded. `host_total_us` includes process startup/exit. Throughput is serial host requests/sec with fresh clients, not warm-session or maximum-load throughput. Peer protocol `coaptic-peer/2` requires integer nanosecond samples and clock metadata. Libcoap uses `CLOCK_MONOTONIC` on POSIX and `QueryPerformanceCounter` on Windows; Rust uses `Instant` (its resolution is reported as unknown). All peers stop timing before JSON formatting. Raw request/host samples and nanosecond summaries are retained; microsecond summaries are derived without integer truncation. OS-reported clock resolution is not measurement accuracy. Old peer executables are rejected: rebuild all peers with this runner. Default and CI use 100 samples per pairing, still a smoke benchmark. Never infer a performance ranking from mixed debug/release builds or different machines. Errors fail the run; successful partial measurements do not make failed cases pass.

This complements the existing ETSI catalog/pcap harness. It does not establish coverage for Observe, OSCORE, certificates, Q-Block, standardized patch formats or complete option handling. The App TD harness reports its known incomplete scenarios explicitly; Engine tests have a separate in-memory scope. No new ETSI identifiers are invented. Capability expansion should add explicit scenarios and proof rather than label a library feature-complete by reputation.

The peers remain separate executables with independent CoAP implementations.
Both Rust peers use the same modern DTLS backend; DTLS implementation diversity
comes from libcoap/OpenSSL. The coap-rs bridge disables its bundled legacy DTLS
feature and adapts its public client/server transport traits. No Coaptic types
enter that executable. Library and peer dependency versions need not match.

The bridge bounds active connection tasks and retained request responders to
128 each, refuses plaintext datagrams above 1,600 bytes, times out idle reads
at 30 seconds, and closes authenticated clients explicitly after measurement.
The separate harness tests mutually authenticated X.509 GET in both mixed CoAP
directions and self-pair, plus untrusted-client/server refusal with a specific
certificate-verifier error. These are not raw-public-key or full ETSI tests.

## Tested surface

The full run contains 51 scenario results (44 with local C-peer DTLS excluded):

- Ten transport/pair scenarios: UDP and PSK DTLS, each with Coaptic self-pair,
  Coaptic/coap-rs both directions and Coaptic/libcoap both directions. Each checks
  exact GET bytes, 4.04, 2,000-byte Block2 and repeated fresh clients. DTLS also
  checks wrong-key refusal and subsequent valid service.
- Five IPv6 UDP scenarios use the same client/server pairings and check exact GET,
  4.04, 2,000-byte Block2 and a subsequent valid GET. An independent IPv6 socket
  probe records actual request/response bytes from `::1`, preventing silent IPv4
  fallback from passing. These are loopback checks, not scoped/link-local, DTLS
  or IPv6 fault-injection qualification.
- Fifteen method workflows across IPv4 UDP, PSK DTLS and IPv6 UDP use the same
  pairings and execute 29 ordered
  steps each: PUT creation/replacement, GET readback, POST append, PATCH changes,
  repeated idempotent iPATCH, FETCH selection and DELETE. Invalid selection/patch
  instructions and individual/aggregate 64-byte state overflow must preserve
  exact prior bytes. Content-Format is application/octet-stream; the fixture's
  private PATCH syntax is `+suffix`, iPATCH is `=replacement`, and FETCH selects
  `value`. This proves wire method dispatch, body transport and those application
  state effects, not JSON Patch, arbitrary patch formats, conditions, IPv6 DTLS
  or complete RFC 8132 conformance.
  DTLS rejects a wrong-key replacement PUT and checks the original state afterward.
  IPv6 workflows require an independent AF_INET6 socket probe before any method
  steps. Rust fixtures share application
  state logic but use independent CoAP codecs; C implements the fixture separately.
- Twelve UDP reliability scenarios: each of the three clients against a Coaptic
  server, with dropped reply, duplicated request, blackhole and server restart.
  This does not test datagram loss against alternative servers or over DTLS.
- Six DTLS endpoint-reuse scenarios: each client against each Rust server, using
 a relay with one fixed server-visible UDP endpoint. Three fresh authenticated
 connections, one wrong-key refusal, then another successful connection.
- Two DTLS churn scenarios: 140 fresh authenticated counter POSTs and a GET
 proving all 140 effects, at one reused endpoint, against each Rust server.

The Rust fixture listeners share routing source, compiled independently against
their respective DTLS versions. Routing preserves the active association until
the backend verifies a replacement handshake (RFC 6347 section 4.2.8). Bounds:
128 peer endpoints, 16 pending handshakes, 32 packets per route, 4,096-byte
datagrams, two-second handshake deadline and 30-second route idle deadline.
Initial fragmented ClientHello messages are unsupported by these PSK fixtures.
Unit tests in both peers cover unverified replacement preserving an active
association, malformed routing input, pending capacity/timeout reclamation,
stale cleanup and listener shutdown. These are fixture tests, not a general
DTLS implementation qualification.

One additional Linux case sends 140 fresh Coaptic counter POSTs and a GET to
libcoap at one fixed endpoint, checking explicit client shutdown and all effects.
This does not qualify abrupt-client replacement at the libcoap server. Both
Rust clients now explicitly close authenticated connections after measurement.
Coaptic awaits bounded DTLS shutdown before process exit; request timing stops
at response assembly and host timing includes shutdown.

Only the repeated small GET (17-byte payload) is benchmarked. Block2, refusal and
fault checks establish correctness; they are not throughput benchmarks. Each
sample creates a fresh client process. Adapter polling and session setup are part
of the result, so these timings do not isolate engine execution cost.

Not measured here: warm sessions, concurrent/saturated load, CPU or memory cost,
OSCORE performance across independent stacks, real radios/WANs, long soaks,
combined loss/reordering/delay, or durable state recovery. Compiling a libcoap
feature does not establish interoperability coverage for it.

To inspect a retained CI result, download `process-interop` from its workflow run.
Check `source`, `dirty`, `build_note`, executable hashes and `iterations` before
comparing `benchmarks`. Report request and host timings separately, retain failure
counts, and accompany numbers with the scenario scope above. Checked-in dogfood
baselines belong to a different harness and are coverage floors, not current
process measurements.

Failed timed requests stop that pairing without retry or replacement. The report
retains successful raw samples, the requested sample count and the zero-based
failed sample index/error. Partial summaries cover successes only; a failed
measurement has no throughput value and the suite exits unsuccessfully.

## Executable capability inventory

[`capabilities.json`](capabilities.json) maps each process case to client/server
roles, transport/security, platform, positive and failure assertions, and its
runner function. The manifest also records unqualified or fixture-unsupported
surfaces and their tracking issues. It is an execution contract, not a stored
conformance score. The separate ETSI and App test harnesses have narrower,
distinct evidence and do not acquire process coverage from this inventory.

Each report records the manifest SHA-256 and a `coverage` result. Every enabled
case must appear exactly once, pass and retain evidence. Missing, duplicate,
unknown, disabled-but-executed and failed cases fail the run. An empty run cannot
pass. `--libcoap-udp-only` explicitly marks five cases `build-excluded`; it does
not mark them passed or remove them from the report. `complete` means all enabled
listed assertions passed, while the unqualified surface inventory remains open.
Linux and Windows are the declared execution platforms; other platforms require
an explicit qualification update. Wrong-key/blackhole cases require the peer's
normal error exit and a protocol refusal/timeout indication; crashes and setup
errors cannot satisfy them. Timeout alone does not establish an authenticated
DTLS alert or qualify the literal ETSI failure scenario.
