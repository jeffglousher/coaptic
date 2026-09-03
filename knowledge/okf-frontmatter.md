---
type: Playbook
title: OKF frontmatter pipeline
description: Repo-specific three-pass frontmatter for knowledge/ concepts; fail closed on validate.
resource: ../.agents/skills/okf-frontmatter/scripts/extract_frontmatter.py
tags: [okf, frontmatter, knowledge]
status: stable
sources:
  - id: skill
    resource: ../.agents/skills/okf-frontmatter/SKILL.md
    title: okf-frontmatter skill
  - id: okf-spec
    resource: "https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md"
    title: OKF v0.2 spec
deterministic:
  by: "process:okf-frontmatter"
  at: "2026-09-03T12:49:03Z"
  fields: [type, resource]
generated:
  by: cursor_agent/cursor-grok-4.6
  at: "2026-09-03T12:49:03Z"
---

# OKF frontmatter

This bundle's concept YAML is produced in three passes. Procedure: [okf-frontmatter skill](../.agents/skills/okf-frontmatter/SKILL.md).

| Pass | Who | Command |
| --- | --- | --- |
| 1 extract | `process:okf-frontmatter` | `python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py` |
| 2 enrich | LLM, then dedupe (pass-1 keys win) | (agent) |
| 3 validate | same process, no LLM | `python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate` |

Pass 3 reviews files on disk. Critical failure is a NO-GO. Do not invent `verified` human review.
