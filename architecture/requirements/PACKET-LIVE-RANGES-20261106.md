# Packet lifetimes, static body borrowing and ordinary response encoding

The integrated comparison passes every predeclared runtime and selected-memory
bound against `fd7d18a`. Complete-App CPU cost falls 2.91% in sync operation and
3.28% in async operation across the declared 28-workload mix. Selected Service
progress futures shrink by 2,536 bytes (16.37%), and native Service Apps shrink
by 320 bytes with unchanged capacities. Feature combinations, strict lint,
Rust 1.89 and rustdoc checks pass. The native ordinary encoder's larger local
frame and code are retained as explicit tradeoffs below.

The selected candidate is core `8d9e08232865bd41049d3d8062bd4b87328c28b0`.
Its baseline is merged `fd7d18a72f795779e352443ebdeef9533be7fb27`. The reused
baseline executables come from `3e592adb1fd52c5e6918af1b4e810bb874413bad`, with
all 148 original core inputs identical to that merge. Candidate reconstruction
retains all 148 original inputs. All seven original probe inputs are unchanged.

## What changed

Packet preparation finishes before suspension where its borrows permit it.
Completed deferred replies borrow their retained metadata instead of copying
it into a sending future. Q-Block2 selection carries a reference to the encoded
option descriptor rather than a whole parsed-message value. That reference is
nongeneric: a token that is Send but not Sync does not acquire a new Sync bound.

Received responses keep identity together and borrow their immutable reply
metadata. This removes duplicate fields while preserving original options,
token, peer and reception timestamp when response contents are edited. The
native Response representation shrinks from 608 to 536 bytes.

Ordinary successful responses with an explicit static payload now reserve the
same bounded body slot while borrowing the immutable application bytes. A typed
alternative reuses the transfer's otherwise inactive receive-timer storage.
Incoming transfer timers, Observe and deferred response ownership remain separate.
Both fixed and allocated backing stay allocated with the same capacities.
Writable access, length changes and sidecar replacement materialize the static
prefix into that existing backing. Public sidecar snapshots omit the private
backing descriptor, so restoring an old snapshot cannot resurrect stale bytes.

Exact address-and-length equality skips redundant retained-body comparison.
Different addresses still require exact byte equality; an ETag alone is
insufficient. The normal handler API is unchanged: `Response::content` and
`with_static` preserve the lifetime needed for this optimization. Advanced Engine
and BodySlots APIs expose explicit static admission; custom BodySlots backends
must implement these methods under the same slot and capacity contract.

Retained-body encoding keeps the original bounded chunk staging. Integrations
that removed that copy, added combined body/TX borrowing and reused one packet
guard failed the complete workload bounds. Their exact sources, removed-work
diagnostics and failed cohorts remain evidence; their API and machinery are
absent from the selected implementation.

Completed deferred followups now compare their borrowed request fields against
the stored canonical selection. Admission still serializes and enforces its
24-option/512-byte bound. Exact code, token bytes, selected raw option values,
duplicate order and assembled FETCH payload determine identity. Incoming type
and MID remain excluded, and all existing Q-mode, Observe and non-FETCH-payload
checks remain. No hash, new public API or additional allocation is involved.

Each eight-entry Location array remains absent until its first valid append.
Both lists can still hold eight entries simultaneously, including empty string
values. Const builders, validation order and insertion order remain supported.
This changes initialization work, not Location capacity or retained array size.

Ordinary replies whose only optional metadata is ETag and Content-Format now
borrow a directly constructed two-option array. All four presence combinations
are supported, including present Content-Format zero. Location, Observe,
Max-Age, Echo, block options and fresh-PIV replies retain the general encoder.
Sticky validation still runs before bounded access or TX effects, and both paths
use the same security encoder and caller-supplied payload. The actual ordinary
fixture supplies both ETag and Content-Format, so this branch is exercised.

## Measured memory and emitted work

| Owner or observation | Baseline bytes | Candidate bytes |
| --- | ---: | ---: |
| SmallDevice complete progress future | 15,952 | 13,416 |
| Service fixed complete progress future | 15,496 | 12,960 |
| Service allocated complete progress future | 15,496 | 12,960 |
| x64 Service fixed App | 315,656 | 315,336 |
| x64 Service allocated App | 35,672 | 35,352 |
| x64 Service allocated requested pool heap | 282,512 | 282,512 |
| Thumbv7EM Service fixed App | 307,504 | 307,248 |
| Thumbv7EM Service allocated App | 28,184 | 27,928 |

Selected SmallDevice App and pool owners are unchanged. The native polling,
`encode_issued` and reclaim frames selected by the prospective memory check meet
its original nongrowth guard. The
selected retained encoder frames stay at 1,296/1,280 bytes. Future
sizes, local frames and resident App/pool storage are separate observations;
do not add them into an inferred peak or reduce reserved task stacks from them.

