use coaptic::{
    Code, Endpoint, Method, Response, profiles,
    provisioning::{Identity, LocalPairing, ManagedConnection},
    storage::UdpSocketIo,
};
use coaptic_enroll::pairing;
use std::{
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

pub fn run(
    identity: &Identity,
    pairing: &LocalPairing,
    local: &str,
    remote: &str,
    probe: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer = pairing.peer()?;
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
    if probe {
        connection.connect(0)?;
    }
    println!("Pairing restored; waiting for peer at {remote}.");
    let mut reported = false;
    let mut call = None;
    let mut output = [0; 128];
    loop {
        let now = started.elapsed().as_millis().try_into()?;
        connection.poll_with(now, |_, request, _| {
            if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                Response::content_copy(identity.peer().principal().fingerprint())
            } else if request.path() == ["echo"] && request.method() == Some(Method::Post) {
                if !write {
                    Response::new(Code::FORBIDDEN)
                } else if request.payload().len() > 128 {
                    Response::new(Code::REQUEST_ENTITY_TOO_LARGE)
                } else {
                    Response::content_copy(request.payload())
                }
            } else {
                Response::new(Code::NOT_FOUND)
            }
        })?;
        if connection.is_connected() && !reported {
            println!(
                "Fresh EDHOC/OSCORE session connected to {}.",
                super::super::hex(peer.principal().fingerprint())
            );
            reported = true;
            if probe {
                call = Some(connection.request(Method::Get, "identity", &[], None, now)?);
            }
        }
        if let Some(current) = call
            && let Some(response) = connection.take_response_into(current, &mut output, now)?
        {
            let response =
                response.map_err(|error| format!("protected probe failed: {error:?}"))?;
            if response.code() != Code::CONTENT
                || response.payload() != peer.principal().fingerprint()
            {
                return Err("protected response differs".into());
            }
            println!("Protected probe passed (code {}).", response.code());
            return Ok(());
        }
        if probe && started.elapsed() >= Duration::from_secs(30) {
            return Err("paired probe deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}
