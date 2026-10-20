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
    ffi::OsStr,
    net::{SocketAddr, UdpSocket},
    path::Path,
    time::{Duration, Instant},
};
#[path = "peer_device_restart.rs"]
mod device_restart;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Connect,
    Identity,
    Echo,
    DeniedEcho,
    /// Explicit qualification fault: exit after one durable journal sync,
    /// before sending its receipt. Never enabled by ordinary peer modes.
    LostReceipt,
    /// Keep this host alive while one new receipt waits for device-only restart.
    DeviceRestart,
}

impl Probe {
    pub fn from_flag(flag: &OsStr) -> Option<Self> {
        match flag.to_str()? {
            "--connect" => Some(Self::Connect),
            "--probe" => Some(Self::Identity),
            "--probe-echo" => Some(Self::Echo),
            "--probe-denied" => Some(Self::DeniedEcho),
            "--qualify-lost-receipt" => Some(Self::LostReceipt),
            "--qualify-device-restart" => Some(Self::DeviceRestart),
            _ => None,
        }
    }

    fn one_shot(self) -> bool {
        matches!(self, Self::Identity | Self::Echo | Self::DeniedEcho)
    }
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
    let persistent = probe.is_none() || probe.is_some_and(|probe| !probe.one_shot());
    let initiates = identity.peer().principal().fingerprint() > peer.principal().fingerprint();
    let permission = pairing
        .checkpoint()
        .policy()
        .ok_or("missing policy")?
        .resource_policy();
    let telemetry = *permission == pairing::permissions(true);
    if matches!(probe, Some(Probe::LostReceipt | Probe::DeviceRestart)) && !telemetry {
        return Err("receipt qualification requires an approved telemetry grant".into());
    }
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
    if matches!(probe, Some(Probe::LostReceipt)) {
        journal
            .as_mut()
            .ok_or("missing telemetry journal")?
            .set_checkpoint(lost_receipt_checkpoint);
    } else if matches!(probe, Some(Probe::DeviceRestart)) {
        journal
            .as_mut()
            .ok_or("missing telemetry journal")?
            .set_checkpoint(device_restart_checkpoint);
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
    // Every mode uses the same role as the automatically recovering device.
    // Simultaneous initiators would contend for the single bootstrap candidate.
    if initiates {
        connection.connect(0)?;
    }
    let mut sample = [0; 36];
    sample[..4].copy_from_slice(&[0x82, 1, 0x58, 32]);
    getrandom::fill(&mut sample[4..])?;
    println!("Pairing restored; waiting for peer at {remote}.");
    let mut liveness = PeerLiveness::default();
    let mut call = None;
    let mut probe_due = 0;
    let mut probe_attempts = 0;
    let mut output = [0; 128];
    let mut retry_at = 30_000;
    let mut observation_reported = false;
    let mut restart =
        matches!(probe, Some(Probe::DeviceRestart)).then(|| device_restart::Qualification::new(0));
    loop {
        let now = started.elapsed().as_millis().try_into()?;
        let was_quiet = liveness.quiet;
        let mut fixture_error = None;
        let result = connection.poll_with(now, |principal, request, _| {
            liveness.authenticated(now);
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
                if restart
                    .as_ref()
                    .is_some_and(|fixture| fixture.expired(now) || !fixture.permits(&operation))
                {
                    return Response::new(Code::SERVICE_UNAVAILABLE).max_age(1);
                }
                let before = if restart.is_some() {
                    match journal.effects() {
                        Ok(count) => count,
                        Err(_) => return Response::new(Code::INTERNAL_SERVER_ERROR),
                    }
                } else {
                    0
                };
                match journal.commit(&anchor, principal, &operation) {
                    Ok(receipt) => {
                        if let Some(fixture) = &mut restart {
                            match fixture.recorded(receipt, before) {
                                Ok(device_restart::Action::Withhold) => {
                                    return Response::new(Code::SERVICE_UNAVAILABLE).max_age(1);
                                }
                                Ok(device_restart::Action::Deliver) => {}
                                Err(error) => {
                                    fixture_error = Some(error);
                                    return Response::new(Code::INTERNAL_SERVER_ERROR);
                                }
                            }
                        }
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
        if let Some(error) = fixture_error {
            return Err(error.into());
        }
        if let Some(fixture) = &mut restart {
            match fixture.after_poll(connection.session_generation())? {
                Some(device_restart::Event::Armed {
                    sequence,
                    generation,
                }) => println!(
                    "Device-restart checkpoint armed: host PID {} remains alive; receipt {sequence} withheld at generation {generation}. Restart only the ESP32 now; preserve this host and journal.",
                    std::process::id()
                ),
                Some(device_restart::Event::Released {
                    sequence,
                    previous,
                    current,
                }) => println!(
                    "Device-restart receipt released: host PID {}, original receipt {sequence}, generation {previous} -> {current}. Confirm device durable completion and quiesce while this host remains alive; a fresh session alone does not prove a device restart.",
                    std::process::id()
                ),
                None => {}
            }
        }
        if liveness.generation(connection.session_generation(), now) {
            println!(
                "Fresh EDHOC/OSCORE session generation {} connected to {}.",
                liveness.generation.expect("connected generation"),
                super::super::hex(peer.principal().fingerprint())
            );
            retry_at = u64::MAX;
        }
        if was_quiet && !liveness.quiet {
            subscription = ManagedSubscription::new("identity");
            observation_reported = false;
        }
        if connection.is_connected()
            && call.is_none()
            && now >= probe_due
            && let Some(probe) = probe.filter(|probe| probe.one_shot())
        {
            let (method, path, payload, format) = match probe {
                Probe::Connect | Probe::LostReceipt | Probe::DeviceRestart => unreachable!(),
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
                        Probe::Connect | Probe::LostReceipt | Probe::DeviceRestart => {
                            unreachable!()
                        }
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
                    if response.observe_seq().is_some() && !observation_reported {
                        println!("Protected identity observation confirmed.");
                        observation_reported = true;
                    }
                    liveness.authenticated(now);
                }
                Ok(None) => {}
                Ok(Some(Ok(_))) => liveness.authenticated(now),
                Ok(Some(Err(_))) => {}
                Err(
                    ManagedError::Busy | ManagedError::NotConnected | ManagedError::StaleHandle,
                ) => {}
                Err(error) => return Err(error.into()),
            }
            if liveness.expired(now) {
                connection.stop_subscription(&mut subscription, now)?;
                liveness.quiet = true;
                retry_at = now;
            }
            if !liveness.quiet {
                match connection.poll_subscription(&mut subscription, now) {
                    Ok(_) => {}
                    Err(ref error) if retryable_subscription(error) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if persistent
            && initiates
            && (liveness.quiet || !connection.is_connected())
            && now >= retry_at
        {
            retry_at = now.saturating_add(match connection.connect(now) {
                Ok(()) => 30_000,
                Err(ManagedError::Busy | ManagedError::Capacity) => 1_000,
                Err(ManagedError::Recovery(_)) => 5_000,
                Err(error) => return Err(error.into()),
            });
        }
        if let Some(fixture) = &restart
            && fixture.expired(now)
        {
            if fixture.released() {
                println!(
                    "Device-restart qualification window ended. Device completion requires the separately recorded inspection."
                );
                return Ok(());
            }
            return Err("device-restart qualification reached its90-second deadline before receipt release; preserve the journal and device Pending".into());
        }
        if probe.is_some_and(Probe::one_shot) && started.elapsed() >= Duration::from_secs(30) {
            return Err("paired probe deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn device_restart_checkpoint(stage: coaptic_receipts::Stage) -> std::io::Result<()> {
    if stage == coaptic_receipts::Stage::AfterSync {
        println!(
            "Device-restart qualification: new effect synced. Wait for the armed checkpoint before restarting the device."
        );
    }
    Ok(())
}

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}

// A peer's temporary overload may place identity registration in pre-send
// cooldown. Keep the host owner alive; post-send commit uncertainty stays fatal.
fn retryable_subscription<E>(error: &ManagedError<E>) -> bool {
    matches!(
        error,
        ManagedError::Busy
            | ManagedError::NotConnected
            | ManagedError::Application(
                coaptic::app::Error::Io(_)
                    | coaptic::app::Error::Saturated
                    | coaptic::app::Error::Backoff { .. }
            )
    )
}

// Only authenticated peer traffic establishes liveness. Application refusal or
// an unaccepted Observe response leaves retry scheduling with ManagedSubscription.
#[derive(Default)]
struct PeerLiveness {
    last_authenticated: u64,
    generation: Option<u64>,
    quiet: bool,
}
impl PeerLiveness {
    fn authenticated(&mut self, now: u64) {
        self.last_authenticated = now;
        self.quiet = false;
    }

    fn generation(&mut self, generation: Option<u64>, now: u64) -> bool {
        if generation.is_some() && generation != self.generation {
            self.generation = generation;
            self.authenticated(now);
            true
        } else {
            false
        }
    }

    fn expired(&self, now: u64) -> bool {
        self.generation.is_some()
            && !self.quiet
            && now.saturating_sub(self.last_authenticated) >= 30_000
    }
}

// Process termination is deliberately before receipt construction. The journal
// has synchronized the complete effect; a new process must reopen and retry it.
fn lost_receipt_checkpoint(stage: coaptic_receipts::Stage) -> std::io::Result<()> {
    if stage == coaptic_receipts::Stage::AfterSync {
        use std::io::Write;
        println!("Qualification checkpoint: durable effect synced; exiting75 before receipt.");
        std::io::stdout().flush()?;
        std::process::exit(75);
    }
    Ok(())
}

// Only the fixed identity/echo probes call this policy, never durable telemetry.
pub(super) fn probe_retry_at(
    code: Code,
    max_age_secs: u32,
    now_ms: u64,
    attempts: u8,
) -> Option<u64> {
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
    fn device_restart_subscription_cooldown_keeps_owner_but_uncertain_commit_stops() {
        assert!(retryable_subscription(&ManagedError::<()>::Application(
            coaptic::app::Error::Backoff { until_ms: 42_000 }
        )));
        assert!(!retryable_subscription(&ManagedError::<()>::Application(
            coaptic::app::Error::CommitAfterSend
        )));
        assert!(!retryable_subscription(&ManagedError::<()>::Clock));
    }

    #[test]
    fn device_restart_mode_is_explicit_and_not_an_ordinary_probe() {
        assert_eq!(
            Probe::from_flag(OsStr::new("--qualify-device-restart")),
            Some(Probe::DeviceRestart)
        );
        assert!(!Probe::DeviceRestart.one_shot());
        assert_eq!(Probe::from_flag(OsStr::new("--device-restart")), None);
        assert_eq!(
            Probe::from_flag(OsStr::new("--qualify-device-restart=yes")),
            None
        );
        assert_eq!(
            Probe::from_flag(OsStr::new("--connect")),
            Some(Probe::Connect)
        );
        assert_eq!(
            Probe::from_flag(OsStr::new("--qualify-lost-receipt")),
            Some(Probe::LostReceipt)
        );
        assert_eq!(
            Probe::from_flag(OsStr::new("--probe-denied")),
            Some(Probe::DeniedEcho)
        );
        assert!(Probe::DeniedEcho.one_shot());
    }

    #[test]
    fn device_restart_qualification_replays_existing_journal_row_without_another_effect() {
        let policy = fault_policy();
        let path = std::env::temp_dir().join(format!(
            "coaptic-device-restart-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut journal = Journal::create(&path, Authority::new(policy), 2).unwrap();
        let old = fault_operation();
        let old_receipt = journal
            .commit(&policy.anchor, &policy.principal, &old)
            .unwrap();
        let mut fixture = device_restart::Qualification::new(0);
        assert_eq!(
            fixture.recorded(old_receipt, journal.effects().unwrap()),
            Ok(device_restart::Action::Deliver)
        );
        let fresh = TelemetryOperation::new(
            OperationId::new([10; 16]),
            policy.resource,
            Some(60),
            b"new selected effect",
        )
        .unwrap();
        let before = journal.effects().unwrap();
        journal.set_checkpoint(device_restart_checkpoint);
        let selected = journal
            .commit(&policy.anchor, &policy.principal, &fresh)
            .unwrap();
        drop(journal);
        let bytes = std::fs::read(&path).unwrap();
        let mut journal = Journal::open(&path, Authority::new(policy)).unwrap();
        assert_eq!(
            fixture.recorded(selected, before),
            Ok(device_restart::Action::Withhold)
        );
        fixture.after_poll(Some(4)).unwrap();
        for generation in [None, Some(4), Some(5), Some(5)] {
            fixture.after_poll(generation).unwrap();
            let before = journal.effects().unwrap();
            let actual = journal
                .commit(&policy.anchor, &policy.principal, &fresh)
                .unwrap();
            assert_eq!(actual, selected);
            assert_eq!(
                fixture.recorded(actual, before),
                Ok(if fixture.released() {
                    device_restart::Action::Deliver
                } else {
                    device_restart::Action::Withhold
                })
            );
            assert_eq!(journal.effects().unwrap(), 2);
            // Windows exclusive locks forbid opening another path handle.
            // Only this unit-test snapshot releases/reopens journal custody.
            drop(journal);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            journal = Journal::open(&path, Authority::new(policy)).unwrap();
        }
        assert!(!fixture.permits(&old));
        drop(journal);
        std::fs::remove_file(path).unwrap();
    }

    fn fault_policy() -> Policy {
        let identity = Identity::from_private_key([1; 32], 1).unwrap();
        Policy {
            anchor: coaptic::provisioning::TrustAnchor::genesis([3; 32]).unwrap(),
            principal: identity.peer().principal(),
            resource: [7; 32],
            enabled: true,
        }
    }
    fn fault_operation() -> TelemetryOperation<'static> {
        TelemetryOperation::new(
            OperationId::new([9; 16]),
            [7; 32],
            Some(60),
            b"complete effect",
        )
        .unwrap()
    }
    #[test]
    fn after_sync_process_child() {
        let Some(path) = std::env::var_os("COAPTIC_RECEIPT_FAULT_CHILD") else {
            return;
        };
        let policy = fault_policy();
        let mut journal = Journal::open(Path::new(&path), Authority::new(policy)).unwrap();
        journal.set_checkpoint(lost_receipt_checkpoint);
        let _ = journal.commit(&policy.anchor, &policy.principal, &fault_operation());
        panic!("AfterSync fault did not terminate before receipt");
    }
    #[test]
    fn explicit_after_sync_fault_preserves_one_effect_across_process_restart() {
        let path = std::env::temp_dir().join(format!(
            "coaptic-peer-fault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let policy = fault_policy();
        drop(Journal::create(&path, Authority::new(policy), 2).unwrap());
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pairing_cli::peer::retry_tests::after_sync_process_child",
                "--nocapture",
            ])
            .env("COAPTIC_RECEIPT_FAULT_CHILD", &path)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(75));
        assert!(String::from_utf8_lossy(&result.stdout).contains("durable effect synced"));
        let mut journal = Journal::open(&path, Authority::new(policy)).unwrap();
        assert_eq!(journal.effects().unwrap(), 1);
        let receipt = journal
            .commit(&policy.anchor, &policy.principal, &fault_operation())
            .unwrap();
        assert_eq!(receipt.sequence(), 1);
        assert_eq!(journal.effects().unwrap(), 1);
        drop(journal);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn authenticated_activity_prevents_rekey_and_silence_quiesces() {
        assert!(core::mem::size_of::<PeerLiveness>() <= 32);
        let mut state = PeerLiveness::default();
        assert!(!state.expired(60_000));
        assert!(state.generation(Some(1), 0));
        for now in (5_000..=90_000).step_by(5_000) {
            assert!(!state.expired(now));
            state.authenticated(now);
            assert!(!state.generation(Some(1), now));
        }
        assert!(!state.expired(119_999));
        assert!(state.expired(120_000));
        state.quiet = true;
        assert!(!state.generation(None, 121_000));
        assert!(!state.generation(Some(1), 122_000));
        assert!(state.quiet);
        assert!(state.generation(Some(2), 123_000));
        assert!(!state.quiet);
        state.quiet = true;
        state.authenticated(124_000);
        assert!(!state.quiet);
    }

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
