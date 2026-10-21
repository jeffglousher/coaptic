//! Bounded timings for one paired host's protected identity reads.
//!
//! Durable pairing is read under the caller's existing exclusive lock. This
//! fixture never opens a receipt journal or requests a management operation.
//! Fresh EDHOC session setup is outside request intervals; runtime OSCORE state
//! is volatile. This is not a durable per-request I/O or throughput benchmark.

use coaptic::{
    Code, Endpoint, Method, Response, profiles,
    provisioning::{Identity, LocalPairing, ManagedConnection, ManagedError},
    storage::{MemoryProfile, UdpSocketIo},
};
use std::{
    io::{self, Write},
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

const WARMUPS: usize = 4;
const MEASURED: usize = 24;
const OPERATIONS: usize = WARMUPS + MEASURED;
const EVENTS: usize = 64;
const RUN_NS: u64 = 30_000_000_000;
const OPERATION_NS: u64 = 5_000_000_000;
const NS_PER_MS: u64 = 1_000_000;
const POLL_WAIT: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    attempts: u16,
    accepted: u16,
    refusals: u16,
    cooldowns: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    start_ns: u64,
    elapsed_ns: u64,
    counts: Counts,
}

#[derive(Clone, Copy, Debug, Default)]
struct Active {
    start_ns: u64,
    counts: Counts,
    waiting: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Event {
    kind: &'static str,
    operation: usize,
    at_ns: u64,
    until_ms: u64,
    code: u8,
    max_age_secs: u32,
}

struct Timing {
    generation: Option<u64>,
    connected_ns: Option<u64>,
    last_ns: u64,
    due_ms: u64,
    active: Option<Active>,
    samples: [Sample; OPERATIONS],
    completed: usize,
    events: [Event; EVENTS],
    event_count: usize,
    failure: Option<&'static str>,
}

// Metadata has its own whole-owner ceiling; it is not a packet pool allowance.
const _: () = assert!(std::mem::size_of::<Timing>() <= 6 * 1024);

impl Timing {
    fn new() -> Self {
        Self {
            generation: None,
            connected_ns: None,
            last_ns: 0,
            due_ms: 0,
            active: None,
            samples: [Sample::default(); OPERATIONS],
            completed: 0,
            events: [Event::default(); EVENTS],
            event_count: 0,
            failure: None,
        }
    }

    fn fail<T>(&mut self, message: &'static str) -> Result<T, &'static str> {
        self.failure = Some(message);
        Err(message)
    }

    fn time(&mut self, ns: u64) -> Result<(), &'static str> {
        if self.failure.is_some() {
            return Err("timing run already failed");
        }
        if ns < self.last_ns {
            return self.fail("timing clock regressed");
        }
        self.last_ns = ns;
        if ns >= RUN_NS {
            return self.fail("paired timing reached30-second run deadline");
        }
        if self
            .active
            .is_some_and(|active| ns - active.start_ns >= OPERATION_NS)
        {
            return self.fail("paired timing reached five-second operation deadline");
        }
        Ok(())
    }

    fn check(&mut self, ns: u64, generation: Option<u64>) -> Result<(), &'static str> {
        self.time(ns)?;
        if let Some(selected) = self.generation {
            if generation != Some(selected) {
                return self.fail("paired timing session changed or disconnected");
            }
        } else if let Some(generation) = generation {
            self.generation = Some(generation);
            self.connected_ns = Some(ns);
        }
        Ok(())
    }

    fn ready(&self, now_ms: u64) -> bool {
        self.generation.is_some()
            && self.completed < OPERATIONS
            && self.active.is_none_or(|active| !active.waiting)
            && now_ms >= self.due_ms
    }

