# Receive scratch investigation: no accepted optimization

Four candidates against #741 (`bd63199b652193b4c0a882e5840e72d992cf1362`) failed
the frozen complete-App runtime checks. All production changes were retired;
the baseline behavior and previously delivered improvements remain intact.
Do not repeat these unchanged hypotheses or change their acceptance criteria.

The [portable evidence](https://github.com/jeffglousher/coaptic-validation/tree/main/tools/benchmark/evidence/receive-scratch-20261105)
retains exact patches, raw outputs, failures, compiler layouts, fixture hashes,
source reconstruction and independent verifiers. No candidate is accepted because
it compiles, passes regression tests or improves one execution mode.

- Direct App-private OSCORE request opening (`79a79ea3`) removes eligible
  plaintext staging, but neither sync nor async establishes lower runtime cost.
- Request-phase separation (`bd28f6fd`) reduces the selected complete async future
  by 1,952 bytes, but increases runtime cost.
- Combining No-Response/proxy/validation option walks (`fbba2ab4`) with that split
  improves sync runtime; async acceptance and two per-cell guards fail.
- Grouping coroutine inputs (`40f13c91`) retains the selected future reduction
  and compacts its initial input span, but async acceptance and four guards fail.

All comparisons execute the complete server App in both synchronous and native
asynchronous modes with unchanged capacities. The 28-cell mix includes ordinary
SmallDevice/Constrained and fixed/allocated Service workloads, plus retained
Q-Block2, each unprotected and established OSCORE. Four paired AB/BA/BA/AB process
means use one-sided 95% bounds; both ordinary 20 and all 28 sync/async bounds must be
below parity, and every cell point ratio must be at most 1.02. All four candidates
fail this same rule. Earlier failures are not replaced or pooled.

The memory result is a matched Windows release `std,alloc,oscore`
Constrained/CoreIo/routes4 `progress_async` future: 15,952 to 14,000 bytes. It is
compiler layout, not stack use, peak RAM or minimum device memory. The earlier
all-feature/routes1 future is a different fixture. Grouping reduces the child's
initial input span 7,328 to 208 bytes without further shrinking the complete future;
that does not establish a runtime explanation.

Corrected direct opening passes 147 focused tests. Request-phase and metadata
pass 870 no-default+oscore library tests each. General read-only reviews report
no actionable defects. The grouped candidate compiles in ordinary/retained native
and layout lanes; further library qualification stopped after runtime rejection.
An initial all-feature LLVM memory failure, prototype public error-buffer
immutability failures, incomplete baseline layout export and a retained-probe
feature qualification failure remain in the evidence. No timing preceded that
qualification failure; corrected flags were used for its sole measured cohort.

## Libcoap comparison and next work

Both the companion's pinned libcoap upstream `851533c3` and inspected current
`45ace062` use temporary OSCORE plaintext PDUs followed by option merge and payload
copy into a final PDU. Their parse view over decrypted wire is useful; importing
dynamic growth is inappropriate for Coaptic's bounded storage contract. The
direct-to-final Rust adaptation above did not establish a runtime benefit.
See the evidence for pinned source links. No cross-library speed or memory claim
is supported by this source inspection.

Next isolate material complete-App async costs before another implementation.
Reuse the existing fixtures and build lanes. Preserve security, cancellation,
configured capacities and both execution modes; firmware/platform work remains
deferred. These rejected experiments do not complete the optimization goal.
