//! Two-credential recovery on the existing privileged local commissioning channel.
use super::{
    pairing_cli::{approval_owner, approve},
    serial_cli::{Usb, commissioning_error},
};
use coaptic::{
    Endpoint,
    provisioning::{CommissioningReply, CommissioningRequest, EnrollmentPolicy, PairingUpdate},
};
use coaptic_console_core::ApprovalMode;
use coaptic_enroll::{
    Error, Request, device,
    pairing::{self, PairingFile},
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn mode(value: &OsStr) -> Result<ApprovalMode> {
    match value.to_str() {
        Some("webauthn") => Ok(ApprovalMode::WebAuthn),
        Some("touch") => Ok(ApprovalMode::PivTouch),
        Some("touchless") => Ok(ApprovalMode::PivTouchless),
        _ => Err("co-signer mode must be webauthn, touch or touchless".into()),
    }
}

pub(super) fn run(
    command: &OsStr,
    mut args: impl Iterator<Item = OsString>,
    directory: &Path,
) -> Result<()> {
    let port = args.next().ok_or("missing serial port")?;
    let recovering = command == "serial-owner-recover";
    let removing = command == "serial-backup-remove";
    let destination = if recovering {
        let host = args.next().ok_or("missing new host identity name")?;
        let address: std::net::SocketAddr = args
            .next()
            .ok_or("missing host IPv4 address:port")?
            .to_str()
            .ok_or("invalid address")?
            .parse()?;
        if !address.is_ipv4()
            || address.ip().is_unspecified()
            || address.ip().is_multicast()
            || address.port() == 0
        {
            return Err("host endpoint must be unicast IPv4 with a nonzero port".into());
        }
        Some((host, Endpoint::from(address)))
    } else {
        None
    };
    let cosigner = if removing {
        None
    } else {
        let path = PathBuf::from(
            args.next()
                .ok_or("missing co-signer credential directory")?,
        );
        let mode = mode(&args.next().ok_or("missing co-signer mode")?)?;
        Some((path, mode))
    };
    let mut approval = ApprovalMode::WebAuthn;
    let mut selected = false;
    let mut access = pairing::Access::default();
    let mut access_selected = false;
    for option in args {
        if let Some(value) = option.to_str().and_then(ApprovalMode::from_flag) {
            if selected {
                return Err("repeated approval mode".into());
            }
            approval = value;
            selected = true;
        } else if let Some(value) = option.to_str().and_then(pairing::Access::from_flag) {
            if access_selected || !recovering {
                return Err("unexpected or repeated access option".into());
            }
            access = value;
            access_selected = true;
        } else {
            return Err("unexpected recovery option".into());
        }
    }
    let other_owner = cosigner
        .as_ref()
        .map(|(path, mode)| approval_owner(path, *mode))
        .transpose()?;
    let host = destination
        .as_ref()
        .map(|(name, _)| {
            super::existing_device_directory(directory, name)
                .and_then(|path| Ok((device::restore(&path)?, PairingFile::open(&path)?)))
        })
        .transpose()?;
    let mut host_state = host
        .as_ref()
        .map(|(identity, store)| store.restore(identity))
        .transpose()?;
    let _receipts = match (&host,&destination) {
        (Some((identity,_)),Some((name,_))) if access == pairing::Access::Telemetry => Some(pairing::preflight_receipts(&super::existing_device_directory(directory,name)?,identity).map_err(|_|"destination receipt journal is missing or invalid; inspect it or explicitly initialize its first journal before recovery")?),
        _ => None,
    };
    if host_state
        .as_ref()
        .is_some_and(|state| state.checkpoint().policy().is_some())
    {
        return Err(
            "owner recovery requires an explicitly initialized unpaired destination host".into(),
        );
    }
    let started = Instant::now();
    let local_challenge = match (&host, &mut host_state) {
        (Some((identity, _)), Some(state)) => {
            Some(state.begin(identity, 0, |bytes| getrandom::fill(bytes).is_ok())?)
        }
        _ => None,
    };
    let mut usb = Usb::open(port.to_str().ok_or("invalid serial port")?)?;
    let CommissioningReply::State {
        device: device_pin,
        record,
        ..
    } = usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("device has no existing identity to recover".into());
    };
    let policy = record.policy().ok_or("device has no existing owner")?;
    let CommissioningReply::Challenge(challenge) = usb.request(&CommissioningRequest::Challenge)?
    else {
        return Err("unexpected challenge response".into());
    };
    if challenge.device().principal() != device_pin.principal()
        || challenge.expected() != (*record.store(), record.revision(), record.commitment())
    {
        return Err("device identity or policy changed during recovery".into());
    }
    let outcome = approve(
        directory,
        approval,
        |owner| {
            if recovering {
                if record.recovery_owner() != Some(&owner) {
                    return Err(Error::Authentication);
                }
                let next = EnrollmentPolicy::new(
                    host.as_ref().ok_or(Error::Input)?.0.peer(),
                    other_owner.ok_or(Error::Input)?,
                    access.permissions(),
                )
                .map_err(|_| Error::Input)?;
                Request::for_owner_recovery(&challenge, owner, &next)
            } else {
                if policy.owner() != &owner {
                    return Err(Error::Authentication);
                }
                Request::for_recovery_configuration(&challenge, policy, other_owner)
            }
        },
        |request| {
            let mut commit = || -> std::result::Result<(), Error> {
                request.matches_pairing(&challenge)?;
                let next_policy = request.pairing_policy()?;
                let update = if recovering {
                    PairingUpdate::recover(challenge.clone(), request.owner, next_policy.clone())
                } else {
                    PairingUpdate::set_recovery(challenge.clone(), next_policy.clone(), other_owner)
                }
                .map_err(|_| Error::Input)?;
                let endpoint = destination
                    .as_ref()
                    .map_or(Endpoint::v4([127, 0, 0, 1], 1), |(_, endpoint)| *endpoint);
                let prepared = if let (Some(state), Some(local_challenge)) =
                    (&mut host_state, &local_challenge)
                {
                    let reciprocal = EnrollmentPolicy::new(
                        device_pin.clone(),
                        *next_policy.owner(),
                        *next_policy.resource_policy(),
                    )
                    .map_err(|_| Error::Input)?;
                    let now = started
                        .elapsed()
                        .as_millis()
                        .try_into()
                        .map_err(|_| Error::Expired)?;
                    Some(coaptic_enroll::recovery::PreparedHostPairing::prepare(
                        state,
                        local_challenge,
                        reciprocal,
                        now,
                    )?)
                } else {
                    None
                };
                let CommissioningReply::Committed(next) = usb
                    .request(&CommissioningRequest::Update { update, endpoint })
                    .map_err(commissioning_error)?
                else {
                    return Err(Error::Persistence);
                };
                let next_grant = next.policy().ok_or(Error::Persistence)?;
                if next.store() != record.store()
                    || next.local_principal() != record.local_principal()
                    || record.revision().checked_add(1) != Some(next.revision())
                    || next_grant.owner() != next_policy.owner()
                    || next_grant.peer().principal() != next_policy.peer().principal()
                    || next_grant.resource_policy() != next_policy.resource_policy()
                    || next.enabled() != if recovering { true } else { record.enabled() }
                    || next.recovery_owner()
                        != if recovering {
                            None
                        } else {
                            other_owner.as_ref()
                        }
                {
                    return Err(Error::Persistence);
                }
                if let (Some((_, store)), Some(prepared)) = (&host, prepared) {
                    prepared.commit(|old, next| store.commit(old, next))?;
                }
                Ok(())
            };
            if let Some((path, mode)) = &cosigner {
                let proof_request = request.recovery_cosignature()?;
                let expected = proof_request.commitment()?;
                approve(
                    path,
                    *mode,
                    |owner| {
                        if owner != proof_request.owner {
                            return Err(Error::Authentication);
                        }
                        Ok(proof_request)
                    },
                    |proof| {
                        if proof.commitment()? != expected {
                            return Err(Error::Authentication);
                        }
                        commit()
                    },
                )
                .map_err(commissioning_error)
            } else {
                commit()
            }
        },
    );
    if let Err(error) = outcome {
        if recovering {
            eprintln!(
                "If the device committed but the host did not, inspect it and run serial-recover PORT HOST --approval-directory NEW_OWNER_DIRECTORY with the new owner's approval mode. This repairs only the exact current device owner and destination host; do not repeat the ownership update."
            );
        }
        return Err(error);
    }
    println!(
        "{}",
        if recovering {
            "Ownership recovered; prior recovery authorization cleared. Device identity preserved."
        } else if removing {
            "Recovery credential removed."
        } else {
            "Recovery credential authorized after both credentials proved possession."
        }
    );
    Ok(())
}
