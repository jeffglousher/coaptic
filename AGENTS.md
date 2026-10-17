# AGENTS.md

Humans and agents may author and review.

## Pre-release policy

Coaptic is pre-release. Backwards compatibility and versioning are not development
requirements. Change or remove project APIs, formats and obsolete paths directly;
update affected callers in the same batch. Do not add compatibility shims,
deprecation periods or version bumps unless explicitly requested for a release.
Protocol standards, security invariants and preservation of user data still apply.

## Deterministic resource design

Constrained means intentionally bounded memory and work for reliability,
performance and ease of integration. It does not mean an arbitrarily reduced
feature set. Complete accepted capabilities within explicit ownership, capacity,
refusal and recovery contracts. Do not claim safety-standard certification.

- Reuse storage across mutually exclusive lifetimes before adding buffers or
  tables. Account for metadata, alignment, scratch, initialization peaks,
  transport and security state. Keep independent whole-object ceilings; adding
  a feature allowance alone is not adequate RAM accounting.
- Start device comparisons with `resources::SmallDevice` and its
  `profiles::Constrained` pools. Name route, Call, deferred and security capacities
  too: the pool profile alone does not define an App. Change one dimension at a
  time; use larger presets only for a concrete workload requirement.
- Describe one App with named options: security, blockwise transfers and storage
  backend. Blockwise is a transfer capability, not a separate product. Compare
  equivalent capabilities and workload semantics before comparing performance.
- Report current source and configuration first. Use matching-source evidence;
  otherwise state what is unmeasured. Preserve older evidence for traceability,
  but omit historical comparisons unless requested or needed for a specific fault.
- Finish coding and integration before benchmark expansion, presentation polish
  or stability campaigns. Focused correctness and resource checks remain part of
  each implementation batch, including real loopback and the reserved ESP32.

## Own delivery

Done means the accepted behavior is implemented and supported by relevant proof.

- Verify current source, requirements and retained evidence on resume. Treat
  previous summaries as leads, not proof; reuse evidence whose source still matches.
- Implement the next unfinished requirement, including refusal and recovery.
  Resolve routine engineering decisions and tooling failures within existing authority.
- Keep useful work moving while builds, reviews and CI run. A pending dependency
  blocks only dependent work. Watching a PR is not completion.
- Implement coherent batches, then run checks that can detect faults in the batch.
  Check earlier when needed to resolve an implementation uncertainty. Once results
  pass and make sense, proceed; repeat or broaden only for a concrete concern.
- Drive review fixes, authorized squash merges and cleanup of settled owned work.
  Preserve unrelated changes, evidence, credentials, firmware and shared resources.
- Prioritize aggressive capability development, simplification and measured
  optimization, especially UDP/OSCORE, real loopback and the reserved ESP32.
  Keep documentation polish and repeated qualification campaigns off the critical path.
- Report changes, executed evidence and remaining work. Distinguish implementation,
  interoperability and physical qualification. Do not claim completion from CI alone.
- Stop when scope is complete, the user pauses it, or every useful authorized path
  is blocked. State the exact prerequisite and continue independent work where possible.

## Keep the repository clear

Disk space is constrained. Reuse compatible build lanes. Promptly remove settled,
owned worktrees, branches and exclusive generated output after verifying ownership,
merge/settlement and absence of active consumers. Retain compact evidence and
uncommitted work; do not blanket-clean shared targets. Report reclaimed space and
large retained paths with their owner and release condition.

Public API contracts belong in rustdoc; scope and delivery belong in GitHub
[issues/project](https://github.com/users/jeffglousher/projects/2). Keep the README
focused on starting and choosing an integration. Explain invariants and decisions
in comments; remove narration, repeated guidance and stale progress reports.

Call the security-disabled mode **unprotected**. Retain standard cryptographic
terms and standards-defined wire identifiers; remove obsolete project aliases.
Use original protocol text in `knowledge/rfcs/`; do not duplicate RFC wire formats.

The repository is public. Follow [DEVELOPMENT.md](DEVELOPMENT.md) for validation
and release boundaries. Merges to `main` are squash-only.
