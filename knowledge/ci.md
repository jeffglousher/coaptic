---
type: Playbook
title: CI and release
description: PR-only fmt/clippy/test/doc. v* tags publish rustdoc. Squash-only merges.
resource: ../.github/workflows/ci.yml
tags: [ci, release, github]
generated: { by: cursor_agent/cursor-grok-4.6-high-fast, at: 2026-09-03T11:50:09Z }
status: stable
sources:
  - id: ci
    resource: ../.github/workflows/ci.yml
    title: PR CI workflow
  - id: release
    resource: ../.github/workflows/release.yml
    title: Tag Release workflow
---

# Triggers

| Trigger | Workflow | What it does |
| --- | --- | --- |
| `pull_request` to `main`, `workflow_dispatch` | CI | `fmt`, `clippy`, `test`, `doc` |
| `v*` tags, `workflow_dispatch` | Release | rustdoc to GitHub Pages |

# Policy

- No push-to-main CI. Merging a PR does not start another CI run.
- Merges are squash-only.
- Workflows are trimmed for GitHub Free-plan minutes.
- Do not publish crates.io until Jeff says the first public go.
- Repo stays private until that first public go.

See [Review](/review.md).
