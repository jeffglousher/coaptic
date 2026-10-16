# AGENTS.md

Humans and agents may author and review.

## Own delivery

Take charge of the agreed outcome. "Done" means development completed against
the accepted requirements, with relevant proof. Plans, open PRs, green checks and
registered watchers are progress, not completion of missing requirements.

- Drive authorized work through implementation, focused validation, review fixes,
  merge when authorized, and cleanup of owned settled work. Take the next accepted
  requirement without asking the user to manage routine engineering decisions.
- Keep developing while CI, reviews, builds or other agents run. A pending
  dependency blocks only dependent work. Advance an independent requirement,
  prepare a focused stacked change, investigate a failure or review the next
  change. Babysit PRs in the background; never stop at watching while useful
  authorized work remains.
- Resolve obstacles actively. Inspect the environment and logs, test promising
  alternatives with small reversible experiments, and fix causes. If one path
  is blocked, continue another. Ask only for missing facts, decisions or authority
  that cannot be established from the session and repository.
- Hold the quality bar. Finish the requirement and its refusal/recovery behavior;
  do not accept unexplained failures, weaken checks or reduce agreed scope to
  claim completion. Run the required checks and focused proof of changed behavior.
  Repeat broader testing when a change, failure or unresolved concern justifies it.
- Prioritize accepted UDP/OSCORE feature development and the real loopback and
  reserved ESP32 paths. Complete requirements before extensive optimization,
  API presentation, documentation polish or repeated integration campaigns.
- Report concrete changes, executed evidence and remaining requirements. Keep
  implementation, local checks, independent interoperability and physical-device
  qualification distinct; expose gaps rather than treating partial proof as done.
- Stop only when the requested scope is complete, the user explicitly pauses it,
  or every useful authorized path is blocked. For a real blocker, state the exact
  prerequisite, attempted remedies and next action; continue independent work
  while seeking the missing input.

## Repository contracts

Public docs are rustdoc. Architecture planning is GitHub Issues / [project](https://github.com/users/jeffglousher/projects/2). Do not restate RFC wire format; open `knowledge/rfcs/`.

The repository is public. Merges to `main` are squash-only. Follow
[DEVELOPMENT.md](DEVELOPMENT.md) for required validation and release policy.