    fn attempt(&mut self, ns: u64) -> Result<(), &'static str> {
        self.time(ns)?;
        if !self.ready(ns / NS_PER_MS) {
            return self.fail("timing request attempted without ready credit");
        }
        let active = self.active.get_or_insert(Active {
            start_ns: ns,
            ..Active::default()
        });
        active.counts.attempts = active
            .counts
            .attempts
            .checked_add(1)
            .ok_or("timing request attempt counter exhausted")?;
        Ok(())
    }

    fn admitted(&mut self, ns: u64) -> Result<(), &'static str> {
        self.time(ns)?;
        let active = self.active.as_mut().ok_or("no active timing operation")?;
        active.counts.accepted += 1;
        active.waiting = true;
        Ok(())
    }

    fn event(
        &mut self,
        kind: &'static str,
        ns: u64,
        until_ms: u64,
        code: Code,
        max_age_secs: u32,
    ) -> Result<(), &'static str> {
        if self.event_count == EVENTS {
            return self.fail("timing refusal/cooldown evidence capacity exhausted");
        }
        self.events[self.event_count] = Event {
            kind,
            operation: self.completed,
            at_ns: ns,
            until_ms,
            code: code.as_raw(),
            max_age_secs,
        };
        self.event_count += 1;
        self.due_ms = until_ms;
        Ok(())
    }

    fn cooldown(&mut self, ns: u64, until_ms: u64) -> Result<(), &'static str> {
        self.time(ns)?;
        if until_ms <= ns / NS_PER_MS {
            return self.fail("timing local cooldown did not advance");
        }
        let active = self.active.as_mut().ok_or("no active timing operation")?;
        active.counts.cooldowns += 1;
        self.event("local-cooldown", ns, until_ms, Code::EMPTY, 0)
    }

    fn response(
        &mut self,
        ns: u64,
        code: Code,
        exact_identity: bool,
        max_age_secs: u32,
    ) -> Result<(), &'static str> {
        self.time(ns)?;
        let Some(active) = &mut self.active else {
            return self.fail("timing response has no active operation");
        };
        if !active.waiting {
            return self.fail("timing response was not awaited");
        }
        if code == Code::CONTENT && exact_identity {
            self.samples[self.completed] = Sample {
                start_ns: active.start_ns,
                elapsed_ns: ns - active.start_ns,
                counts: active.counts,
            };
            self.completed += 1;
            self.active = None;
            self.due_ms = ns / NS_PER_MS;
            return Ok(());
        }
        if matches!(code, Code::TOO_MANY_REQUESTS | Code::SERVICE_UNAVAILABLE) {
            active.counts.refusals += 1;
            active.waiting = false;
            let until_ms = (ns / NS_PER_MS)
                .checked_add((u64::from(max_age_secs) * 1000).max(1))
                .ok_or("timing overload deadline overflow")?;
            return self.event("authenticated-refusal", ns, until_ms, code, max_age_secs);
        }
        if code.class() >= 4 {
            active.counts.refusals += 1;
        }
        self.event(
            if code.class() >= 4 {
                "authenticated-refusal"
            } else {
                "invalid-response"
            },
            ns,
            ns / NS_PER_MS,
            code,
            max_age_secs,
        )?;
        self.fail("timing response status or complete identity differs")
    }

    fn complete(&self) -> bool {
        self.failure.is_none() && self.completed == OPERATIONS && self.active.is_none()
    }

    fn report(
        &mut self,
        passed: bool,
        elapsed_ns: u64,
        managed_owner_bytes: usize,
        mut out: impl Write,
    ) -> io::Result<()> {
        writeln!(
            out,
            "COAPTIC_PAIRED_TIMING_LAYOUT {{\"timing_metadata_owner_bytes\":{},\"timing_metadata_ceiling_bytes\":6144,\"sample_limit\":28,\"event_limit\":64,\"response_buffer_bytes\":32,\"managed_owner_constructed\":{},\"managed_owner_bytes_including_adapter\":{},\"udp_adapter_bytes\":{},\"adapter_scratch_bytes\":1473,\"scope\":\"Timing metadata and response output are separate; adapter is nested in Managed owner; no additive stack or whole-process RAM claim\"}}",
            std::mem::size_of::<Self>(),
            managed_owner_bytes != 0,
            managed_owner_bytes,
            std::mem::size_of::<UdpSocketIo<[u8; 1473]>>()
        )?;
        writeln!(
            out,
            "COAPTIC_PAIRED_TIMING_META {{\"schema\":\"coaptic-paired-timing/1\",\"passed\":{},\"method\":\"GET\",\"resource\":\"identity\",\"response_bytes\":32,\"warmups\":4,\"measured\":24,\"concurrency\":1,\"generation\":{},\"session_established\":{},\"setup_ns\":{},\"setup_scope\":\"ManagedConnection construction and EDHOC establishment; prior CLI restore/socket binding excluded\",\"elapsed_ns\":{},\"poll_wait_ms\":2,\"host_profile\":\"Constrained\",\"host_rx_slots\":2,\"host_tx_slots\":2,\"datagram_bytes\":1152,\"blockwise\":false,\"routes\":8,\"calls\":4,\"deferred\":0,\"security_bindings\":4,\"associations\":1,\"recovery_caches\":2,\"security\":\"fresh EDHOC/OSCORE session; durable pairing reused\",\"scope\":\"complete host-observed logical operation including admission, authentication, retries, cooldown and polling wait; no per-request durable storage or library-only latency claim\"}}",
            passed && self.complete(),
            self.generation.unwrap_or(0),
            self.generation.is_some(),
            self.connected_ns.unwrap_or(0),
            elapsed_ns
        )?;
        for (index, sample) in self.samples[..self.completed].iter().enumerate() {
            writeln!(
                out,
                "COAPTIC_PAIRED_TIMING_SAMPLE {{\"index\":{index},\"warmup\":{},\"start_ns\":{},\"elapsed_ns\":{},\"request_attempts\":{},\"accepted_requests\":{},\"authenticated_refusals\":{},\"local_cooldowns\":{}}}",
                index < WARMUPS,
                sample.start_ns,
                sample.elapsed_ns,
                sample.counts.attempts,
                sample.counts.accepted,
                sample.counts.refusals,
                sample.counts.cooldowns
            )?;
        }
        for event in &self.events[..self.event_count] {
            writeln!(
                out,
                "COAPTIC_PAIRED_TIMING_EVENT {{\"operation\":{},\"kind\":\"{}\",\"at_ns\":{},\"retry_at_ms\":{},\"code\":{},\"max_age_secs\":{}}}",
                event.operation,
                event.kind,
                event.at_ns,
                event.until_ms,
                event.code,
                event.max_age_secs
            )?;
        }
        if let Some(active) = self.active {
            writeln!(
                out,
                "COAPTIC_PAIRED_TIMING_PARTIAL {{\"index\":{},\"start_ns\":{},\"request_attempts\":{},\"accepted_requests\":{},\"authenticated_refusals\":{},\"local_cooldowns\":{}}}",
                self.completed,
                active.start_ns,
                active.counts.attempts,
                active.counts.accepted,
                active.counts.refusals,
                active.counts.cooldowns
            )?;
        }
        if passed && self.complete() {
            // Raw rows have been emitted in operation order. Reuse their storage
            // for quantiles rather than retaining a second timing array.
            let samples = &mut self.samples[WARMUPS..];
            samples.sort_unstable_by_key(|sample| sample.elapsed_ns);
            let mean =
                samples.iter().map(|sample| sample.elapsed_ns).sum::<u64>() / MEASURED as u64;
            writeln!(
                out,
                "COAPTIC_PAIRED_TIMING_RESULT {{\"passed\":true,\"completed\":28,\"measured\":24,\"min_ns\":{},\"mean_ns\":{mean},\"p50_nearest_rank_ns\":{},\"p95_nearest_rank_ns\":{},\"max_ns\":{}}}",
                samples[0].elapsed_ns,
                samples[MEASURED.div_ceil(2) - 1].elapsed_ns,
                samples[(MEASURED * 95).div_ceil(100) - 1].elapsed_ns,
                samples[MEASURED - 1].elapsed_ns
            )?;
        } else {
            writeln!(
                out,
                "COAPTIC_PAIRED_TIMING_RESULT {{\"passed\":false,\"completed\":{},\"failure\":\"{}\"}}",
                self.completed,
                self.failure
                    .unwrap_or("transport, trust or session operation failed")
            )?;
        }
        Ok(())
    }
}

