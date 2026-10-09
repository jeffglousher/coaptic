//! Real UDP host fixture for two managed devices and one shared cloud listener.
//!
//! Run `edhoc_cloud_peer DATABASE RUN_ID [reverse]` from the repository root.
//! The fixture uses public scalar-1/scalar-2/scalar-254 identities and simulated
//! commissioned anchors. Private-key, commissioning, authority and hardware
//! providers are unqualified. A resident Python SQLite provider commits complete
//! telemetry receipts and desired/applied state. Its current policy is a bounded
//! explicit fixture, and database restart checks the prior in-memory checkpoint;
//! this does not qualify an independently protected anti-rollback authority.
//!
//! One event loop receives from the shared listener, routes one datagram to its
//! authenticated managed owner and preserves exact endpoint isolation. Clients
//! interleave protected telemetry, durable duplicates, desired/applied state and
//! cross-device refusal. A deliberately lost receipt is recovered by CoAP retry.
//! Both clients then restart, authenticate fresh sessions and recover identical
//! durable receipts. Replayed prior-session traffic produces no application work.
//! JSON evidence includes complete response metadata and payloads. Timings include
//! intentional packet loss, provider IPC and socket waits; they are qualification
//! samples, not a throughput comparison or cryptographic CPU breakdown.
//! The debug host worker reserves a 16 MiB stack; stack high-water is unqualified.

use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use coaptic::message::{Type, decode};
use coaptic::provisioning::{
    AdmissionLimits, AdmissionRegistry, Association, CloudAdmission, CredentialReference, Identity,
    ManagedCall, ManagedConnection, OperationId, PeerTrust, PolicyValidity, Principal, Receipt,
    ReceiptError, ReceiptStore, TelemetryOperation, TrustAnchor, TrustGrant, commit_telemetry,
};
use coaptic::storage::{DatagramIo, UdpSocketIo};
use coaptic::{Code, ContentFormat, Endpoint, Method, Request, Response};

const CAPACITY: usize = 1472;
const DEADLINE_MS: u64 = 60_000;

fn checked<T, E: Debug>(result: Result<T, E>) -> io::Result<T> {
    result.map_err(|error| io::Error::other(format!("{error:?}")))
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).expect("public host-fixture identity")
}

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> io::Result<Vec<u8>> {
    if !value.is_ascii() || value.len() % 2 != 0 || value.len() > 4096 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    (0..value.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&value[offset..offset + 2], 16)
                .map_err(|_| io::ErrorKind::InvalidData.into())
        })
        .collect()
}

fn anchor_bytes(anchor: TrustAnchor) -> [u8; 72] {
    let (store, generation, digest) = anchor.parts();
    let mut bytes = [0; 72];
    bytes[..32].copy_from_slice(&store);
    bytes[32..40].copy_from_slice(&generation.to_be_bytes());
    bytes[40..].copy_from_slice(&digest);
    bytes
}

fn resource(device: usize) -> [u8; 32] {
    blake3::derive_key(
        "coaptic cloud host fixture device resource v1",
        &[device as u8 + 1],
    )
}

fn install(local: &Identity, peer: &Identity, store: u8) -> PeerTrust {
    PeerTrust::install(
        local,
        peer.peer(),
        TrustAnchor::genesis([store; 32]).unwrap(),
        |_, _, _| Ok::<_, ()>(()),
    )
    .expect("explicit simulated commissioning authority")
}

struct Registry {
    peers: [PeerTrust; 2],
}

impl AdmissionRegistry for Registry {
    fn resolve(&self, reference: CredentialReference) -> Option<&PeerTrust> {
        let mut entries = self
            .peers
            .iter()
            .filter(|trust| trust.checkpoint().peer().kid() == reference.kid());
        let trust = entries.next()?;
        if entries.next().is_some() {
            None
        } else {
            Some(trust)
        }
    }
}

struct Provider {
    process: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    database: PathBuf,
    peers: [(Principal, TrustAnchor); 2],
    active: bool,
}

