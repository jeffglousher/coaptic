//! Receipt-only repair under an unchanged revoked device policy.
use super::{
    pairing_cli::approve,
    serial_cli::{Usb, commissioning_error},
};
use coaptic::provisioning::{CommissioningPending, CommissioningReply, CommissioningRequest};
use coaptic_console_core::ApprovalMode;
use coaptic_enroll::{
    Error, Request, device,
    pairing::{self, PairingFile},
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn run(
    mut args: impl Iterator<Item = OsString>,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let port = args.next().ok_or("missing serial port")?;
    let name = args.next().ok_or("missing existing host identity name")?;
    let prior_path = PathBuf::from(
        args.next()
            .ok_or("missing prior public device checkpoint file")?,
    );
    let mut approval = ApprovalMode::WebAuthn;
    let mut selected = false;
    let mut approval_directory = directory.to_path_buf();
    let mut directory_selected = false;
    while let Some(option) = args.next() {
        if let Some(mode) = option.to_str().and_then(ApprovalMode::from_flag) {
            if selected {
                return Err("repeated approval mode".into());
            }
            approval = mode;
            selected = true;
        } else if option == "--approval-directory" && !directory_selected {
            approval_directory = args.next().ok_or("missing approval directory")?.into();
            directory_selected = true;
        } else {
            return Err("unexpected or repeated reconciliation option".into());
        }
    }
    let prior = pairing::read_device_checkpoint(&prior_path)?;
    let host_directory = super::existing_device_directory(directory, &name)?;
    let host = device::restore(&host_directory)?;
    // Both locks remain held through approval, the device response and readback.
    let store = PairingFile::open(&host_directory)?;
    let local = store.restore(&host)?;
    let journal = pairing::preflight_receipts(&host_directory, &host).map_err(|_| {
        "existing receipt journal is missing, corrupt, ambiguous or in use; preserve it and recover its exact history"
    })?;
    let mut usb = Usb::open(port.to_str().ok_or("invalid serial port")?)?;
    let CommissioningReply::State { device, record, .. } =
        usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("device has no retained identity and policy to reconcile".into());
    };
    record.validate_reconciliation_prior(&prior)?;
    let policy = record.policy().ok_or("device has no retained owner")?;
    let local_policy = local
        .checkpoint()
        .policy()
        .ok_or("host has no retained pairing")?;
    if record.local_principal() != device.principal().fingerprint()
        || policy.peer().principal() != host.peer().principal()
        || policy.resource_policy() != &pairing::permissions(true)
        || local_policy.peer().principal() != device.principal()
        || local_policy.owner() != policy.owner()
        || local_policy.resource_policy() != policy.resource_policy()
    {
        return Err(
            "device, original service, owner or telemetry policy does not match this host".into(),
        );
    }
    let CommissioningReply::Pending {
        id,
        resource,
        digest,
        receipt: completed,
    } = usb.request(&CommissioningRequest::Pending {
        prior: prior.clone(),
    })?
    else {
        return Err("device did not authenticate retained operation metadata".into());
    };
    let receipt = journal
        .lookup_receipt(&device.principal(), id, &resource, &digest)
        .map_err(|_| "journal has uncertain or conflicting operation metadata")?
        .ok_or(
            "no exact committed receipt exists; this operation cannot be invented or discarded",
        )?;
    if resource != pairing::telemetry_resource() || completed.is_some_and(|old| old != receipt) {
        return Err(
            "device retained completion conflicts with its original service journal".into(),
        );
    }
    let CommissioningReply::Challenge(challenge) = usb.request(&CommissioningRequest::Challenge)?
    else {
        return Err("unexpected device challenge response".into());
    };
    let mut attempted = false;
    let outcome = approve(
        &approval_directory,
        approval,
        |owner| {
            if policy.owner() != &owner {
                return Err(Error::Authentication);
            }
            Request::for_receipt_reconciliation(&challenge, &record, &prior, receipt)
        },
        |request| {
            request.matches_receipt_reconciliation(&challenge, &prior, receipt)?;
            let CommissioningReply::State {
                device: fresh,
                record: current,
                ..
            } = usb
                .request(&CommissioningRequest::Inspect)
                .map_err(commissioning_error)?
            else {
                return Err(Error::Persistence);
            };
            if fresh.principal() != device.principal()
                || current.commitment() != record.commitment()
                || current.store() != record.store()
            {
                return Err(Error::Persistence);
            }
            attempted = true;
            if !matches!(usb.request(&CommissioningRequest::Reconcile {
            challenge: challenge.clone(), prior: prior.clone(), receipt,
        }).map_err(commissioning_error)?, CommissioningReply::Reconciled(value) if value == receipt)
            {
                return Err(Error::Persistence);
            }
            let CommissioningReply::State {
                device: fresh,
                record: current,
                pending,
                ..
            } = usb
                .request(&CommissioningRequest::Inspect)
                .map_err(commissioning_error)?
            else {
                return Err(Error::Persistence);
            };
            if fresh.principal() != device.principal()
                || current.commitment() != record.commitment()
                || current.store() != record.store()
                || current.enabled()
                || pending != CommissioningPending::Complete
            {
                return Err(Error::Persistence);
            }
            Ok(())
        },
    );
    if let Err(error) = outcome {
        if attempted {
            eprintln!(
                "Receipt reconciliation may have committed. Preserve the prior checkpoint and journal; inspect the device before retrying this same receipt-only repair. No grant was restored."
            );
        }
        return Err(error);
    }
    println!(
        "Committed receipt {} reconciled. Device policy remains revoked at revision {}; no effect, identity or permission was created.",
        receipt.sequence(),
        record.revision()
    );
    Ok(())
}
