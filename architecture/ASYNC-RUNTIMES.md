# Async runtime and comparison policy

Decision adopted 2026-11-04. This selects integration direction; it does not
claim that a planned adapter has been implemented or qualified.

## Keep the protocol core independent

Coaptic uses standard Rust `Future`/`async`/`await`. The core must not create an
executor, thread, socket, hidden packet queue or runtime, or depend on Tokio,
Embassy, async-io or an ESP SDK. The application owns execution, transport,
monotonic time, capacity and shutdown. Sync and async remain supported ways to
drive the same App protocol behavior.

There is no standard-library async networking runtime to switch to. `async-std`
is a separate crate, not Rust's standard library; its maintainers now describe it
as discontinued. Do not introduce it as a new project default. `async-io` is
another, distinct crate providing host readiness and timers; it is not our code.

## Platform choices and actual status

- **Windows/Linux/macOS applications:** prefer a Tokio adapter when the caller
  uses Tokio. The reusable [coaptic-tokio adapter](../tools/coaptic-tokio/README.md)
  accepts caller-owned sockets and uses the caller's runtime, with no nested
  `block_on` or hidden runtime. Its Windows loopback tests cover current-thread
  execution, socket ownership, cancellation, oversize recovery, timers and a
  complete unprotected blockwise App exchange. Other hosts remain untested here.
  Do not infer multithreaded `Send`/`Sync` support from current-thread success.
- **Small standalone host tools:** retain the existing `coaptic-async-host`
  async-io adapter as an explicit alternative. It is implemented and tested.
  Account for its reactor thread/registrations, even if `block_on` also drives
  readiness. Do not wrap it in Tokio by default and silently run two reactors.
- **Native embedded Rust applications:** Embassy is the preferred integration
  target. Add transport and timer adapters plus a working target example before
  labeling it supported. No Embassy integration is currently delivered here.
  Executor, timer driver and network/HAL selection are separate decisions; using
  an Embassy executor does not itself provide an ESP Wi-Fi/socket integration.
- **Existing ESPHome/ESP-IDF applications:** retain the current small FreeRTOS
  single-root-future driver until a replacement works in that environment. It
  handles wakeups, socket readiness and deadlines; it is not a general-purpose
  runtime. Its socket futures are driver-bound and cannot be polled by an unrelated
  executor. Keep it in the platform integration, without expanding it into a
  general task scheduler. Replacing it is a separate evaluated change, not a
  prerequisite for every protocol optimization or an automatic switch to a new
  Wi-Fi stack/firmware architecture.

Do not call the current host async-io adapter, Tokio benchmark adapter and ESP
driver interchangeable implementations. Name the selected one in examples and
results. A user's existing executor should normally determine the adapter.

## Adapter contract and replacement criteria

Implement `AsyncDatagramIo` and `AsyncTimer` with actual asynchronous readiness,
one matching monotonic timestamp domain, bounded caller-owned packet storage,
and no blocking operations inside future polling. Preserve packet boundaries,
oversize detection, backpressure and endpoint metadata. A cancelled wait must
release registrations and packet borrows; an accepted send must not become an
invented non-send. Preserve authentication, replay, durable commit ordering and
unresolved-effect semantics through cancellation and shutdown.

First validate the adapter with existing loopback/protocol tests and a runnable
target example, including readiness races, cancellation, deadlines, truncation,
blocked sends, recovery and socket ownership. Then compare complete App work with
matched capacities against the existing adapter. Keep request performance primary;
report idle CPU, lifecycle cost, code size, heap, static/future storage and task
stack separately. Reuse unchanged evidence; do not create a new validation campaign
for documentation or repeat broad checks without a concrete concern.

Replace an existing platform driver when the new integration preserves its
required behavior and offers a demonstrated maintenance or resource/performance
advantage with acceptable tradeoffs. A familiar runtime name alone is not evidence
of smaller memory or faster requests. Preserve existing device identity, NVS,
credentials and recovery during any runtime migration.

## Compare equivalent services, not identical allocation strategies

Define two explicit views: equivalent-capability services for cross-library
claims, and each library's practical minimal configuration for a named workload.
Never merge their rankings. For libcoap and other peers, match endpoint role,
routes, methods, security, payloads, concurrency, outstanding requests, Observe,
block-transfer retention/assembly, retransmission and duplicate-response behavior.
Prove the required behavior and capacity rather than merely enabling a feature.

Do not force libcoap to preallocate our buffers. Dynamic allocation or stateless
block serving may legitimately use less memory for the same required service.
Conversely, a static GET/Block2 fixture is not evidence about retained uploads,
client Calls or concurrent transfers it never exercises. Configure both as
server-only when that is the question; include client capabilities explicitly
when they are required. Record unsupported combinations instead of substituting
different semantics. Include actual sync and async modes with named executors and
wait policies; busy spin is a separate CPU/latency tradeoff.

Fixed pool capacities do not bound compiler-generated temporaries, async frames
or the complete call stack. Count nested owners once. Task stack reservations may
already be included in heap totals. A complete static bound can justify a stack
reservation; selected frame sizes cannot. Measured lifetime watermarks cover the
executed paths and provide evidence, not a proof of every possible peak. Large
temporary frames are an optimization target, not a CoAP compliance requirement.

## References

- [Rust futures](https://doc.rust-lang.org/std/future/index.html)
- [Tokio runtime](https://docs.rs/tokio/latest/tokio/runtime/)
- [Embassy book](https://embassy.dev/book/)
- [async-io](https://docs.rs/async-io/latest/async_io/)
- [async-std maintenance status](https://docs.rs/async-std/latest/async_std/)
- [Tokio adapter](../tools/coaptic-tokio/README.md)
- [async-io host adapter](../tools/coaptic-async-host/README.md)
- [Block-Wise RFC](../knowledge/rfcs/rfc7959.txt)

Runtime maintenance status was checked on the decision date. Verify current
upstream support when implementing or updating an adapter; pin actual dependencies
and record tested targets instead of treating this document as a version lock.
