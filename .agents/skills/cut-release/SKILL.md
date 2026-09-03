---
name: cut-release
description: Cut a version tag that publishes rustdoc. Use when releasing or tagging a version. Do not publish crates.io until Jeff says the first public go.
license: MIT OR Apache-2.0
---

# Cut a release

- Tag `vX.Y.Z` only. That is what runs the Release workflow and publishes rustdoc to GitHub Pages.
- Do not add push-to-main CI.
- Do not publish to crates.io until Jeff says the first public go.
- Repo stays private until that first public go.

See [knowledge/ci.md](../../../knowledge/ci.md).
