# Contributing

Humans and agents are the same kind of contributor. Either may author. Either may review.

- Agents: follow [AGENTS.md](AGENTS.md).
- External review brief: [REVIEW.md](REVIEW.md). Caller contract: [CALLER.md](CALLER.md).
- Knowledge: [knowledge/](knowledge/) (OKF v0.2). Protocol copies: [knowledge/rfcs/](knowledge/rfcs/). Plugtest mapping: [knowledge/plugtest/](knowledge/plugtest/).
- Architecture names: [knowledge/memory.md](knowledge/memory.md). UDP peer sidecar is `Endpoint` (not `Peer`). Application pin is `Access`. One bounded pass is `Engine::progress` → `Progress`.
- Rules are not forked here. [AGENTS.md](AGENTS.md) is the policy.

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

CI (`fmt`, `clippy`, `test`, `doc`, `okf`) runs on pull requests to `main` (and on `workflow_dispatch`). Merging a PR does not start another CI run. Version tags (`v*`) run the Release workflow, which publishes rustdoc to GitHub Pages. The crate is not published to crates.io.

Merges are squash-only. The repository stays private until a first public go.

Toolchain is `stable` via `rust-toolchain.toml`. Edition is 2024 (MSRV 1.85).
