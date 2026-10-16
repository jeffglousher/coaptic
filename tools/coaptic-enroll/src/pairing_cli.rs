use coaptic::provisioning::EnrollmentPolicy;
use coaptic_enroll::{
    Action, Error, Request, device, origin,
    pairing::{self, PairingFile},
    store::OperatorStore,
};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    time::Instant,
};

#[path = "peer_cli.rs"]
mod peer;

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
        println!("Pairing store initialized; no peer is authorized.");
        return Ok(());
    }
    let mut state = store.restore(&identity)?;
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
        let probe = match args.next() {
            None => false,
            Some(value) if value == "--probe" => true,
            _ => return Err("expected --probe".into()),
        };
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        // Keep the pairing store locked for this entire runtime. Management
        // refuses until this process and every session/application owner stop.
        return peer::run(
            &identity,
            &state,
            local.to_str().ok_or("invalid address")?,
            remote.to_str().ok_or("invalid address")?,
            probe,
        );
    }
    let peer = if command == "pair" {
        let peer_name = args.next().ok_or("missing peer device name")?;
        let peer_directory = super::device_directory(directory, &peer_name)?;
        Some(device::restore(&peer_directory)?.peer())
    } else {
        None
    };
    let mut development = false;
    let mut write = true;
    for option in args {
        if option == "--development-no-touch" && !development {
            development = true;
        } else if option == "--read-only" && write && command == "pair" {
            write = false;
        } else {
            return Err("unexpected or repeated pairing option".into());
        }
    }
    let started = Instant::now();
    let challenge = state.begin(&identity, 0, |out| getrandom::fill(out).is_ok())?;
    let action = if command == "pair" {
        Action::Pair
    } else {
        Action::Revoke
    };
    let existing = state.checkpoint().policy().cloned();
    let permissions = pairing::permissions(write);
    let make_request = |owner| {
        let policy = match &peer {
            Some(peer) => {
                EnrollmentPolicy::new(peer.clone(), owner, permissions).map_err(|_| Error::Input)?
            }
            None => existing.clone().ok_or(Error::Input)?,
        };
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
    approve(directory, development, make_request, commit)?;
    println!(
        "Pairing committed at revision {} (enabled {}).",
        state.checkpoint().revision(),
        state.checkpoint().enabled()
    );
    Ok(())
}

pub fn approve<T>(
    directory: &Path,
    development: bool,
    request: impl FnOnce([u8; 32]) -> Result<Request, Error>,
    commit: impl FnOnce(&Request) -> Result<T, Error>,
) -> Result<T, Box<dyn std::error::Error>> {
    if development {
        #[cfg(feature = "development-piv")]
        {
            use coaptic_enroll::{
                development_piv::{self, DevelopmentOperator},
                store::RecordStore,
            };
            let store =
                RecordStore::<DevelopmentOperator>::open_named(directory, "piv-development")?;
            let operator = store.load()?;
            let request = request(operator.owner()?)?;
            review(&request, true)?;
            let pending = operator.begin(request)?;
            let proof =
                development_piv::sign(operator.serial(), operator.slot(), pending.digest())?;
            let approval =
                pending.finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))?;
            return Ok(approval.commit(commit)?);
        }
        #[cfg(not(feature = "development-piv"))]
        return Err("binary was built without the development-piv feature".into());
    }
    let store = OperatorStore::open(directory)?;
    let operator = store.load()?;
    let request = request(*operator.owner())?;
    review(&request, false)?;
    let (challenge, pending) = operator.begin(request)?;
    use webauthn_authenticator_rs::{WebauthnAuthenticator, win10::Win10};
    let proof =
        WebauthnAuthenticator::new(Win10::default()).do_authentication(origin(), challenge)?;
    let approval = pending.finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))?;
    Ok(approval.commit_local(commit)?)
}

fn review(request: &Request, development: bool) -> Result<(), Error> {
    println!(
        "Approve {:?}: device {} peer {} owner {} permissions {}",
        request.action,
        super::hex(request.device.peer()?.principal().fingerprint()),
        super::hex(request.peer.peer()?.principal().fingerprint()),
        super::hex(&request.owner),
        super::hex(&request.permissions)
    );
    println!(
        "Transaction {}{}",
        super::hex(&request.commitment()?),
        if development {
            " (development, no touch)"
        } else {
            ""
        }
    );
    Ok(())
}
