# AGENTS.md

Agents and humans are the same kind of contributor. Either may author. Either may review. Do not add extra gates for agents.

## Docs

Public docs live in rustdoc (`//!` / `///`). Do not narrate protocol or architecture in `//` comments. Point rustdoc at `design.md` and `knowledge/rfcs/`. External-reviewer brief: `REVIEW.md`. UDP peer sidecar is `Endpoint` (not `Peer`).

## History

Git is the history. Do not keep deprecated files, name graveyards, or compatibility shims "for later." Delete from the tree; old commits remain.

## Architecture

Read `knowledge/index.md` then `design.md` before architecture work. Do not rewrite `design.md` or `knowledge/rfcs/` unless the task says to. Do not restate RFC wire format, timers, or option semantics.

## Crate

Default is `no_std` with no allocator. `alloc` and `std` are optional (`std` implies `alloc`). Own the slot and table types in this crate. Do not add `heapless` as the architecture.

## Pull requests

Keep PRs small. One concern per PR. Squash merge. CI is PR-only (`pull_request` + `workflow_dispatch`; no push-to-main CI). The repo stays private until a first public go.

## Checks

When you touch Rust:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo doc --no-deps --all-features
```

When you touch `knowledge/`:

```bash
python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate
```

## Skills and knowledge

Skills live in `.agents/skills/`:

- `bounded-engine` — memory areas, slots, tables, progress, pin/app access
- `plugtest` — CoAP interoperability against this repo's plugtest bundle
- `okf-frontmatter` — three-pass OKF frontmatter (extract → enrich → validate; fail closed)

Knowledge lives in `knowledge/` (OKF v0.2). Protocol copies: `knowledge/rfcs/`. Plugtest: `knowledge/plugtest/`.
