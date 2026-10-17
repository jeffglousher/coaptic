# AGENTS.md

Humans and agents may author and review.

## Own delivery

Done means the accepted behavior is implemented and supported by relevant proof.

- Verify current source, requirements and retained evidence on resume. Treat
  previous summaries as leads, not proof; reuse evidence whose source still matches.
- Implement the next unfinished requirement, including refusal and recovery.
  Resolve routine engineering decisions and tooling failures within existing authority.
- Keep useful work moving while builds, reviews and CI run. A pending dependency
  blocks only dependent work. Watching a PR is not completion.
- Run checks that can detect faults in the change. When they pass and the result
  makes sense, proceed. Repeat or broaden checks only for a new change, failure,
  unexplained result or an applicable release requirement.
- Drive review fixes, authorized squash merges and cleanup of settled owned work.
  Preserve unrelated changes, evidence, credentials, firmware and shared resources.
- Prioritize UDP/OSCORE development, real loopback and the reserved ESP32. Finish
  accepted requirements before optimization or repeated qualification campaigns.
- Report changes, executed evidence and remaining work. Distinguish implementation,
  interoperability and physical qualification. Do not claim completion from CI alone.
- Stop when scope is complete, the user pauses it, or every useful authorized path
  is blocked. State the exact prerequisite and continue independent work where possible.

## Keep the repository clear

Public API contracts belong in rustdoc; scope and delivery belong in GitHub
[issues/project](https://github.com/users/jeffglousher/projects/2). Keep the README
focused on starting and choosing an integration. Explain invariants and decisions
in comments; remove narration, repeated guidance and stale progress reports.

Call the security-disabled mode **unprotected**. Retain standard cryptographic
terms, wire identifiers and compatibility names where changing them breaks a contract.
Use original protocol text in `knowledge/rfcs/`; do not duplicate RFC wire formats.

The repository is public. Follow [DEVELOPMENT.md](DEVELOPMENT.md) for validation
and release boundaries. Merges to `main` are squash-only.
