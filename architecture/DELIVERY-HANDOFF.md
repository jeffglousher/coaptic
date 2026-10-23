# Delivery handoff — October 23, 2026

Accepted protocol and integration software is implemented. The remaining work
is performance qualification and the physical checks
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
- [Companion #170](https://github.com/jeffglousher/coaptic-validation/pull/170)
  is merged as `4d65781`. The server-App CPU mode has completed focused native
  checks, independent source review and finite Linux window qualification.
  Its tested companion source is `ff46fd5`, with identical merged file bytes.
- PR watches and recurring tasks are off. Registered Coaptic build lanes are
  idle. No performance run is in progress. Historical child entries are records,
  not evidence of active agents.

## Current server-App CPU result

The [qualified run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40024678302)
verifies 200,512 exchanges across four serial processes, including 512 warmups.
All four CPU windows qualify. Unprotected server-App driver CPU is
0.901244–0.916253 microseconds/op; OSCORE is 2.032878–2.034727 microseconds/op.
These are two observations per mode on one EPYC 7763 runner, not confidence
intervals or an optimization comparison. Core is `b9b83d8`, Rust is 1.99.0,
and full source/compiler/clock/fixture receipts are in the run artifact.

The probe prepares requests outside a server-App CPU window, runs full App polls
and captures complete responses inside it, then authenticates/verifies every
response outside. It retains protocol maintenance and server crypto. This is
a server-driver measurement including fixture copies, monotonic reads and
loop/gate work. It cannot replace socket acceptance, establish either target
or yield a transport percentage by subtracting another run's timing.

The earlier [short-window run](https://github.com/jeffglousher/coaptic-validation/actions/runs/40022418841)
retains its failed unprotected 10 ms qualification. One 50,000-operation window
now replaces five 10,000-operation windows, with unchanged total work and guards.
Each fixed-workload capture buffer is 256 bytes; a complete row contains two
buffers plus 40 bytes of metadata and padding, totaling 552 bytes. Actual
App/CoreIo datagrams stay 1,472 bytes. Enclosing requested fixture plus App is 27,658,654 bytes below
32 MiB, including metadata/padding and a 16 KiB scratch allowance. Allocator
overhead, generated peak stack and whole RAM remain unknown.

Six release Rust tests, strict Clippy, formatting and build pass. Linux passes
the 54-test Python set with two optional skips, then both native worker-death/EOF
checks; 1,001 independent evidence checks pass. EOF tests do not exercise
production marker/error cleanup, which retains mocked coverage; whole-tree
failure cleanup remains unqualified. The separate existing global
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