impl Provider {
    fn start(
        database: PathBuf,
        peers: [(Principal, TrustAnchor); 2],
        checkpoint: Option<&[u8]>,
    ) -> io::Result<Self> {
        let python = std::env::var("COAPTIC_PYTHON").unwrap_or_else(|_| "python".into());
        let mut command = Command::new(python);
        command
            .arg("tools/provisioning/cloud_store_peer.py")
            .arg("--database")
            .arg(&database);
        for (index, (principal, anchor)) in peers.iter().enumerate() {
            command
                .arg("--peer")
                .arg(hex(principal.fingerprint()))
                .arg(hex(&anchor_bytes(*anchor)))
                .arg(hex(&resource(index)));
        }
        if let Some(checkpoint) = checkpoint {
            command.arg("--checkpoint").arg(hex(checkpoint));
        }
        let mut process = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = process.stdin.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let output = BufReader::new(process.stdout.take().ok_or(io::ErrorKind::BrokenPipe)?);
        let mut provider = Self {
            process,
            input,
            output,
            database,
            peers,
            active: true,
        };
        if provider.line()? != "ready" {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(provider)
    }

    fn line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        if Read::by_ref(&mut self.output)
            .take(4097)
            .read_line(&mut line)?
            == 0
            || line.len() > 4096
            || !line.ends_with('\n')
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn exchange(&mut self, command: &str) -> Result<Vec<u8>, ReceiptError<io::Error>> {
        if !self.active || command.len() > 4095 {
            return Err(ReceiptError::Persistence(
                io::ErrorKind::InvalidInput.into(),
            ));
        }
        writeln!(self.input, "{command}")
            .and_then(|_| self.input.flush())
            .map_err(ReceiptError::Persistence)?;
        let line = self.line().map_err(ReceiptError::Persistence)?;
        if let Some(value) = line.strip_prefix("ok ") {
            return unhex(value).map_err(ReceiptError::Persistence);
        }
        match line.as_str() {
            "conflict" => Err(ReceiptError::Conflict),
            "unauthorized" => Err(ReceiptError::Unauthorized),
            "capacity" => Err(ReceiptError::Capacity),
            _ => Err(ReceiptError::Persistence(io::ErrorKind::InvalidData.into())),
        }
    }

    fn desired(
        &mut self,
        principal: &Principal,
        anchor: TrustAnchor,
        device: usize,
    ) -> Result<Vec<u8>, ReceiptError<io::Error>> {
        self.exchange(&format!(
            "desired {} {} {}",
            hex(principal.fingerprint()),
            hex(&anchor_bytes(anchor)),
            hex(&resource(device))
        ))
    }

    fn applied(
        &mut self,
        principal: &Principal,
        anchor: TrustAnchor,
        device: usize,
        version: u64,
    ) -> Result<Vec<u8>, ReceiptError<io::Error>> {
        self.exchange(&format!(
            "applied {} {} {} {version}",
            hex(principal.fingerprint()),
            hex(&anchor_bytes(anchor)),
            hex(&resource(device))
        ))
    }

    fn count(&mut self) -> io::Result<u64> {
        let bytes = checked(self.exchange("count"))?;
        Ok(u64::from_be_bytes(
            bytes.try_into().map_err(|_| io::ErrorKind::InvalidData)?,
        ))
    }

    fn stop(&mut self) -> io::Result<()> {
        if self.active {
            writeln!(self.input, "stop")?;
            self.input.flush()?;
            if self.line()? != "ok" || !self.process.wait()?.success() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.active = false;
        }
        Ok(())
    }

    fn restart(&mut self) -> io::Result<()> {
        let checkpoint = checked(self.exchange("checkpoint"))?;
        self.stop()?;
        *self = Self::start(self.database.clone(), self.peers, Some(&checkpoint))?;
        Ok(())
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        if self.stop().is_err() {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}

impl ReceiptStore for Provider {
    type Error = io::Error;

    fn commit(
        &mut self,
        expected: &TrustAnchor,
        principal: &Principal,
        operation: &TelemetryOperation<'_>,
    ) -> Result<Receipt, ReceiptError<Self::Error>> {
        let bytes = self.exchange(&format!(
            "telemetry {} {} {} {} {} {} {}",
            hex(principal.fingerprint()),
            hex(&anchor_bytes(*expected)),
            hex(operation.resource()),
            hex(operation.id().as_bytes()),
            hex(operation.digest()),
            operation
                .content_format()
                .ok_or(ReceiptError::InvalidReceipt)?,
            if operation.payload().is_empty() {
                "-".to_owned()
            } else {
                hex(operation.payload())
            }
        ))?;
        Receipt::decode(&bytes).ok_or(ReceiptError::InvalidReceipt)
    }
}

#[derive(Clone)]
struct SharedSocket {
    socket: Rc<UdpSocket>,
    drop_next: Rc<Cell<bool>>,
    dropped: Rc<Cell<usize>>,
    sends: Rc<Cell<usize>>,
}

impl SharedSocket {
    fn send(&self, endpoint: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.sends.set(self.sends.get() + 1);
        if self.drop_next.replace(false) {
            self.dropped.set(self.dropped.get() + 1);
            return Ok(bytes.len());
        }
        self.socket.send_to(bytes, SocketAddr::from(endpoint))
    }
}

struct AdmissionIo(SharedSocket);

impl DatagramIo for AdmissionIo {
    type Error = io::Error;
    fn recv(&mut self, _: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        panic!("the shared event loop is the sole UDP receive owner")
    }
    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.0.send(endpoint, bytes)
    }
}

struct Queue {
    bytes: [u8; CAPACITY],
    packet: Option<(usize, Endpoint)>,
}

impl Queue {
    fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            packet: None,
        }
    }
    fn put(&mut self, bytes: &[u8], endpoint: Endpoint) -> io::Result<()> {
        if self.packet.is_some() || bytes.len() > CAPACITY {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.packet = Some((bytes.len(), endpoint));
        Ok(())
    }
}

struct PeerIo {
    shared: SharedSocket,
    queue: Rc<RefCell<Queue>>,
}

impl DatagramIo for PeerIo {
    type Error = io::Error;
    fn recv(&mut self, output: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        let mut queue = self.queue.borrow_mut();
        let Some((len, endpoint)) = queue.packet else {
            return Ok(None);
        };
        if len > output.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        output[..len].copy_from_slice(&queue.bytes[..len]);
        queue.packet = None;
        Ok(Some((len, endpoint)))
    }
    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        self.shared.send(endpoint, bytes)
    }
}