Eight untimed diagnostic cases cover fixed/allocated storage, unprotected/OSCORE
and synchronous/asynchronous processing of the actual retained fixture. Each
verifies 136 operations and eleven payload packets per operation. Snapshot bytes
fall from 11,264 per operation to zero; the one full-body equality call is resolved
by address and length, with zero byte-comparison candidates. This demonstrates
removed work, independently of the non-instrumented CPU comparison. Original
chunk staging remains 11,264 bytes per operation, and wire output still matches.

The separate direct-body experiment verified removal of 11,264 staged bytes
per declared retained operation, but failed the runtime bounds. That is a
useful negative result: the selected implementation makes no staging-removal or
end-to-end zero-copy claim.

Bounded inspection of three ordinary constructors records 333 to 80 explicit
store bytes, 57 to 22 instructions and 517 to 149 code bytes against the pinned
baseline. Their eight-byte frames are unchanged. Location backing accounts for
240 removed store bytes; compact received metadata accounts for the earlier
13-byte reduction. The ordinary selector reads only the two Location counts.
These observations identify removed work, not causal shares of total CPU time.
The detailed store-count proof above belongs to the retained `97d0c9b` precursor;
the final binary separately confirms the same 149-byte ordinary constructor
and eight-byte local frame.

In the final fixed Service ordinary encoder, bounded before/after inspection
confirms removal of 192 bytes of placeholder initialization, a 192-byte array
copy and 42 unique bytes of Echo metadata copying on the ETag/Content-Format
branch. Direct construction writes six fields totaling 36 bytes. The one-byte
Content-Format encoding copy and downstream message, security and TX encoding
remain. This is emitted-path evidence, not a measured share of total CPU cost.

For completed-deferred followup matching, assembly from the `97d0c9b` to
`eefc387` precursor comparison proves
removal of 576 bytes of option initialization, 1,616 bytes across three copies,
520 bytes of zeroing, message serialization and a full 512-byte comparison.
The frozen retained fixture returns immediate responses and never executes this
completed-deferred path. The cohort cannot establish a CPU gain from the matcher.
Field comparisons remain. The matcher has a 144-byte local frame versus the
former constructor's 1,328 bytes, with additional small decode/equality callees.
Complete synchronous dispatch frames are unchanged in that precursor comparison;
no peak-stack reduction is inferred from these local frames. Fixed/allocated sync
and async call sites were inspected. A separate sidecar investigation disproved
a suspected 688-byte copy:
LLVM already reduces those getters to scalar accesses, so no API change followed.

## Runtime method and capacities

| Workload group | Mode | Candidate / baseline cycles | One-sided 95% upper ratio | Mean reduction |
| --- | --- | ---: | ---: | ---: |
| Original 20 ordinary cells | Sync | 0.978320 | 0.981229 | 2.17% |
| Original 20 ordinary cells | Async | 0.970204 | 0.973976 | 2.98% |
| Eight retained-Q cells | Sync | 0.952644 | 0.957636 | 4.74% |
| Eight retained-Q cells | Async | 0.959895 | 0.962844 | 4.01% |
| All 28 cells | Sync | 0.970914 | 0.973522 | 2.91% |
| All 28 cells | Async | 0.967248 | 0.969960 | 3.28% |

All 28 per-workload upper bounds are below 1.02. The largest is 1.016257 for
SmallDevice protected async processing; that cell establishes preservation
within the declared tolerance, not a statistically resolved CPU improvement.
The group results do not mean every individual workload demonstrates a gain.

The frozen comparison uses 28 cells, 64 paired processes per cell and AB, BA,
BA, AB repeated sixteen times. Each process warms up 128 operations. Short
ordinary workloads measure 8,192 operations; large and retained workloads
measure 1,024. No observations are trimmed, replaced or selectively rerun.

Windows server-thread cycles cover complete in-memory App processing through
actual synchronous poll or asynchronous wait/progress. Client preparation,
authentication preparation and verification are outside the clock. Equal-weight
geometric means retain separate original-20 and all-28 sync/async groups.
Every workload upper one-sided 95% confidence ratio, and both mode aggregates,
must be at most 1.02. A CPU improvement claim additionally needs an aggregate
upper bound below 1. The t(63) factor is conservatively fixed at 1.67.

SmallDevice/Constrained uses four routes, one Call, no deferred rows, four
security requests, one association, two RX/TX datagrams each of 1,152 bytes,
four dedup entries, two observations and no body pools. Service uses 32 routes
(one populated), four Calls, eight deferred rows, four security requests, one
association, 32 RX/TX datagrams each of 1,472 bytes, 64 dedup entries, 32
observations and four RX/TX bodies each of 16,384 bytes.

