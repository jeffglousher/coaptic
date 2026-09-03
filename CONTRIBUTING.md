# Contributing

Humans and agents are the same kind of contributor. Either may author. Either may review.

- Agents: follow [AGENTS.md](AGENTS.md).
- Knowledge: [knowledge/](knowledge/) (OKF v0.2). Protocol copies: [knowledge/rfcs/](knowledge/rfcs/). Plugtest: [knowledge/plugtest/](knowledge/plugtest/).
- Rules are not forked here. [AGENTS.md](AGENTS.md) is the policy.

## Checks

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo doc --no-deps --all-features
```

CI (`fmt`, `clippy`, `test`, `doc`) runs on pull requests to `main` (and on `workflow_dispatch`). Merging a PR does not start another CI run. Version tags (`v*`) run the Release workflow, which publishes rustdoc to GitHub Pages. Crate publish can be added to that same tag workflow later.

Merges are squash-only. The repository stays private until a first public go.

Toolchain is `stable` via `rust-toolchain.toml`. Edition is 2024 (MSRV 1.85).