struct ClientIo {
    io: UdpSocketIo<[u8; CAPACITY + 1]>,
    last: Rc<RefCell<Queue>>,
}

impl DatagramIo for ClientIo {
    type Error = io::Error;
    fn recv(&mut self, output: &mut [u8]) -> io::Result<Option<(usize, Endpoint)>> {
        self.io.recv(output)
    }
    fn send(&mut self, endpoint: Endpoint, bytes: &[u8]) -> io::Result<usize> {
        let mut capture = self.last.borrow_mut();
        if bytes.len() > CAPACITY {
            return Err(io::ErrorKind::InvalidData.into());
        }
        capture.bytes[..bytes.len()].copy_from_slice(bytes);
        capture.packet = Some((bytes.len(), endpoint));
        self.io.send(endpoint, bytes)
    }
}

type Client<'a> = ManagedConnection<'a, ClientIo>;
type ServerOwner<'a> = ManagedConnection<'a, PeerIo>;
type ClientSet<'a> = ([Client<'a>; 2], [Endpoint; 2], [Rc<RefCell<Queue>>; 2]);

struct Owner<'a> {
    managed: ServerOwner<'a>,
    association: Association,
    queue: Rc<RefCell<Queue>>,
}

struct Harness<'a> {
    identity: &'a Identity,
    admission: CloudAdmission<'a, 4, 2, 4, 4>,
    registry: Registry,
    shared: SharedSocket,
    owners: [Option<Owner<'a>>; 2],
    provider: Provider,
    ignored_old: usize,
}

fn denial<E>(error: ReceiptError<E>) -> Response<'static> {
    match error {
        ReceiptError::Unauthorized => Response::forbidden(),
        ReceiptError::Conflict => Response::new(Code::CONFLICT),
        ReceiptError::Capacity => Response::service_unavailable(),
        _ => Response::internal_error(),
    }
}

