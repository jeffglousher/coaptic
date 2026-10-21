use coaptic::provisioning::EnrollmentPolicy;
use coaptic_console_core::ApprovalMode;
use coaptic_enroll::{
    Action, Error, Request, device,
    pairing::{self, PairingFile},
};
#[cfg(all(windows, feature = "webauthn"))]
use coaptic_enroll::{origin, store::OperatorStore};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    time::Instant,
};

#[path = "peer_cli.rs"]
mod peer;

#[path = "shared_probe.rs"]
mod shared_probe;

pub fn run(
    command: &OsStr,
    mut args: impl Iterator<Item = OsString>,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = args.next().ok_or("missing device name")?;
    let device_directory = super::device_directory(directory, &name)?;
    let identity = device::restore(&device_directory)?;
    let store = PairingFile::open(&device_directory)?;
    if command == "pairing-init" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        store.initialize(&identity)?;
        pairing::initialize_receipts(&device_directory, &identity)?;
        println!("Pairing store initialized; no peer is authorized.");
        return Ok(());
    }
    let mut state = store.restore(&identity)?;
    if command == "receipts-init" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        pairing::initialize_receipts(&device_directory, &identity)?;
        println!(
            "New receipt journal initialized. Existing or missing-after-use journals must never be reset."
        );
        return Ok(());
    }
    if command == "pairing-info" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        let record = state.checkpoint();
        println!(
            "Device {} revision {} paired {}",
            name.to_string_lossy(),
            record.revision(),
            record.enabled()
        );
        if let Some(policy) = record.policy() {
            println!(
                "Peer {} owner {} permissions {}",
                super::hex(policy.peer().principal().fingerprint()),
                super::hex(policy.owner()),
                super::hex(policy.resource_policy())
            );
        }
        return Ok(());
    }
    if command == "peer" {
        let local = args.next().ok_or("missing local address")?;
        let remote = args.next().ok_or("missing peer address")?;
        let mode = args.next();
        if mode.as_deref() == Some(OsStr::new("--qualify-queued-revoke")) {
            let port = args.next().ok_or("missing qualification serial port")?;
            if args.next().as_deref() != Some(OsStr::new("--development-no-touch"))
                || args.next().is_some()
            {
                return Err(
                    "queued-CON qualification requires exactly PORT --development-no-touch".into(),
                );
            }
            return peer::qualify_queued_revoke(
                super::serial_cli::LockedHost {
                    directory: &device_directory,
                    identity: &identity,
                    store: &store,
                    pairing: &mut state,
                },
                directory,
                local.to_str().ok_or("invalid address")?,
                remote.to_str().ok_or("invalid address")?,
                port.to_str().ok_or("invalid serial port")?,
            );
        }
        if mode.as_deref() == Some(OsStr::new("--probe-shared")) {
            if args.next().is_some() {
                return Err("unexpected arguments".into());
            }
            return shared_probe::run(
                &identity,
                &state,
                local.to_str().ok_or("invalid address")?,
                remote.to_str().ok_or("invalid address")?,
            );
        }
        if mode.as_deref() == Some(OsStr::new("--qualify-pressure")) {
            if args.next().is_some() {
                return Err("unexpected arguments after --qualify-pressure".into());
            }
            return peer::qualify_pressure(
                &identity,
                &state,
                local.to_str().ok_or("invalid address")?,
                remote.to_str().ok_or("invalid address")?,
            );
        }
        let probe = mode.map(|value| peer::Probe::from_flag(&value).ok_or("expected --connect, --probe, --probe-echo, --probe-denied, --qualify-lost-receipt or --qualify-device-restart")).transpose()?;
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        // Keep the pairing store locked for this entire runtime. Management
        // refuses until this process and every session/application owner stop.
        return peer::run(
            &device_directory,
            &identity,
            &state,
            local.to_str().ok_or("invalid address")?,
            remote.to_str().ok_or("invalid address")?,
            probe,
        );
    }
    let peer = if command == "pair" || command == "transfer" {
        let peer_name = args.next().ok_or("missing peer device name")?;
        let peer_directory = super::device_directory(directory, &peer_name)?;
        Some(device::restore(&peer_directory)?.peer())
    } else {
        None
    };
    let next_owner = if command == "transfer" {
        Some(parse_owner(
            &args.next().ok_or("missing new owner commitment")?,
        )?)
    } else {
        None
    };
    let mut approval = ApprovalMode::WebAuthn;
    let mut approval_selected = false;
    let mut access = pairing::Access::default();
    let mut access_selected = false;
    for option in args {
        if let Some(mode) = option.to_str().and_then(ApprovalMode::from_flag) {
            if approval_selected {
                return Err("repeated approval mode".into());
            }
            approval = mode;
            approval_selected = true;
        } else if let Some(value) = option.to_str().and_then(pairing::Access::from_flag) {
            if access_selected || !(command == "pair" || command == "transfer") {
                return Err("unexpected or repeated access option".into());
            }
            access = value;
            access_selected = true;
        } else {
            return Err("unexpected or repeated pairing option".into());
        }
    }
    let started = Instant::now();
    let challenge = state.begin(&identity, 0, |out| getrandom::fill(out).is_ok())?;
    let action = if command == "pair" {
        Action::Pair
    } else if command == "transfer" {
        Action::Transfer
    } else {
        Action::Revoke
    };
    let existing = state.checkpoint().policy().cloned();
    let permissions = access.permissions();
    let make_request = |owner| {
        let policy = match &peer {
            Some(peer) => {
                EnrollmentPolicy::new(peer.clone(), next_owner.unwrap_or(owner), permissions)
                    .map_err(|_| Error::Input)?
            }
            None => existing.clone().ok_or(Error::Input)?,
        };
        if action == Action::Transfer {
            if existing.as_ref().is_none_or(|old| old.owner() != &owner) {
                return Err(Error::Input);
            }
            return Request::for_transfer(&challenge, owner, &policy);
        }
        if policy.owner() != &owner {
            return Err(Error::Input);
        }
        Request::for_pairing(&challenge, &policy, action)
    };
    let commit = |request: &Request| {
        pairing::apply(
            request,
            &challenge,
            &mut state,
            started
                .elapsed()
                .as_millis()
                .try_into()
                .map_err(|_| Error::Expired)?,
            |old, next| store.commit(old, next),
        )
    };
    approve(directory, approval, make_request, commit)?;
    println!(
        "Pairing committed at revision {} (enabled {}).",
        state.checkpoint().revision(),
        state.checkpoint().enabled()
    );
    Ok(())
}

