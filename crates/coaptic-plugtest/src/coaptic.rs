//! [`CoapticPeer`]: App server + Engine client over a capturing [`DatagramIo`].

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use coaptic::message::{
    BlockValue, Code, ContentFormat, Ids, Message, MessageId, Opt, OptionsBuilder, Token, Type,
    decode, empty_ack, encode, encode_observe,
};
use coaptic::storage::{DatagramIo, Endpoint, Engine, EngineBuilder, Memory};
use coaptic::{App, Response, profiles};

use crate::pcap::{Capture, CapturingIo, bind_loopback};
use crate::peer::{ClientRequest, ClientResponse, Peer, PeerError};
use crate::site;

type ServerApp = App<profiles::Default, InterceptIo<UdpSocket>, 24>;

/// Coaptic backend (library App/Engine; no extra runtime deps).
pub struct CoapticPeer {
    capture: Capture,
    server: Option<ServerCtl>,
    client_addr: Option<SocketAddr>,
}

struct ServerCtl {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    notify: crate::peer::NotifyMailbox,
    join: Option<JoinHandle<()>>,
}

impl Default for CoapticPeer {
    fn default() -> Self {
        Self::new()
    }
}

impl CoapticPeer {
    /// New peer with a fresh capture log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capture: Capture::new(),
            server: None,
            client_addr: None,
        }
    }
}

impl Peer for CoapticPeer {
    fn name(&self) -> &'static str {
        "coaptic"
    }

    fn start_server(&mut self) -> Result<SocketAddr, PeerError> {
        self.stop_server();
        site::reset();
        let (sock, addr) = bind_loopback().map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Mutex::new(None));
        let capture = self.capture.clone();
        let stop_t = Arc::clone(&stop);
        let notify_t = Arc::clone(&notify);
        let join = thread::Builder::new()
            .name("coaptic-plugtest-server".into())
            .spawn(move || server_loop(sock, addr, capture, stop_t, notify_t))
            .map_err(|e| e.to_string())?;
        // Let the thread bind the App.
        thread::sleep(Duration::from_millis(20));
        self.server = Some(ServerCtl {
            addr,
            stop,
            notify,
            join: Some(join),
        });
        Ok(addr)
    }

    fn stop_server(&mut self) {
        if let Some(mut srv) = self.server.take() {
            srv.stop.store(true, Ordering::SeqCst);
            if let Some(join) = srv.join.take() {
                let _ = join.join();
            }
        }
    }

    fn send_request(
        &mut self,
        dest: SocketAddr,
        req: &ClientRequest,
    ) -> Result<ClientResponse, PeerError> {
        let (sock, local) = bind_loopback().map_err(|e| e.to_string())?;
        self.client_addr = Some(local);
        let io = CapturingIo::new(sock, local, self.capture.clone());
        client_exchange(io, dest, req)
    }

    fn poll(&mut self, _now_ms: u64) -> Result<(), PeerError> {
        Ok(())
    }

    fn local_addr(&self) -> Option<SocketAddr> {
        self.server.as_ref().map(|s| s.addr).or(self.client_addr)
    }

    fn take_capture(&mut self) -> Capture {
        self.capture.clone()
    }

    fn notify(&mut self, path: &[&str], payload: &[u8]) -> Result<(), PeerError> {
        let srv = self.server.as_ref().ok_or("no server")?;
        *srv.notify.lock().expect("notify") = Some((
            path.iter().map(|s| (*s).to_owned()).collect(),
            payload.to_vec(),
        ));
        Ok(())
    }

    fn supports_dtls(&self) -> bool {
        cfg!(feature = "dtls")
    }
}

impl Drop for CoapticPeer {
    fn drop(&mut self) {
        self.stop_server();
    }
}

struct SeparateJob {
    dest: Endpoint,
    token: Token,
    con: bool,
}

struct InterceptIo<T> {
    inner: CapturingIo<T>,
    separate: Arc<Mutex<Vec<SeparateJob>>>,
    ids: Arc<Mutex<Ids>>,
}

impl<T: DatagramIo<Error = std::io::Error>> DatagramIo for InterceptIo<T> {
    type Error = std::io::Error;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        match self.inner.recv(buf)? {
            Some((n, ep)) => {
                if intercept_datagram(&mut self.inner, &self.separate, &self.ids, &buf[..n], ep)? {
                    return Ok(None);
                }
                Ok(Some((n, ep)))
            }
            None => Ok(None),
        }
    }

    fn send(&mut self, dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.inner.send(dest, bytes)
    }
}