fn dispatch(
    principal: &Principal,
    request: Request<'_>,
    trust: &PeerTrust,
    grant: TrustGrant,
    device: usize,
    provider: &mut Provider,
) -> Response<'static> {
    let path = request.path();
    if !trust.permits(&grant, principal) {
        return Response::forbidden();
    }
    if path.len() != 3 || path[0] != "devices" {
        return Response::not_found();
    }
    let target = match path[1] {
        "first" => 0,
        "second" => 1,
        _ => return Response::not_found(),
    };
    if target != device {
        return Response::forbidden();
    }
    match path[2] {
        "telemetry" => {
            if request.code() != Code::POST {
                return Response::method_not_allowed();
            }
            if request.content_format() != Some(Ok(ContentFormat::OCTET_STREAM))
                || request.payload().len() < 16
            {
                return Response::bad_request();
            }
            let id = OperationId::new(
                request.payload()[..16]
                    .try_into()
                    .expect("checked operation identity"),
            );
            let operation = match TelemetryOperation::new(
                id,
                resource(device),
                Some(42),
                &request.payload()[16..],
            ) {
                Ok(operation) => operation,
                Err(error) => return denial(error),
            };
            match commit_telemetry(trust, &grant, &operation, provider) {
                Ok(receipt) => Response::changed()
                    .payload_copy(&receipt.encode())
                    .content_format(ContentFormat::OCTET_STREAM),
                Err(error) => denial(error),
            }
        }
        "desired" => {
            if request.code() != Code::GET
                || !request.payload().is_empty()
                || request.content_format().is_some()
            {
                return Response::bad_request();
            }
            match provider.desired(principal, grant.anchor(), device) {
                Ok(payload) => {
                    Response::content_copy(&payload).content_format(ContentFormat::OCTET_STREAM)
                }
                Err(error) => denial(error),
            }
        }
        "applied" => {
            if request.code() != Code::PUT
                || request.payload().len() != 8
                || request.content_format() != Some(Ok(ContentFormat::OCTET_STREAM))
            {
                return Response::bad_request();
            }
            let version = u64::from_be_bytes(
                request
                    .payload()
                    .try_into()
                    .expect("checked applied version"),
            );
            match provider.applied(principal, grant.anchor(), device, version) {
                Ok(payload) => Response::changed()
                    .payload_copy(&payload)
                    .content_format(ContentFormat::OCTET_STREAM),
                Err(error) => denial(error),
            }
        }
        _ => Response::not_found(),
    }
}

