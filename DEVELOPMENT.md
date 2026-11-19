# Development

Use the pinned Rust toolchain. The library uses Rust 2024 with MSRV 1.89.
Some test peers need a newer compiler.

[AGENTS.md](AGENTS.md) contains project policy.
[PLAN.md](PLAN.md) identifies the current work.
[coaptic-validation](https://github.com/jeffglousher/coaptic-validation) owns
independent peers, benchmarks and device tooling; this repository owns the
library, examples and core tests.

## Check a change

### Rust design guidance

Use [Google's Rust Style Guide](https://google.github.io/styleguide/rustguide.html)
as supplementary review guidance, checked 2026-11-04. Our API contracts, bounded
memory requirements, runtime policy and MSRV take precedence.

Prefer clear ownership and borrowing; justify deep copies and shared ownership
where they simplify a real requirement. Accept borrowed data without unnecessarily
requiring a particular container. Use meaningful types, standard traits and
straightforward functions before introducing abstraction or code generation.
Document public behavior, failures, panic conditions and unsafe invariants.
Keep unsafe operations narrowly encapsulated with a safety argument and a reason
safe code is insufficient; performance justifications need measurements.

Apply this during changed-code review, not through repository-wide style churn.
Keep rustfmt/Clippy as our mechanical checks. Chromium/Fuchsia-specific rules,
including Chromium's async prohibition and source-layout conventions, do not
apply here. Do not introduce allocation to satisfy a host-oriented example,
blindly mark protocol enums non-exhaustive, or remove existing generation without
considering its role in keeping sync/async semantics aligned. Choose generics
versus dynamic dispatch with code size and measured work in mind.

### Power of Ten applied to Rust

Use Holzmann's NASA/JPL [Power of Ten](https://spinroot.com/gerard/pdf/Power_of_Ten.pdf)
as a reliability baseline. The original targets C; the following is our Rust
adaptation, not a NASA certification or a claim of literal compliance:

1. Keep protocol control flow explicit; avoid direct or indirect recursion on
   packet-processing and bounded-storage paths.
2. Bound each processing pass by capacities, input length or an explicit work
   budget. Long-lived service loops must yield/wait and support termination;
   retry limits and deadlines must be explicit.
3. Allocate constrained pools at startup and reuse them. Account separately for
   host/runtime allocations; do not imply the entire system is allocation-free.
4. Keep functions focused. Treat roughly 60 lines as a review prompt, not a reason
   to obscure an invariant through arbitrary splitting.
5. Check meaningful invariants at boundaries. Use compile-time checks where
   possible; reject bad packets with errors, not panics. Do not pad assertion
   counts or rely on debug-only checks for required validation.
6. Minimize state scope, lifetime and mutable access.
7. Validate inputs and handle failure results. Explain intentional error disposal,
   especially during cleanup; never convert uncertain effects into success.
8. Keep macros and generated code narrow, inspectable and tested.
9. Prefer safe references and ownership. Isolate raw-pointer/FFI operations;
   prove lifetime, aliasing and cancellation requirements.
10. Keep compiler/Clippy warnings clean in the selected feature configurations;
    use targeted tests, fuzzing and analysis for changed risk boundaries.

These rules constrain implementation choices; speed claims still require matched
measurements. Preserve failure recovery and security even when an alternative is
faster. Document a scoped exception and its reasoning rather than silently
weakening a bound or importing a C restriction that has no Rust equivalent.

### Verification commands

Choose checks for the code you changed. Run them after a coherent batch:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Use the relevant test filter while developing. Check without defaults when
feature gates or shared modules change:

```sh
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
```

Allocation and security changes also need their affected `alloc`, `oscore`,
`alloc,oscore` and `std` combinations. Transport, persistence and hardware
changes need relevant loopback or device proof. Keep both repository revisions,
features, capacities and runner identities with the result.

For rustdoc changes, build the docs and run relevant doctests. For Markdown-only
changes, review claims, examples and links; do not rerun protocol campaigns.

Pull requests run the advisory **Quick development checks** job. Use the
workflow's `full_validation` input for a requested full run. Enrollment-provider
checks are manual. GitHub's default CodeQL setup runs separately.

## Record useful evidence

Apply [runtime and comparison policy](architecture/ASYNC-RUNTIMES.md) when changing
adapters or designing peer comparisons. Equivalent-capability and minimal-workload
comparisons answer different questions; retain them as separate result groups.

Name the workload, security mode, storage and capacities. Preserve raw failures
and unsupported cases. Identify the source that actually ran; a newer CI pin
does not update an old measurement.

Separate object layout, heap requests, reserved stacks and process memory.
Nested objects are already included in their owners. Do not turn finite samples
into minimum-RAM, peak-memory or maximum-throughput claims.
See the [qualification checklist](architecture/requirements/VALIDATION-PLAN.md).

For routine performance work, batch coherent changes and use a short matched
baseline/candidate check of the integrated App. Reuse unchanged passing checks;
repeat or broaden only to answer a specific unresolved question. Preserve failed
comparisons, but keep development checks separate from release qualification.

Performance comparisons must include actual synchronous and asynchronous App
execution with matched capacities and security settings. Label runtime policy
and unsupported combinations explicitly; a sync measurement is not async
evidence. Apply the same mode coverage to device-to-host comparisons. Keep
steady-state results primary, with memory and lifecycle costs separately named.

## Finish a batch or hand off

### Continue across batches

Project continuity is part of delivery. Work toward the full authorized objective
across multiple coherent batches in one sustained run. After implementation,
integration, relevant validation and authorized delivery, reconcile what remains
and start the next unblocked task. A passing check, merged PR, rejected candidate,
clean tree or completed evidence report does not by itself end the run. On receipt
of a handoff, verify current state and custody, then execute the first unblocked
task; acknowledgment, advisory CI inspection and a next-batch recommendation alone
are not substantive continuation.

At meaningful batch boundaries, update `PLAN.md`: date and source identities,
current position, active batch (or none), one recommended next batch with its
completion criteria, and blocked work with the exact prerequisite. Order work
by reliability, performance, then ease of use. Keep this page short; move detailed
results and historical work orders into linked evidence or history documents.
Keep actionable acceptance criteria in issues and API contracts in rustdoc.

Treat this update as a checkpoint and continue. Use commentary for progress while
working. Keep independent work moving during waits. When an experiment fails,
retain its evidence and select a different justified approach or remaining task;
avoid repeated unchanged campaigns and diagnostics without a decision they enable.
There is no arbitrary time or batch quota: maximize useful progress without
expanding scope, permissions, concurrency or resource budgets.

### Decide whether to stop or transfer

End the run when the user's objective is verified complete, the user requests
stop/pause/handover, or further authorized progress is prevented by a concrete
blocker or execution limit. Verify that independent remaining work cannot proceed
before reporting the whole objective blocked. Name the exact prerequisite and
keep partial completion distinct from goal completion.

Prefer checkpoints within the existing run. An autonomous handover normally
follows substantial productive work at a validated boundary and needs a concrete
continuity reason: for example, observed context loss not repaired by a compact
checkpoint or an approaching enforced execution limit. A single finished batch,
routine compaction, pending advisory checks or a preference for a fresh thread is
insufficient. An explicit request or unavoidable interruption may require an
earlier handover with incomplete validation recorded honestly.

### Prepare and transfer custody

Verify the checkout, uncommitted/unpushed work, PR state and remaining jobs.
Identify who owns ongoing work; avoid two threads mutating the same checkout.
Record source-matched proof, rejected approaches and any limitations that change
the next decision. Reuse valid evidence rather than rerunning settled work.
During authorized batch work, close stale bugs that no longer reproduce on the
declared current versions, retaining evidence and historical failures; a recurrence
can open a fresh issue. Handover preparation alone does not require reproduction
or issue changes. Unexecuted qualification remains open until run or explicitly
descoped.

Carry the full objective and completion criteria forward, with accepted batch
statuses, the remaining requirements in priority order and the first executable
action. Preserve dirty work, evidence and unresolved failures; do not force a
merge, clean tree or passing result to manufacture a handover boundary. Validate
the latest coherent change where possible, recording exact source and limits.
Confirm normal firmware/runtime restoration where required. Each live job and
mutable resource must have a named continuing owner or be safely stopped and
verified within existing authority; retain generated inputs still in use.

A prepared prompt is not proof of a running replacement. Use only an available,
authorized continuation mechanism. Freeze sender mutations and record final
state; the receiver verifies that state and accepts conditional custody; the
sender confirms release; only then may the receiver mutate. If no receiver is
confirmed, label continuation
pending and preserve a recoverable checkpoint with safe runtime/job state.
Never claim guaranteed resumption or leave live effects unowned. Honor an explicit
stop without waiting indefinitely for a receiver. If safe stop is unauthorized
or fails and no receiver exists, promptly name the live effect and exact operator
takeover needed; do not invent authority or report fictitious continuing ownership.

For a requested status report, actual stop or handover, use these compact labels;
they do not require ending execution at every batch boundary:

- **Delivered:** behavior changed and merged PRs, or explicit delivery status.
- **Verified:** source identities, relevant checks and what they establish.
- **Remaining:** unfinished work, failures and limits; distinguish batch status
  from the broader project goal.
- **Next:** the next executable action and batch, why it comes next and its
  completion criteria; continue it now unless a stated stop/transfer reason applies.
- **Handoff:** checkout/branch/revision, dirty work, active jobs/ownership,
  evidence locations, stop/transfer reason, receiver acceptance or pending state,
  and any exact human action needed.

When a fresh thread is requested, give it a self-contained assignment based on
this updated state. Do not treat a stopped run or merged PR as completion of an
unfinished objective. Private host paths and session state belong in the private
handoff, not the public project plan. This workflow adds no approval gate and
does not expand publication, hardware or cleanup authority.

## Prepare an authorized release

Publication is separate work under
[#132](https://github.com/jeffglousher/coaptic/issues/132).
Select the exact candidate before running the complete matrix:

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
cargo package --workspace --locked
cargo +1.89.0 check --locked -p coaptic --no-default-features
cargo +1.89.0 check --locked -p coaptic --all-features
```

Inspect packaged files, licenses and dependency provenance, and complete the
agreed review. Package verification does not authorize publishing.

The `coaptic-io` macro crate generates async counterparts at compile time and
must be published before `coaptic`. Release tags trigger documentation and
crates.io publication; the tag must match the selected manifest version.
Use the authorized `CARGO_REGISTRY_TOKEN` secret. Never commit it.
After publication, verify installation and update dependency/documentation links.
