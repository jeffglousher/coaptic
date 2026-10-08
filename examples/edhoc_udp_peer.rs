//! UDP interoperability fixture for authenticated bootstrap and secure App handoff.
//!
//! Public scalar-1/scalar-2 identities and exported keys are test fixtures only.
//! Never deploy these credentials or export production session keys. Run with
//! `initiator|responder LOCAL_ADDRESS PEER_ADDRESS`; stdin accepts `stats` and
//! `stop`. Each process admits one fresh session and lives for at most 15 seconds.
//! Restart both participants to replace that session. An unauthenticated new
//! bootstrap request never replaces a live App or its completion caches.
//!
//! The caller receives once into bounded storage and classifies OSCORE before
//! the reserved bootstrap resource. App's transport receives only the queued
//! application packet. Completion duplicates keep their original controller,
//! while the authenticated principal supplies the application authorization.
//! Empty ACK/RST after handoff belong to App. The host-only entropy adapter
//! avoids locally allocated bootstrap MIDs, and a send guard rejects later
//! collisions while the completion cache remains live. This fixture sends one
//! outgoing application CON; it does not implement shared multi-owner scheduling.

use std::cell::Cell;
use std::io::{self, BufRead};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use coaptic::message::{MessageId, OptionNumber, ParsedMessage, Type, decode, decode_uint16};
use coaptic::provisioning::{CoapProvisioner, Identity, Status};
use coaptic::storage::{DatagramIo, UdpSocketIo};
use coaptic::{App, Code, ContentFormat, Endpoint, Response, profiles};

const REQUEST: &[u8] = b"coaptic-edhoc-udp-request\x00\xff";
const RESPONSE: &[u8] = b"coaptic-edhoc-udp-response\xff\x00";
const CAPACITY: usize = 1280;

thread_local! {
    static RESERVED: Cell<[Option<MessageId>; 2]> = const { Cell::new([None; 2]) };
}

fn entropy(bytes: &mut [u8]) -> bool {
    for _ in 0..8 {
        if getrandom::fill(bytes).is_err() {
            return false;
        }
        if bytes.len() != 2
            || RESERVED.with(|table| {
                !table
                    .get()
                    .contains(&Some(MessageId::new(u16::from_be_bytes([
                        bytes[0], bytes[1],
                    ]))))
            })
        {
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

fn protected(message: ParsedMessage<'_>) -> bool {
    message
        .options()
        .any(|option| option.number() == OptionNumber::OSCORE)
}

fn bootstrap(message: ParsedMessage<'_>) -> bool {
    if protected(message) {
        return false;
    }
    let mut path = message
        .options()
        .filter(|option| option.number() == OptionNumber::URI_PATH);
    let resource = matches!(path.next(), Some(option) if option.value() == b".well-known")
        && matches!(path.next(), Some(option) if option.value() == b"edhoc")
        && path.next().is_none();
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
    reserved: [Option<MessageId>; 2],
    cache_live: bool,
}

impl QueuedIo {
    fn new(socket: UdpSocket) -> io::Result<Self> {
        Ok(Self {
            udp: UdpSocketIo::new(socket, [0; CAPACITY + 1])?,
            bytes: [0; CAPACITY],
            queued: None,
            reserved: [None; 2],
            cache_live: true,
        })
    }

    fn queue(&mut self, bytes: &[u8], endpoint: Endpoint) -> io::Result<()> {
        if self.queued.is_some() || bytes.len() > self.bytes.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.queued = Some((bytes.len(), endpoint));
        Ok(())
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

    fn send_wire(&mut self, dest: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        if let Ok(message) = decode(bytes) {
            if matches!(message.ty(), Type::Confirmable | Type::NonConfirmable) {
                let mid = Some(message.message_id());
                if bootstrap(message) {
                    if !self.reserved.contains(&mid) {
                        let row = self
                            .reserved
                            .iter_mut()
                            .find(|row| row.is_none())
                            .ok_or(io::ErrorKind::InvalidData)?;
                        *row = mid;
                    }
                } else if self.cache_live && self.reserved.contains(&mid) {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "application MID collides with live bootstrap exchange",
                    ));
                }
            }
        }
        self.udp.send(dest, bytes)
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

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.send_wire(dest, bytes)
    }
}

struct BootstrapIo<'a>(&'a mut QueuedIo);

impl DatagramIo for BootstrapIo<'_> {
    type Error = io::Error;

    fn recv(&mut self, output: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        self.0.receive(output)
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.0.send_wire(dest, bytes)
    }
}

