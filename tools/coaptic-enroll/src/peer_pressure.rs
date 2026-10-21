//! Finite installed TX admission pressure with a lost echo ACK, not a packet queue.

use coaptic::{
    Code, ContentFormat, Endpoint, Method, Response,
    message::{Type, decode},
    profiles,
    provisioning::{Identity, LocalPairing, ManagedCall, ManagedConnection, ManagedError},
    storage::{DatagramIo, DatagramTransport, UdpSocketIo},
};
use coaptic_enroll::pairing;
use std::{
    cell::Cell,
    io,
    net::UdpSocket,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CYCLES: u8 = 3;
const RUN_LIMIT_MS: u64 = 45_000;
const RELEASE_BEFORE_MS: u64 = 4_000;
const RECOVER_BEFORE_MS: u64 = 5_000;
const SETTLE_MS: u64 = 300;
// This installed service starts its next Call 5s after collecting an echo reply,
// polls to a 250ms deadline, retries at 2..3s, and cancels at 10s. The before-send
// anchor and bounded first arrival exclude a backlogged, nearly expired Call.
const FIRST_MIN_MS: u64 = 4_900;
const FIRST_MAX_MS: u64 = 5_500;
const RETRY_MIN_MS: u64 = 1_500;
const RETRY_MAX_MS: u64 = 3_500;
const RETRY_FROM_ANCHOR_MAX_MS: u64 = 8_800;
const TARGET_SERVICE_SHA256: &str =
    "0ec66bd4f946fe826c3451c861c6269a9893624c2e9259287aed12022284e930";

fn selected_echo(payload: &[u8], principal: &[u8; 32]) -> bool {
    payload.len() == 45
        && payload[..4] == [0x83, 1, 0x58, 32]
        && payload[4..36] == *principal
        && payload[36] == 0x1b
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    ControlWaiting,
    ControlHeld,
    ControlRelease,
    Waiting,
    Held,
    Release,
    Recovering,
    Done,
}

#[derive(Clone, Copy, Default)]
struct State {
    last_hash: [u8; 32],
    selected_hash: [u8; 32],
    last_token: [u8; 8],
    token: [u8; 8],
    first_at: u64,
    retry_at: u64,
    anchor: u64,
    release_at: u64,
    cooldown_at: u64,
    generation: u64,
    last_mid: u16,
    mid: u16,
    phase: Phase,
    cycles: u8,
    dropped: u8,
    malformed_sent: u8,
    last_token_len: u8,
    token_len: u8,
    last_valid: bool,
}

impl State {
    fn received(&mut self, bytes: &[u8], now: u64) -> std::result::Result<(), &'static str> {
        self.last_hash = *blake3::hash(bytes).as_bytes();
        self.last_valid = false;
        if let Ok(packet) = decode(bytes)
            && packet.ty() == Type::Confirmable
            && packet.oscore().is_some()
        {
            let token = packet.token();
            let token = token.as_bytes();
            if token.len() <= self.last_token.len() {
                self.last_mid = packet.message_id().get();
                self.last_token_len = token.len() as u8;
                self.last_token[..token.len()].copy_from_slice(token);
                self.last_valid = true;
            }
        }
        if matches!(self.phase, Phase::ControlHeld | Phase::Held)
            && self.last_hash == self.selected_hash
        {
            if !(RETRY_MIN_MS..=RETRY_MAX_MS).contains(&now.saturating_sub(self.first_at)) {
                return Err("selected CON retry is compressed or outside its finite window");
            }
            self.retry_at = now;
            self.phase = if self.phase == Phase::ControlHeld {
                Phase::ControlRelease
            } else {
                if self.cooldown_at == 0
                    || now > self.anchor.saturating_add(RETRY_FROM_ANCHOR_MAX_MS)
                    || now >= self.first_at.saturating_add(RELEASE_BEFORE_MS)
                {
                    return Err("pressure refusal was not verified before the bounded exact retry");
                }
                Phase::Release
            };
        }
        Ok(())
    }

    fn authenticated_echo(
        &mut self,
        mid: u16,
        token: &[u8],
        now: u64,
    ) -> std::result::Result<(), &'static str> {
        if !self.last_valid
            || self.last_mid != mid
            || token != &self.last_token[..usize::from(self.last_token_len)]
        {
            return Err("authenticated echo does not match the captured protected ingress");
        }
        match self.phase {
            Phase::ControlWaiting => self.phase = Phase::ControlHeld,
            Phase::Waiting => {
                if !(FIRST_MIN_MS..=FIRST_MAX_MS).contains(&now.saturating_sub(self.anchor)) {
                    return Err("new target Call age is not bounded by the previous full ACK send");
                }
                self.phase = Phase::Held;
            }
            _ => return Err("another target echo arrived before this pressure cycle completed"),
        }
        self.selected_hash = self.last_hash;
        self.mid = mid;
        self.token_len = self.last_token_len;
        self.token = self.last_token;
        self.first_at = now;
        self.retry_at = 0;
        self.release_at = 0;
        self.cooldown_at = 0;
        self.dropped = 0;
        Ok(())
    }

    fn selected_ack(&self, bytes: &[u8]) -> bool {
        decode(bytes).is_ok_and(|packet| {
            packet.ty() == Type::Acknowledgement
                && packet.message_id().get() == self.mid
                && (packet.token().as_bytes() == &self.token[..usize::from(self.token_len)]
                    || (packet.code() == Code::EMPTY && packet.token().as_bytes().is_empty()))
        })
    }

    fn protected_ack(&self, bytes: &[u8]) -> bool {
        self.selected_ack(bytes)
            && decode(bytes)
                .is_ok_and(|packet| packet.code() != Code::EMPTY && packet.oscore().is_some())
    }

    fn bind_generation(&mut self, generation: u64) -> std::result::Result<(), &'static str> {
        if self.generation != 0 && generation != self.generation {
            return Err("pressure qualification changed its authenticated session generation");
        }
        self.generation = generation;
        Ok(())
    }

    fn refused(
        &mut self,
        code: Code,
        max_age: Option<u32>,
        until: u64,
        now: u64,
    ) -> std::result::Result<(), &'static str> {
        if self.phase != Phase::Held
            || self.dropped == 0
            || self.retry_at != 0
            || self.generation == 0
            || code != Code::SERVICE_UNAVAILABLE
            || max_age != Some(1)
            || until <= now
            || until > now.saturating_add(1_000)
            || now >= self.first_at.saturating_add(RETRY_MIN_MS)
        {
            return Err("expected authenticated 5.03/Max-Age1 during the retained target TX hold");
        }
        self.cooldown_at = until;
        Ok(())
    }

    fn recovered(&mut self, now: u64) -> std::result::Result<(), &'static str> {
        if self.phase != Phase::Recovering
            || self.cycles >= CYCLES
            || self.release_at == 0
            || now < self.cooldown_at
            || now >= self.first_at.saturating_add(RECOVER_BEFORE_MS)
        {
            return Err("pressure recovery missed its same-Call, pre-expiry window");
        }
        self.cycles += 1;
        self.phase = if self.cycles == CYCLES {
            Phase::Done
        } else {
            Phase::Waiting
        };
        Ok(())
    }
}