/// Read a registered credential's commitment without signing or changing its state.
pub(super) fn approval_owner(directory: &Path, approval: ApprovalMode) -> Result<[u8; 32], Error> {
    if approval != ApprovalMode::WebAuthn {
        #[cfg(feature = "development-piv")]
        {
            use coaptic_enroll::{development_piv::DevelopmentOperator, store::RecordStore};
            let touch = approval == ApprovalMode::PivTouch;
            let store = RecordStore::<DevelopmentOperator>::open_named(
                directory,
                if touch {
                    "piv-touch"
                } else {
                    "piv-development"
                },
            )?;
            let operator = store.load()?;
            if operator.requires_touch() != touch {
                return Err(Error::Authentication);
            }
            return operator.owner();
        }
        #[cfg(not(feature = "development-piv"))]
        return Err(Error::Authentication);
    }
    #[cfg(all(windows, feature = "webauthn"))]
    {
        Ok(*OperatorStore::open(directory)?.load()?.owner())
    }
    #[cfg(not(all(windows, feature = "webauthn")))]
    {
        let _ = directory;
        Err(Error::Authentication)
    }
}

pub fn approve<T>(
    directory: &Path,
    approval: ApprovalMode,
    request: impl FnOnce([u8; 32]) -> Result<Request, Error>,
    commit: impl FnOnce(&Request) -> Result<T, Error>,
) -> Result<T, Box<dyn std::error::Error>> {
    #[cfg(not(any(feature = "development-piv", all(windows, feature = "webauthn"))))]
    let _ = (directory, &request, &commit);
    if approval != ApprovalMode::WebAuthn {
        #[cfg(feature = "development-piv")]
        {
            use coaptic_enroll::{development_piv::DevelopmentOperator, store::RecordStore};
            let touch = approval == ApprovalMode::PivTouch;
            let store = RecordStore::<DevelopmentOperator>::open_named(
                directory,
                if touch {
                    "piv-touch"
                } else {
                    "piv-development"
                },
            )?;
            let operator = store.load()?;
            if operator.requires_touch() != touch {
                return Err(Error::Authentication.into());
            }
            let request = request(operator.owner()?)?;
            review(&request, approval)?;
            let pending = operator.begin(request)?;
            if touch {
                println!("Touch the selected YubiKey to approve this transaction.");
            }
            let proof = operator.sign(pending.digest())?;
            let approval =
                pending.finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))?;
            return Ok(approval.commit(commit)?);
        }
        #[cfg(not(feature = "development-piv"))]
        return Err("binary was built without the development-piv feature".into());
    }
    #[cfg(not(all(windows, feature = "webauthn")))]
    {
        Err(
            "native WebAuthn approval is unavailable; select an explicitly registered PIV profile"
                .into(),
        )
    }
    #[cfg(all(windows, feature = "webauthn"))]
    {
        let store = OperatorStore::open(directory)?;
        let operator = store.load()?;
        let request = request(*operator.owner())?;
        review(&request, approval)?;
        let (challenge, pending) = operator.begin(request)?;
        use webauthn_authenticator_rs::{WebauthnAuthenticator, win10::Win10};
        let proof =
            WebauthnAuthenticator::new(Win10::default()).do_authentication(origin(), challenge)?;
        let approval =
            pending.finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))?;
        Ok(approval.commit_local(commit)?)
    }
}

