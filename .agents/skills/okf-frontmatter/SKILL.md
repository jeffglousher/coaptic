---
name: okf-frontmatter
description: Use when creating or updating OKF concepts in knowledge/. Three-pass frontmatter: run the deterministic extractor first, then LLM-enrich and dedupe, then validate (fail closed). Not for generic markdown.
license: MIT OR Apache-2.0
---

# OKF frontmatter (extract → enrich → validate)

OKF v0.2: `type` is required; `index.md` and `log.md` are reserved. Spec: https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md

Three passes, fail closed. Do not ship extract+enrich without validate. An LLM "looks good" cannot override a NO-GO.

## Pass 1 — extract (no LLM)

From repo root:

```bash
python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py
```

Walks `knowledge/**/*.md`, skips reserved names. Emits a YAML mapping of concept-id → proven frontmatter only (paths, `stat`, IETF txt header, `Cargo.toml`, workflow `on:` keys, vendored `TD_*` keys). No network. If a field cannot be parsed, omit it. Stamps `deterministic: { by: process:okf-frontmatter, at, fields }`.

`--check` fails if an RFC concept `title` or `resource` disagrees with the sibling txt.

## Pass 2 — enrich and dedupe

Load pass-1 YAML. May add: one-sentence `description`, extra `tags`, body cross-links, TD objective tables from YAML `obj:` (not in pass 1), `generated: { by: <actor>, at: <now> }`.

Merge (mandatory):

- Start from the pass-1 mapping.
- For each LLM key: if pass 1 already has that key, **drop the LLM value**. Exception: `tags` is a set-union; `sources` is union keyed by `id` or `resource` (pass-1 entry wins on the same id).
- Never change `type`, `resource`, parsed RFC title, dates, file sizes, `ietf_status`, Cargo fields, TD id lists, or `deterministic`.
- `generated` is the LLM pass. Keep `deterministic` from pass 1.
- Do not add `verified` unless a human actually reviewed.
- Do not restate RFC wire format in new body text.

## Pass 3 — validate (no LLM)

Reviews **finished files on disk**, not the LLM's claims:

```bash
python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate
```

or `python3 .agents/skills/okf-frontmatter/scripts/validate_frontmatter.py`.

PR CI job `okf` runs `--validate` (same `pull_request` / `workflow_dispatch` triggers as the rest of CI; no push-to-main). Non-zero validate fails the check (PR NO-GO).

Critical (exit non-zero, NO-GO): missing frontmatter or `type`; reserved files with a concept `type`; any pass-1 field disagrees with a fresh extract; RFC without sibling `rfcNNNN.txt` or filename ≠ `Request for Comments:`; duplicate `sources[].id` with different `resource`; new `verified` `human:` actor not already on `main`; plugtest `td_ids` not a subset of vendored YAML keys; missing `deterministic` / `deterministic.fields` after regenerate.

Non-critical (warn, still GO): extra unknown keys, broken optional cross-links, description style.

If validate fails, fix pass-2 output and re-run pass 3 until green, or report the NO-GO. Do not merge-quality-claim a red validate.
