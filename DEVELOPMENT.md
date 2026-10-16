# Development

Humans and agents follow [AGENTS.md](AGENTS.md). Planning is
[GitHub project 2](https://github.com/users/jeffglousher/projects/2). Protocol
copies are `knowledge/rfcs/`. Public API documentation is rustdoc.

Use the toolchain in `rust-toolchain.toml`. The library is edition 2024 and
MSRV 1.85. Test peers have their own dependency requirements. Merges to `main`
are squash-only and must pass CI.

## Agent delivery loop

1. Resume from the accepted scope and current source/PR state. Use the GitHub
   Issues/project requirements to choose the next unfinished behavior and its
   acceptance evidence. Keep the user informed of concrete progress and decisions.
2. Implement a focused increment and its refusal/recovery paths. Validate the
   changed behavior locally; use real loopback and the reserved ESP32 when those
   boundaries need proof. Preserve source identities and limitations in results.
3. Open a reviewable PR and keep working. CI and review run alongside development:
   start the next independent requirement or a focused stack, fix known gaps,
   or investigate failures. For example, prepare the next provisioning increment
   while its parent PR runs; after the parent merges, rebase and retarget it.
   Registering a watcher is never a substitute for advancing available work.
4. Triage failures, review findings, conflicts and stalled tooling yourself.
   Resolve routine problems within existing authority. Bring the user a concrete
   decision only when a missing fact, permission or requirement prevents progress;
   continue work that does not depend on that answer.
5. When authorized, merge the reviewed change with required CI passing for the
   intended source. Settle owned branches/output after verifying merge and lack
   of active consumers, preserve evidence and shared resources, then advance the
   next accepted requirement. An open PR or passed campaign does not complete
   an unfinished feature or the overall development scope.

Required validation and release boundaries below remain gates. A dependency
delays its dependent action; it does not suspend the rest of the assignment.

## Local validation

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
cargo test --all-features
cargo test --no-default-features --features alloc
cargo test --no-default-features --features oscore
cargo test --no-default-features --features alloc,oscore
cargo test --no-default-features --features std
cargo test --features std --test std_fixed_storage
cargo doc --no-deps --all-features
cargo package -p coaptic --locked
cargo +1.85.0 check --locked -p coaptic --no-default-features
cargo +1.85.0 check --locked -p coaptic --all-features
```

## Companion validation

[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) holds
interoperability peers, hardware tooling, benchmarks, and qualification runners.
The library, examples, and unit/integration tests here are Rust.

CI calls the companion workflow at a pinned commit against the exact library
revision. Follow its [run instructions](https://github.com/jeffglousher/coaptic-validation#run)
for transport, security, or harness changes. Preserve both source revisions,
lockfiles, setup limitations, and reports. The process timings are smoke samples;
measurements remain preliminary. Updating the suite pin is a reviewed change.

## Release

Only `coaptic` is published. Harnesses and peers stay out of the crate.
`cargo package` includes only the library, Rust examples/tests, and supporting notices.
Release tags start the rustdoc and crates.io workflows. Ordinary development
does not create those tags.

For a release, first merge the reviewed change to `main` with required CI green
and complete the validation above. Inspect the packaged files and license
notices. Configure the repository's `CARGO_REGISTRY_TOKEN` secret with a
crates.io token authorized to publish `coaptic`; do not put credentials in git.
Preparation is followed by maintainer/external review and agreement on the
final release snapshot before publication. The publication tag must match the
manifest version exactly (`v0.0.10` for `0.0.10`). Pushing that tag publishes
the crate. Package preparation alone does not authorize that step.
After publication, replace the
README's Git dependency with the released version and point the manifest's
documentation URL to `https://docs.rs/coaptic`.
