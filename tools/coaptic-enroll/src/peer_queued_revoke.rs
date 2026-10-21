//! One live-CON management-teardown check. The gate loses replies, never queues
//! packets. The ordinary revocation keeps its original host-store lock.

use crate::serial_cli::{LockedHost, RevokeProgress, Usb, revoke_locked};
use coaptic::{
    Code, Endpoint, Method, Response,
    message::{Type, decode},
    profiles,
    provisioning::{
        CommissioningPending, CommissioningReply, CommissioningRequest, ManagedConnection,
        ManagedError,
    },
    storage::{DatagramIo, DatagramTransport, UdpSocketIo},
};
use coaptic_console_core::ApprovalMode;
use coaptic_enroll::pairing;
use std::{
    cell::Cell,
    io,
    net::UdpSocket,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const COMMIT_BEFORE_MS: u64 = 4_000;
const OLD_CALL_END_MS: u64 = 10_500;
const RUN_LIMIT_MS: u64 = 45_000;
// The selected target waits 5s after the successful control response before its
// next Call, retries at 2..3s then at most 6s later, and cancels at 10s. A
// conservative before-send anchor bounds packet age despite host scheduling.
const HELD_FIRST_MIN_MS: u64 = 4_900;
const HELD_FIRST_MAX_MS: u64 = 5_500;
const RETRY_MIN_MS: u64 = 1_500;
const RETRY_MAX_MS: u64 = 3_500;
const HELD_RETRY_MAX_MS: u64 = 8_800;
// This expectation must be bound to the installed image by the execution receipt;
// the privileged Inspect reply does not attest firmware provenance.
const TARGET_SERVICE_SHA256: &str =
    "0ec66bd4f946fe826c3451c861c6269a9893624c2e9259287aed12022284e930";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Waiting,
    Control,
    Released,
    Held,
}

#[derive(Clone, Copy, Default)]
struct State {
    last_hash: [u8; 32],
    selected_hash: [u8; 32],
    last_token: [u8; 8],
    token: [u8; 8],
    first_at: u64,
    retry_at: u64,
    control_send_at: u64,
    generation: u64,
    last_mid: u16,
    mid: u16,
    phase: Phase,
    dropped: u8,
    accepted: u8,
    repeats: u8,
    last_token_len: u8,
    token_len: u8,
    last_valid: bool,
    timing_invalid: bool,
}