fn now_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn stats(handled: u64, responses: u64, complete: bool) {
    println!(
        "{{\"stats\":true,\"handled_count\":{handled},\"response_count\":{responses},\"complete\":{complete}}}"
    );
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.len() != 4 || !matches!(arguments[1].as_str(), "initiator" | "responder") {
        return Err("expected initiator|responder LOCAL_ADDRESS PEER_ADDRESS".into());
    }
    let initiator = arguments[1] == "initiator";
    let socket = UdpSocket::bind(&arguments[2]).map_err(|e| e.to_string())?;
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    let local = socket.local_addr().map_err(|e| e.to_string())?;
    let endpoint = Endpoint::from(
        arguments[3]
            .parse::<SocketAddr>()
            .map_err(|e| e.to_string())?,
    );
    let origin = Instant::now();
    let local_identity = identity(if initiator { 1 } else { 2 }, if initiator { 0 } else { 1 });
    let pin = identity(if initiator { 2 } else { 1 }, if initiator { 1 } else { 0 }).peer();
    let expected = pin.principal();
    let mut controller = if initiator {
        CoapProvisioner::start(&local_identity, pin, endpoint, 0, entropy)
            .map_err(|e| format!("{e:?}"))?
    } else {
        CoapProvisioner::listen(&local_identity, pin, endpoint, 0)
    };
    let (commands, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if commands.send(line).is_err() {
                break;
            }
        }
    });
    let mut io = QueuedIo::new(socket).map_err(|e| e.to_string())?;
    println!("{{\"ready\":true,\"local\":\"{local}\"}}");
    let session = loop {
        match receiver.try_recv() {
            Ok(command) if command == "stop" => {
                stats(0, 0, false);
                return Ok(());
            }
            Ok(command) if command == "stats" => stats(0, 0, false),
            Ok(_) | Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
        if origin.elapsed() >= Duration::from_secs(15) {
            return Err("bootstrap fixture deadline".into());
        }
        controller
            .poll(&mut BootstrapIo(&mut io), now_ms(origin), entropy, |p| {
                *p == expected
            })
            .map_err(|e| format!("{e:?}"))?;
        if let Some(session) = controller.take_session() {
            break session;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let (context, principal) = session.into_parts();
    if principal != expected {
        return Err("unexpected authenticated principal".into());
    }
    let completion = format!(
        "{{\"complete\":true,\"sender_key\":\"{}\",\"recipient_key\":\"{}\",\"common_iv\":\"{}\",\"sender_id\":\"{}\",\"recipient_id\":\"{}\",\"principal\":\"{}\",\"method\":3,\"suite\":2,\"message4\":true}}",
        hex(context.sender_key()),
        hex(context.recipient_key()),
        hex(context.common_iv()),
        hex(context.sender_id()),
        hex(context.recipient_id()),
        hex(principal.fingerprint()),
    );
    RESERVED.with(|table| table.set(io.reserved));
    let mut app = App::profile::<profiles::Constrained>()
        .block_wise::<false>()
        .randomness(entropy)
        .oscore(context)
        .bind(io)
        .map_err(|e| format!("{e:?}"))?;
    println!("{completion}");
    let mut call = if initiator {
        Some(
            app.put("provisioned")
                .to(endpoint)
                .payload(REQUEST)
                .content_format(ContentFormat::OCTET_STREAM)
                .deadline(now_ms(origin) + 10_000)
                .send(now_ms(origin))
                .map_err(|e| format!("{e:?}"))?,
        )
    } else {
        None
    };
    let mut handled = 0;
    let mut responses = 0;
    let mut bytes = [0; CAPACITY];
    while origin.elapsed() < Duration::from_secs(15) {
        let now = now_ms(origin);
        match receiver.try_recv() {
            Ok(command) if command == "stop" => {
                stats(handled, responses, true);
                return Ok(());
            }
            Ok(command) if command == "stats" => stats(handled, responses, true),
            Ok(_) | Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
        if app.transport().queued.is_none() {
            if let Some((len, from)) = app
                .transport_mut()
                .receive(&mut bytes)
                .map_err(|e| e.to_string())?
            {
                if from == endpoint {
                    if decode(&bytes[..len]).is_ok_and(bootstrap) {
                        controller
                            .ingest(&bytes[..len], from, now, entropy, |p| *p == expected)
                            .map_err(|e| format!("{e:?}"))?;
                    } else {
                        app.transport_mut()
                            .queue(&bytes[..len], from)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        if controller
            .flush(app.transport_mut(), now)
            .map_err(|e| format!("{e:?}"))?
            == Status::Expired
        {
            app.transport_mut().cache_live = false;
        }
        app.poll_with(now, |request, _| {
            if principal != expected || request.peer() != endpoint
                || request.code() != Code::PUT || request.path() != ["provisioned"]
                || request.payload() != REQUEST
                || request.content_format() != Some(Ok(ContentFormat::OCTET_STREAM))
            {
                return Response::bad_request();
            }
            handled += 1;
            let options: Vec<_> = request.options().map(|option| format!(
                "{{\"number\":{},\"hex\":\"{}\"}}", option.number().get(), hex(option.value()),
            )).collect();
            println!(
                "{{\"handled\":true,\"code\":{},\"mtype\":{},\"payload\":\"{}\",\"count\":{handled},\"principal\":\"{}\",\"mid\":{},\"token\":\"{}\",\"path\":\"provisioned\",\"content_format\":42,\"options\":[{}]}}",
                request.code().as_raw(), request.ty().to_bits(), hex(request.payload()),
                hex(principal.fingerprint()), request.message_id().get(), hex(request.token().as_bytes()), options.join(","),
            );
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
                    "{{\"response\":true,\"code\":{},\"mtype\":2,\"payload\":\"{}\",\"mid\":{},\"token\":\"{}\",\"content_format\":42}}",
                    response.code().as_raw(),
                    hex(response.payload()),
                    response.message_id().ok_or("response MID absent")?.get(),
                    hex(pending.token().as_bytes()),
                );
                call = None;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("application fixture deadline".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::message::{Message, Opt, Token};

    fn socket() -> UdpSocket {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        socket
    }

    #[test]
    fn protected_reserved_path_and_empty_ack_are_app_traffic() {
        let options = [
            Opt::new(OptionNumber::OSCORE, &[9, 0, 1]),
            Opt::new(OptionNumber::URI_PATH, b".well-known"),
            Opt::new(OptionNumber::URI_PATH, b"edhoc"),
            Opt::new(OptionNumber::CONTENT_FORMAT, &[64]),
        ];
        let mut bytes = [0; CAPACITY];
        let len = Message::con(Code::POST, MessageId::new(10), Token::EMPTY)
            .with_options(&options)
            .with_payload(b"ciphertext")
            .encode(&mut bytes)
            .unwrap();
        assert!(!bootstrap(decode(&bytes[..len]).unwrap()));
        let len = Message::empty_ack(MessageId::new(10))
            .encode(&mut bytes)
            .unwrap();
        assert!(!bootstrap(decode(&bytes[..len]).unwrap()));
        let len = Message::con(Code::POST, MessageId::new(11), Token::EMPTY)
            .with_options(&options[1..])
            .with_payload(b"bootstrap")
            .encode(&mut bytes)
            .unwrap();
        assert!(bootstrap(decode(&bytes[..len]).unwrap()));
    }

    #[test]
    fn queued_packet_survives_small_buffer_without_reading_socket() {
        let transport = socket();
        let destination = transport.local_addr().unwrap();
        let sender = socket();
        let endpoint = Endpoint::from(sender.local_addr().unwrap());
        let mut io = QueuedIo::new(transport).unwrap();
        io.queue(b"queued", endpoint).unwrap();
        assert!(io.queue(b"overwrite", endpoint).is_err());
        assert!(io.recv(&mut [0; 3]).is_err());
        let mut bytes = [0; CAPACITY];
        assert_eq!(io.recv(&mut bytes).unwrap(), Some((6, endpoint)));
        assert_eq!(&bytes[..6], b"queued");
        sender.send_to(b"socket", destination).unwrap();
        assert_eq!(io.recv(&mut bytes).unwrap(), None);
        for _ in 0..100 {
            if let Some((len, from)) = io.udp.recv(&mut bytes).unwrap() {
                assert_eq!(from, endpoint);
                assert_eq!(&bytes[..len], b"socket");
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("outer receiver lost a complete socket packet");
    }

    #[test]
    fn live_bootstrap_mid_cannot_be_allocated_by_app_but_ack_can_echo_it() {
        let receiver = socket();
        let destination = Endpoint::from(receiver.local_addr().unwrap());
        let mut io = QueuedIo::new(socket()).unwrap();
        let mid = MessageId::new(61);
        let mut bytes = [0; CAPACITY];
        let options = [
            Opt::new(OptionNumber::URI_PATH, b".well-known"),
            Opt::new(OptionNumber::URI_PATH, b"edhoc"),
        ];
        let len = Message::con(Code::POST, mid, Token::EMPTY)
            .with_options(&options)
            .with_payload(b"bootstrap")
            .encode(&mut bytes)
            .unwrap();
        assert_eq!(io.send(destination, &bytes[..len]).unwrap(), len);
        let len = Message::con(Code::POST, mid, Token::EMPTY)
            .with_options(&[Opt::new(OptionNumber::OSCORE, &[9, 0, 1])])
            .with_payload(b"ciphertext")
            .encode(&mut bytes)
            .unwrap();
        assert_eq!(
            io.send(destination, &bytes[..len]).unwrap_err().kind(),
            io::ErrorKind::AddrInUse
        );
        io.cache_live = false;
        assert_eq!(io.send(destination, &bytes[..len]).unwrap(), len);
        io.cache_live = true;
        let len = Message::empty_ack(mid).encode(&mut bytes).unwrap();
        assert_eq!(io.send(destination, &bytes[..len]).unwrap(), len);
    }

    #[test]
    fn oversized_packets_are_consumed_one_at_a_time_and_valid_packets_survive() {
        let receiver = socket();
        let destination = receiver.local_addr().unwrap();
        let sender = socket();
        let endpoint = Endpoint::from(sender.local_addr().unwrap());
        let mut io = QueuedIo::new(receiver).unwrap();
        for capacity in [CoapProvisioner::DATAGRAM_CAPACITY, CAPACITY] {
            for length in [capacity + 1, 8192] {
                sender.send_to(&vec![0xA5; length], destination).unwrap();
                sender.send_to(b"valid", destination).unwrap();
                let mut bytes = [0; CAPACITY];
                for _ in 0..100 {
                    if let Some((len, from)) = io.receive(&mut bytes[..capacity]).unwrap() {
                        assert_eq!(from, endpoint);
                        assert_eq!(&bytes[..len], b"valid");
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(&bytes[..5], b"valid");
            }
        }
    }
}
