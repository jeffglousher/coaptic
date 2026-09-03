---
name: review-change
description: Review a PR or diff with the shared human/agent checklist. Use when reviewing a pull request, commit, or working-tree diff.
license: MIT OR Apache-2.0
---

# Review a change

Agents and humans use the same checklist. No extra gates for agents.

## Checklist

- Public docs are rustdoc (`//!` / `///`), not `//` comments that narrate protocol or architecture.
- Do not restate RFC wire format, timers, or option semantics. Point at `rfcs/`.
- Do not leave leftover or deprecated names in the tree. Git is history.
- PR is squash-friendly: one concern. Title reads like a squash commit.
- CI is PR-only (`fmt`, `clippy`, `test`, `doc`). Confirm the PR will run those jobs.

Read [AGENTS.md](../../../AGENTS.md) and [knowledge/review.md](../../../knowledge/review.md).
