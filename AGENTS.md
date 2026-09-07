# AGENTS.md

Agents and humans are the same kind of contributor. Either may author. Either may review.

Public docs are rustdoc. Architecture planning is GitHub Issues / [project](https://github.com/users/jeffglousher/projects/2). Do not restate RFC wire format; open `knowledge/rfcs/`.

The repository is public. Merges to `main` are squash-only. Pull requests must pass CI (`fmt`, `clippy`, `test`, `doc`).

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
cargo test -p coaptic-plugtest --features dtls
cargo doc --no-deps --all-features
cargo package -p coaptic --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md).
