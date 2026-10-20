//! Physical USB commissioning. The host verifies hardware approval before sending
//! one exact update; timeout never triggers automatic key regeneration or retry.

use coaptic::{
    Endpoint,
    provisioning::{
        CommissioningDecoder, CommissioningPending, CommissioningReply, CommissioningRequest,
        EnrollmentPolicy, PairingRecord, PairingUpdate, PinnedPeer,
    },
};
use coaptic_console_core::ApprovalMode;
use coaptic_enroll::{
    Action, Error, Request, device,
    pairing::{self, PairingFile},
};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) struct Usb {
    port: Box<dyn serialport::SerialPort>,
    decoder: CommissioningDecoder,
}

impl Usb {
    pub(super) fn open(name: &str) -> Result<Self> {
        let port = serialport::new(name, 115200)
            .timeout(Duration::from_millis(200))
            .open()?;
        Ok(Self {
            port,
            decoder: CommissioningDecoder::new(),
        })
    }
    pub(super) fn request(&mut self, request: &CommissioningRequest) -> Result<CommissioningReply> {
        exchange(&mut *self.port, &mut self.decoder, request)
    }
}

pub(super) fn commissioning_error(error: Box<dyn std::error::Error>) -> Error {
    match error.downcast::<Error>() {
        Ok(error) => *error,
        Err(error) => Error::Commissioning(error.to_string()),
    }
}

