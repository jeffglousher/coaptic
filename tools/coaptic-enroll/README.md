# Coaptic commissioning

Run `coaptic-enroll` without arguments to open the guide. It supports inspection,
identity setup, pairing, ownership transfer, recovery and protected peer operation.
Enter `:cancel` to exit before executing an operation. The guide returns to its
menu after completion or failure; it never automatically retries a mutation.

Build with `cargo build --manifest-path tools/coaptic-enroll/Cargo.toml --features development-piv`.
Direct commands remain available through `--help` for automation.

For first setup:

1. Prepare native credential storage, then initialize a named host identity.
2. Inspect the device over its serial port. Initialize its identity only if it has
   never been provisioned. Missing or corrupt existing state requires recovery.
3. Register an existing P-256 PIV credential using its serial number, slot and
   public PEM. Registration proves possession; it does not generate or replace keys.
4. Pair the device with the host address and chosen permissions. Review the exact
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

Windows uses DPAPI for local custody and also supports Windows WebAuthn approval.
Linux uses Secret Service; macOS uses Keychain. Linux needs PC/SC and a running,
unlocked Secret Service. macOS and Linux use the PIV approval paths. Unavailable
credential storage fails closed without an unencrypted-file fallback. Keep the
OS credential store together with enrollment records for recovery.

The communication library remains allocation-free with `no_std` support. Shared
input and operation definitions live in `coaptic-console-core` and also use neither
`std` nor allocation. The desktop transport, custody and approval adapters use both.

If a serial operation times out, inspect device and host state before retrying.
Recovery reconciles a committed device update whose reply was lost. Reinitializing
an existing identity is not a recovery operation.

If networking stops after a fatal session error while USB inspection still works,
use the guide's Restart device operation. It retains identity and pairing; it does
not recreate keys or authorize a different owner.