impl State {
    fn received(&mut self, bytes: &[u8], now: u64) {
        self.last_hash = *blake3::hash(bytes).as_bytes();
        self.last_valid = false;
        if let Ok(packet) = decode(bytes) {
            let token = packet.token();
            let token = token.as_bytes();
            if token.len() <= self.last_token.len() {
                self.last_mid = packet.message_id().get();
                self.last_token_len = token.len() as u8;
                self.last_token[..token.len()].copy_from_slice(token);
                self.last_valid = true;
            }
        }
        if matches!(self.phase, Phase::Control | Phase::Held)
            && self.last_hash == self.selected_hash
        {
            if !(RETRY_MIN_MS..=RETRY_MAX_MS).contains(&now.saturating_sub(self.first_at)) {
                self.timing_invalid = true;
                return;
            }
            self.repeats = self.repeats.saturating_add(1);
            self.retry_at = now;
            if self.phase == Phase::Control {
                // Release before this exact retry reaches the cached-response
                // send. No retained response packet or second queue is needed.
                self.phase = Phase::Released;
            }
        }
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
            return Err("authenticated echo does not match captured ingress");
        }
        match self.phase {
            Phase::Waiting | Phase::Released => {
                if self.phase == Phase::Released {
                    if self.accepted == 0 || self.control_send_at == 0 {
                        return Err("positive retry control was not delivered");
                    }
                    if !(HELD_FIRST_MIN_MS..=HELD_FIRST_MAX_MS)
                        .contains(&now.saturating_sub(self.control_send_at))
                    {
                        return Err(
                            "held CON age is not bounded by the delivered control response",
                        );
                    }
                }
                self.phase = if self.phase == Phase::Waiting {
                    Phase::Control
                } else {
                    Phase::Held
                };
                self.selected_hash = self.last_hash;
                self.mid = mid;
                self.token_len = self.last_token_len;
                self.token = self.last_token;
                self.first_at = now;
                self.retry_at = 0;
                self.repeats = 0;
                self.dropped = 0;
            }
            Phase::Control | Phase::Held => {
                if self.selected_hash != self.last_hash {
                    return Err("another echo displaced the held CON");
                }
            }
        }
        Ok(())
    }

    fn drop_response(&mut self, bytes: &[u8]) -> bool {
        let matches = decode(bytes).is_ok_and(|packet| {
            packet.ty() == Type::Acknowledgement
                && packet.message_id().get() == self.mid
                && (packet.token().as_bytes() == &self.token[..usize::from(self.token_len)]
                    || (packet.code() == Code::EMPTY && packet.token().as_bytes().is_empty()))
        });
        if !matches {
            return false;
        }
        if matches!(self.phase, Phase::Control | Phase::Held) {
            self.dropped = self.dropped.saturating_add(1);
            return true;
        }
        if self.phase == Phase::Released
            && decode(bytes)
                .is_ok_and(|packet| packet.code() != Code::EMPTY && packet.oscore().is_some())
        {
            self.accepted = self.accepted.saturating_add(1);
        }
        false
    }

    fn armed(self, now: u64) -> bool {
        self.phase == Phase::Held
            && !self.timing_invalid
            && self.generation != 0
            && self.repeats != 0
            && self.dropped >= 2
            && self.retry_at <= self.control_send_at.saturating_add(HELD_RETRY_MAX_MS)
            && now < self.first_at.saturating_add(COMMIT_BEFORE_MS)
    }

    fn bind_generation(&mut self, current: u64) -> Result<()> {
        if self.phase == Phase::Waiting {
            return Ok(());
        }
        if current == 0 || (self.generation != 0 && self.generation != current) {
            return Err("protected session changed during queued-CON qualification".into());
        }
        self.generation = current;
        Ok(())
    }
}

struct Gate<'a> {
    io: &'a mut UdpSocketIo<[u8; 1473]>,
    state: &'a Cell<State>,
    epoch: &'a Instant,
    peer: &'a Endpoint,
}

const _: () = assert!(std::mem::size_of::<State>() + std::mem::size_of::<Gate<'static>>() <= 160);
const _: () = assert!(
    std::mem::size_of::<ManagedConnection<'static, Gate<'static>, profiles::Constrained>>()
        + std::mem::size_of::<UdpSocketIo<[u8; 1473]>>()
        + std::mem::size_of::<State>()
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
            state.received(&out[..len], self.epoch.elapsed().as_millis() as u64);
            self.state.set(state);
        }
        Ok(incoming)
    }
    fn send(&mut self, to: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.state.get();
        let before = state.accepted;
        if to == *self.peer && state.drop_response(bytes) {
            self.state.set(state);
            return Ok(bytes.len()); // Explicit receive-loss injection, not remote receipt.
        }
        let control = before == 0 && state.accepted != 0;
        if control {
            state.control_send_at = self.epoch.elapsed().as_millis() as u64;
        }
        let sent = self.io.send(to, bytes)?;
        if sent == bytes.len() {
            if control {
                println!(
                    "Queued-CON positive control: first {} ms; exact retry {} ms; protected response before-send anchor {} ms.",
                    state.first_at, state.retry_at, state.control_send_at
                );
            }
            self.state.set(state);
        }
        Ok(sent)
    }
}