impl<'a> Harness<'a> {
    fn step(&mut self, now: u64, order: [usize; 2]) -> io::Result<()> {
        let mut bytes = [0; CAPACITY + 1];
        match self.shared.socket.recv_from(&mut bytes) {
            Ok((len, address)) if len <= CAPACITY => {
                let endpoint = Endpoint::from(address);
                if decode(&bytes[..len]).is_ok_and(|message| message.oscore().is_some()) {
                    if let Some(association) = checked(self.admission.route_oscore(
                        &bytes[..len],
                        endpoint,
                        now,
                        &self.registry,
                    ))? {
                        let owner = self
                            .owners
                            .iter_mut()
                            .flatten()
                            .find(|owner| owner.association == association)
                            .ok_or(io::ErrorKind::InvalidData)?;
                        owner.queue.borrow_mut().put(&bytes[..len], endpoint)?;
                    } else {
                        self.ignored_old += 1;
                    }
                } else {
                    checked(self.admission.ingest(
                        &bytes[..len],
                        endpoint,
                        now,
                        &self.registry,
                        entropy,
                    ))?;
                }
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(10040) => {}
            Err(error) => return Err(error),
        }
        checked(
            self.admission
                .flush(&mut AdmissionIo(self.shared.clone()), now, &self.registry),
        )?;
        for _ in 0..2 {
            let Some(admitted) = checked(self.admission.take_session(now, &self.registry))? else {
                break;
            };
            let (session, association) = admitted.into_parts();
            let device = self
                .registry
                .peers
                .iter()
                .position(|trust| trust.checkpoint().peer().principal() == *session.principal())
                .ok_or(io::ErrorKind::PermissionDenied)?;
            if self.owners[device].is_some() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let policy = &self.registry.peers[device];
            let trust = checked(PeerTrust::restore(
                self.identity,
                policy.checkpoint().clone(),
                policy.checkpoint().anchor(),
            ))?;
            let queue = Rc::new(RefCell::new(Queue::new()));
            let managed = checked(ManagedConnection::from_admitted_session(
                self.identity,
                trust,
                *association.grant(),
                session,
                association.endpoint(),
                PeerIo {
                    shared: self.shared.clone(),
                    queue: queue.clone(),
                },
                entropy,
                PolicyValidity::OnlineUntil {
                    accepted_ms: now,
                    deadline_ms: DEADLINE_MS,
                },
                now,
            ))?;
            println!(
                "{{\"admitted\":true,\"device\":{},\"principal\":\"{}\",\"resource\":\"{}\",\"local_cid\":\"{}\",\"endpoint\":\"{}\"}}",
                device + 1,
                hex(association.grant().principal().fingerprint()),
                hex(&resource(device)),
                hex(association.local_connection_id().as_bytes()),
                association.endpoint()
            );
            self.owners[device] = Some(Owner {
                managed,
                association,
                queue,
            });
        }
        for device in order {
            if let Some(owner) = &mut self.owners[device] {
                let trust = &self.registry.peers[device];
                let grant = *owner.association.grant();
                checked(owner.managed.poll_with(now, |principal, request, _| {
                    dispatch(principal, request, trust, grant, device, &mut self.provider)
                }))?;
            }
        }
        Ok(())
    }

    fn retire(&mut self) -> io::Result<()> {
        for owner in &mut self.owners {
            if let Some(owner) = owner.take() {
                if !self.admission.release(owner.association) {
                    return Err(io::ErrorKind::InvalidData.into());
                }
            }
        }
        Ok(())
    }
}

fn now(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn make_clients<'a>(
    identities: &'a [Identity; 2],
    server: &Identity,
    endpoint: Endpoint,
    now: u64,
) -> io::Result<ClientSet<'a>> {
    let sockets = [
        UdpSocket::bind("127.0.0.1:0")?,
        UdpSocket::bind("127.0.0.1:0")?,
    ];
    for socket in &sockets {
        socket.set_nonblocking(true)?;
    }
    let endpoints = [
        Endpoint::from(sockets[0].local_addr()?),
        Endpoint::from(sockets[1].local_addr()?),
    ];
    let captures = [
        Rc::new(RefCell::new(Queue::new())),
        Rc::new(RefCell::new(Queue::new())),
    ];
    let [first, second] = sockets;
    let peers = [
        checked(ManagedConnection::new(
            &identities[0],
            install(&identities[0], server, 100),
            endpoint,
            ClientIo {
                io: UdpSocketIo::new(first, [0; CAPACITY + 1])?,
                last: captures[0].clone(),
            },
            entropy,
            PolicyValidity::OnlineUntil {
                accepted_ms: now,
                deadline_ms: DEADLINE_MS,
            },
            now,
        ))?,
        checked(ManagedConnection::new(
            &identities[1],
            install(&identities[1], server, 101),
            endpoint,
            ClientIo {
                io: UdpSocketIo::new(second, [0; CAPACITY + 1])?,
                last: captures[1].clone(),
            },
            entropy,
            PolicyValidity::OnlineUntil {
                accepted_ms: now,
                deadline_ms: DEADLINE_MS,
            },
            now,
        ))?,
    ];
    Ok((peers, endpoints, captures))
}

