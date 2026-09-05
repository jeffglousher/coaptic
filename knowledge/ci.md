---
type: Playbook
title: CI and release
description: "PR-only fmt/clippy/test/doc/okf. v* tags publish rustdoc. Squash-only merges."
resource: ../.github/workflows/ci.yml
ci_triggers: [on.pull_request, workflow_dispatch]
release_triggers: [on.push.tags, workflow_dispatch]
tags: [ci, release, github]
status: stable
sources:
  - id: ci
    resource: ../.github/workflows/ci.yml
    title: PR CI workflow
  - id: release
    resource: ../.github/workflows/release.yml
    title: Tag Release workflow
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource, ci_triggers, release_triggers]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# Triggers

| Trigger | Workflow | What it does |
| --- | --- | --- |
| `pull_request` to `main`, `workflow_dispatch` | CI | `fmt`, `clippy`, `test`, `doc`, `okf` |
| `v*` tags, `workflow_dispatch` | Release | rustdoc to GitHub Pages |

# Policy

- No push-to-main CI. Merging a PR does not start another CI run.
- Merges are squash-only.
- Workflows are trimmed for GitHub Free-plan minutes.
- Do not publish crates.io until Jeff says the first public go.
- Repo stays private until that first public go.

See [Review policy](/review.md) and the external-reviewer brief [`REVIEW.md`](../REVIEW.md).
