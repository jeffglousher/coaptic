//! Short physical-peer qualification using one App and two bounded associations.

use coaptic::{
    App, Code, Endpoint, Method, Response, get,
    oscore::{DeriveParams, SecurityContext},
    provisioning::{CoapProvisioner, Identity, LocalPairing, Status},
    resources::{Fixed, Resources, SmallDevice},
    storage::UdpSocketIo,
};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

// Two simultaneous full-response CON owners need two RX credits and two
// retained TX packets, each in addition to the pool's ingress/control headroom.
// Every other pool dimension and the datagram-only selection stay Constrained.
coaptic::memory_profile! {
    SharedConstrained {
        rx_datagram: (3, 1152), tx_datagram: (3, 1152),
        rx_body: (1, 4096), tx_body: (1, 4096), dedup: 4, observe: 2,
    }
}
type SharedResources = Resources<SharedConstrained, 4, false, 0, 2, Fixed, 4, 2>;

fn shared_resources() -> SharedResources {
    Resources::<SharedConstrained>::new()
        .routes::<4>()
        .client_calls::<2>()
        .associations::<2>()
        .security_requests::<4>()
}

fn check_owner_ceiling<A>(_: &A) {
    const {
        assert!(std::mem::size_of::<A>() <= 20 * 1024);
    }
}

fn is_initiator(local: &[u8; 32], peer: &[u8; 32]) -> bool {
    local > peer
}

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
    let write = *permissions == coaptic_enroll::pairing::permissions(true)
        || *permissions == coaptic_enroll::pairing::echo_permissions();
    if !write && *permissions != coaptic_enroll::pairing::permissions(false) {
        return Err("unknown application permissions".into());
    }
    let endpoint: Endpoint = remote.parse::<std::net::SocketAddr>()?.into();
    let socket = UdpSocket::bind(local)?;
    let local_endpoint: Endpoint = socket.local_addr()?.into();
    socket.set_nonblocking(true)?;
    let mut io = UdpSocketIo::new(socket, [0; 1473])?;
    let started = Instant::now();
    // Match the device's role selection; a responder needs incoming UDP access.
    let mut bootstrap = if is_initiator(
        identity.peer().principal().fingerprint(),
        peer.principal().fingerprint(),
    ) {
        CoapProvisioner::start(identity, peer.clone(), endpoint, 0, entropy)
            .map_err(|e| format!("bootstrap start: {e:?}"))?
    } else {
        CoapProvisioner::listen(identity, peer.clone(), endpoint, 0)
    };
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
    let mut app = App::with_resources(shared_resources())
        .randomness(entropy)
        .require_associations()
        .bind(io)?;
    check_owner_ceiling(&app);
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
    let capacities = app.engine().capacities();
    let usage = app.resource_usage();
    println!(
        "Shared App bytes {}; Constrained two-CON override; RX {}x{}; TX {}x{}; associations {}; routes {}; Calls {}; bindings/context {}; body pools RX {} TX {}.",
        std::mem::size_of_val(&app),
        capacities.rx_datagram_slots,
        capacities.rx_datagram_bytes,
        capacities.tx_datagram_slots,
        capacities.tx_datagram_bytes,
        app.association_capacity(),
        usage.routes.capacity,
        app.client_capacity(),
        app.security_request_capacity(),
        capacities.rx_body_slots.unwrap_or(0),
        capacities.tx_body_slots.unwrap_or(0),
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

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::{
        app::Error,
        message::{Type, decode},
        storage::{DatagramIo, DatagramTransport},
    };

    #[derive(Default)]
    struct Sink {
        sent: usize,
    }
    impl DatagramTransport for Sink {
        type Error = ();
        fn fill_random(&mut self, bytes: &mut [u8]) -> bool {
            entropy(bytes)
        }
    }
    impl DatagramIo for Sink {
        fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
            Ok(None)
        }
        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
            let packet = decode(bytes).unwrap();
            assert_eq!(packet.ty(), Type::Confirmable);
            assert!(packet.oscore().is_some());
            self.sent += 1;
            Ok(bytes.len())
        }
    }

    #[test]
    fn shared_probe_handshake_roles_are_complementary() {
        let lower = [1; 32];
        let greater = [2; 32];
        assert!(is_initiator(&greater, &lower));
        assert!(!is_initiator(&lower, &greater));
        assert!(!is_initiator(&lower, &lower));
    }

    #[test]
    fn shared_probe_two_protected_con_calls_preserve_headroom_and_recover_capacity() {
        let mut app = App::with_resources(shared_resources())
            .randomness(entropy)
            .require_associations()
            .bind(Sink::default())
            .unwrap();
        check_owner_ceiling(&app);
        let capacities = app.engine().capacities();
        assert_eq!(
            (capacities.rx_datagram_slots, capacities.tx_datagram_slots),
            (3, 3)
        );
        assert_eq!(
            (capacities.rx_body_slots, capacities.tx_body_slots),
            (None, None)
        );
        let context = |id: u8| {
            SecurityContext::derive(DeriveParams {
                master_secret: &[id; 16],
                master_salt: &[],
                sender_id: &[0],
                recipient_id: &[id],
                id_context: &[],
            })
            .unwrap()
        };
        let a = app
            .install_association(
                Endpoint::v4([192, 0, 2, 1], 5683),
                [1; 32],
                1,
                &mut Some(context(1)),
            )
            .unwrap();
        let b = app
            .install_association(
                Endpoint::v4([192, 0, 2, 2], 5683),
                [2; 32],
                1,
                &mut Some(context(2)),
            )
            .unwrap();
        let first = app.get("identity").to_association(a).send(0).unwrap();
        let second = app.get("identity").to_association(b).send(0).unwrap();
        assert_eq!(app.transport().sent, 2);
        let usage = app.resource_usage();
        assert_eq!(usage.client_requests.occupied, 2);
        assert_eq!(usage.storage.tx_datagram.occupied, 2);
        assert_eq!(usage.ordinary_tx_capacity, 2);
        assert_eq!(usage.full_response_capacity, 2);
        assert!(matches!(
            app.get("third").to_association(a).send(1),
            Err(Error::Saturated)
        ));
        assert_eq!(
            app.transport().sent,
            2,
            "refusal must not send or displace either owner"
        );
        assert!(app.cancel(first));
        assert!(app.take_response(first).unwrap().is_err());
        assert!(app.get("replacement").to_association(a).send(2).is_ok());
        assert_eq!(app.transport().sent, 3);
        assert!(app.take_response(second).is_none());
        assert_eq!(app.resource_usage().storage.tx_datagram.occupied, 2);
    }
}
