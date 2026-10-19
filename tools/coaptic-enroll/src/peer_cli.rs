use coaptic::{
    Code, ContentFormat, Endpoint, Method, Response, profiles,
    provisioning::{
        Identity, LocalPairing, ManagedConnection, ManagedError, ManagedSubscription, OperationId,
        ReceiptError, ReceiptStore, TelemetryOperation, TrustAnchor,
    },
    storage::UdpSocketIo,
};
use coaptic_enroll::pairing;
use coaptic_receipts::{Authority, Journal, Policy};
use std::{
    net::{SocketAddr, UdpSocket},
    path::Path,
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
    directory: &Path,
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
    let telemetry = *permission == pairing::permissions(true);
    let write = telemetry || *permission == pairing::echo_permissions();
    if !write && *permission != pairing::permissions(false) {
        return Err("unknown application permissions".into());
    }
    let record = pairing.checkpoint();
    let anchor = TrustAnchor::from_parts(*record.store(), record.revision(), record.commitment())?;
    let resource = pairing::telemetry_resource();
    // PairingFile remains exclusively locked by the caller for this entire
    // service. Management must stop it before committing another grant.
    let mut journal = if telemetry {
        Some(Journal::open(
            &directory.join("receipts.bin"),
            Authority::new(Policy {
                anchor,
                principal: peer.principal(),
                resource,
                enabled: record.enabled(),
            }),
        )?)
    } else {
        None
    };
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
    let mut probe_due = 0;
    let mut probe_attempts = 0;
    let mut output = [0; 128];
    let mut retry_at = 30_000;
    let mut subscription_received = 0;
    loop {
        let now = started.elapsed().as_millis().try_into()?;
        let result = connection.poll_with(now, |principal, request, _| {
            if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                let response =
                    Response::content_copy(identity.peer().principal().fingerprint()).max_age(10);
                if request.is_observe_register() {
                    response.observe(0)
                } else {
                    response
                }
            } else if request.path() == ["telemetry"] && request.method() == Some(Method::Post) {
                let Some(journal) = journal.as_mut() else {
                    return Response::new(Code::FORBIDDEN);
                };
                // This managed service accepts complete datagram operations.
                // Never commit a fragment as an application's complete effect.
                if request
                    .get_option(coaptic::message::OptionNumber::BLOCK1)
                    .is_some()
                    || request
                        .get_option(coaptic::message::OptionNumber::Q_BLOCK1)
                        .is_some()
                {
                    return Response::new(Code::BAD_REQUEST);
                }
                let Some(id) = request.payload().get(..16) else {
                    return Response::new(Code::BAD_REQUEST);
                };
                let format = match request.content_format() {
                    Some(Ok(format)) => Some(format.get()),
                    Some(Err(_)) => return Response::new(Code::BAD_REQUEST),
                    None => None,
                };
                let operation = match TelemetryOperation::new(
                    OperationId::new(id.try_into().unwrap()),
                    resource,
                    format,
                    &request.payload()[16..],
                ) {
                    Ok(operation) => operation,
                    Err(_) => return Response::new(Code::REQUEST_ENTITY_TOO_LARGE),
                };
                match journal.commit(&anchor, principal, &operation) {
                    Ok(receipt) => {
                        println!(
                            "Durable telemetry receipt {} ({} bytes).",
                            receipt.sequence(),
                            operation.payload().len()
                        );
                        Response::new(Code::CHANGED).payload_copy(&receipt.encode())
                    }
                    Err(ReceiptError::Unauthorized) => Response::new(Code::FORBIDDEN),
                    Err(ReceiptError::Conflict) => Response::new(Code::CONFLICT),
                    Err(ReceiptError::Capacity) => Response::new(Code::SERVICE_UNAVAILABLE),
                    Err(_) => Response::new(Code::INTERNAL_SERVER_ERROR),
                }
            } else if request.path() == ["echo"] && request.method() == Some(Method::Post) {
                if !write {
                    Response::new(Code::FORBIDDEN)
                } else if request.payload().len() > 128 {
                    Response::new(Code::REQUEST_ENTITY_TOO_LARGE)
                } else {
                    println!(
                        "Protected echo received and acknowledged ({} bytes).",
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
        }
        if connection.is_connected()
            && call.is_none()
            && now >= probe_due
            && let Some(probe) = probe.filter(|probe| !matches!(probe, Probe::Connect))
        {
            let (method, path, payload, format) = match probe {
                Probe::Connect => unreachable!(),
                Probe::Identity => (Method::Get, "identity", &[][..], None),
                Probe::Echo | Probe::DeniedEcho => {
                    (Method::Post, "echo", &sample[..], Some(ContentFormat::CBOR))
                }
            };
            match connection.request(method, path, payload, format, now) {
                Ok(request) => {
                    call = Some(request);
                    probe_attempts += 1;
                }
                Err(ManagedError::Application(coaptic::app::Error::Backoff { until_ms })) => {
                    probe_due = until_ms;
                }
                Err(error) => return Err(error.into()),
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
                        let max_age = response.max_age_secs().unwrap_or(60);
                        if let Some(next) =
                            probe_retry_at(response.code(), max_age, now, probe_attempts)
                        {
                            println!(
                                "Protected probe received code {} (Max-Age {max_age}s); retrying this idempotent probe in the same session.",
                                response.code()
                            );
                            call = None;
                            probe_due = next;
                            continue;
                        }
                        return Err(format!("protected response differs: code {}, Max-Age {max_age}s, payload {} bytes, attempt {probe_attempts}", response.code(), response.payload().len()).into());
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

// Only the fixed identity/echo probes call this policy, never durable telemetry.
fn probe_retry_at(code: Code, max_age_secs: u32, now_ms: u64, attempts: u8) -> Option<u64> {
    if !matches!(code, Code::TOO_MANY_REQUESTS | Code::SERVICE_UNAVAILABLE) || attempts >= 3 {
        return None;
    }
    let next = now_ms.saturating_add(u64::from(max_age_secs).saturating_mul(1000).max(1));
    (next < 30_000).then_some(next)
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn retry_respects_overload_deadline_and_attempt_bound() {
        assert_eq!(
            probe_retry_at(Code::SERVICE_UNAVAILABLE, 1, 100, 1),
            Some(1100)
        );
        assert_eq!(
            probe_retry_at(Code::TOO_MANY_REQUESTS, 0, 100, 2),
            Some(101)
        );
        assert_eq!(probe_retry_at(Code::SERVICE_UNAVAILABLE, 60, 100, 1), None);
        assert_eq!(
            probe_retry_at(Code::SERVICE_UNAVAILABLE, 1, 29_000, 1),
            None
        );
        assert_eq!(probe_retry_at(Code::SERVICE_UNAVAILABLE, 1, 100, 3), None);
        assert_eq!(probe_retry_at(Code::FORBIDDEN, 0, 100, 1), None);
    }
}