fn elapsed(epoch: Instant) -> Result<u64, &'static str> {
    epoch
        .elapsed()
        .as_nanos()
        .try_into()
        .map_err(|_| "timing clock exceeds nanosecond range")
}

pub(super) fn run(
    identity: &Identity,
    pairing: &LocalPairing,
    local: &str,
    remote: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer = pairing.peer()?;
    let permissions = pairing.checkpoint();
    let permissions = permissions
        .policy()
        .ok_or("missing paired timing policy")?
        .resource_policy();
    if *permissions != coaptic_enroll::pairing::echo_permissions() {
        return Err("paired timing requires the existing echo-only grant; no permission change is performed".into());
    }
    let socket = UdpSocket::bind(local)?;
    socket.set_nonblocking(true)?;
    let endpoint = Endpoint::from(remote.parse::<SocketAddr>()?);
    let io = UdpSocketIo::new(socket, [0; 1473])?;
    let epoch = Instant::now();
    let mut timing = Timing::new();
    let mut managed_owner_bytes = 0;
    let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut connection = ManagedConnection::<_, profiles::Constrained>::from_local_pairing(
            identity,
            pairing,
            endpoint,
            io,
            super::entropy,
            0,
        )?;
        managed_owner_bytes = std::mem::size_of_val(&connection);
        connection.observable_manual("identity")?;
        if identity.peer().principal().fingerprint() > peer.principal().fingerprint() {
            connection.connect(0)?;
        }
        let mut call = None;
        let mut output = [0; 32];
        let mut configuration_checked = false;
        // An independent iteration bound also refuses a frozen timing source.
        for _ in 0..30_000 {
            let ns = elapsed(epoch)?;
            timing.check(ns, connection.session_generation())?;
            let now = ns / NS_PER_MS;
            match connection.poll_with(now, |_, request, _| {
                if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                    let response =
                        Response::content_copy(identity.peer().principal().fingerprint())
                            .max_age(10);
                    if request.is_observe_register() {
                        response.observe(0)
                    } else {
                        response
                    }
                } else if request.path() == ["echo"] && request.method() == Some(Method::Post) {
                    if request.payload().len() <= 128 {
                        Response::content_copy(request.payload())
                    } else {
                        Response::new(Code::REQUEST_ENTITY_TOO_LARGE)
                    }
                } else {
                    // In particular, timing never accepts telemetry effects.
                    Response::new(Code::FORBIDDEN)
                }
            }) {
                Ok(()) | Err(ManagedError::Recovery(_)) => {}
                Err(error) => return Err(error.into()),
            }
            timing.check(elapsed(epoch)?, connection.session_generation())?;
            if timing.generation.is_some() && !configuration_checked {
                let usage = connection
                    .diagnostic_snapshot(now)
                    .ok_or("timing established owner has no diagnostic snapshot")?
                    .usage;
                if usage.storage.rx_datagram.capacity != profiles::Constrained::RX_DATAGRAM_SLOTS
                    || usage.storage.tx_datagram.capacity
                        != profiles::Constrained::TX_DATAGRAM_SLOTS
                    || usage.storage.rx_body.is_some()
                    || usage.storage.tx_body.is_some()
                    || usage.routes.capacity != 8
                    || usage.client_requests.capacity != 4
                    || usage.client_replies.capacity != 4
                    || usage.deferred.capacity != 0
                    || usage.security.is_none_or(|security| security.capacity != 1)
                {
                    return Err(
                        "paired timing host configuration differs from declared profile".into(),
                    );
                }
                configuration_checked = true;
            }
            if let Some(current) = call
                && let Some(response) = connection.take_response_into(current, &mut output, now)?
            {
                let response = response?;
                let exact = response.payload() == peer.principal().fingerprint();
                let max_age = response.max_age_secs().unwrap_or(60);
                // Capture the end after complete-byte verification, retaining
                // the original start across authenticated retry/cooldown.
                let ns = elapsed(epoch)?;
                timing.response(ns, response.code(), exact, max_age)?;
                call = None;
            }
            if timing.complete() {
                return Ok(());
            }
            let ns = elapsed(epoch)?;
            timing.check(ns, connection.session_generation())?;
            if call.is_none() && timing.ready(ns / NS_PER_MS) {
                // This timestamp precedes request preparation and admission.
                timing.attempt(ns)?;
                match connection.request(Method::Get, "identity", &[], None, ns / NS_PER_MS) {
                    Ok(request) => {
                        timing.admitted(elapsed(epoch)?)?;
                        call = Some(request);
                    }
                    Err(ManagedError::Application(coaptic::app::Error::Backoff { until_ms })) => {
                        timing.cooldown(elapsed(epoch)?, until_ms)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            std::thread::sleep(POLL_WAIT);
        }
        Err("paired timing polling bound reached".into())
    })();
    timing.report(
        outcome.is_ok(),
        elapsed(epoch)?,
        managed_owner_bytes,
        io::stdout().lock(),
    )?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_and_local_cooldown_keep_original_logical_operation_start() {
        let mut timing = Timing::new();
        timing.check(100, Some(7)).unwrap();
        timing.attempt(200).unwrap();
        timing.admitted(300).unwrap();
        timing
            .response(1_000_000, Code::SERVICE_UNAVAILABLE, false, 1)
            .unwrap();
        assert!(!timing.ready(1000));
        timing.attempt(1_001_000_000).unwrap();
        timing.cooldown(1_001_100_000, 1002).unwrap();
        timing.attempt(1_002_000_000).unwrap();
        timing.admitted(1_002_100_000).unwrap();
        timing
            .response(1_003_000_000, Code::CONTENT, true, 0)
            .unwrap();
        assert_eq!(timing.completed, 1);
        assert_eq!(timing.samples[0].start_ns, 200);
        assert_eq!(timing.samples[0].elapsed_ns, 1_002_999_800);
        assert_eq!(timing.samples[0].counts.attempts, 3);
        assert_eq!(timing.samples[0].counts.accepted, 2);
        assert_eq!(timing.samples[0].counts.refusals, 1);
        assert_eq!(timing.samples[0].counts.cooldowns, 1);
        assert_eq!(timing.events[0].code, Code::SERVICE_UNAVAILABLE.as_raw());
        assert_eq!(timing.events[1].code, Code::EMPTY.as_raw());
    }

    #[test]
    fn generation_loss_change_regression_and_deadlines_cannot_pass() {
        for generation in [None, Some(8)] {
            let mut timing = Timing::new();
            timing.check(10, Some(7)).unwrap();
            assert!(timing.check(11, generation).is_err());
            assert!(!timing.complete());
        }
        let mut timing = Timing::new();
        assert!(timing.check(RUN_NS, None).is_err());
        let mut timing = Timing::new();
        timing.check(10, Some(7)).unwrap();
        timing.attempt(20).unwrap();
        assert!(timing.check(20 + OPERATION_NS, Some(7)).is_err());
        let mut timing = Timing::new();
        timing.check(10, Some(7)).unwrap();
        assert!(timing.check(9, Some(7)).is_err());
    }

    #[test]
    fn exact_finite_completion_is_required_for_success_summary() {
        let mut timing = Timing::new();
        timing.check(1, Some(7)).unwrap();
        for index in 0..OPERATIONS {
            let ns = (index as u64 + 1) * NS_PER_MS;
            timing.attempt(ns).unwrap();
            timing.admitted(ns + 1).unwrap();
            timing.response(ns + 2, Code::CONTENT, true, 0).unwrap();
            assert_eq!(timing.complete(), index + 1 == OPERATIONS);
        }
        assert!(!timing.ready(1000));
        let mut output = Vec::new();
        timing
            .report(true, 100 * NS_PER_MS, 0, &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("COAPTIC_PAIRED_TIMING_SAMPLE"))
                .count(),
            OPERATIONS
        );
        assert_eq!(output.matches("\"warmup\":true").count(), WARMUPS);
        assert!(output.contains("COAPTIC_PAIRED_TIMING_RESULT {\"passed\":true,\"completed\":28"));
        assert!(timing.attempt(100 * NS_PER_MS).is_err());
        let mut output = Vec::new();
        timing
            .report(true, 100 * NS_PER_MS, 0, &mut output)
            .unwrap();
        assert!(
            !String::from_utf8(output)
                .unwrap()
                .contains("\"passed\":true")
        );
    }

    #[test]
    fn wrong_body_or_status_and_evidence_overflow_refuse() {
        for (code, exact) in [(Code::CONTENT, false), (Code::FORBIDDEN, true)] {
            let mut timing = Timing::new();
            timing.check(1, Some(7)).unwrap();
            timing.attempt(2).unwrap();
            timing.admitted(3).unwrap();
            assert!(timing.response(4, code, exact, 0).is_err());
            assert!(!timing.complete());
        }
        let mut timing = Timing::new();
        timing.check(1, Some(7)).unwrap();
        timing.attempt(2).unwrap();
        timing.event_count = EVENTS;
        assert!(timing.cooldown(3, 1).is_err());
        assert!(!timing.complete());
    }
}