fn connect(
    harness: &mut Harness<'_>,
    clients: &mut [Client<'_>; 2],
    started: Instant,
    order: [usize; 2],
) -> io::Result<()> {
    for device in order {
        checked(clients[device].connect(now(started)))?;
    }
    let until = now(started) + 20_000;
    loop {
        let now = now(started);
        if now >= until {
            return Err(io::ErrorKind::TimedOut.into());
        }
        harness.step(now, order)?;
        for device in order {
            checked(clients[device].poll(now))?;
        }
        if clients.iter().all(ManagedConnection::is_connected)
            && harness.owners.iter().all(Option::is_some)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Spec<'a> {
    method: Method,
    path: &'static str,
    target: usize,
    payload: &'a [u8],
    format: Option<ContentFormat>,
    code: Code,
    response: Option<&'a [u8]>,
}

fn round(
    harness: &mut Harness<'_>,
    clients: &mut [Client<'_>; 2],
    captures: &[Rc<RefCell<Queue>>; 2],
    started: Instant,
    order: [usize; 2],
    phase: &str,
    specs: [Spec<'_>; 2],
) -> io::Result<[Vec<u8>; 2]> {
    let mut calls: [Option<ManagedCall>; 2] = [None, None];
    let mut outputs: [Option<Vec<u8>>; 2] = [None, None];
    for device in order {
        calls[device] = Some(checked(clients[device].request(
            specs[device].method,
            specs[device].path,
            specs[device].payload,
            specs[device].format,
            now(started),
        ))?);
    }
    let until = now(started) + 12_000;
    loop {
        let now = now(started);
        if now >= until {
            return Err(io::Error::other(format!("timed out during {phase}")));
        }
        harness.step(now, order)?;
        for device in order {
            checked(clients[device].poll(now))?;
            let Some(call) = calls[device] else {
                continue;
            };
            let mut bytes = [0; CAPACITY];
            let Some(response) =
                checked(clients[device].take_response_into(call, &mut bytes, now))?
            else {
                continue;
            };
            let response = checked(response)?;
            let capture = captures[device].borrow();
            let (request_len, request_peer) = capture.packet.ok_or(io::ErrorKind::InvalidData)?;
            let sent = checked(decode(&capture.bytes[..request_len]))?;
            if sent.ty() != Type::Confirmable
                || sent.token() != call.token()
                || sent.oscore().is_none()
                || request_peer != call.peer()
            {
                return Err(io::Error::other(
                    "complete protected request capture mismatch",
                ));
            }
            let request_mid = sent.message_id();
            let format = if specs[device].code.is_success() {
                Some(ContentFormat::OCTET_STREAM)
            } else {
                None
            };
            if response.code() != specs[device].code
                || response.ty() != Some(Type::Acknowledgement)
                || response.token() != Some(call.token())
                || response.peer() != Some(call.peer())
                || response.message_id() != Some(request_mid)
                || response.format() != format
                || response.payload_truncated()
                || response.payload_src_len() != response.payload().len()
                || specs[device]
                    .response
                    .is_some_and(|expected| expected != response.payload())
            {
                return Err(io::Error::other(format!(
                    "complete response mismatch during {phase}: {response:?}"
                )));
            }
            println!(
                "{{\"phase\":\"{phase}\",\"device\":{},\"target\":{},\"code\":{},\"mtype\":2,\"mid\":{},\"request_mid\":{},\"token\":\"{}\",\"peer\":\"{}\",\"content_format\":{},\"payload\":\"{}\",\"request_payload\":\"{}\",\"source_len\":{},\"truncated\":false}}",
                device + 1,
                specs[device].target + 1,
                response.code().as_raw(),
                response.message_id().unwrap().get(),
                request_mid.get(),
                hex(call.token().as_bytes()),
                call.peer(),
                format.map_or("null".to_owned(), |format| format.get().to_string()),
                hex(response.payload()),
                hex(specs[device].payload),
                response.payload_src_len()
            );
            outputs[device] = Some(response.payload().to_vec());
            calls[device] = None;
        }
        if calls.iter().all(Option::is_none) {
            return Ok(outputs.map(|value| value.expect("complete protected response")));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn telemetry_specs<'a>(
    payloads: &'a [Vec<u8>; 2],
    code: Code,
    responses: Option<&'a [Vec<u8>; 2]>,
    cross: bool,
) -> [Spec<'a>; 2] {
    core::array::from_fn(|device| {
        let target = if cross { 1 - device } else { device };
        Spec {
            method: Method::Post,
            path: if target == 0 {
                "devices/first/telemetry"
            } else {
                "devices/second/telemetry"
            },
            target,
            payload: &payloads[device],
            format: Some(ContentFormat::OCTET_STREAM),
            code,
            response: responses
                .map(|responses| responses[device].as_slice())
                .or_else(|| (!code.is_success()).then_some(&[][..])),
        }
    })
}

fn run(database: &Path, run_id: u64, reverse: bool) -> io::Result<()> {
    let started = Instant::now();
    let identities = [identity(1, 1), identity(2, 2)];
    let server_identity = identity(254, 254);
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let endpoint = Endpoint::from(socket.local_addr()?);
    let registry = Registry {
        peers: [
            install(&server_identity, &identities[0], 1),
            install(&server_identity, &identities[1], 2),
        ],
    };
    let peers = core::array::from_fn(|device| {
        (
            identities[device].peer().principal(),
            registry.peers[device].checkpoint().anchor(),
        )
    });
    let provider = Provider::start(database.to_owned(), peers, None)?;
    let shared = SharedSocket {
        socket: Rc::new(socket),
        drop_next: Rc::new(Cell::new(false)),
        dropped: Rc::new(Cell::new(0)),
        sends: Rc::new(Cell::new(0)),
    };
    let limits = AdmissionLimits {
        global_crypto: 16,
        source_crypto: 16,
        source_datagrams: 64,
        ..AdmissionLimits::default()
    };
    let mut harness = Harness {
        identity: &server_identity,
        admission: checked(CloudAdmission::new(&server_identity, 0, limits, entropy))?,
        registry,
        shared,
        owners: [None, None],
        provider,
        ignored_old: 0,
    };
    let order = if reverse { [1, 0] } else { [0, 1] };
    let (mut clients, old_endpoints, captures) =
        make_clients(&identities, &server_identity, endpoint, now(started))?;
    println!(
        "{{\"started\":true,\"run_id\":{run_id},\"reverse\":{reverse},\"server\":\"{endpoint}\",\"production_providers_qualified\":false,\"hardware_qualified\":false,\"listener_sockets\":1,\"devices\":2,\"client_order\":[{},{}],\"global_datagrams\":64,\"source_datagrams\":64,\"global_crypto\":16,\"source_crypto\":16}}",
        order[0] + 1,
        order[1] + 1
    );
    let before = harness.provider.count()?;
    connect(&mut harness, &mut clients, started, order)?;
    let bodies = [
        b"device-one-telemetry\x00\xff".as_slice(),
        b"device-two-telemetry\xff\x00".as_slice(),
    ];
    let operations: [OperationId; 2] = core::array::from_fn(|device| {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&(device as u64 + 1).to_be_bytes());
        bytes[8..].copy_from_slice(&run_id.to_be_bytes());
        OperationId::new(bytes)
    });
    let payloads: [Vec<u8>; 2] = core::array::from_fn(|device| {
        let mut payload = operations[device].as_bytes().to_vec();
        payload.extend_from_slice(bodies[device]);
        payload
    });
    harness.shared.drop_next.set(true);
    let receipts = round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "telemetry_lost_receipt",
        telemetry_specs(&payloads, Code::CHANGED, None, false),
    )?;
    for device in 0..2 {
        let receipt = Receipt::decode(&receipts[device]).ok_or(io::ErrorKind::InvalidData)?;
        let operation = checked(TelemetryOperation::new(
            operations[device],
            resource(device),
            Some(42),
            bodies[device],
        ))?;
        if receipt.id() != operations[device] || receipt.digest() != operation.digest() {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    if receipts[0] == receipts[1] || harness.provider.count()? != before + 2 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "duplicate",
        telemetry_specs(&payloads, Code::CHANGED, Some(&receipts), false),
    )?;
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "cross_device_denied",
        telemetry_specs(&payloads, Code::FORBIDDEN, None, true),
    )?;
    let mut changed = payloads.clone();
    for payload in &mut changed {
        *payload.last_mut().expect("nonempty telemetry") ^= 1;
    }
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "operation_content_conflict",
        telemetry_specs(&changed, Code::CONFLICT, None, false),
    )?;
    let desired: [Vec<u8>; 2] = core::array::from_fn(|device| {
        let mut payload = 1_u64.to_be_bytes().to_vec();
        payload.extend_from_slice(b"device-config-");
        payload.extend_from_slice(&[device as u8 + 1, 0, 255]);
        payload
    });
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "desired",
        core::array::from_fn(|device| Spec {
            method: Method::Get,
            path: if device == 0 {
                "devices/first/desired"
            } else {
                "devices/second/desired"
            },
            target: device,
            payload: &[],
            format: None,
            code: Code::CONTENT,
            response: Some(&desired[device]),
        }),
    )?;
    let version = 1_u64.to_be_bytes();
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "applied",
        core::array::from_fn(|device| Spec {
            method: Method::Put,
            path: if device == 0 {
                "devices/first/applied"
            } else {
                "devices/second/applied"
            },
            target: device,
            payload: &version,
            format: Some(ContentFormat::OCTET_STREAM),
            code: Code::CHANGED,
            response: Some(&version),
        }),
    )?;
    let old_packets: [Vec<u8>; 2] = core::array::from_fn(|device| {
        let capture = captures[device].borrow();
        let (len, _) = capture.packet.expect("prior-session request");
        capture.bytes[..len].to_vec()
    });
    harness.retire()?;
    drop(clients);
    let replay_sockets = [
        UdpSocket::bind(SocketAddr::from(old_endpoints[0]))?,
        UdpSocket::bind(SocketAddr::from(old_endpoints[1]))?,
    ];
    harness.provider.restart()?;
    let (mut clients, _, captures) =
        make_clients(&identities, &server_identity, endpoint, now(started))?;
    connect(&mut harness, &mut clients, started, order)?;
    for device in order {
        replay_sockets[device].send_to(&old_packets[device], SocketAddr::from(endpoint))?;
    }
    let replay_until = now(started) + 1000;
    while harness.ignored_old < 2 && now(started) < replay_until {
        harness.step(now(started), order)?;
        std::thread::sleep(Duration::from_millis(1));
    }
    if harness.ignored_old != 2 || harness.provider.count()? != before + 2 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    round(
        &mut harness,
        &mut clients,
        &captures,
        started,
        order,
        "restart_receipt",
        telemetry_specs(&payloads, Code::CHANGED, Some(&receipts), false),
    )?;
    if harness.provider.count()? != before + 2 || harness.shared.dropped.get() != 1 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    harness.provider.stop()?;
    println!(
        "{{\"complete\":true,\"run_id\":{run_id},\"reverse\":{reverse},\"wall_ms\":{},\"server_sends\":{},\"intentionally_lost_receipts\":1,\"old_session_packets_refused\":2,\"new_effects\":2,\"receipt_count\":{},\"managed_client_bytes\":{},\"managed_server_owner_bytes\":{},\"cloud_admission_bytes\":{},\"production_providers_qualified\":false,\"throughput_comparison\":false}}",
        now(started),
        harness.shared.sends.get(),
        before + 2,
        core::mem::size_of::<Client<'_>>(),
        core::mem::size_of::<ServerOwner<'_>>(),
        core::mem::size_of::<CloudAdmission<'_, 4, 2, 4, 4>>()
    );
    Ok(())
}

fn main() -> io::Result<()> {
    let arguments: Vec<_> = std::env::args().collect();
    if !(3..=4).contains(&arguments.len()) || (arguments.len() == 4 && arguments[3] != "reverse") {
        return Err(io::Error::other(
            "usage: edhoc_cloud_peer DATABASE RUN_ID [reverse]",
        ));
    }
    let run_id = arguments[2]
        .parse::<u64>()
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    std::thread::Builder::new()
        .name("two-device-cloud-fixture".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || run(Path::new(&arguments[1]), run_id, arguments.len() == 4))?
        .join()
        .map_err(|_| io::Error::other("host fixture panicked"))?
}