pub(crate) fn run_qualified(
    host: LockedHost<'_>,
    approval_directory: &Path,
    local: &str,
    remote: &str,
    port: &str,
) -> Result<()> {
    let expected = host.pairing.peer()?.clone();
    if !host.pairing.checkpoint().enabled()
        || host
            .pairing
            .checkpoint()
            .policy()
            .is_none_or(|policy| *policy.resource_policy() != pairing::echo_permissions())
        || host.identity.peer().principal().fingerprint() <= expected.principal().fingerprint()
    {
        return Err(
            "queued-CON qualification requires the current echo-only outbound-initiating host"
                .into(),
        );
    }
    let mut usb = Usb::open(port)?;
    check_device(&mut usb, &host, true)?;
    let peer = Endpoint::from(remote.parse::<std::net::SocketAddr>()?);
    let socket = UdpSocket::bind(local)?;
    socket.set_nonblocking(true)?;
    let mut io = UdpSocketIo::new(socket, [0; 1473])?;
    let epoch = Instant::now();
    let state = Cell::new(State::default());
    println!(
        "Queued-CON qualification expected target async_service SHA256 {TARGET_SERVICE_SHA256}; control metadata {} bytes; original UDP scratch 1473 bytes, counted once.",
        std::mem::size_of::<State>() + std::mem::size_of::<Gate<'_>>()
    );
    {
        let gate = Gate {
            io: &mut io,
            state: &state,
            epoch: &epoch,
            peer: &peer,
        };
        let mut connection = ManagedConnection::<_, profiles::Constrained>::from_local_pairing(
            host.identity,
            host.pairing,
            peer,
            gate,
            super::entropy,
            0,
        )?;
        connection.observable_manual("identity")?;
        connection.connect(0)?;
        println!(
            "Queued-CON owner bytes {}; adapter/scratch owner bytes {}; no added packet queues.",
            std::mem::size_of_val(&connection),
            std::mem::size_of::<UdpSocketIo<[u8; 1473]>>()
        );
        loop {
            let now = epoch.elapsed().as_millis() as u64;
            if now >= RUN_LIMIT_MS {
                return Err("queued-CON qualification deadline".into());
            }
            let mut failure = None;
            match connection.poll_with(now, |principal, request, _| {
                if principal != &expected.principal() {
                    failure = Some("unexpected authenticated device");
                    return Response::new(Code::FORBIDDEN);
                }
                if request.path() == ["identity"] && request.method() == Some(Method::Get) {
                    let response =
                        Response::content_copy(host.identity.peer().principal().fingerprint())
                            .max_age(10);
                    if request.is_observe_register() {
                        response.observe(0)
                    } else {
                        response
                    }
                } else if request.path() == ["echo"]
                    && request.method() == Some(Method::Post)
                    && request.ty() == Type::Confirmable
                    && request.payload().len() <= 128
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
                    Response::new(Code::NOT_FOUND)
                }
            }) {
                Ok(()) | Err(ManagedError::Recovery(_)) => {}
                Err(error) => return Err(error.into()),
            }
            if let Some(error) = failure {
                return Err(error.into());
            }
            let mut current = state.get();
            current.bind_generation(connection.session_generation().unwrap_or(0))?;
            if current.timing_invalid {
                return Err(
                    "CON retry timing was compressed or outside the selected source window".into(),
                );
            }
            state.set(current);
            if current.armed(now) {
                println!(
                    "Queued-CON armed: exact ciphertext BLAKE3 {}; MID {}; first {} ms; retry {} ms; dropped replies {}; control before-send {} ms, accepted replies {}; session generation {}.",
                    crate::hex(&current.selected_hash),
                    current.mid,
                    current.first_at,
                    current.retry_at,
                    current.dropped,
                    current.control_send_at,
                    current.accepted,
                    current.generation
                );
                break;
            }
            if matches!(current.phase, Phase::Control | Phase::Held)
                && now >= current.first_at.saturating_add(COMMIT_BEFORE_MS)
            {
                return Err(
                    "selected CON did not retry within the source-matched live-call window".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // Ends the immutable LocalPairing borrow and all host App/cryptographic
        // ownership before the same locked record can be changed.
    }
    let armed = state.into_inner();
    let (socket, mut scratch) = io.into_parts();
    let mut observe = |point| -> Result<()> {
        // Pre-Challenge traffic is drained without attributing it to teardown.
        // AfterChallenge runs only after that USB frame is written and flushed.
        drain(
            &socket,
            &mut scratch,
            peer,
            (point == RevokeProgress::AfterChallenge).then_some(&armed.selected_hash),
        )?;
        if epoch.elapsed().as_millis() as u64 >= armed.first_at.saturating_add(COMMIT_BEFORE_MS) {
            return Err("revocation did not commit before the old Call's next possible retry; inspect state before recovery".into());
        }
        Ok(())
    };
    let committed = revoke_locked(
        &mut usb,
        host,
        approval_directory,
        ApprovalMode::PivTouchless,
        &mut observe,
    )?;
    observe(RevokeProgress::AfterChallenge)?;
    let committed_at = epoch.elapsed().as_millis() as u64;
    while (epoch.elapsed().as_millis() as u64) < armed.first_at.saturating_add(OLD_CALL_END_MS) {
        drain(&socket, &mut scratch, peer, Some(&armed.selected_hash))?;
        std::thread::sleep(Duration::from_millis(1));
    }
    drain(&socket, &mut scratch, peer, Some(&armed.selected_hash))?;
    let CommissioningReply::State {
        device,
        record,
        pending: CommissioningPending::Complete,
        ..
    } = usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("final revoked device custody is not complete".into());
    };
    if device.principal() != expected.principal()
        || record.enabled()
        || record.commitment() != committed.commitment()
    {
        return Err("final revoked device checkpoint differs".into());
    }
    drain(&socket, &mut scratch, peer, Some(&armed.selected_hash))?;
    println!(
        "Queued-CON qualification passed: positive byte-identical retry/release, held retry stopped across privileged management teardown; ordinary approved revocation committed at {committed_at} ms, revision {}; no old ciphertext through {} ms. Device and host remain revoked; use ordinary approved pairing for fresh work. This is not physical power-loss or all-queue-family proof.",
        record.revision(),
        armed.first_at.saturating_add(OLD_CALL_END_MS)
    );
    Ok(())
}

fn check_device(usb: &mut Usb, host: &LockedHost<'_>, enabled: bool) -> Result<()> {
    let CommissioningReply::State {
        device,
        record,
        pending: CommissioningPending::Complete,
        ..
    } = usb.request(&CommissioningRequest::Inspect)?
    else {
        return Err("queued-CON qualification requires authenticated complete custody".into());
    };
    let local = host
        .pairing
        .checkpoint()
        .policy()
        .ok_or("missing host policy")?;
    if record.enabled() != enabled
        || device.principal() != host.pairing.peer()?.principal()
        || record.local_principal() != device.principal().fingerprint()
        || record.policy().is_none_or(|policy| {
            policy.peer().principal() != host.identity.peer().principal()
                || policy.owner() != local.owner()
                || *policy.resource_policy() != pairing::echo_permissions()
        })
    {
        return Err(
            "device identity/current owner/echo-only policy differs from held host record".into(),
        );
    }
    Ok(())
}

fn drain(
    socket: &UdpSocket,
    scratch: &mut [u8; 1473],
    peer: Endpoint,
    selected: Option<&[u8; 32]>,
) -> Result<()> {
    for _ in 0..32 {
        match socket.recv_from(scratch) {
            Ok((len, from))
                if Endpoint::from(from) == peer
                    && selected.is_some_and(|expected| {
                        blake3::hash(&scratch[..len]).as_bytes() == expected
                    }) =>
            {
                return Err("old held CON continued after management teardown".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
    Err("receive-only observation exceeded its finite ingress budget".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::message::{Message, MessageId, Opt, Token, encode};

    fn packet(ty: Type, mid: u16) -> Vec<u8> {
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
        .with_options(&options)
        .with_payload(b"tag");
        let mut out = [0; 32];
        let len = encode(&message, &mut out).unwrap();
        out[..len].to_vec()
    }

    #[test]
    fn exact_retry_releases_control_but_held_retry_never_releases() {
        let mut state = State::default();
        let first = packet(Type::Confirmable, 10);
        let ack = packet(Type::Acknowledgement, 10);
        state.received(&first, 0);
        state.authenticated_echo(10, &[], 0).unwrap();
        state.bind_generation(1).unwrap();
        assert!(state.drop_response(&ack));
        assert!(!state.armed(1));
        state.received(&first, 2_000);
        assert_eq!(state.phase, Phase::Released);
        assert!(!state.drop_response(&ack));
        state.control_send_at = 2_000; // Full physical send commits the gate state.
        let next = packet(Type::Confirmable, 11);
        state.received(&next, 7_000);
        state.authenticated_echo(11, &[], 7_000).unwrap();
        assert!(state.drop_response(&packet(Type::Acknowledgement, 11)));
        let changed = packet(Type::Confirmable, 12);
        state.received(&changed, 8_000);
        assert_eq!(state.repeats, 0);
        state.received(&next, 9_000);
        assert!(state.drop_response(&packet(Type::Acknowledgement, 11)));
        assert!(state.armed(9_000));
        assert!(!state.armed(11_000));
    }

    #[test]
    fn unrelated_ack_and_unauthenticated_ingress_do_not_arm_or_release() {
        let mut state = State::default();
        state.received(&packet(Type::Confirmable, 9), 0);
        assert!(!state.armed(1));
        assert!(state.authenticated_echo(8, &[], 1).is_err());
        assert!(state.authenticated_echo(9, b"different", 1).is_err());
        state.authenticated_echo(9, &[], 1).unwrap();
        assert!(!state.drop_response(&packet(Type::Acknowledgement, 8)));
        state.received(&packet(Type::Confirmable, 8), 2_001);
        assert_eq!(state.repeats, 0);
        assert!(state.drop_response(&packet(Type::Acknowledgement, 9)));
    }

    #[test]
    fn queued_packets_expired_calls_session_changes_and_empty_acks_cannot_qualify() {
        let first = packet(Type::Confirmable, 10);
        let ack = packet(Type::Acknowledgement, 10);
        let mut state = State::default();
        state.received(&first, 6_000);
        state.authenticated_echo(10, &[], 6_000).unwrap();
        state.bind_generation(1).unwrap();
        assert!(state.drop_response(&ack));
        state.received(&first, 6_001); // Initial packet and retry were queued together.
        assert!(state.timing_invalid);
        assert!(!state.armed(6_001));

        let mut state = State {
            phase: Phase::Released,
            accepted: 1,
            control_send_at: 2_000,
            generation: 1,
            ..State::default()
        };
        let held = packet(Type::Confirmable, 11);
        state.received(&held, 8_001);
        assert!(state.authenticated_echo(11, &[], 8_001).is_err());
        assert!(state.bind_generation(2).is_err());
        state.received(&held, 7_500);
        state.authenticated_echo(11, &[], 7_500).unwrap();
        assert!(state.drop_response(&packet(Type::Acknowledgement, 11)));
        state.received(&held, 11_000);
        assert!(state.drop_response(&packet(Type::Acknowledgement, 11)));
        // A 3.5s retry can fit the local window yet leave no guaranteed retry
        // before the earliest possible natural cancellation. It must refuse.
        assert!(!state.armed(11_000));

        let mut state = State {
            phase: Phase::Released,
            mid: 10,
            ..State::default()
        };
        let empty = Message::new(Type::Acknowledgement, Code::EMPTY, MessageId::new(10));
        let mut bytes = [0; 8];
        let len = encode(&empty, &mut bytes).unwrap();
        assert!(!state.drop_response(&bytes[..len]));
        assert_eq!(
            state.accepted, 0,
            "empty ACK is not a protected echo response"
        );
    }

    #[test]
    fn receive_only_observation_rejects_old_wire_on_the_preserved_socket() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver.set_nonblocking(true).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = Endpoint::from(sender.local_addr().unwrap());
        let old = packet(Type::Confirmable, 12);
        let digest = *blake3::hash(&old).as_bytes();
        let mut scratch = [0; 1473];
        drain(&receiver, &mut scratch, peer, Some(&digest)).unwrap();
        sender
            .send_to(&old, receiver.local_addr().unwrap())
            .unwrap();
        // Already queued pre-Challenge traffic is not a post-teardown failure.
        drain(&receiver, &mut scratch, peer, None).unwrap();
        sender
            .send_to(&old, receiver.local_addr().unwrap())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Err(error) = drain(&receiver, &mut scratch, peer, Some(&digest)) {
                assert!(error.to_string().contains("old held CON continued"));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
