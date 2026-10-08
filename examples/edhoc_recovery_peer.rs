//! Host fixture for pinned EDHOC recovery while a secured App remains live.
//!
//! The scalar-1/scalar-2 identities and exported session keys are public test
//! data. Run with `initiator|responder LOCAL_ADDRESS PEER_ADDRESS`; stdin accepts
//! `stats`, `probe`, `recover`, `revoke`, `regrant`, `lost-ack`, `restore` and
//! `stop`. An explicit recovery command requires
//! the current outgoing Confirmable operation to have finished. Unauthenticated
//! candidate failures preserve the old App; a fully authorized new session
//! replaces the complete App before the handoff is confirmed.
//!
//! The fixture reserves every locally sent request MID for its entire bounded
//! lifetime. It receives once, routes unprotected bootstrap traffic to recovery
//! and queues application traffic for App. Four completion/retired-attempt
//! slots bound recovery memory. A generation-bound trust grant gates every App
//! and bootstrap action in the single management/I/O event loop. The in-memory
//! test authority simulates committed-but-unacknowledged trust transitions;
//! it provides no power-loss durability or production commissioning authority.
//! Production entropy, trust persistence and device runtime qualification are
//! outside this public-key test fixture.

use std::cell::Cell;
use std::io::{self, BufRead};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use coaptic::message::{MessageId, OptionNumber, ParsedMessage, Type, decode, decode_uint16};
use coaptic::provisioning::{
    CoapRecovery, Identity, PeerTrust, PollError, RecoveryError, TrustAnchor, TrustRecord,
};
use coaptic::storage::{DatagramIo, UdpSocketIo};
use coaptic::{App, Code, ContentFormat, Endpoint, Response, profiles};

const REQUEST: &[u8] = b"coaptic-edhoc-udp-request\x00\xff";
const RESPONSE: &[u8] = b"coaptic-edhoc-udp-response\xff\x00";
const CAPACITY: usize = 1280;
const MID_CAPACITY: usize = 16;

thread_local! {
    static USED: Cell<[Option<MessageId>; MID_CAPACITY]> = const { Cell::new([None; MID_CAPACITY]) };
}

fn entropy(bytes: &mut [u8]) -> bool {
    for _ in 0..8 {
        if getrandom::fill(bytes).is_err() {
            return false;
        }
        if !matches!(bytes.len(), 2 | 22) {
            return true;
        }
        let first = MessageId::new(u16::from_be_bytes([bytes[0], bytes[1]]));
        if USED.with(|table| {
            !table.get().contains(&Some(first))
                && (bytes.len() == 2 || !table.get().contains(&Some(first.wrapping_add(1))))
        }) {
            return true;
        }
    }
    false
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut bytes = [0; 32];
    bytes[31] = value;
    Identity::from_private_key(bytes, kid).expect("public test identity")
}

fn bootstrap(message: ParsedMessage<'_>) -> bool {
    if message
        .options()
        .any(|option| option.number() == OptionNumber::OSCORE)
    {
        return false;
    }
    let mut paths = message
        .options()
        .filter(|option| option.number() == OptionNumber::URI_PATH);
    let resource = matches!(paths.next(), Some(option) if option.value() == b".well-known")
        && matches!(paths.next(), Some(option) if option.value() == b"edhoc")
        && paths.next().is_none();
    resource
        || (message.code().is_response()
            && message.options().any(|option| {
                option.number() == OptionNumber::CONTENT_FORMAT
                    && decode_uint16(option.value()) == Ok(64)
            }))
}

struct QueuedIo {
    udp: UdpSocketIo<[u8; CAPACITY + 1]>,
    bytes: [u8; CAPACITY],
    queued: Option<(usize, Endpoint)>,
    used: [Option<(MessageId, bool)>; MID_CAPACITY],
}

impl QueuedIo {
    fn new(socket: UdpSocket) -> io::Result<Self> {
        Ok(Self {
            udp: UdpSocketIo::new(socket, [0; CAPACITY + 1])?,
            bytes: [0; CAPACITY],
            queued: None,
            used: [None; MID_CAPACITY],
        })
    }

    fn receive(&mut self, bytes: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        match self.udp.recv(bytes) {
            Err(error)
                if error.kind() == io::ErrorKind::InvalidData
                    || (cfg!(windows) && error.raw_os_error() == Some(10040)) =>
            {
                Ok(None)
            }
            result => result,
        }
    }

