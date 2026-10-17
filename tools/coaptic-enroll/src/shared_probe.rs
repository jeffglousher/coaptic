//! Short physical-peer qualification using one App and two bounded associations.

use coaptic::{
    App, Code, Endpoint, Method, Response, get,
    oscore::{DeriveParams, SecurityContext},
    provisioning::{CoapProvisioner, Identity, LocalPairing, Status},
    resources::SmallDevice,
    storage::UdpSocketIo,
};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}

pub fn run(
    identity: &Identity,
    pairing: &LocalPairing,
    local: &str,
    remote: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer = pairing.peer()?;
    let permissions = pairing
        .checkpoint()
        .policy()
        .ok_or("missing policy")?
        .resource_policy();
    let write = *permissions == coaptic_enroll::pairing::permissions(true);
    if !write && *permissions != coaptic_enroll::pairing::permissions(false) {
        return Err("unknown application permissions".into());
    }
    let endpoint: Endpoint = remote.parse::<std::net::SocketAddr>()?.into();
    let socket = UdpSocket::bind(local)?;
    let local_endpoint: Endpoint = socket.local_addr()?.into();
    socket.set_nonblocking(true)?;
    let mut io = UdpSocketIo::new(socket, [0; 1473])?;
    let started = Instant::now();
    let mut bootstrap = CoapProvisioner::start(identity, peer.clone(), endpoint, 0, entropy)
        .map_err(|e| format!("bootstrap start: {e:?}"))?;
    let session = loop {
        bootstrap
            .poll(
                &mut io,
                started.elapsed().as_millis() as u64,
                entropy,
                |principal| pairing.permits(principal),
            )
            .map_err(|e| format!("bootstrap poll: {e:?}"))?;
        if let Some(session) = bootstrap.take_session() {
            break session;
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err("shared probe handshake deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    if !pairing.permits(session.principal())
        || session.local_principal() != &identity.peer().principal()
    {
        return Err("shared probe session authorization changed".into());
    }
    let (context, principal) = session.into_parts();
    // Only Call capacity changes from SmallDevice: both outbound operations live together.
    let mut app = App::with_resources(SmallDevice::new().client_calls::<2>().associations::<2>())
        .randomness(entropy)
        .require_associations()
        .bind(io)?;
    let hardware =
        app.install_association(endpoint, *principal.fingerprint(), 1, &mut Some(context))?;
    let second_socket = UdpSocket::bind("0.0.0.0:0")?;
    let second_port = second_socket.local_addr()?.port();
    second_socket.set_nonblocking(true)?;
    let second_endpoint = Endpoint::from(std::net::SocketAddr::new(
        local.parse::<std::net::SocketAddr>()?.ip(),
        second_port,
    ));
    let mut secret = [0; 16];
    getrandom::fill(&mut secret)?;
    let derive = |sender, recipient| {
        SecurityContext::derive(DeriveParams {
            master_secret: &secret,
            master_salt: &[],
            sender_id: sender,
            recipient_id: recipient,
            id_context: &[],
        })
    };
    let mut second = App::with_resources(SmallDevice::new())
        .randomness(entropy)
        .oscore(derive(&[7], &[8])?)
        .route("identity", get(|_| Response::content(b"second-peer")))
        .bind(UdpSocketIo::new(second_socket, [0; 1473])?)?;
    let software =
        app.install_association(second_endpoint, [7; 32], 2, &mut Some(derive(&[8], &[7])?))?;
    println!(
        "Shared App bytes {}; associations 2; routes 4; Calls 2; bindings/context 4; no body pools.",
        std::mem::size_of_val(&app)
    );
    let mut elapsed = [0; 2];
    for (round, elapsed_us) in elapsed.iter_mut().enumerate() {
        let now = started.elapsed().as_millis() as u64;
        let began = Instant::now();
        let hardware_call = app.get("identity").to_association(hardware).send(now)?;
        let software_call = if round == 0 {
            Some(app.get("identity").to_association(software).send(now)?)
        } else {
            None
        };
        let mut hardware_done = false;
        let mut software_done = software_call.is_none();
        while !hardware_done || !software_done {
            if !pairing.permits(&principal) {
                return Err("pairing no longer permits probe".into());
            }
            let now = started.elapsed().as_millis() as u64;
            let mut ingress_error = None;
            app.poll_with_association_ingress(
                now,
                |bytes, source| match bootstrap
                    .ingest(bytes, source, now, entropy, |p| pairing.permits(p))
                {
                    Ok(Status::Ignored | Status::Expired) => false,
                    Ok(_) => true,
                    Err(error) => {
                        ingress_error = Some(error);
                        true
                    }
                },
                |_, _| {
                    // These fresh EDHOC and random test keys are discarded on exit and
                    // never restored. There is no durable session checkpoint backend.
                    true
                },
                |request, _| {
                    if request.association() != Some(hardware) {
                        return Response::new(Code::FORBIDDEN);
                    }
                    if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                        Response::content_copy(identity.peer().principal().fingerprint())
                    } else if request.path() == ["echo"] && request.method() == Some(Method::Post) {
                        if write {
                            Response::content_copy(request.payload())
                        } else {
                            Response::new(Code::FORBIDDEN)
                        }
                    } else {
                        Response::new(Code::NOT_FOUND)
                    }
                },
            )?;
            if let Some(error) = ingress_error {
                return Err(format!("bootstrap ingress: {error:?}").into());
            }
            bootstrap
                .flush(app.transport_mut(), now)
                .map_err(|e| format!("bootstrap flush: {e:?}"))?;
            second.poll(now)?;
            if !hardware_done && let Some(response) = app.take_response(hardware_call) {
                let response = response.map_err(|e| format!("hardware call: {e:?}"))?;
                if response.code() != Code::CONTENT || response.payload() != principal.fingerprint()
                {
                    return Err("hardware identity differs".into());
                }
                hardware_done = true;
                *elapsed_us = began.elapsed().as_micros();
            }
            if !software_done
                && let Some(response) =
                    app.take_response(software_call.ok_or("missing software Call")?)
            {
                let response = response.map_err(|e| format!("software call: {e:?}"))?;
                if response.code() != Code::CONTENT || response.payload() != b"second-peer" {
                    return Err("software identity differs".into());
                }
                software_done = true;
            }
            if began.elapsed() > Duration::from_secs(15) {
                return Err("shared probe application deadline".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if round == 0 {
            app.remove_association(software)?;
            if app
                .get("identity")
                .to_association(software)
                .send(now)
                .is_ok()
            {
                return Err("revoked handle accepted".into());
            }
        }
    }
    println!(
        "Shared probe passed: concurrent ESP32 and software peer; revoked software handle refused; ESP32 survived. ESP32 round trips {} / {} us; total {} ms; local {}.",
        elapsed[0],
        elapsed[1],
        started.elapsed().as_millis(),
        local_endpoint
    );
    Ok(())
}