#[cfg(any(feature = "development-piv", all(windows, feature = "webauthn")))]
fn review(request: &Request, approval: ApprovalMode) -> Result<(), Error> {
    println!(
        "Approve {:?}: device {} peer {} owner {} permissions {}",
        request.action,
        super::hex(request.device.peer()?.principal().fingerprint()),
        super::hex(request.peer.peer()?.principal().fingerprint()),
        super::hex(&request.owner),
        super::hex(&request.permissions)
    );
    if request.action == Action::Transfer {
        println!(
            "Transfer to owner {}. Verify this owner and the destination peer before approval.",
            super::hex(&request.next_owner)
        );
    }
    if matches!(
        request.action,
        Action::ConfigureRecovery
            | Action::ConfirmRecoveryCredential
            | Action::RecoverOwner
            | Action::AcceptRecoveredOwnership
    ) {
        println!(
            "Recovery co-signer {} (zero removes backup authorization).",
            super::hex(&request.next_owner)
        );
    }
    if let Some(metadata) = &request.reconciliation {
        let receipt = metadata.receipt()?;
        println!(
            "Receipt-only repair: prior checkpoint {}, operation {}, content {}, committed sequence {}. Access remains revoked.",
            super::hex(metadata.prior_commitment()),
            super::hex(receipt.id().as_bytes()),
            super::hex(receipt.digest()),
            receipt.sequence()
        );
    }
    println!(
        "Transaction {}{}",
        super::hex(&request.commitment()?),
        approval.label()
    );
    Ok(())
}

pub fn parse_owner(value: &OsStr) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let value = value.to_str().ok_or("invalid new owner commitment")?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "new owner must be an independently verified 32-byte hexadecimal commitment".into(),
        );
    }
    let mut owner = [0; 32];
    for (index, byte) in owner.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    if owner == [0; 32] {
        return Err("new owner must be nonzero".into());
    }
    Ok(owner)
}
