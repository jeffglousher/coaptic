//! Physical USB commissioning. The host verifies hardware approval before sending
//! one exact update; timeout never triggers automatic key regeneration or retry.

use coaptic::{
    Endpoint,
    provisioning::{
        CommissioningDecoder, CommissioningReply, CommissioningRequest, EnrollmentPolicy,
        PairingUpdate, PinnedPeer,
    },
};
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

struct Usb {
    port: Box<dyn serialport::SerialPort>,
    decoder: CommissioningDecoder,
}

impl Usb {
    fn open(name: &str) -> Result<Self> {
        let port = serialport::new(name, 115200)
            .timeout(Duration::from_millis(200))
            .open()?;
        Ok(Self {
            port,
            decoder: CommissioningDecoder::new(),
        })
    }
    fn request(&mut self, request: &CommissioningRequest) -> Result<CommissioningReply> {
        let mut payload = [0; 512];
        let len = request.encode(&mut payload)?;
        let mut frame = [0; 518];
        let len = CommissioningDecoder::frame(&payload[..len], &mut frame)?;
        self.port.write_all(&frame[..len])?;
        self.port.flush()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut bytes = [0; 64];
        while Instant::now() < deadline {
            match self.port.read(&mut bytes) {
                Ok(len) => {
                    for byte in &bytes[..len] {
                        if let Some(payload) = self.decoder.feed(*byte) {
                            let reply = CommissioningReply::decode(payload)?;
                            if let CommissioningReply::Refused(code) = reply {
                                return Err(format!(
                                    "device refused commissioning request (code {code})"
                                )
                                .into());
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
    let endpoint = if command == "serial-pair" {
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
    let mut development = false;
    let mut write = true;
    for option in args {
        if option == "--development-no-touch" && !development {
            development = true;
        } else if option == "--read-only" && write && command == "serial-pair" {
            write = false;
        } else {
            return Err("unexpected or repeated commissioning option".into());
        }
    }
    let CommissioningReply::State {
        device: device_pin,
        record,
    } = usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("device has no identity; explicit serial-init is required".into());
    };
    if device_pin.principal() == host.peer().principal() {
        return Err("device and host identities must differ".into());
    }
    let started = Instant::now();
    let local_challenge = local.begin(&host, 0, |out| getrandom::fill(out).is_ok())?;
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
    } else {
        Action::Pair
    };
    let prior = local.checkpoint().policy().cloned();
    let device_policy = record.policy().cloned();
    let permission = pairing::permissions(write);
    let make_request = |owner| {
        if prior
            .as_ref()
            .is_some_and(|policy| policy.owner() != &owner)
        {
            return Err(Error::Input);
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
    super::pairing_cli::approve(directory, development, make_request, |request| {
        request.matches_pairing(&challenge)?;
        let policy =
            EnrollmentPolicy::new(request.peer.peer()?, request.owner, request.permissions)
                .map_err(|_| Error::Input)?;
        let update =
            PairingUpdate::new(challenge.clone(), policy.clone(), action == Action::Revoke);
        // Revocation retains the existing routing hint. The destination itself
        // is not an identity; this value is unused once access is disabled.
        let destination = endpoint.unwrap_or(Endpoint::v4([127, 0, 0, 1], 1));
        let response = usb
            .request(&CommissioningRequest::Update {
                update,
                endpoint: destination,
            })
            .map_err(|_| Error::Persistence)?;
        let CommissioningReply::Committed(committed) = response else {
            return Err(Error::Persistence);
        };
        let approved = committed.policy().ok_or(Error::Persistence)?;
        if committed.store() != record.store()
            || record.revision().checked_add(1) != Some(committed.revision())
            || committed.local_principal() != device_pin.principal().fingerprint()
            || committed.enabled() != (action == Action::Pair)
            || approved.peer().principal() != policy.peer().principal()
            || approved.owner() != policy.owner()
            || approved.resource_policy() != policy.resource_policy()
        {
            return Err(Error::Persistence);
        }
        let now = started
            .elapsed()
            .as_millis()
            .try_into()
            .map_err(|_| Error::Expired)?;
        if action == Action::Pair {
            let reciprocal =
                EnrollmentPolicy::new(device_pin.clone(), request.owner, request.permissions)
                    .map_err(|_| Error::Input)?;
            local
                .pair(&local_challenge, reciprocal, now, |old, next| {
                    store.commit(old, next)
                })
                .map_err(|_| Error::Persistence)?;
        } else {
            local
                .revoke(&local_challenge, &request.owner, now, |old, next| {
                    store.commit(old, next)
                })
                .map_err(|_| Error::Persistence)?;
        }
        Ok(())
    })?;
    println!(
        "Device and host pairing committed. Peer {}.",
        super::hex(device_pin.principal().fingerprint())
    );
    Ok(())
}

fn show(reply: CommissioningReply) -> Result<()> {
    match reply {
        CommissioningReply::Empty => println!("Device identity is not initialized."),
        CommissioningReply::State { device, record } => {
            show_peer(&device);
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
        }
        _ => return Err("unexpected device state response".into()),
    }
    Ok(())
}

fn show_peer(peer: &PinnedPeer) {
    println!(
        "Device {} public key {} credential ID {}",
        super::hex(peer.principal().fingerprint()),
        super::hex(&peer.public_key()),
        peer.kid()
    );
}