fn exchange<T: Read + Write + ?Sized>(
    port: &mut T,
    decoder: &mut CommissioningDecoder,
    request: &CommissioningRequest,
) -> Result<CommissioningReply> {
    let mut payload = [0; 512];
    let len = request.encode(&mut payload)?;
    let mut frame = [0; 518];
    let len = CommissioningDecoder::frame(&payload[..len], &mut frame)?;
    port.write_all(&frame[..len])?;
    port.flush()?;
    let deadline = Instant::now()
        + Duration::from_secs(if matches!(request, CommissioningRequest::Quiesce) {
            35
        } else {
            10
        });
    let mut bytes = [0; 64];
    while Instant::now() < deadline {
        match port.read(&mut bytes) {
            Ok(len) => {
                for byte in &bytes[..len] {
                    if let Some(payload) = decoder.feed(*byte) {
                        let reply = CommissioningReply::decode(payload)?;
                        if let CommissioningReply::Refused(code) = reply {
                            return Err(Error::DeviceRefused(code).into());
                        }
                        return Ok(reply);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err("Device response timed out; inspect its state before retrying.".into())
}
pub fn run(
    command: &OsStr,
    mut args: impl Iterator<Item = OsString>,
    directory: &Path,
) -> Result<()> {
    let port = args.next().ok_or("missing serial port")?;
    let mut usb = Usb::open(port.to_str().ok_or("invalid serial port")?)?;
    if command == "serial-info" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        return show(usb.request(&CommissioningRequest::Inspect)?);
    }
    if command == "serial-init" {
        let kid: u8 = args
            .next()
            .ok_or("missing credential ID")?
            .to_str()
            .ok_or("invalid credential ID")?
            .parse()?;
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        return show(usb.request(&CommissioningRequest::Initialize { kid })?);
    }
    if command == "serial-quiesce" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        if !matches!(
            usb.request(&CommissioningRequest::Quiesce)?,
            CommissioningReply::Quiesced
        ) {
            return Err("unexpected quiescence response".into());
        }
        println!(
            "Durable work drained; production held for 60 seconds. Stop the host service before changing its pairing; an administrative challenge extends the hold through approval."
        );
        return Ok(());
    }
    if command == "serial-restart" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        if !matches!(
            usb.request(&CommissioningRequest::Restart)?,
            CommissioningReply::Restarting
        ) {
            return Err("unexpected restart response".into());
        }
        println!(
            "Device restarting; it will restore pairing and establish a fresh session automatically."
        );
        return Ok(());
    }
    let name = args.next().ok_or("missing host identity name")?;
    let host_directory = super::device_directory(directory, &name)?;
    let host = device::restore(&host_directory)?;
    let store = PairingFile::open(&host_directory)?;
    let mut local = store.restore(&host)?;
    let endpoint = if command == "serial-pair" || command == "serial-transfer" {
        let address = args.next().ok_or("missing host IPv4 address:port")?;
        let address = address
            .to_str()
            .ok_or("invalid address")?
            .parse::<SocketAddr>()?;
        if !address.is_ipv4()
            || address.ip().is_unspecified()
            || address.ip().is_multicast()
            || address.port() == 0
        {
            return Err("host address must be unicast IPv4 with a nonzero port".into());
        }
        Some(Endpoint::from(address))
    } else {
        None
    };
    let next_owner = if command == "serial-transfer" {
        if local.checkpoint().policy().is_some() {
            return Err(
                "transfer requires an explicitly provisioned, unpaired destination host".into(),
            );
        }
        Some(super::pairing_cli::parse_owner(
            &args.next().ok_or("missing new owner commitment")?,
        )?)
    } else {
        None
    };
    let mut approval = ApprovalMode::WebAuthn;
    let mut approval_selected = false;
    let mut approval_directory = directory.to_path_buf();
    let mut directory_selected = false;
    let mut access = pairing::Access::default();
    let mut access_selected = false;
    while let Some(option) = args.next() {
        if let Some(mode) = option.to_str().and_then(ApprovalMode::from_flag) {
            if approval_selected {
                return Err("repeated approval mode".into());
            }
            approval = mode;
            approval_selected = true;
        } else if option == "--approval-directory" && !directory_selected {
            approval_directory = args.next().ok_or("missing approval directory")?.into();
            directory_selected = true;
        } else if let Some(value) = option.to_str().and_then(pairing::Access::from_flag) {
            if access_selected || !(command == "serial-pair" || command == "serial-transfer") {
                return Err("unexpected or repeated access option".into());
            }
            access = value;
            access_selected = true;
        } else {
            return Err("unexpected or repeated commissioning option".into());
        }
    }
    let CommissioningReply::State {
        device: device_pin,
        record,
        pending,
        ..
    } = usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("device has no identity; explicit serial-init is required".into());
    };
    if device_pin.principal() == host.peer().principal() {
        return Err("device and host identities must differ".into());
    }
    preflight_pending(command.to_str().ok_or("invalid command")?, pending)?;
    // Hold the existing journal's exclusive lock before approving telemetry.
    // Missing state is never initialized as a side effect of changing a grant.
    let telemetry = if command == "serial-recover" {
        record.enabled()
            && record
                .policy()
                .is_some_and(|policy| *policy.resource_policy() == pairing::permissions(true))
    } else {
        access == pairing::Access::Telemetry
            && (command == "serial-pair" || command == "serial-transfer")
    };
    let _receipts = if telemetry {
        Some(pairing::preflight_receipts(&host_directory, &host).map_err(|_| {
            "host receipt journal is missing, invalid or in use; inspect its history and explicitly initialize only its first journal before approving telemetry"
        })?)
    } else {
        None
    };
    let started = Instant::now();
    let recovery = if command == "serial-recover" {
        Some(coaptic_enroll::recovery::PeerRecovery::new(
            device_pin.clone(),
            record.clone(),
            local.checkpoint(),
        )?)
    } else {
        None
    };
    let local_challenge = local.begin(&host, 0, |out| getrandom::fill(out).is_ok())?;
    if let Some(recovery) = recovery {
        super::pairing_cli::approve(
            &approval_directory,
            approval,
            |owner| recovery.request(&local_challenge, owner),
            |request| {
                let CommissioningReply::State { device, record, .. } = usb
                    .request(&CommissioningRequest::Inspect)
                    .map_err(commissioning_error)?
                else {
                    return Err(Error::Persistence);
                };
                recovery.commit(
                    request,
                    &local_challenge,
                    (&device, &record),
                    &mut local,
                    started
                        .elapsed()
                        .as_millis()
                        .try_into()
                        .map_err(|_| Error::Expired)?,
                    |old, next| store.commit(old, next),
                )
            },
        )?;
        println!("Host pairing recovered from the device's current policy.");
        return Ok(());
    }
    let CommissioningReply::Challenge(challenge) = usb.request(&CommissioningRequest::Challenge)?
    else {
        return Err("unexpected device challenge response".into());
    };
    if challenge.device().principal() != device_pin.principal()
        || challenge.expected() != (*record.store(), record.revision(), record.commitment())
    {
        return Err("device identity or revision changed during commissioning".into());
    }
    let action = if command == "serial-revoke" {
        Action::Revoke
    } else if command == "serial-transfer" {
        Action::Transfer
    } else {
        Action::Pair
    };
    let prior = local.checkpoint().policy().cloned();
    let device_policy = record.policy().cloned();
    let permission = access.permissions();
    let make_request = |owner| {
        if prior
            .as_ref()
            .is_some_and(|policy| policy.owner() != &owner)
        {
            return Err(Error::Input);
        }
        if action == Action::Transfer {
            if device_policy
                .as_ref()
                .is_none_or(|old| old.owner() != &owner)
            {
                return Err(Error::Input);
            }
            let policy =
                EnrollmentPolicy::new(host.peer(), next_owner.ok_or(Error::Input)?, permission)
                    .map_err(|_| Error::Input)?;
            return Request::for_transfer(&challenge, owner, &policy);
        }
        let policy = if action == Action::Revoke {
            let policy = device_policy.clone().ok_or(Error::Input)?;
            if policy.owner() != &owner || policy.peer().principal() != host.peer().principal() {
                return Err(Error::Input);
            }
            if prior
                .as_ref()
                .is_none_or(|policy| policy.peer().principal() != device_pin.principal())
            {
                return Err(Error::Input);
            }
            policy
        } else {
            EnrollmentPolicy::new(host.peer(), owner, permission).map_err(|_| Error::Input)?
        };
        Request::for_pairing(&challenge, &policy, action)
    };
    let mut device_update_attempted = false;
    let outcome = super::pairing_cli::approve(
        &approval_directory,
        approval,
        make_request,
        |request| {
            request.matches_pairing(&challenge)?;
            let policy = request.pairing_policy()?;
            let update = if action == Action::Transfer {
                PairingUpdate::transfer(challenge.clone(), request.owner, policy.clone())
                    .map_err(|_| Error::Input)?
            } else {
                PairingUpdate::new(challenge.clone(), policy.clone(), action == Action::Revoke)
            };
            // Revocation retains the existing routing hint. The destination itself
            // is not an identity; this value is unused once access is disabled.
            let destination = endpoint.unwrap_or(Endpoint::v4([127, 0, 0, 1], 1));
            let reciprocal = EnrollmentPolicy::new(
                device_pin.clone(),
                *policy.owner(),
                *policy.resource_policy(),
            )
            .map_err(|_| Error::Input)?;
            let now = started
                .elapsed()
                .as_millis()
                .try_into()
                .map_err(|_| Error::Expired)?;
            let prepared = coaptic_enroll::recovery::PreparedHostPairing::prepare_update(
                &mut local,
                &local_challenge,
                reciprocal,
                action == Action::Revoke,
                now,
            )?;
            if action == Action::Revoke {
                match pairing::save_device_checkpoint(&host_directory, &record) {
                    Ok(checkpoint) => println!(
                        "Prior public device checkpoint retained: {}",
                        checkpoint.display()
                    ),
                    Err(error) => {
                        eprintln!(
                            "Warning: could not retain the prior public device checkpoint ({error:?}). Revocation will continue; preserve the following public CBOR and the original receipt journal for recovery. Existing archive files were not replaced."
                        );
                        let mut bytes = [0; PairingRecord::MAX_ENCODED_LEN];
                        if let Ok(length) = record.encode(&mut bytes) {
                            eprintln!(
                                "Prior public device checkpoint CBOR (hex): {}",
                                super::hex(&bytes[..length])
                            );
                        } else {
                            eprintln!("Warning: prior public checkpoint could not be encoded.");
                        }
                    }
                }
            }
            device_update_attempted = true;
            let response = usb
                .request(&CommissioningRequest::Update {
                    update,
                    endpoint: destination,
                })
                .map_err(commissioning_error)?;
            let CommissioningReply::Committed(committed) = response else {
                return Err(Error::Persistence);
            };
            let approved = committed.policy().ok_or(Error::Persistence)?;
            if committed.store() != record.store()
                || record.revision().checked_add(1) != Some(committed.revision())
                || committed.local_principal() != device_pin.principal().fingerprint()
                || committed.enabled() != (action != Action::Revoke)
                || approved.peer().principal() != policy.peer().principal()
                || approved.owner() != policy.owner()
                || approved.resource_policy() != policy.resource_policy()
            {
                return Err(Error::Persistence);
            }
            prepared.commit(|old, next| store.commit(old, next))?;
            Ok(())
        },
    );
    if let Err(error) = outcome {
        if device_update_attempted {
            eprintln!(
                "Device update may have committed. Inspect device and host policy before any retry; do not repeat the device update blindly. Use serial-recover only when its exact current-owner/destination checks accept the inspected state."
            );
        }
        return Err(error);
    }
    println!(
        "Device and host pairing committed. Peer {}.",
        super::hex(device_pin.principal().fingerprint())
    );
    Ok(())
}

fn show(reply: CommissioningReply) -> Result<()> {
    match reply {
        CommissioningReply::Empty => println!("Device identity is not initialized."),
        CommissioningReply::State {
            device,
            record,
            pending,
            ..
        } => {
            show_peer(&device);
            println!("Durable work: {}", custody_description(pending));
            println!(
                "Pairing revision {} enabled {}",
                record.revision(),
                record.enabled()
            );
            if let Some(policy) = record.policy() {
                println!(
                    "Owner {} permissions {}",
                    super::hex(policy.owner()),
                    super::hex(policy.resource_policy())
                );
            }
            if let Some(owner) = record.recovery_owner() {
                println!("Preauthorized recovery credential {}", super::hex(owner));
            }
        }
        _ => return Err("unexpected device state response".into()),
    }
    Ok(())
}

fn custody_description(pending: CommissioningPending) -> &'static str {
    match pending {
        CommissioningPending::Missing => {
            "missing history; reconciliation required before grant changes"
        }
        CommissioningPending::Complete => "authenticated empty or completed",
        CommissioningPending::Unresolved => {
            "authenticated unresolved work; serve receipts and quiesce before grant changes"
        }
        CommissioningPending::Unverifiable => {
            "unverifiable under current pairing; preserve history for reconciliation"
        }
    }
}

