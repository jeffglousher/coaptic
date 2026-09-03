---
name: okf-update
description: Add or edit OKF knowledge in knowledge/. Use when creating or changing concepts, index.md, or log.md.
license: MIT OR Apache-2.0
---

# Update knowledge

Follow [OKF v0.2](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md).

- Every concept `.md` needs YAML frontmatter with required `type`.
- Recommended: `title`, `description`, `tags`, `generated: { by, at }`.
- Do not add `verified` unless Jeff actually reviewed that file.
- `knowledge/index.md` MAY have only `okf_version: "0.2"`. No `type` on `index.md` or `log.md`.
- Cross-links inside the bundle start with `/` (example: `[Engine](/crate.md)`).
- `sources[].resource` may be a repo-relative path (`../design.md`) or a URL. Do not use `/../design.md`.
- Do not duplicate `design.md` or RFC text. Link them.
- Append a short **Creation**/**Update** line to `knowledge/log.md` (newest date first).
- Keep the bundle small. Progressive disclosure.