    fn queue(&mut self, bytes: &[u8], endpoint: Endpoint) -> io::Result<()> {
        if self.queued.is_some() || bytes.len() > CAPACITY {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.queued = Some((bytes.len(), endpoint));
        Ok(())
    }
}

impl DatagramIo for QueuedIo {
    type Error = io::Error;

    fn recv(&mut self, output: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        let Some((len, endpoint)) = self.queued else {
            return Ok(None);
        };
        if len > output.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        output[..len].copy_from_slice(&self.bytes[..len]);
        self.queued = None;
        Ok(Some((len, endpoint)))
    }

    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        if let Ok(message) = decode(bytes) {
            if matches!(message.ty(), Type::Confirmable | Type::NonConfirmable) {
                let mid = message.message_id();
                let is_bootstrap = bootstrap(message);
                if let Some((_, owner)) = self.used.iter().flatten().find(|(used, _)| *used == mid)
                {
                    if *owner != is_bootstrap {
                        return Err(io::ErrorKind::AddrInUse.into());
                    }
                } else {
                    let row = self
                        .used
                        .iter_mut()
                        .find(|row| row.is_none())
                        .ok_or(io::ErrorKind::OutOfMemory)?;
                    *row = Some((mid, is_bootstrap));
                    USED.with(|table| table.set(self.used.map(|row| row.map(|(mid, _)| mid))));
                }
            }
        }
        self.udp.send(endpoint, bytes)
    }
}

type SecureApp = App<profiles::Constrained, QueuedIo>;

struct TestAuthority {
    record: Option<TrustRecord>,
    anchor: TrustAnchor,
}

impl TestAuthority {
    fn commit(
        &mut self,
        expected: &TrustAnchor,
        record: &TrustRecord,
        anchor: &TrustAnchor,
        lose_ack: bool,
    ) -> Result<(), &'static str> {
        if self.anchor != *expected {
            return Err("compare-and-swap conflict");
        }
        self.record = Some(record.clone());
        self.anchor = *anchor;
        if lose_ack {
            Err("committed acknowledgment lost")
        } else {
            Ok(())
        }
    }
}

