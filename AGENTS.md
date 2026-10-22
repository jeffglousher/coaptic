# Working on Coaptic

Read [PLAN.md](PLAN.md) for the next work and [DEVELOPMENT.md](DEVELOPMENT.md)
for checks. Humans and agents may author and review.

## Deliver

- Verify current source and requirements, then own the next unblocked task.
  Finish code and integration before expanding diagnostics or qualification,
  unless that work is requested.
- Work in coherent batches, then run checks that can catch faults in the change.
  Reuse valid evidence; repeat tests for a concrete concern.
- Keep independent work moving during builds and reviews. A hardware action or
  approval blocks only the work that needs it.
- Fix review findings and merge authorized changes to main by squash only.
  Clean settled branches and worktrees. Hosted CI is advisory during pre-release.
- Report what changed, what ran and what remains. Ask for an exact human action
  only when it cannot be completed autonomously.

## Preserve the design

- Pre-release APIs and formats may change directly. Update callers together;
  no compatibility shims, deprecation periods or version bumps unless requested.
- Constrained means predictable memory and work, with complete supported features,
  explicit capacity refusal and recovery. Allocate pools at startup and reuse them.
  Automatic expansion stays off; any future growth must be explicit and bounded.
- Reuse storage before adding buffers. Count metadata, scratch, alignment,
  initialization, security, transport and runtime costs without double-counting.
  Keep whole-object ceilings. Start device comparisons with SmallDevice/Constrained
  and name the route, Call, deferred, security and body capacities.
- Preserve authentication, protocol invariants, credentials, identity, NVS and
  unresolved effects. Missing history is never first use. Hardware qualification
  and software tests are separate claims.
- Describe security, blockwise transfers and storage as options of one App.
  Compare equivalent workloads and identify the measured sources. Keep failures
  and limitations; no whole-RFC or certification claim from a passing suite.

## Keep it clear

Put API contracts in rustdoc, priorities in PLAN.md and actionable work in issues.
Write the README for users. Comments explain decisions and invariants.
Call security-disabled operation **unprotected**; retain standard protocol terms.
Read original RFCs in `knowledge/rfcs/` instead of copying wire specifications.

Reuse compatible build lanes. Remove only settled, owned output with no active
consumer; preserve evidence and shared caches. Keep private state out of this
public repository. Tags, publication and outreach require separate authorization
under [#132](https://github.com/jeffglousher/coaptic/issues/132).