/// `true` if the datagram was consumed (App should see idle).
fn intercept_datagram<T: DatagramIo<Error = std::io::Error>>(
    io: &mut CapturingIo<T>,
    separate: &Arc<Mutex<Vec<SeparateJob>>>,
    ids: &Arc<Mutex<Ids>>,
    bytes: &[u8],
    ep: Endpoint,
) -> Result<bool, std::io::Error> {
    let Ok(parsed) = decode(bytes) else {
        return Ok(false);
    };
    let path: Vec<&str> = parsed.uri_path().filter_map(|s| s.ok()).collect();
    if parsed.code() == Code::GET && path == ["separate"] {
        if parsed.ty() == Type::Confirmable {
            send_msg(io, ep, &empty_ack(parsed.message_id()))?;
        }
        separate.lock().expect("sep").push(SeparateJob {
            dest: ep,
            token: parsed.token(),
            con: parsed.ty() == Type::Confirmable,
        });
        return Ok(true);
    }
    if parsed.code() == Code::POST && path == ["test"] {
        let loc = [
            Opt::location_path("location1"),
            Opt::location_path("location2"),
            Opt::location_query("first=1"),
            Opt::location_query("second=2"),
        ];
        let mut opts = OptionsBuilder::<8>::new();
        for o in loc {
            let _ = opts.push(o);
        }
        let ty = if parsed.ty() == Type::Confirmable {
            Type::Acknowledgement
        } else {
            Type::NonConfirmable
        };
        let mid = if parsed.ty() == Type::Confirmable {
            parsed.message_id()
        } else {
            ids.lock().expect("ids").next()
        };
        let msg = Message::new(ty, Code::CREATED, mid)
            .with_token(parsed.token())
            .with_options(opts.as_slice());
        send_msg(io, ep, &msg)?;
        return Ok(true);
    }
    Ok(false)
}

fn send_msg<T: DatagramIo<Error = std::io::Error>>(
    io: &mut CapturingIo<T>,
    dest: Endpoint,
    msg: &Message<'_>,
) -> Result<(), std::io::Error> {
    let mut buf = [0u8; 1472];
    let n = encode(msg, &mut buf).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
    io.send(dest, &buf[..n])?;
    Ok(())
}