The retained Q fixture uses custom Buffers rather than Service: one route,
four Calls, two deferred rows, four security requests, one association, four
RX/TX datagrams each of 1,472 bytes, 16 dedup entries, one observation and two
RX/TX bodies each of 12,288 bytes. It verifies eleven selected 1,024-byte
packets per operation from an 11,264-byte body. Fixed/allocated storage and
unprotected/OSCORE operation are represented in both modes. Independent body
and packet capacities, allocation counts and supported features are preserved.

## Tradeoffs and limits

Custom BodySlots backends must implement the two explicit static-admission
methods. No unsafe code or automatic pool growth was introduced. Advanced
mutation intentionally pays a full-body copy into the preallocated backing. Lazy Location backing adds getter and
first-append branches; ordinary encoder selection reads private counts directly.
Location-heavy runtime is outside this cohort; its bounds, empty segments,
encoding and refusal behavior have regression checks. Completed-deferred matching
is also outside the timing fixture and receives no CPU credit from these results.

The direct option helper is inlined. The separately inspected fixed Service
`encode_response` local frame grows from 736 to 784 bytes in sync operation and
768 to 800 bytes in async operation. Those functions grow from 6,045 to 7,109
and 6,128 to 7,481 code bytes respectively. These observations were recorded
before timing; they do not change the predeclared checks on other selected
frames. They prevent a claim that every frame or code owner shrinks. The selected
SmallDevice polling symbol also grows by 336 code bytes; the three
other selected polling symbols shrink. No complete stack peak or device flash
saving is inferred. Original retained-body staging
remains. Whole-system peak RAM, socket latency, throughput, executor cost,
hardware qualification and equivalent-capability libcoap performance are not
measured here.

## Retained alternatives and reproduction

The evidence retains two preflight rejections, eight failed complete cohorts and
one guard-only precursor superseded before timing. The guard precursor passed
memory checks; it is neither a CPU failure nor an accepted implementation.
Earlier response-ownership and replay failures retain their original decisions.
The preceding static-body integration reduced aggregate sync/async CPU cost by
1.54%/2.16%, but one ordinary fixed unprotected 1,024-byte sync upper confidence
ratio was 1.02025494, above 1.02. It remains failed without rounding or resampling.
The current ordinary-option integration is a distinct source; no unchanged failed
cohort was repeated and no acceptance bound was relaxed.

Regression checks cover fixed/allocated static admission, exact slot and byte
refusal, pins, copy-on-write, shrink/reextend, stale sidecar replacement and slot
reuse. App checks distinguish identical pointers and lengths from different
addresses with equal or changed bytes. Async cancellation after one accepted
Q packet releases the exact RX/TX/body owners and permits an exact retry.
Protected operation additionally runs the actual retained fixture and existing
security/cancellation tests.

The ordinary encoder oracle compares bytes, errors, occupied-TX behavior and
subsequent protected-notification state with the general encoder. It covers all
four ETag/Content-Format presence combinations, zero and boundary formats, ACK
and NON messages, full tokens, alternate supplied payloads, sticky validation,
every excluded metadata owner, block options and fresh-PIV behavior. Location
capacity and const-construction checks, deferred identity equivalence checks,
and the Send-but-not-Sync token regression preserve the other changed contracts.

The companion evidence contains exact source patches and hashes, all raw
outputs and process exits, baseline/candidate binaries and PDBs, native/Thumb
layouts, bounded assembly, diagnostic work counts, regression logs, failures
and reconstruction/verification scripts. The exact candidate passes 1,234
all-feature library tests, 420 integration tests and 43 doctests. The intentionally
ignored idle-poll microbenchmark remains ignored; the frozen complete-App cohort
is the performance evidence. Library-only feature lanes pass 730 tests without
defaults, 780 with alloc, 887 with OSCORE, 941 with alloc/OSCORE and 738 with std.
Both strict all-target Clippy lanes, Rust 1.89 checks without defaults and with
all features, rustdoc and formatting pass. Previously passing checks for the
same source are reused where applicable.

Independent general/security review found no actionable findings. Initial
test-wiring compilation failures are preserved with their exact source patch.
The [portable evidence](https://github.com/jeffglousher/coaptic-validation/tree/e602a261fac18d839e92def713ae410f7c78d15d/tools/benchmark/evidence/packet-live-ranges-20261106) is pinned to companion `e602a261fac18d839e92def713ae410f7c78d15d`
and delivered in [validation PR #256](https://github.com/jeffglousher/coaptic-validation/pull/256).
Independent verification checks 66,472 members across twelve bundles and preserves
all eleven previous summaries exactly. A fresh reconstruction verifies all 148
core source hashes and seven original fixture hashes.

From that evidence directory, run `python -B verify.py` to verify the archive and
recompute results. Run `python -B prepare.py CORE_REPO NEW_OUTPUT candidate` to
reconstruct the measured source and probes without changing a checkout. Native
commands retain their Windows paths; adapt paths when replaying elsewhere while
keeping the pinned toolchain, features, frame-pointer settings, capacities and
cohort settings. A matching reconstruction is not a promise of the same
performance on another host.
