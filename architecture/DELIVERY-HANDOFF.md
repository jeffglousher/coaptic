# Delivery handoff — October 23, 2026

Accepted protocol and integration software is implemented. The remaining work
is performance qualification, one unfinished diagnostic, and the physical checks
in [PLAN.md](../PLAN.md). Both 50% server CPU targets remain open under
[#650](https://github.com/jeffglousher/coaptic/issues/650). Release publication
requires separate authorization under #132.

## Saved work

- Core source is on main. The current host ranking measured clean core
  `b9b83d8663cbdcb7a80bf143670a05019f25cf25`; subsequent `11cf5b6` updates a
  dependency and is outside that measurement.
- [Current comparative evidence](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/current-windows-20261023)
  preserves 2,610 cell receipts, 348,000 verified operations and all latency
  samples. It includes a standalone report, CSV, exact identities and regrading
  command. Captures containing fixture credentials and private operational
  records remain retained privately; they are not public source artifacts.
- [Companion draft #170](https://github.com/jeffglousher/coaptic-validation/pull/170),
  branch `wip/server-app-batch-cpu-20261023`, commit `795d570`, saves all four
  unfinished server-App CPU probe files. It is not merged or qualified.
  The last delegated provider turn failed; no child remains running or queued.
- PR watches and recurring tasks are off. Registered Coaptic build lanes are
  idle. No performance run is in progress. Historical child entries are records,
  not evidence of active agents.

## Resume the unfinished diagnostic

Review the full current draft, including its batch-only pidfd ownership and
receipt-scope fixes. R1 findings are addressed in source; combined R2 review is
pending. Python evidence includes 41 earlier passes with two Linux skips and
16 affected unique passes, including five overlaps. Do not sum those counts.
Six focused native Rust tests, affected strict Clippy/build, and real Linux
process/clock/cleanup checks have not run.

The probe prepares requests outside a server-App CPU window, runs full App polls
and captures complete responses inside it, then authenticates/verifies every
response outside. It retains protocol maintenance and server crypto. This is
a proposed isolation tool; it cannot replace socket acceptance or establish
either target before measured qualification. The separate existing global
cleanup concern remains [validation #169](https://github.com/jeffglousher/coaptic-validation/issues/169).

Use [DEVELOPMENT.md](../DEVELOPMENT.md), the registered compatible build wrapper,
and existing compiler output. Disk space is tight. Mixed shared lanes, evidence,
credentials and device state are retained; an idle lease does not make them
disposable. Remove only settled exclusive output with no consumer.

## Socket integration today

The core owns neither an OS socket nor an executor. `DatagramIo` and
`AsyncDatagramIo` move complete packets between caller transports and bounded
Engine slots. Sync/async protocol behavior shares the implementation. The
platform integration supplies clocks, timers and wakeups; pools are allocated
at startup and reused, with automatic growth off.

The benchmark has several explicitly named drivers:

- Bare and reusable std UDP use nonblocking sockets and continuously call
  `App::poll`. Both spin; reusable storage avoids constructing receive scratch
  on every receive. Both still copy accepted bytes into an Engine RX slot.
- Mio uses nonblocking receive/send, drains at most 32 App passes, then waits
  for socket readability or the next App deadline. Its OS selector is epoll on
  Linux and IOCP on Windows.
- Tokio ready uses synchronous App polling with Tokio socket readiness waits.
  Native async uses `wait_async` followed by freshly timed `progress_async`;
  accepted ingress is retained in an existing RX slot. Sends and cancellation
  keep the existing deadline, pin and commit rules.
- Staged-ready, staged-Mio and blocking variants are optional controls.
  Staging did not establish consistent savings; blocking has distinct soft
  send-timeout semantics. They are not defaults or substitutes for qualification.

The normal receive adapters use an N+1 scratch guard to detect oversize input,
then copy accepted bytes to the bounded RX destination. Sending borrows encoded
TX bytes. Async APIs alone do not imply io_uring or zero-copy networking.

## Assessing io_uring

io_uring is a possible Linux-specific completion adapter, not a new protocol
implementation for each CPU architecture. Keep the portable core and existing
Windows/embedded integrations. Rust/OS bindings handle the target ABI; Linux
kernel capabilities still require explicit checks and a fallback.

The potentially useful mechanisms are submission/completion batching and bounded
multishot receives. They do not remove UDP stack work, authentication, replay
checks or required message ownership. At C1, little independent traffic may be
available to batch. Existing system-CPU charge is not proof that all such work
is avoidable, and present measurements do not predict an io_uring speedup.

The difficult seam is lifetime: the current async contract requires dropping a
pending receive to end all access to the borrowed destination. A pending send
must not already have accepted the packet. Kernel-submitted operations cannot
simply retain those short loans. A safe integration needs bounded owned stable
storage and completion/drain rules, including forgotten futures, cancellation
races and exact send acceptance. Count ring memory, all configured packet owners,
kernel resources and any worker CPU; do not hide new buffers or polling threads.

Compare a simple batched socket control and a completion adapter only when a
specific measurable cost justifies the experiment. Preserve clean CPU accounting,
capacities, latency/fairness and refusal/recovery. This is a design consideration,
not an accepted implementation or completed optimization.

References: [Mio portability](https://docs.rs/mio/latest/mio/struct.Poll.html),
[io_uring model](https://man7.org/linux/man-pages/man7/io_uring.7.html),
[cancellation](https://man7.org/linux/man-pages/man7/io_uring_cancelation.7.html),
[ownership in Tokio-uring](https://docs.rs/tokio-uring/latest/tokio_uring/).
