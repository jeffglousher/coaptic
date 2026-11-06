# Response ownership follow-up

This investigation did not produce another accepted implementation. Production
remains the phase-sharing batch merged as `fd7d18a`. The user prioritized CPU and
memory, allowing improvement in one with preservation of the other within the
declared tolerance. Public behavior, security and configured capacities remain
requirements; simpler ownership alone is not evidence of lower runtime cost.

The pinned baseline is merged `fd7d18a72f795779e352443ebdeef9533be7fb27`.
Its 148 production input files match the previously measured `3e592ad` sources,
allowing exact baseline executable reuse. These results are relative to that
accepted implementation, not the earlier `bd63199` baseline.

## Experiments

- `04c1c837`: borrowed handler responses with a split reply-storage view. Selected
  SmallDevice, fixed Service and allocated Service complete futures shrink by
  928, 1,024 and 992 bytes. Complete-App CPU ratios are 1.005496 sync and 1.025043
  async; one-sided upper95 bounds are 1.007985 and 1.027402. Seventeen of 28 cell
  bounds fail. Dispatch code and selected stack frames also grow. Rejected.
- `3a1e600`: one reply view for ordinary and assembled requests. Removes duplicate
  dispatch paths but still increases the four selected dispatch frames by 320,
  784, 304 and 528 bytes. Rejected before runtime measurement.
- `6ad0bfd`: consolidate duplicate response identity and discard the unused
  retained-header payload slice. All three selected complete futures shrink by
  120 bytes. The allocated Service dispatch frame grows 16 bytes. Rejected before
  timing under the prospective per-frame nongrowth guard; 1,219 library tests pass.
- `9e5bad5`: additionally borrow peer and receive time from the existing immutable
  client inbox hold, already borrowed by options and locations. All three selected
  complete futures shrink by 216 bytes. Fixed Service dispatch frame shrinks
  32 bytes, allocated Service grows 16 bytes, and the other two stay equal.
  Rejected before timing by the same guard; 1,220 library tests pass. A later
  visibility-only warning cleanup (`5690c31`) is retained separately and unbuilt.

The last two candidates do not change public signatures or resident owners.
Manual snapshot Debug formatting in the borrowed-origin variant avoids exposing
the raw retained owner or bypassing token formatting. Source review found no
actionable defect. Passing tests and source review do not override the failed
measurement guard, nor establish full qualification.

## Method and scope

The only new runtime cohort is `04c1c837`: 64 paired processes for each of 28
workloads, fixed AB/BA/BA/AB order repeated sixteen times, 128 warmup operations,
and no discarded samples or replacement runs. Twenty ordinary cells cover
SmallDevice32 and fixed/allocated Service1024/16384. Eight retained Q-Block2 cells
use 11,264-byte bodies. Both synchronous poll and asynchronous wait/progress run
in unprotected and established OSCORE modes. All ordinary/all sync/async aggregate
and individual-cell upper95 CPU ratios must be at most 1.02 for the memory route.

QueryThreadCycleTime covers complete server App work with a memory transport.
Client preparation and verification are outside the interval. These are CPU-cost
observations, not network throughput, latency, peak stack, hardware qualification
or a libcoap ranking. The static benchmark handlers do not exercise dynamic
borrowed response payloads; there is no speed claim for that new capability.

Host resident App/pool owners and selected Thumbv7EM App/table layouts remain
unchanged. Capacities are not reduced. Selected compiler future objects and
emitted native frames are distinct measurements, not peak memory. The 16-byte
frame refusals are conservative local guards; they do not prove the complete
application uses more simultaneous memory or runs slower. No runtime cohort was
started for those sources, and no reserved-stack saving is claimed.

The direct higher-ranked lifetime prototype failed to compile because an incoming
body borrow through the whole Engine prevents reply mutations. The subsequent
view adds a storage trait and copies/restores metrics. The fixture adaptation
adds those trait bounds and changes the retained callback return lifetime; its
operation bodies stay unchanged. The metadata variants use the original fixture.
Borrowed-handler's twelve focused passing checks belong to a precursor. Its
expanded protected/counter test patch is unrun and explicitly retained as such.

Exact patches, source hashes, raw outputs, binaries, assembly, layouts, methods,
failed attempts and offline verification are retained in the
[companion evidence package](https://github.com/jeffglousher/coaptic-validation/tree/518c7fd59d0ae29761f59b2aaa8f112c3c3f7bed/tools/benchmark/evidence/response-ownership-20261106).
No failed source
is adopted, no old failure is reclassified and no unchanged cohort is repeated.

## Next constraint

Further work should establish which packet-construction copies and dispatch live
ranges dominate complete-App work before choosing another rewrite. Borrowing
already-held metadata is structurally simpler, but generated frame layout can
still move in the opposite direction. Preserve body/packet independence and
bounded refusal/recovery; defer runtime adapters, firmware migration and platforms.