fn preflight_pending(
    command: &str,
    pending: CommissioningPending,
) -> std::result::Result<(), &'static str> {
    if !matches!(command, "serial-pair" | "serial-transfer") {
        return Ok(());
    }
    match pending {
        CommissioningPending::Complete => Ok(()),
        CommissioningPending::Unresolved => Err(
            "device has unresolved durable work; keep the authorized host serving receipts and run serial-quiesce before changing the grant",
        ),
        CommissioningPending::Missing | CommissioningPending::Unverifiable => Err(
            "device durable history is missing or unverifiable; preserve the device and host journals and reconcile their authenticated history before changing the grant; do not reset or initialize replacement history",
        ),
    }
}

fn show_peer(peer: &PinnedPeer) {
    println!(
        "Device {} public key {} credential ID {}",
        super::hex(peer.principal().fingerprint()),
        super::hex(&peer.public_key()),
        peer.kid()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    #[test]
    fn custody_output_distinguishes_absence_completion_unresolved_and_unverifiable() {
        for (status, expected) in [
            (
                CommissioningPending::Missing,
                "Durable work: missing history; reconciliation required before grant changes",
            ),
            (
                CommissioningPending::Complete,
                "Durable work: authenticated empty or completed",
            ),
            (
                CommissioningPending::Unresolved,
                "Durable work: authenticated unresolved work; serve receipts and quiesce before grant changes",
            ),
            (
                CommissioningPending::Unverifiable,
                "Durable work: unverifiable under current pairing; preserve history for reconciliation",
            ),
        ] {
            assert_eq!(
                format!("Durable work: {}", custody_description(status)),
                expected
            );
        }
    }

    #[test]
    fn pending_preflight_refuses_grants_without_blocking_revocation_or_host_repair() {
        for pending in [
            CommissioningPending::Missing,
            CommissioningPending::Unverifiable,
            CommissioningPending::Unresolved,
        ] {
            assert!(preflight_pending("serial-pair", pending).is_err());
            assert!(preflight_pending("serial-transfer", pending).is_err());
            assert!(preflight_pending("serial-revoke", pending).is_ok());
            assert!(preflight_pending("serial-recover", pending).is_ok());
        }
        assert!(preflight_pending("serial-pair", CommissioningPending::Complete).is_ok());
    }

    struct Port {
        input: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }
    impl Read for Port {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.input.position() == self.input.get_ref().len() as u64 {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.input.read(out)
        }
    }
    impl Write for Port {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn device_refusal_survives_the_approval_callback_boundary() {
        let mut payload = [0; 512];
        let len = CommissioningReply::Refused(5).encode(&mut payload).unwrap();
        let mut frame = [0; 518];
        let len = CommissioningDecoder::frame(&payload[..len], &mut frame).unwrap();
        let mut port = Port {
            input: Cursor::new(frame[..len].to_vec()),
            written: Vec::new(),
        };
        let error = exchange(
            &mut port,
            &mut CommissioningDecoder::new(),
            &CommissioningRequest::Inspect,
        )
        .unwrap_err();
        let error = commissioning_error(error);
        assert!(matches!(error, Error::DeviceRefused(5)));
        assert_eq!(
            error.to_string(),
            "device refused commissioning request (code 5)"
        );
        assert!(!port.written.is_empty());
    }

    #[test]
    fn lost_reply_reports_uncertain_outcome_without_resending() {
        let mut port = Port {
            input: Cursor::new(Vec::new()),
            written: Vec::new(),
        };
        let request = CommissioningRequest::Restart;
        let error = exchange(&mut port, &mut CommissioningDecoder::new(), &request).unwrap_err();
        let error = commissioning_error(error);
        assert!(matches!(error, Error::Commissioning(_)));
        assert!(
            error
                .to_string()
                .contains("inspect device state before retrying")
        );
        let mut payload = [0; 512];
        let len = request.encode(&mut payload).unwrap();
        let mut expected = [0; 518];
        let len = CommissioningDecoder::frame(&payload[..len], &mut expected).unwrap();
        assert_eq!(port.written, expected[..len]);
    }
}
