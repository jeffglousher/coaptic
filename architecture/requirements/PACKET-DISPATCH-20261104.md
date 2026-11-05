# Packet staging and request metadata

The integrated batch lowers complete server App thread cycles by 5.06% in sync
and 4.40% in actual async execution against merged #740, with unchanged supported
capacities. Successful OSCORE sealing no longer owns a 1,280-byte staging array;
the selected emitted response-frame chain is 1,072 bytes smaller. This is a
scoped scratch reduction: the measured complete Constrained progress future
grows by 32 bytes, and the cold refusal-frame chain grows by 176 bytes. App/pool
owners stay equal. Whole-system peak memory and task-stack reductions are not
established.

Baseline is `36f9c55635b6a53a7db4c95b9d99624095895124`; measured candidate is
`f84f3c199763910e246d8599aca0e29077a73ad1`. The later test-only `d828163` adds
repeated-Size1 oracle combinations; measured production code is unchanged.
[Companion #251](https://github.com/jeffglousher/coaptic-validation/pull/251)
retains the exact raw cohorts, methods, patches, fixture references, verifier and
owner ledger under `tools/benchmark/evidence/packet-dispatch-20261104`.

## Why these changes

Inspection of the baseline native executable finds a 1,280-byte zeroed OSCORE
plaintext staging array followed by encryption and a copy into the final packet.
Direct sealing uses the packet tail for plaintext and AEAD output, with the same
inner capacity including tag. The original staged encoder remains a cold helper
to preserve refusal results and error precedence. Nonce consumption, request
binding admission, Class U/E handling, authentication and replay policy retain
their order and semantics. Refusals can perform extra encoding; output bytes on
Err can differ, including partial plaintext. No zeroization promise is added.
App TX lengths commit on success, and occupied TX replacement stays transactional.

The existing critical-option validation walk also decoded values that dispatch
immediately rediscovered. Its private result now carries first Block1/Block2/
Q-Block2 values, Q-Block1/Observe presence and later metadata/selection decisions.
This avoids seven separate option walks on an ordinary one-Uri-Path GET and
later Observe-presence scans. First-value semantics and 4.02-before-4.00 error
precedence are preserved. The metadata adds no persistent App field or borrowed
packet lifetime. These are source/emitted cost observations, not sampled CPU
share attribution; only the combined runtime result is accepted.

## Matched runtime evidence

The 28 cells reuse the prior batch's exact fixtures and capacities:

- SmallDevice Constrained: routes4/populated1, Calls1/deferred0, association1/
  bindings4, RX/TX2x1152, dedup4, Observe2, no bodies; 32-byte response.
- Ordinary Service fixed/allocated: routes32/populated1, Calls4/deferred8,
  association1/bindings4, RX/TX32x1472, dedup64, Observe32, bodies4x16384 per
  direction; 1,024-byte response or complete 16,384-byte Block2.
- Retained Q-Block fixed/allocated: routes1, Calls4/deferred2, association1/
  bindings4, RX/TX4x1472, dedup16, Observe1, bodies2x12288 per direction;
  two requests and 11 verified data packets per 11,264-byte operation.

Each profile runs unprotected and established OSCORE in sync and async modes.
All 56 qualifiers and 224 timing processes complete per cohort. Windows
`QueryThreadCycleTime` surrounds complete server App work; client preparation,
bridge and verification are outside. Async executes native wait/progress with
immediately ready in-memory transport, a pending timer and manual root polling.
This does not measure sockets, executor scheduling, device latency or throughput.

Four paired processes use AB, BA, BA, AB; equal workload weights within pairs,
geometric ratios and one-sided 95% Student-t df3 bounds over paired log ratios.
Short ordinary cells use 8,192 measured operations; large ordinary and retained
cells use 1,024; all follow 128 warmups. No trimming, replacement observations or
calibration subtraction. CPU0 affinity is inherited/restored. Host is Windows
x86_64 MSVC on Ryzen 7 5800X3D, rustc 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), thin LTO,
one codegen unit, release debug1 and frame pointers. Features are `std,alloc,oscore`,
diagnostics off; exact manifests/locks remain in companion evidence.

Across all 28 cells, sync ratio is 0.9493599274 / upper 0.9653095449; async is
0.9560346921 / upper 0.9617326318. Original 20 ordinary controls separately pass:
sync 0.9420083197 / upper 0.9644730991; async 0.9522404906 / upper 0.9578390652.
Both groups and both modes must have upper bounds below parity; every cell point
ratio must be at most 1.02. All pass. The latter is not a per-cell confidence-bound
non-regression claim. Retained groups also improve in both modes. This finite
local workload mix does not establish a universal traffic benefit.

The baseline binaries are the prior measured `d2d4484` executables: source
comparison confirms #740 changes only documentation after that measurement.
All 372 tracked measured-source files match the reconstructed fixture with only
Windows line-ending normalization. Reproduction patches survive squash history.

## Memory and tradeoffs

All paired inline App and requested pool-heap ledgers remain equal. No capacities
were reduced. The release all-feature Constrained/routes1/blockwise=false complete
progress future grows 15,952 to 15,984 bytes (+32). It is separate from the borrowed
App; nested fields are not counted again. The local RX scratch remains 1,152 bytes.

PDB and disassembly identify the selected `protect_response<4, Token>` frame:
baseline 1,544 bytes of explicit RSP reservation plus 64 bytes of pushes = 1,608.
The candidate uses 280 bytes for that frame, 248 for the direct seal helper and
8 for its nested return = 536, a 1,072-byte reduction in these named frames.
The fallback restores the seal frame before a tail jump; it uses the 280-byte
parent plus 1,496-byte staged frame plus 8-byte return = 1,784, or 176 bytes more
than baseline. Do not add direct and staged helper frames together. Surrounding
callers and crypto/encoder callees are excluded, not assumed size-identical.
These are emitted reservations, not measured high-water or whole-call-chain peak.
They do not justify reducing task-stack reservations.

The complete multi-profile host executable shrinks 2,734,080 to 2,729,984 bytes
(-4,096). It is not a firmware flash or resident-memory measurement. Unusual
invalid traffic and Location-response runtime are not separately measured.

## Failures and checks

Direct sealing alone (`4ca4057`) failed its frozen comparison: ordinary-sync
upper ratio 1.0006156789 misses the benefit requirement, and SmallDevice/retained
allocated unprotected-async point ratios exceed 1.02. That source's raw cohort
stays failed. Adding distinct request metadata reuse produces the integrated
result; it does not retroactively rescue or isolate the failed mechanism. An
initial premeasurement type-inference compile error is also retained.

Checks pass: 1,214 all-feature library tests; 714 minimal, 762 alloc, 870 oscore,
922 alloc+oscore and 721 std library tests; 75 deferred FETCH/Observe/retained-Q
integration tests; two option-oracle tests including repeated Size1 combinations;
10 optimized async transaction tests covering cancellation, deadlines, retries,
retained packets and replay barriers. Strict all-target Clippy passes with all
features and without defaults; formatting passes. Differential sealing tests
exercise exact success bytes and refusal results across output/inner boundaries,
extended tokens and option combinations. Error-buffer contents are not an oracle.

An independent general read-only reviewer found no actionable code defect at the
measured source; it was not specialist Bugbot or an independent performance run.
The portable verifier independently recomputes raw process means, workload and
group statistics, capacities and both pass/fail decisions. Hardware qualification,
full RFC certification and whole-memory acceptance remain separate.
