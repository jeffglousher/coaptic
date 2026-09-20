# Contributing

Humans and agents follow [AGENTS.md](AGENTS.md). Architecture planning lives in
[GitHub project 2](https://github.com/users/jeffglousher/projects/2); protocol source
copies live in `knowledge/rfcs/`. Public API documentation belongs in rustdoc.

Use the stable toolchain in `rust-toolchain.toml`. The library uses edition 2024
and supports Rust 1.85; test peers have independent dependency requirements.
Merges to `main` are squash-only and must pass the repository CI checks.

## Local validation

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy -p coaptic-plugtest --all-targets --features dtls,oscore -- -D warnings
cargo test --no-default-features
cargo test --all-features
cargo test -p coaptic-plugtest --features dtls,oscore
cargo doc --no-deps --all-features
cargo package -p coaptic --locked
cargo +1.85.0 check --locked -p coaptic --no-default-features
cargo +1.85.0 check --locked -p coaptic --all-features
```

Run the [dogfood baseline checks](crates/coaptic-plugtest/README.md) and
[independent process suite](tools/interop/README.md) for transport/harness changes.
The [CI workflow](.github/workflows/ci.yml) is authoritative for job flags and
platforms. It runs on pull requests and manual dispatch; process timing JSON is
retained as the `process-interop` artifact. CI smoke timings are not a performance SLA.

## Cross-target build evidence

CI pins Rust 1.97.1 for Cortex-M0 (`thumbv6m-none-eabi`), Cortex-M4
(`thumbv7em-none-eabi`), RV32 (`riscv32imac-unknown-none-elf`) and WebAssembly
(`wasm32-unknown-unknown`). `tools/qualification/cross_build.py` builds a concrete
bounded App probe with core, alloc, OSCORE and alloc+OSCORE configurations and
archives source identity, compiler, commands and results. Install that toolchain
and target, then run it with `--target TARGET --output PATH` to reproduce.

This establishes release code generation, including the library dependency.
It does not link a firmware image, execute on a device, measure stack high-water,
or qualify platform allocators, entropy or networking. Those remain tracked in #202.
The library MSRV remains 1.85; ordinary host CI also checks current stable.

## Packaging and release

Only `coaptic` is published; harnesses and peers are test-only. Packaging checks
exclude vendored knowledge, tools and workspace test crates from the tarball.
Publishing remains parked ([#132](https://github.com/jeffglousher/coaptic/issues/132)).
Version tags can trigger rustdoc and crates.io release workflows; do not create
release tags as part of ordinary development.
