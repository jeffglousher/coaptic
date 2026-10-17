# AGENTS.md

Humans and agents may author and review.

## Pre-release policy

Coaptic is pre-release. Backwards compatibility and versioning are not development
requirements. Change or remove project APIs, formats and obsolete paths directly;
update affected callers in the same batch. Do not add compatibility shims,
deprecation periods or version bumps unless explicitly requested for a release.
Protocol standards, security invariants and preservation of user data still apply.

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
