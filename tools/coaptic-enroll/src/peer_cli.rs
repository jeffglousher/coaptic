use coaptic::{
    Code, ContentFormat, Endpoint, Method, Response, profiles,
    provisioning::{Identity, LocalPairing, ManagedConnection, ManagedError, ManagedSubscription},
    storage::UdpSocketIo,
};
use coaptic_enroll::pairing;
use std::{
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Probe {
    Connect,
    Identity,
    Echo,
    DeniedEcho,
}

pub fn run(
    identity: &Identity,
    pairing: &LocalPairing,
    local: &str,
    remote: &str,
    probe: Option<Probe>,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer = pairing.peer()?;
    let persistent = probe.is_none() || matches!(probe, Some(Probe::Connect));
    let initiates = identity.peer().principal().fingerprint() > peer.principal().fingerprint();
    let permission = pairing
        .checkpoint()
        .policy()
        .ok_or("missing policy")?
        .resource_policy();
    let write = *permission == pairing::permissions(true);
    if !write && *permission != pairing::permissions(false) {
        return Err("unknown application permissions".into());
    }
    let socket = UdpSocket::bind(local)?;
    socket.set_nonblocking(true)?;
    let endpoint = Endpoint::from(remote.parse::<SocketAddr>()?);
    let io = UdpSocketIo::new(socket, [0; 1473])?;
    let started = Instant::now();
    let mut connection = ManagedConnection::<_, profiles::Constrained>::from_local_pairing(
        identity, pairing, endpoint, io, entropy, 0,
    )?;
    connection.observable_manual("identity")?;
    let mut subscription = ManagedSubscription::new("identity");
    if initiates || probe.is_some_and(|probe| !matches!(probe, Probe::Connect)) {
        connection.connect(0)?;
    }
    let mut sample = [0; 36];
    sample[..4].copy_from_slice(&[0x82, 1, 0x58, 32]);
    getrandom::fill(&mut sample[4..])?;
    println!("Pairing restored; waiting for peer at {remote}.");
    let mut reported = false;
    let mut call = None;
    let mut output = [0; 128];
    let mut retry_at = 30_000;
    let mut subscription_received = 0;
    loop {
        let now = started.elapsed().as_millis().try_into()?;
        let result = connection.poll_with(now, |_, request, _| {
            if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                let response =
                    Response::content_copy(identity.peer().principal().fingerprint()).max_age(10);
                if request.is_observe_register() {
                    response.observe(0)
                } else {
                    response
                }
            } else if request.path() == ["echo"] && request.method() == Some(Method::Post) {
                if !write {
                    Response::new(Code::FORBIDDEN)
                } else if request.payload().len() > 128 {
                    Response::new(Code::REQUEST_ENTITY_TOO_LARGE)
                } else {
                    println!(
                        "Protected telemetry received and acknowledged ({} bytes).",
                        request.payload().len()
                    );
                    Response::content_copy(request.payload())
                }
            } else {
                Response::new(Code::NOT_FOUND)
            }
        });
        match result {
            Ok(()) | Err(ManagedError::Recovery(_)) => {}
            Err(ManagedError::Application(coaptic::app::Error::Io(_))) if persistent => {}
            Err(error) => return Err(error.into()),
        }
        if connection.is_connected() && !reported {
            println!(
                "Fresh EDHOC/OSCORE session connected to {}.",
                super::super::hex(peer.principal().fingerprint())
            );
            reported = true;
            if let Some(probe) = probe {
                let (method, path, payload, format) = match probe {
                    Probe::Connect => continue,
                    Probe::Identity => (Method::Get, "identity", &[][..], None),
                    Probe::Echo | Probe::DeniedEcho => {
                        (Method::Post, "echo", &sample[..], Some(ContentFormat::CBOR))
                    }
                };
                call = Some(connection.request(method, path, payload, format, now)?);
            }
        }
        if let Some(current) = call {
            match connection.take_response_into(current, &mut output, now) {
                Ok(Some(response)) => {
                    let response =
                        response.map_err(|error| format!("protected probe failed: {error:?}"))?;
                    let valid = match probe.ok_or("missing probe")? {
                        Probe::Connect => unreachable!(),
                        Probe::Identity => {
                            response.code() == Code::CONTENT
                                && response.payload() == peer.principal().fingerprint()
                        }
                        Probe::Echo => {
                            response.code() == Code::CONTENT && response.payload() == sample
                        }
                        Probe::DeniedEcho => {
                            response.code() == Code::FORBIDDEN && response.payload().is_empty()
                        }
                    };
                    if !valid {
                        return Err("protected response differs".into());
                    }
                    println!("Protected probe passed (code {}).", response.code());
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if persistent && connection.is_connected() {
            match connection.take_subscription_response_into(&mut subscription, &mut output, now) {
                Ok(Some(Ok(response)))
                    if response.code() == Code::CONTENT
                        && response.payload() == peer.principal().fingerprint() =>
                {
                    subscription_received = now;
                    retry_at = u64::MAX;
                }
                Ok(None) => {}
                Ok(Some(_)) => retry_at = now,
                Err(
                    ManagedError::Busy | ManagedError::NotConnected | ManagedError::StaleHandle,
                ) => {}
                Err(error) => return Err(error.into()),
            }
            match connection.poll_subscription(&mut subscription, now) {
                Ok(_) | Err(ManagedError::Busy | ManagedError::NotConnected) => {}
                Err(ManagedError::Application(
                    coaptic::app::Error::Io(_) | coaptic::app::Error::Saturated,
                )) => {}
                Err(error) => return Err(error.into()),
            }
            if now.saturating_sub(subscription_received) >= 30_000 {
                connection.stop_subscription(&mut subscription, now)?;
                subscription = ManagedSubscription::new("identity");
                subscription_received = now;
                retry_at = now;
            }
        }
        if persistent && initiates && now >= retry_at {
            retry_at = now.saturating_add(match connection.connect(now) {
                Ok(()) => 30_000,
                Err(ManagedError::Busy | ManagedError::Capacity) => 1_000,
                Err(ManagedError::Recovery(_)) => 5_000,
                Err(error) => return Err(error.into()),
            });
        }
        if probe.is_some_and(|probe| !matches!(probe, Probe::Connect))
            && started.elapsed() >= Duration::from_secs(30)
        {
            return Err("paired probe deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}