struct Gate<'a> {
    io: &'a mut UdpSocketIo<[u8; 1473]>,
    state: &'a Cell<State>,
    epoch: &'a Instant,
    peer: &'a Endpoint,
}

const _: () = assert!(std::mem::size_of::<State>() + std::mem::size_of::<Gate<'static>>() <= 192);
const _: () = assert!(
    std::mem::size_of::<ManagedConnection<'static, Gate<'static>, profiles::Constrained>>()
        + std::mem::size_of::<UdpSocketIo<[u8; 1473]>>()
        + std::mem::size_of::<State>()
        + std::mem::size_of::<Option<ManagedCall>>()
        + std::mem::size_of::<Instant>()
        + std::mem::size_of::<Endpoint>()
        + 32
        <= 64 * 1024
);

impl DatagramTransport for Gate<'_> {
    type Error = io::Error;
}

impl DatagramIo for Gate<'_> {
    fn recv(&mut self, out: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        let incoming = self.io.recv(out)?;
        if let Some((len, from)) = incoming
            && from == *self.peer
        {
            let mut state = self.state.get();
            state
                .received(&out[..len], self.epoch.elapsed().as_millis() as u64)
                .map_err(io::Error::other)?;
            self.state.set(state);
        }
        Ok(incoming)
    }

    fn send(&mut self, to: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.state.get();
        if to == *self.peer && state.selected_ack(bytes) {
            if matches!(state.phase, Phase::ControlHeld | Phase::Held) {
                state.dropped = state.dropped.saturating_add(1);
                if state.phase == Phase::Held && state.dropped == 1 {
                    // A one-byte invalid-version datagram is never a CoAP request.
                    // This finite interleaving adds no retained packet storage.
                    for _ in 0..2 {
                        if self.io.send(to, &[0xff])? != 1 {
                            return Err(io::Error::other("partial malformed-datagram send"));
                        }
                        state.malformed_sent += 1;
                    }
                }
                self.state.set(state);
                return Ok(bytes.len()); // Explicit ACK loss, not remote acceptance.
            }
            if matches!(state.phase, Phase::ControlRelease | Phase::Release) {
                if !state.protected_ack(bytes) {
                    return Err(io::Error::other(
                        "empty/unprotected ACK cannot release the hold",
                    ));
                }
                let before = self.epoch.elapsed().as_millis() as u64;
                if state.phase == Phase::Release
                    && before >= state.first_at.saturating_add(RELEASE_BEFORE_MS)
                {
                    return Err(io::Error::other("selected echo ACK release deadline"));
                }
                let sent = self.io.send(to, bytes)?;
                if sent == bytes.len() {
                    let control = state.phase == Phase::ControlRelease;
                    state.anchor = before;
                    state.release_at = before;
                    state.phase = if control {
                        Phase::Waiting
                    } else {
                        Phase::Recovering
                    };
                    self.state.set(state);
                    println!(
                        "Pressure {} ACK accepted: first {} ms; exact retry {} ms; before-send {} ms; generation {}.",
                        if control { "control" } else { "held" },
                        state.first_at,
                        state.retry_at,
                        before,
                        state.generation
                    );
                }
                return Ok(sent);
            }
        }
        self.io.send(to, bytes)
    }
}

