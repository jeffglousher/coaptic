# Contributing

Humans and agents are the same kind of contributor. Follow [AGENTS.md](AGENTS.md).

Architecture planning: [GitHub project](https://github.com/users/jeffglousher/projects/2). Protocol copies: [knowledge/rfcs/](knowledge/rfcs/).

The repository is public. Merges to `main` are squash-only. Pull requests must pass CI (`fmt`, `clippy`, `test`, `doc`); those jobs run on pull requests to `main` (and `workflow_dispatch`). Version tags (`v*`) publish rustdoc to GitHub Pages. The crate is not published to crates.io.

Toolchain is `stable` (`rust-toolchain.toml`). Edition is 2024 (MSRV 1.85).
