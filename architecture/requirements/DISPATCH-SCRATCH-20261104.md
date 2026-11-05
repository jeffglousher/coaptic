# Integrated dispatch and response scratch

Baseline `dcf82cbb75d903bd9ab44ccd64ba3e6584bd6f93`, measured implementation
`d2d4484cc5c6beb66856511e6d86ce75ce3cefd2`. The final integrated batch demonstrates
lower complete server-App thread-cycle cost in actual sync and async operation,
and a smaller complete progress-future owner, with configured capacities intact.

## Implementation

Dispatch reuses its initial RX scratch for authenticated plaintext instead of
keeping a second profile-sized optional buffer live across sends. Protected
processing borrows the unchanged retained RX slot for ciphertext. A private
validated layout rebinds the already checked outer message without decoding it
again; it is not a parser for unvalidated or changed bytes. The access guard ends
before checkpoint flush, RX plaintext writeback and asynchronous sends. Lazy
scratch selection, replay persistence, outer endpoint matching, silent protected
failure and unprotected error behavior remain intact. The OSCORE-option selection
result is reused within this dispatch pass.

Ordinary responses with no Location entries use eight private option slots,
the maximum reachable from ETag, Observe, Content-Format, Max-Age, Block2/Q-Block2,
Size2, Block1 and Echo. A separate Location helper retains the original 24 slots,
including both eight-entry lists. Validation, option ordering, protected failure
and capacity refusal are shared. No public limits or retained fields change.

## Complete-App runtime evidence

The declared 28-cell mix has sync B/A ratio **0.9914867457**, upper95
**0.9968455900**, and async ratio **0.9864243475**, upper95 **0.9917282395**:
0.85% and 1.36% fewer measured cycles. The original 20 ordinary controls also
pass independently: sync ratio0.9917414901/upper0.9978287654 and async
ratio0.9858266850/upper0.9898558051. All 28 individual point ratios meet the
predeclared1.02 guard. That is not a confidence-bound guarantee for each cell.

The comparison includes Constrained SmallDevice32-byte replies, fixed/allocated
Service1024-byte and complete16KiB Block2 replies, and fixed/allocated complete
11KiB retained Q-Block2 replies. Every profile runs unprotected/established OSCORE
and actual synchronous/asynchronous App operations with identical capacities.
Four paired processes per cell use AB,BA,BA,AB; 8192 short operations or1024 large
operations follow128 warmups. All56 qualifiers and224 timing processes complete.
Group estimates equally weight paired process-mean log ratios; upper bounds use
one-sided95 Student-t df3. Every observation remains; no trimming or calibration
subtraction. The short-cell count was set before this source's first sample.

The timer covers complete server App drives, including transport fixture copies,
maintenance, dispatch and crypto. Client preparation, bridge and authenticated
complete-response verification are outside. Async invokes wait plus progress
with an immediately ready in-memory transport, pending timer and manually polled
root future. This is native Windows CPU-cycle evidence, not socket latency,
executor scheduling, throughput, a device claim or either original50% objective.
Location-response runtime is unmeasured. Four finite local pairs do not establish
universal improvement across hosts or traffic distributions.

## Memory and tradeoffs

The existing release all-feature Constrained/routes1/blockwise=false test measures
the complete concrete `progress_async` future at **17128 ->15952 bytes**, saving
**1176 bytes (6.87%)**. The future borrows the App and is a separate owner.
All paired runtime-fixture App-inline and requested pool-heap ledgers are equal.
The result reduces transient owner storage; it does not reduce configured pools,
App fields, reserved ESP stacks or establish whole-system/peak-memory savings.

Emitted ordinary-response PDB frames for selected Constrained/NoBodies instances
decrease1472->768 bytes with the async error type and1456->736 with the sync error
type. Assembly shows options initialization/copy reduced576->192 bytes. These
are selected frame records, not call-chain high-water measurements. The inspected
Location branch restores the ordinary frame before tail-jumping; the frames must
not be added. Neither should frame bytes be added to the future reduction.

Specializing the response path increases the complete multi-profile Windows probe
executable2672128->2734080 bytes, +61952 (2.32%). That is executable size, not a
firmware flash estimate. Location responses retain their full builder and pay a
branch; their runtime is not measured by this ordinary/Q workload set.

## Checks, review and reproducibility

The final implementation passes1213 all-feature and714 minimal library tests,
strict all-target Clippy with all/minimal features, and75 affected integration
tests: deferred FETCH validator6, deferred Observe/Block242, deferred Q-Block227.
The affected no-default library combinations also pass: alloc762, oscore869,
alloc+oscore921 and std721. Formatting passes.
The all-feature suite includes async cancellation/checkpoint, authentication,
replay, Location maximum counts/empty segments, and protected options-full errors.
The added message-layout test compares rebound decoding and rejects wrong lengths.
A general independent read-only reviewer found no supported correctness,
security or capacity defect. No named specialist was available.

Rust1.99.1 x86_64 MSVC, LLVM23.1.1; Windows Ryzen7 5800X3D, CPU0, release thinLTO,
one codegen unit, debug1 and forced frame pointers for the probes; locked identical
dependencies, diagnostics disabled, library features std/alloc/oscore. The native
release library future check uses the same compiler/profile on both arms.

Portable exact fixtures, reconstruction patches, all five source-specific cohorts,
raw observations with hashes, memory records and an independent standard-library
verifier are in [companion evidence at its merged source](https://github.com/jeffglousher/coaptic-validation/tree/4d0694803132df4512539afb493372c323bd18b9/tools/benchmark/evidence/dispatch-scratch-20261104)
([PR250](https://github.com/jeffglousher/coaptic-validation/pull/250)).
The verifier reproduces all112 paired means,28 cell and9 group statistics and the
acceptance decision. Source reconstruction preserves the original fixture layout.
Original binaries/PDBs and build/preparation failures remain locally in
`D:/CoapticCurrent/dispatch-memory-20261104` under128MiB; the portable export is
below5MiB. The existing shared build lane and other consumers' output remain intact.

## Rejected experiments retained

The lexical-only deferred-Q scratch scope experiment did not change future size
and was reverted. Receive-scratch reuse alone and validated-layout reuse alone
both failed their complete-App runtime criteria. Their memory benefit did not
override those failures.

Direct retained-body-to-TX borrowing removed11264 staged bytes per actual complete
retained-Q operation but did not establish runtime improvement in either mode.
Its earlier20-cell comparison also failed; ordinary16KiB transfers were correctly
reclassified as controls because the existing borrowed-classic path bypassed the
changed staging. The new storage/API complexity was removed. All failed cohorts,
source patches and actual-path diagnostic evidence remain separately identifiable.
The final result does not retroactively turn any earlier failure into a success.