pub(crate) fn run(
    identity: &Identity,
    pairing: &LocalPairing,
    local: &str,
    remote: &str,
) -> Result<()> {
    let expected = pairing.peer()?;
    if !pairing.checkpoint().enabled()
        || pairing
            .checkpoint()
            .policy()
            .is_none_or(|policy| *policy.resource_policy() != pairing::echo_permissions())
        || identity.peer().principal().fingerprint() <= expected.principal().fingerprint()
    {
        return Err(
            "pressure qualification requires the current echo-only outbound-initiating host".into(),
        );
    }
    let peer = Endpoint::from(remote.parse::<std::net::SocketAddr>()?);
    let socket = UdpSocket::bind(local)?;
    socket.set_nonblocking(true)?;
    let mut io = UdpSocketIo::new(socket, [0; 1473])?;
    let epoch = Instant::now();
    let state = Cell::new(State::default());
    let gate = Gate {
        io: &mut io,
        state: &state,
        epoch: &epoch,
        peer: &peer,
    };
    let mut connection = ManagedConnection::<_, profiles::Constrained>::from_local_pairing(
        identity,
        pairing,
        peer,
        gate,
        super::entropy,
        0,
    )?;
    connection.observable_manual("identity")?;
    connection.connect(0)?;
    let mut call = None;
    let mut output = [0; 32];
    println!(
        "Pressure target async_service SHA256 {TARGET_SERVICE_SHA256}; expected EnrolledDevice RX3/TX2x1152, Observe2/dedup4, routes8/Calls4/deferred0/security4/association1, blockwise off. External installation receipt required; no firmware attestation."
    );
    println!(
        "Pressure fixed owners: Managed {} bytes (Gate nested); adapter/scratch {}; metadata {}; response {}; Call {}; clock {}; endpoint {}; metadata+Gate ceiling192, named-owner ceiling65536. No new packet queue; not whole-host peak RAM.",
        std::mem::size_of_val(&connection),
        std::mem::size_of::<UdpSocketIo<[u8; 1473]>>(),
        std::mem::size_of::<State>(),
        output.len(),
        std::mem::size_of::<Option<ManagedCall>>(),
        std::mem::size_of::<Instant>(),
        std::mem::size_of::<Endpoint>()
    );
    loop {
        let now = epoch.elapsed().as_millis() as u64;
        if now >= RUN_LIMIT_MS {
            return Err("installed pressure qualification 45-second deadline".into());
        }
        let mut failure = None;
        match connection.poll_with(now, |principal, request, _| {
            if principal != &expected.principal() {
                failure = Some("unexpected authenticated pressure peer");
                return Response::new(Code::FORBIDDEN);
            }
            if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                let response =
                    Response::content_copy(identity.peer().principal().fingerprint()).max_age(10);
                if request.is_observe_register() {
                    response.observe(0)
                } else {
                    response
                }
            } else if request.path() == ["echo"]
                && request.method() == Some(Method::Post)
                && request.ty() == Type::Confirmable
                && request.content_format() == Some(Ok(ContentFormat::CBOR))
                && selected_echo(request.payload(), expected.principal().fingerprint())
            {
                let mut current = state.get();
                if let Err(error) = current.authenticated_echo(
                    request.message_id().get(),
                    request.token().as_bytes(),
                    now,
                ) {
                    failure = Some(error);
                }
                state.set(current);
                Response::content_copy(request.payload())
            } else {
                failure = Some("request differs from the selected installed echo fixture");
                Response::new(Code::NOT_FOUND)
            }
        }) {
            Ok(()) => {}
            Err(ManagedError::Recovery(_)) if state.get().generation == 0 => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(error) = failure {
            return Err(error.into());
        }
        let mut current = state.get();
        current.bind_generation(connection.session_generation().unwrap_or(0))?;
        state.set(current);
        if let Some(active) = call
            && let Some(response) = connection.take_response_into(active, &mut output, now)?
        {
            let response =
                response.map_err(|error| format!("pressure identity Call: {error:?}"))?;
            let code = response.code();
            let max_age = response.max_age_secs();
            let empty = response.payload().is_empty();
            let identity_matches = response.payload() == expected.principal().fingerprint();
            call = None;
            if current.phase == Phase::Held {
                if code != Code::SERVICE_UNAVAILABLE || max_age != Some(1) || !empty {
                    return Err(
                        "held-TX probe did not receive the exact protected admission refusal"
                            .into(),
                    );
                }
                let until = match connection.request(Method::Get, "identity", &[], None, now) {
                    Err(ManagedError::Application(coaptic::app::Error::Backoff { until_ms })) => {
                        until_ms
                    }
                    Ok(unexpected) => {
                        connection.cancel(unexpected, now)?;
                        return Err("5.03 did not impose its required local cooldown".into());
                    }
                    Err(error) => return Err(error.into()),
                };
                current.refused(code, max_age, until, now)?;
                println!(
                    "Pressure cycle {} refused: authenticated5.03/Max-Age1 at {} ms; local Backoff until {} ms; first {} ms; generation {}.",
                    current.cycles + 1,
                    now,
                    until,
                    current.first_at,
                    current.generation
                );
            } else if current.phase == Phase::Recovering
                && code == Code::CONTENT
                && identity_matches
            {
                current.recovered(now)?;
                println!(
                    "Pressure cycle {} recovered: exact protected identity2.05 at {} ms; first {} ms; ACK {} ms; cooldown {} ms; generation {}.",
                    current.cycles,
                    now,
                    current.first_at,
                    current.release_at,
                    current.cooldown_at,
                    current.generation
                );
            } else {
                return Err(
                    "pressure identity response differs from the required phase/code/payload"
                        .into(),
                );
            }
            state.set(current);
        }
        if current.phase == Phase::Done {
            if current.cycles != CYCLES || current.malformed_sent != CYCLES * 2 {
                return Err("incomplete finite pressure/churn counts".into());
            }
            println!(
                "Installed pressure passed: {CYCLES} authenticated refusals, cooldown refusals and same-session recoveries; {} malformed1B datagrams sent; generation {}; no telemetry or management changes. Selected retained-TX admission/reuse only, not every queue, ingress-rate fairness, installed NSTART refusal or memory peak.",
                current.malformed_sent, current.generation
            );
            return Ok(());
        }
        if matches!(
            current.phase,
            Phase::Held | Phase::Release | Phase::Recovering
        ) && now >= current.first_at.saturating_add(RECOVER_BEFORE_MS)
        {
            return Err("pressure cycle missed the bounded pre-expiry recovery window".into());
        }
        if call.is_none()
            && connection.is_connected()
            && ((current.phase == Phase::Held && current.cooldown_at == 0)
                || (current.phase == Phase::Recovering
                    && now >= current.cooldown_at
                    && now >= current.release_at.saturating_add(SETTLE_MS)))
        {
            call = Some(connection.request(Method::Get, "identity", &[], None, now)?);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::message::{Message, MessageId, Opt, Token, encode};

    fn packet(ty: Type, mid: u16, protected: bool) -> Vec<u8> {
        let options = [Opt::oscore(&[])];
        let message = Message::new(
            ty,
            if ty == Type::Confirmable {
                Code::POST
            } else {
                Code::CONTENT
            },
            MessageId::new(mid),
        )
        .with_token(Token::EMPTY)
        .with_options(if protected { &options } else { &[] })
        .with_payload(b"tag");
        let mut out = [0; 32];
        let len = encode(&message, &mut out).unwrap();
        out[..len].to_vec()
    }

    fn held() -> State {
        let mut state = State {
            phase: Phase::Waiting,
            anchor: 2_000,
            generation: 1,
            ..State::default()
        };
        state
            .received(&packet(Type::Confirmable, 11, true), 7_000)
            .unwrap();
        state.authenticated_echo(11, &[], 7_000).unwrap();
        state.dropped = 1;
        state
    }

    #[test]
    fn pressure_requires_refusal_before_exact_retry_then_bounded_recovery() {
        let mut state = held();
        state
            .refused(Code::SERVICE_UNAVAILABLE, Some(1), 8_100, 7_100)
            .unwrap();
        state
            .received(&packet(Type::Confirmable, 12, true), 8_000)
            .unwrap();
        assert_eq!(state.phase, Phase::Held);
        state
            .received(&packet(Type::Confirmable, 11, true), 9_000)
            .unwrap();
        assert_eq!(state.phase, Phase::Release);
        assert!(state.protected_ack(&packet(Type::Acknowledgement, 11, true)));
        state.phase = Phase::Recovering;
        state.release_at = 9_000;
        state.recovered(9_300).unwrap();
        assert_eq!(state.cycles, 1);
        assert_eq!(state.phase, Phase::Waiting);
    }

    #[test]
    fn no_refusal_compressed_retry_backlog_or_rekey_cannot_pass() {
        assert!(
            held()
                .received(&packet(Type::Confirmable, 11, true), 9_000)
                .is_err()
        );
        let mut state = held();
        state.cooldown_at = 8_100;
        assert!(
            state
                .received(&packet(Type::Confirmable, 11, true), 7_001)
                .is_err()
        );
        assert!(state.bind_generation(2).is_err());
        state.phase = Phase::Waiting;
        state
            .received(&packet(Type::Confirmable, 12, true), 8_000)
            .unwrap();
        assert!(state.authenticated_echo(12, &[], 8_000).is_err());
    }

    #[test]
    fn refusal_oracle_rejects_wrong_code_max_age_missing_hold_and_late_response() {
        for (code, age, until, now) in [
            (Code::CONTENT, Some(1), 8_100, 7_100),
            (Code::SERVICE_UNAVAILABLE, None, 8_100, 7_100),
            (Code::SERVICE_UNAVAILABLE, Some(2), 8_100, 7_100),
            (Code::SERVICE_UNAVAILABLE, Some(1), 7_100, 7_100),
            (Code::SERVICE_UNAVAILABLE, Some(1), 9_500, 8_500),
        ] {
            assert!(held().refused(code, age, until, now).is_err());
        }
        let mut state = held();
        state.dropped = 0;
        assert!(
            state
                .refused(Code::SERVICE_UNAVAILABLE, Some(1), 8_100, 7_100)
                .is_err()
        );
    }

    #[test]
    fn unauthenticated_ingress_wrong_mid_token_and_empty_ack_do_not_qualify() {
        let mut state = State::default();
        state
            .received(&packet(Type::Confirmable, 10, false), 1_000)
            .unwrap();
        assert!(state.authenticated_echo(10, &[], 1_000).is_err());
        state
            .received(&packet(Type::Confirmable, 10, true), 1_000)
            .unwrap();
        assert!(state.authenticated_echo(11, &[], 1_000).is_err());
        assert!(state.authenticated_echo(10, b"other", 1_000).is_err());
        state.authenticated_echo(10, &[], 1_000).unwrap();
        assert!(!state.selected_ack(&packet(Type::Acknowledgement, 11, true)));
        let empty = Message::new(Type::Acknowledgement, Code::EMPTY, MessageId::new(10));
        let mut out = [0; 8];
        let len = encode(&empty, &mut out).unwrap();
        assert!(state.selected_ack(&out[..len]));
        assert!(!state.protected_ack(&out[..len]));
    }

    #[test]
    fn recovery_before_cooldown_or_after_call_age_bound_cannot_pass() {
        let mut state = held();
        state.phase = Phase::Recovering;
        state.release_at = 9_000;
        state.cooldown_at = 9_500;
        assert!(state.recovered(9_300).is_err());
        assert!(state.recovered(12_000).is_err());
        state.recovered(9_500).unwrap();
        assert_eq!(state.cycles, 1);
    }

    #[test]
    fn only_three_complete_cycles_reach_done() {
        let mut state = held();
        for completed in 1..=CYCLES {
            state.phase = Phase::Recovering;
            state.release_at = 9_000;
            state.cooldown_at = 8_100;
            state.recovered(9_300).unwrap();
            assert_eq!(state.cycles, completed);
            assert_eq!(state.phase == Phase::Done, completed == CYCLES);
        }
        state.phase = Phase::Recovering;
        assert!(state.recovered(9_300).is_err());
    }

    #[test]
    fn selected_echo_requires_exact_shape_and_authenticated_device_identity() {
        let mut payload = [0; 45];
        payload[..4].copy_from_slice(&[0x83, 1, 0x58, 32]);
        payload[4..36].fill(7);
        payload[36] = 0x1b;
        assert!(selected_echo(&payload, &[7; 32]));
        assert!(!selected_echo(&payload, &[8; 32]));
        assert!(!selected_echo(&payload[..44], &[7; 32]));
        payload[36] = 0;
        assert!(!selected_echo(&payload, &[7; 32]));
    }

    #[test]
    fn long_locally_valid_retry_outside_anchor_budget_is_refused() {
        let mut state = State {
            phase: Phase::Waiting,
            anchor: 2_000,
            generation: 1,
            ..State::default()
        };
        let request = packet(Type::Confirmable, 11, true);
        state.received(&request, 7_500).unwrap();
        state.authenticated_echo(11, &[], 7_500).unwrap();
        state.dropped = 1;
        state
            .refused(Code::SERVICE_UNAVAILABLE, Some(1), 8_600, 7_600)
            .unwrap();
        assert!(state.received(&request, 11_000).is_err());
    }
}
