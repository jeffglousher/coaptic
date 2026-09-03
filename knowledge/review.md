---
type: Policy
title: Review and authorship
description: Agents and humans are equal authors and reviewers. rustdoc over comments. Git is history.
resource: ../AGENTS.md
tags: [review, policy, rustdoc]
status: stable
sources:
  - id: agents
    resource: ../AGENTS.md
    title: Always-on agent file
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Policy

- Agents and humans are the same kind of contributor. Either may author. Either may review. No extra gates for agents.
- Public docs live in rustdoc (`//!` / `///`). Do not narrate protocol or architecture in `//` comments. Point rustdoc at `design.md` and `knowledge/rfcs/`.
- Git is the history. Delete deprecated files, leftover names, and "for later" shims from the tree. Old commits remain.

See [CI](/ci.md) for squash and PR-only checks.
