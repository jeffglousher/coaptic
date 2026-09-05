# AGENTS.md

Agents and humans are the same kind of contributor. Either may author. Either may review.

Public docs are rustdoc. Architecture planning is GitHub Issues / [project](https://github.com/users/jeffglousher/projects/2). Do not restate RFC wire format; open `knowledge/rfcs/`.

Merges are squash-only. The repository stays private until a first public go.

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo doc --no-deps --all-features
```

See [CONTRIBUTING.md](CONTRIBUTING.md).