fn now_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 || !matches!(args[1].as_str(), "initiator" | "responder") {
        return Err(
            "usage: edhoc_recovery_peer initiator|responder LOCAL_ADDRESS PEER_ADDRESS".into(),
        );
    }
    let initiator = args[1] == "initiator";
    let socket = UdpSocket::bind(args[2].parse::<SocketAddr>().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    let local = socket.local_addr().map_err(|e| e.to_string())?;
    let endpoint = Endpoint::from(args[3].parse::<SocketAddr>().map_err(|e| e.to_string())?);
    let local_identity = identity(if initiator { 1 } else { 2 }, if initiator { 0 } else { 1 });
    let peer = identity(if initiator { 2 } else { 1 }, if initiator { 1 } else { 0 }).peer();
    let expected = peer.principal();
    let origin = Instant::now();
    let mut authority = TestAuthority {
        record: None,
        anchor: TrustAnchor::genesis([42; 32]).map_err(|e| format!("{e:?}"))?,
    };
    let mut trust = PeerTrust::install(
        &local_identity,
        peer.clone(),
        authority.anchor,
        |old, record, new| authority.commit(old, record, new, false),
    )
    .map_err(|e| format!("{e:?}"))?;
    let mut grant = Some(trust.grant(&local_identity).map_err(|e| format!("{e:?}"))?);
    let mut recovery = Some(
        CoapRecovery::<4>::new(&local_identity, peer.clone(), endpoint, 0)
            .map_err(|e| format!("{e:?}"))?,
    );
    if initiator {
        recovery
            .as_mut()
            .expect("initial owner")
            .start(0, entropy)
            .map_err(|e| format!("{e:?}"))?;
    }
    let mut io = Some(QueuedIo::new(socket).map_err(|e| e.to_string())?);
    let mut app: Option<SecureApp> = None;
    let mut call = None;
    let mut handled = 0;
    let mut responses = 0;
    let mut sessions = 0;
    let mut probe = false;
    let mut bytes = [0; CAPACITY];
    let (commands, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if commands.send(line).is_err() {
                break;
            }
        }
    });
    println!("{{\"ready\":true,\"local\":\"{local}\"}}");
    while origin.elapsed() < Duration::from_secs(30) {
        let now = now_ms(origin);
        match receiver.try_recv() {
            Ok(command) if command == "stop" => {
                println!(
                    "{{\"stopped\":true,\"sessions\":{sessions},\"handled\":{handled},\"responses\":{responses}}}"
                );
                return Ok(());
            }
            Ok(command) if command == "stats" => println!(
                "{{\"stats\":true,\"sessions\":{sessions},\"handled\":{handled},\"responses\":{responses},\"candidate\":{},\"trust_generation\":{},\"trust_enabled\":{},\"trust_blocked\":{},\"owners\":{}}}",
                recovery.as_ref().is_some_and(CoapRecovery::has_candidate),
                authority.anchor.parts().1,
                trust.checkpoint().enabled(),
                trust.is_blocked(),
                grant.is_some()
            ),
            Ok(command) if command == "probe" => probe = true,
            Ok(command) if command == "recover" => {
                if call.is_some() {
                    println!("{{\"recover_blocked\":true,\"reason\":\"outgoing-CON\"}}");
                } else if let Some(recovery) = recovery.as_mut() {
                    recovery.start(now, entropy).map_err(|e| format!("{e:?}"))?;
                    println!("{{\"recover_started\":true}}");
                } else {
                    println!("{{\"recover_blocked\":true,\"reason\":\"trust\"}}");
                }
            }
            Ok(command) if command == "revoke" => {
                trust
                    .revoke(|old, record, new| authority.commit(old, record, new, false))
                    .map_err(|e| format!("{e:?}"))?;
                println!(
                    "{{\"trust_changed\":true,\"enabled\":false,\"trust_generation\":{}}}",
                    authority.anchor.parts().1
                );
            }
            Ok(command) if command == "regrant" => {
                trust
                    .replace(&local_identity, peer.clone(), true, |old, record, new| {
                        authority.commit(old, record, new, false)
                    })
                    .map_err(|e| format!("{e:?}"))?;
                println!(
                    "{{\"trust_changed\":true,\"enabled\":true,\"trust_generation\":{}}}",
                    authority.anchor.parts().1
                );
            }
            Ok(command) if command == "lost-ack" => {
                if trust
                    .replace(&local_identity, peer.clone(), true, |old, record, new| {
                        authority.commit(old, record, new, true)
                    })
                    .is_ok()
                    || !trust.is_blocked()
                {
                    return Err("ambiguous commit did not block trust".into());
                }
                println!(
                    "{{\"trust_blocked\":true,\"trust_generation\":{}}}",
                    authority.anchor.parts().1
                );
            }
            Ok(command) if command == "restore" => {
                trust = PeerTrust::restore(
                    &local_identity,
                    authority.record.clone().ok_or("record absent")?,
                    authority.anchor,
                )
                .map_err(|e| format!("{e:?}"))?;
                println!(
                    "{{\"trust_restored\":true,\"trust_generation\":{}}}",
                    authority.anchor.parts().1
                );
            }
            Ok(_) | Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
        if grant
            .as_ref()
            .is_some_and(|grant| !trust.permits(grant, &expected))
        {
            if let Some(old) = app.take() {
                io = Some(old.into_io());
            }
            io.as_mut().expect("retired transport").queued = None;
            recovery = None;
            grant = None;
            call = None;
            probe = false;
            println!(
                "{{\"owners_dropped\":true,\"trust_generation\":{}}}",
                authority.anchor.parts().1
            );
        }
        if grant.is_none() {
            if let Ok(fresh) = trust.grant(&local_identity) {
                recovery = Some(
                    CoapRecovery::<4>::new(&local_identity, peer.clone(), endpoint, now)
                        .map_err(|e| format!("{e:?}"))?,
                );
                grant = Some(fresh);
                println!(
                    "{{\"owners_admitted\":true,\"trust_generation\":{}}}",
                    authority.anchor.parts().1
                );
            }
        }
        let Some(recovery) = recovery.as_mut() else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        let grant = grant.as_ref().ok_or("owner grant absent")?;
        let have_app = app.is_some();
        let transport = if let Some(app) = &mut app {
            app.transport_mut()
        } else {
            io.as_mut().expect("unbound transport")
        };
        if transport.queued.is_none() {
            if let Some((len, from)) = transport.receive(&mut bytes).map_err(|e| e.to_string())? {
                if from == endpoint {
                    if decode(&bytes[..len]).is_ok_and(bootstrap) {
                        if let Err(error) =
                            recovery.ingest(&bytes[..len], from, now, entropy, |p| {
                                trust.permits(grant, p)
                            })
                        {
                            println!(
                                "{{\"candidate_failed\":true,\"error\":\"{error:?}\",\"sessions\":{sessions}}}"
                            );
                            recovery
                                .discard_candidate(now)
                                .map_err(|e| format!("{e:?}"))?;
                        }
                    } else if have_app {
                        transport
                            .queue(&bytes[..len], from)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        if let Err(error) = recovery.flush(transport, now) {
            match error {
                RecoveryError::Bootstrap(PollError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                RecoveryError::Bootstrap(
                    PollError::Timeout | PollError::Provisioning(_) | PollError::Rejected,
                ) => {
                    println!(
                        "{{\"candidate_failed\":true,\"error\":\"{error:?}\",\"sessions\":{sessions}}}"
                    );
                    recovery
                        .discard_candidate(now)
                        .map_err(|e| format!("{e:?}"))?;
                }
                error => return Err(format!("{error:?}")),
            }
        }
        if let Some(session) = call.is_none().then(|| recovery.take_session()).flatten() {
            let session = trust
                .accept_session(grant, session)
                .map_err(|e| format!("{e:?}"))?;
            let (context, principal) = session.into_parts();
            let completion = format!(
                "{{\"complete\":true,\"generation\":{},\"sender_key\":\"{}\",\"recipient_key\":\"{}\",\"common_iv\":\"{}\",\"sender_id\":\"{}\",\"recipient_id\":\"{}\",\"principal\":\"{}\",\"method\":3,\"suite\":2,\"message4\":true}}",
                sessions + 1,
                hex(context.sender_key()),
                hex(context.recipient_key()),
                hex(context.common_iv()),
                hex(context.sender_id()),
                hex(context.recipient_id()),
                hex(principal.fingerprint())
            );
            let transport = if let Some(old) = app.take() {
                old.into_io()
            } else {
                io.take().expect("initial transport")
            };
            app = Some(
                App::profile::<profiles::Constrained>()
                    .block_wise::<false>()
                    .randomness(entropy)
                    .oscore(context)
                    .bind(transport)
                    .map_err(|e| format!("{e:?}"))?,
            );
            if !recovery.confirm_handoff() {
                return Err("handoff confirmation absent".into());
            }
            call = None;
            sessions += 1;
            println!("{completion}");
            probe |= initiator;
        }
        let Some(app) = &mut app else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        if probe && call.is_none() && !recovery.has_candidate() {
            call = Some(
                app.put("provisioned")
                    .to(endpoint)
                    .payload(REQUEST)
                    .content_format(ContentFormat::OCTET_STREAM)
                    .deadline(now + 10_000)
                    .send(now)
                    .map_err(|e| format!("{e:?}"))?,
            );
            probe = false;
        }
        app.poll_with(now, |request, _| {
            if request.peer() != endpoint || request.code() != Code::PUT || request.path() != ["provisioned"] || request.payload() != REQUEST || request.content_format() != Some(Ok(ContentFormat::OCTET_STREAM)) { return Response::bad_request(); }
            handled += 1;
            let options: Vec<_> = request.options().map(|option| format!("{{\"number\":{},\"hex\":\"{}\"}}", option.number().get(), hex(option.value()))).collect();
            println!("{{\"handled\":true,\"count\":{handled},\"generation\":{sessions},\"principal\":\"{}\",\"code\":{},\"mtype\":{},\"mid\":{},\"token\":\"{}\",\"payload\":\"{}\",\"path\":\"provisioned\",\"content_format\":42,\"options\":[{}]}}", hex(expected.fingerprint()), request.code().as_raw(), request.ty().to_bits(), request.message_id().get(), hex(request.token().as_bytes()), hex(request.payload()), options.join(","));
            Response::changed().payload_borrowed(RESPONSE).content_format(ContentFormat::OCTET_STREAM)
        }).map_err(|e| format!("{e:?}"))?;
        if let Some(pending) = call {
            if let Some(response) = app.take_response(pending) {
                let response = response.map_err(|e| format!("{e:?}"))?;
                if response.code() != Code::CHANGED
                    || response.payload() != RESPONSE
                    || response.payload_truncated()
                    || response.payload_src_len() != RESPONSE.len()
                    || response.format() != Some(ContentFormat::OCTET_STREAM)
                    || response.ty() != Some(Type::Acknowledgement)
                    || response.token() != Some(pending.token())
                    || response.peer() != Some(endpoint)
                {
                    return Err("invalid complete protected response".into());
                }
                responses += 1;
                println!(
                    "{{\"response\":true,\"count\":{responses},\"generation\":{sessions},\"code\":{},\"mtype\":2,\"mid\":{},\"token\":\"{}\",\"payload\":\"{}\",\"content_format\":42}}",
                    response.code().as_raw(),
                    response.message_id().ok_or("response MID absent")?.get(),
                    hex(pending.token().as_bytes()),
                    hex(response.payload())
                );
                call = None;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("recovery fixture deadline".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