fn server_loop(
    sock: UdpSocket,
    addr: SocketAddr,
    capture: Capture,
    stop: Arc<AtomicBool>,
    notify: crate::peer::NotifyMailbox,
) {
    let separate = Arc::new(Mutex::new(Vec::new()));
    let ids = Arc::new(Mutex::new(Ids::new(0x9000)));
    let io = InterceptIo {
        inner: CapturingIo::new(sock, addr, capture),
        separate: Arc::clone(&separate),
        ids: Arc::clone(&ids),
    };
    let mut app = bind_site(io);
    let origin = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let now = u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        let _ = app.poll(now);
        flush_separate(&mut app, &separate, &ids);
        if let Some((path, payload)) = notify.lock().expect("n").take() {
            let segs: Vec<&str> = path.iter().map(String::as_str).collect();
            let body: &'static [u8] = if payload == site::OBS_BODY_2 {
                site::OBS_BODY_2
            } else if payload == site::OBS_BODY {
                site::OBS_BODY
            } else {
                site::OBS_BODY_2
            };
            let _ = app.notify(
                now,
                &segs,
                Response::content(body).content_format(ContentFormat::TEXT_PLAIN),
            );
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Bind the plugtest site onto `io` (plaintext UDP or a harness DTLS adapter).
pub(crate) fn bind_site<T: DatagramIo>(io: T) -> App<profiles::Default, T, 24>
where
    T::Error: std::fmt::Debug,
{
    let mut b = App::profile::<profiles::Default>()
        .block_wise(true)
        .routes::<24>();
    for (path, router) in site::routers() {
        b = b.route(path, router);
    }
    for (path, router) in site::extra_routers() {
        b = b.route(path, router);
    }
    b.bind(io).expect("bind plugtest App")
}

fn flush_separate(
    app: &mut ServerApp,
    separate: &Arc<Mutex<Vec<SeparateJob>>>,
    ids: &Arc<Mutex<Ids>>,
) {
    let jobs = std::mem::take(&mut *separate.lock().expect("sep"));
    for job in jobs {
        let ty = if job.con {
            Type::Confirmable
        } else {
            Type::NonConfirmable
        };
        let mid = ids.lock().expect("ids").next();
        let cf = ContentFormat::TEXT_PLAIN.encode();
        let extra = [Opt::content_format(&cf)];
        let mut opts = OptionsBuilder::<4>::new();
        for o in extra {
            let _ = opts.push(o);
        }
        let msg = Message::new(ty, Code::CONTENT, mid)
            .with_token(job.token)
            .with_options(opts.as_slice())
            .with_payload(site::SEP_BODY);
        let _ = send_msg(app.transport_mut().inner_access(), job.dest, &msg);
    }
}

impl InterceptIo<UdpSocket> {
    fn inner_access(&mut self) -> &mut CapturingIo<UdpSocket> {
        &mut self.inner
    }
}

fn client_exchange<T: DatagramIo<Error = std::io::Error>>(
    mut io: T,
    dest: SocketAddr,
    req: &ClientRequest,
) -> Result<ClientResponse, PeerError> {
    let dest_ep = Endpoint::from(dest);
    if req.code == Code::EMPTY {
        return client_ping(&mut io, dest_ep, req.timeout);
    }
    if (req.code == Code::PUT || req.code == Code::POST)
        && (req.payload.len() > 512 || req.path.iter().any(|p| p.starts_with("large")))
    {
        return client_block1(&mut io, dest_ep, req);
    }
    if req.path.iter().any(|p| p == "large") && req.code == Code::GET {
        return client_block2(&mut io, dest_ep, req);
    }
    let (wire, token, mid) = encode_client_req(req)?;
    io.send(dest_ep, &wire)
        .map_err(|e| PeerError(e.to_string()))?;
    let deadline = Instant::now() + req.timeout;
    let mut notifications = 0u8;
    while Instant::now() < deadline {
        let mut buf = [0u8; 1472];
        match io.recv(&mut buf) {
            Ok(Some((n, _))) => {
                let parsed = decode(&buf[..n]).map_err(|e| format!("decode: {e:?}"))?;
                if parsed.is_empty_ack() && parsed.message_id().get() == mid.get() {
                    continue;
                }
                if parsed.token() != token && !parsed.is_empty_rst() {
                    continue;
                }
                if parsed.ty() == Type::Confirmable && !parsed.code().is_request() {
                    let _ = send_raw(&mut io, dest_ep, &empty_ack(parsed.message_id()));
                }
                if req.observe == Some(0) && parsed.observe().and_then(Result::ok).unwrap_or(0) > 0
                {
                    notifications = notifications.saturating_add(1);
                    if notifications < 1 {
                        continue;
                    }
                }
                return Ok(view_response(&parsed));
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(PeerError(e.to_string())),
        }
    }
    Err(PeerError(format!(
        "timeout waiting for {} {}",
        req.code,
        req.path.join("/")
    )))
}

fn client_ping<T: DatagramIo<Error = std::io::Error>>(
    io: &mut T,
    dest: Endpoint,
    timeout: Duration,
) -> Result<ClientResponse, PeerError> {
    let mid = MessageId::new(0x5049);
    let ping = Message::new(Type::Confirmable, Code::EMPTY, mid);
    send_raw(io, dest, &ping)?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let mut buf = [0u8; 64];
        match io.recv(&mut buf) {
            Ok(Some((n, _))) => {
                let parsed = decode(&buf[..n]).map_err(|e| format!("{e:?}"))?;
                if parsed.message_id() == mid && parsed.is_empty_rst() {
                    return Ok(ClientResponse {
                        ty: Type::Reset,
                        code: Code::EMPTY,
                        payload: Vec::new(),
                        body: None,
                        content_format: None,
                        observe: None,
                        etag: Vec::new(),
                        location_path: Vec::new(),
                        location_query: Vec::new(),
                        rst: true,
                    });
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(PeerError(e.to_string())),
        }
    }
    Err(PeerError("ping timeout (no RST)".into()))
}

fn client_block2<T: DatagramIo<Error = std::io::Error>>(
    io: &mut T,
    dest: Endpoint,
    req: &ClientRequest,
) -> Result<ClientResponse, PeerError> {
    let mut assembled = Vec::new();
    let mut num = 0u32;
    let mut size = req.block2.map(|(_, _, s)| s).unwrap_or(1024);
    let token = mint_token(req.token_len);
    let mut ids = Ids::new(0xB200);
    let deadline = Instant::now() + req.timeout + Duration::from_secs(2);
    loop {
        if Instant::now() > deadline {
            return Err(PeerError("Block2 timeout".into()));
        }
        let mut extra = Vec::new();
        if num == 0 {
            if let Some((_, _, s)) = req.block2 {
                size = s;
                extra.push(block2_opt(0, false, s));
            }
        } else {
            extra.push(block2_opt(num, false, size));
        }
        let wire = encode_path_req(ids.next(), token, req, &extra)?;
        io.send(dest, &wire).map_err(|e| e.to_string())?;
        let parsed = recv_matching(io, token, deadline)?;
        if parsed.ty == Type::Confirmable {
            let _ = send_raw(io, dest, &empty_ack(MessageId::new(parsed.mid)));
        }
        assembled.extend_from_slice(&parsed.payload);
        match parsed.block2 {
            Some((n, true)) => {
                num = n + 1;
                continue;
            }
            Some((_, false)) | None => {
                return Ok(ClientResponse {
                    ty: parsed.ty,
                    code: parsed.code,
                    payload: parsed.payload,
                    body: Some(assembled),
                    content_format: parsed.content_format,
                    observe: parsed.observe,
                    etag: parsed.etag,
                    location_path: parsed.location_path,
                    location_query: parsed.location_query,
                    rst: false,
                });
            }
        }
    }
}

fn client_block1<T: DatagramIo<Error = std::io::Error>>(
    io: &mut T,
    dest: Endpoint,
    req: &ClientRequest,
) -> Result<ClientResponse, PeerError> {
    let body = if req.payload.is_empty() {
        site::large_body()
    } else {
        req.payload.clone()
    };
    let size = 64u16;
    let blocks = body.len().div_ceil(usize::from(size)) as u32;
    let token = mint_token(req.token_len);
    let mut ids = Ids::new(0xB100);
    let deadline = Instant::now() + req.timeout + Duration::from_secs(3);
    let mut last = None;
    for num in 0..blocks {
        if Instant::now() > deadline {
            return Err(PeerError("Block1 timeout".into()));
        }
        let start = (num as usize) * usize::from(size);
        let end = (start + usize::from(size)).min(body.len());
        let more = end < body.len();
        let extra = [block1_opt(num, more, size)];
        let mut r = req.clone();
        r.payload = body[start..end].to_vec();
        if r.content_format.is_none() {
            r.content_format = Some(0);
        }
        let wire = encode_path_req(ids.next(), token, &r, &extra)?;
        io.send(dest, &wire).map_err(|e| e.to_string())?;
        let parsed = recv_matching(io, token, deadline)?;
        last = Some(parsed);
    }
    let parsed = last.ok_or("Block1 sent nothing")?;
    Ok(ClientResponse {
        ty: parsed.ty,
        code: parsed.code,
        payload: parsed.payload,
        body: None,
        content_format: parsed.content_format,
        observe: parsed.observe,
        etag: parsed.etag,
        location_path: parsed.location_path,
        location_query: parsed.location_query,
        rst: false,
    })
}

fn block2_opt(num: u32, more: bool, size: u16) -> EncodedOpt {
    EncodedOpt::Block2(
        BlockValue::from_size(num, more, size)
            .expect("szx")
            .encode(),
    )
}

fn block1_opt(num: u32, more: bool, size: u16) -> EncodedOpt {
    EncodedOpt::Block1(
        BlockValue::from_size(num, more, size)
            .expect("szx")
            .encode(),
    )
}

enum EncodedOpt {
    Block2(coaptic::message::EncodedUint),
    Block1(coaptic::message::EncodedUint),
}

fn encode_path_req(
    mid: MessageId,
    token: Token,
    req: &ClientRequest,
    extra_block: &[EncodedOpt],
) -> Result<Vec<u8>, PeerError> {
    let mut opts = OptionsBuilder::<16>::new();
    for seg in &req.path {
        opts.push(Opt::uri_path(seg)).map_err(|_| "opt full")?;
    }
    for q in &req.query {
        opts.push(Opt::uri_query(q)).map_err(|_| "opt full")?;
    }
    let cf = req.content_format.map(ContentFormat::new);
    let cf_enc = cf.map(ContentFormat::encode);
    if let Some(ref e) = cf_enc {
        opts.push(Opt::content_format(e)).map_err(|_| "opt full")?;
    }
    let acc = req.accept.map(ContentFormat::new);
    let acc_enc = acc.map(ContentFormat::encode);
    if let Some(ref e) = acc_enc {
        opts.push(Opt::accept(e)).map_err(|_| "opt full")?;
    }
    for t in &req.etag {
        opts.push(Opt::etag(t)).map_err(|_| "opt full")?;
    }
    for t in &req.if_match {
        opts.push(Opt::if_match(t)).map_err(|_| "opt full")?;
    }
    if req.if_none_match {
        opts.push(Opt::if_none_match()).map_err(|_| "opt full")?;
    }
    let obs = req.observe.map(encode_observe);
    if let Some(ref e) = obs {
        opts.push(Opt::observe(e)).map_err(|_| "opt full")?;
    }
    for b in extra_block {
        match b {
            EncodedOpt::Block2(e) => {
                opts.push(Opt::block2(e)).map_err(|_| "opt full")?;
            }
            EncodedOpt::Block1(e) => {
                opts.push(Opt::block1(e)).map_err(|_| "opt full")?;
            }
        }
    }
    let msg = Message::new(req.ty, req.code, mid)
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(&req.payload);
    let mut buf = [0u8; 1472];
    let n = encode(&msg, &mut buf).map_err(|e| format!("encode: {e:?}"))?;
    Ok(buf[..n].to_vec())
}

fn encode_client_req(req: &ClientRequest) -> Result<(Vec<u8>, Token, MessageId), PeerError> {
    let token = mint_token(req.token_len);
    let mid = MessageId::new(0x1001);
    let extra = if let Some((num, more, size)) = req.block2 {
        vec![block2_opt(num, more, size)]
    } else {
        Vec::new()
    };
    let wire = encode_path_req(mid, token, req, &extra)?;
    Ok((wire, token, mid))
}

fn mint_token(len: Option<usize>) -> Token {
    match len {
        Some(0) => Token::EMPTY,
        Some(n) => Token::mint(n, &[0xC0, 0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07])
            .unwrap_or(Token::from_checked(&[0xC0, 0xA1])),
        None => Token::from_checked(&[0xC0, 0xA1]),
    }
}

struct WireView {
    ty: Type,
    code: Code,
    mid: u16,
    payload: Vec<u8>,
    content_format: Option<u16>,
    observe: Option<u32>,
    etag: Vec<Vec<u8>>,
    location_path: Vec<String>,
    location_query: Vec<String>,
    block2: Option<(u32, bool)>,
}

fn recv_matching<T: DatagramIo<Error = std::io::Error>>(
    io: &mut T,
    token: Token,
    deadline: Instant,
) -> Result<WireView, PeerError> {
    while Instant::now() < deadline {
        let mut buf = [0u8; 1472];
        match io.recv(&mut buf) {
            Ok(Some((n, _))) => {
                let parsed = decode(&buf[..n]).map_err(|e| format!("{e:?}"))?;
                if parsed.is_empty_ack() {
                    continue;
                }
                if parsed.token() != token {
                    continue;
                }
                return Ok(wire_view(&parsed));
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(PeerError(e.to_string())),
        }
    }
    Err(PeerError("recv timeout".into()))
}

fn wire_view(parsed: &coaptic::message::ParsedMessage<'_>) -> WireView {
    WireView {
        ty: parsed.ty(),
        code: parsed.code(),
        mid: parsed.message_id().get(),
        payload: parsed.payload().to_vec(),
        content_format: parsed
            .content_format()
            .and_then(Result::ok)
            .map(|c| c.get()),
        observe: parsed.observe().and_then(Result::ok),
        etag: parsed.etag().map(|t| t.to_vec()).collect(),
        location_path: parsed
            .location_path()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect(),
        location_query: parsed
            .location_query()
            .filter_map(|s| s.ok().map(str::to_owned))
            .collect(),
        block2: parsed
            .block2()
            .and_then(Result::ok)
            .map(|b| (b.num(), b.more())),
    }
}

fn view_response(parsed: &coaptic::message::ParsedMessage<'_>) -> ClientResponse {
    let w = wire_view(parsed);
    ClientResponse {
        ty: w.ty,
        code: w.code,
        payload: w.payload,
        body: None,
        content_format: w.content_format,
        observe: w.observe,
        etag: w.etag,
        location_path: w.location_path,
        location_query: w.location_query,
        rst: parsed.is_empty_rst(),
    }
}

fn send_raw<T: DatagramIo<Error = std::io::Error>>(
    io: &mut T,
    dest: Endpoint,
    msg: &Message<'_>,
) -> Result<(), PeerError> {
    let mut buf = [0u8; 1472];
    let n = encode(msg, &mut buf).map_err(|e| format!("{e:?}"))?;
    io.send(dest, &buf[..n]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build a boxed Engine (used by DTLS client path and tests).
#[must_use]
pub fn build_engine()
-> Box<Engine<Memory<profiles::Default, coaptic::storage::WithBodies<profiles::Default>>>> {
    Box::new(
        EngineBuilder::new()
            .profile::<profiles::Default>()
            .block_wise(true)
            .build(Memory::<profiles::Default>::with_block_wise())
            .expect("engine"),
    )
}
