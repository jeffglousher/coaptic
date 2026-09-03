# Contributing

Humans and agents follow the same rules.

## Rules

- Read `design.md` first. Do not rewrite `design.md` or the files in `rfcs/` unless the change is specifically about those documents.
- Do not restate IETF wire format, protocol state machines, or option semantics. Point at `rfcs/`.
- The crate is `#![no_std]` by default. `alloc` and `std` are optional features (`std` implies `alloc`).
- Slot and table types live in this crate. Do not depend on `heapless` for that architecture.
- Keep pull requests small.

## Checks

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo doc --no-deps --all-features
```

CI (`fmt`, `clippy`, `test`, `doc`) runs on pull requests to `main` (and on `workflow_dispatch`). Merging a PR does not start another CI run. Version tags (`v*`) run the Release workflow, which publishes rustdoc to GitHub Pages. Crate publish can be added to that same tag workflow later.

Toolchain is `stable` via `rust-toolchain.toml`. Edition is 2024 (MSRV 1.85).
