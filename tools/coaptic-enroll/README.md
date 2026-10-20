# Coaptic commissioning

Run `coaptic-enroll` without arguments to open the guide. It supports inspection,
identity setup, pairing, ownership transfer, recovery and protected peer operation.
The menu groups inspection, setup, pairing/connections, credentials and maintenance.
Enter `:back` to return to the previous menu or `:cancel` to exit before execution.
Unavailable credential providers are hidden. The guide returns to its
menu after completion or failure; it never automatically retries a mutation.

Build with `cargo build --manifest-path tools/coaptic-enroll/Cargo.toml`.
Direct commands remain available through `--help` for automation.
Use `inspect --help` or `peer --help` for selection and connection options.
Help works before credential storage or a device is available.

`inspect` shows local credential-record presence, host names and serial ports
without contacting devices or asking for a key touch. Add `--host NAME` for
verified identity and pairing details, or `--port PORT` for live device state,
including authenticated pending-work custody. Missing or unverifiable pending
history never authorizes creating an empty replacement.
Only the selected port is contacted, with a ten-second response limit. A failed
target does not suppress the other results; the command exits unsuccessfully
if a selected target cannot be inspected. Inventory is not a live credential test.

The default cross-platform PIV build uses RustCrypto for signatures, hashes,
attestation verification and sealed records, with no OpenSSL dependency. It
still uses native PC/SC and OS credential storage. This tool has no TLS transport;
rustls is not needed for its USB and EDHOC/OSCORE communication.
Linux Secret Service uses the Rust zbus client rather than libdbus. BLAKE3 keeps
the official Rust crate's normal optimized backend selection, including supported
assembly/C implementations. Rust ownership does not require disabling maintained
platform acceleration. The core still disables BLAKE3's `std` feature.
Optional `--features webauthn` enables Windows WebAuthn and retains OpenSSL
through the upstream WebAuthn verifier. `--all-features` also enables its
software-authenticator test fixtures. Existing credentials and encrypted records
remain readable; cryptographic provider selection does not change their identity.

For first setup:

1. Prepare native credential storage, then initialize a named host identity.
2. Inspect the device over its serial port. Initialize its identity only if it has
   never been provisioned. Missing or corrupt existing state requires recovery.
3. Register an existing P-256 PIV credential using its serial number, slot and
   public PEM. Registration proves possession; it does not generate or replace keys.
4. Pair the device with the host address and chosen permissions: read-only, echo
   only, or echo plus durable telemetry. Direct commands default to telemetry;
   `--read-only` and `--echo-only` select the other policies. Review the exact
   transaction, then approve with the registered credential.
5. Connect the protected peer using the host and device addresses. Restart restores
   persistent identity and authorization and establishes a fresh session.

`--piv-touch` selects a credential whose hardware policy requires a touch for every
signature. `--development-no-touch` explicitly selects a touchless development
credential. These profiles are stored separately. Both check the live public key
and policy before signing; a policy mismatch is refused. Both currently require
PIN policy Never. PIV touch presence is not WebAuthn user verification.

`piv-touch-test` and `development-piv-test` verify the registered credential with a
fresh signature without changing pairing or operator records. A different PIV
public key represents a different owner: switching approval profiles does not
transfer existing ownership. Use the ownership-transfer operation deliberately.

Windows uses DPAPI for local custody and optionally supports Windows WebAuthn approval.
Linux uses Secret Service; macOS uses Keychain. Linux needs PC/SC and a running,
unlocked Secret Service. macOS and Linux use the PIV approval paths. Unavailable
credential storage fails closed without an unencrypted-file fallback. Keep the
OS credential store together with enrollment records for recovery.

Supported desktop targets are Windows, Linux and macOS. On Debian/Ubuntu, building
the default PIV tool requires `libpcsclite-dev` and `pkg-config`; running it needs
PC/SC (`pcscd`) and an unlocked Secret Service such as GNOME Keyring. Windows and
macOS use their system smart-card APIs. Installing a Rust crate does not replace
these device drivers or credential services. Optional WebAuthn's vendored OpenSSL
also needs its native build prerequisites. The workflow tests all three OSes,
including native Linux/macOS custody and async UDP loopback. USB drivers, token
availability and desktop credential-service policy remain deployment prerequisites.

The communication library remains allocation-free with `no_std` support. Shared
input and operation definitions live in `coaptic-console-core` and also use neither
`std` nor allocation. The desktop transport, custody and approval adapters use both.

If a serial operation times out, inspect device and host state before retrying.
Recovery reconciles a committed device update whose reply was lost. Reinitializing
an existing identity is not a recovery operation.

Before changing a telemetry grant, keep its authorized host serving receipts and
run `serial-quiesce PORT`. The device retries the existing operation for at most
30 seconds and refuses if it cannot finish. Success holds new production for
60 seconds; an administrative challenge extends the hold through approval.
Stop the serving host after successful quiescence, then change pairing. This
releases its store locks without racing the next effect. Revocation remains
immediate and preserves unresolved work; restoring authority does not erase it.
Use `serial-quiesce --help` for the precise boundaries.

If revocation left an operation whose effect was already committed at the host,
use `serial-reconcile PORT HOST PRIOR_PAIRING.cbor APPROVAL` to reconcile its
exact receipt while access remains revoked. `serial-revoke` tries to retain the
prior public device checkpoint in the host directory before device mutation. If
storage fails, it warns and prints the exact public CBOR without delaying
revocation or replacing existing archives. Keep that checkpoint and the original
receipt journal.
The guide exposes the same operation under Maintenance. Current-owner approval
binds the live challenge, old grant and complete receipt. The device authenticates
retained work under that old grant; the host only looks up the matching durable
row. Missing/conflicting history refuses. No new effect or grant is created; an
uncertain reply can be inspected and retried with the same checkpoint and journal.

For an explicit lost-receipt qualification, `peer ... --qualify-lost-receipt`
requires an approved telemetry grant and exits with code 75 after synchronizing
the first new durable effect, before sending its receipt. Preserve the same
journal and restart normally to recover that effect's receipt. This mode is
never selected by ordinary peer operation.

If networking stops after a fatal session error while USB inspection still works,
use the guide's Restart device operation. It retains identity and pairing; it does
not recreate keys or authorize a different owner.
