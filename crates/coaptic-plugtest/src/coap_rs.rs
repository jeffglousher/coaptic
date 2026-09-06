//! [`CoapRsPeer`]: first external backend (`coap` / coap-rs on crates.io).

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coap::Server;
use coap::client::UdpCoAPClient;
use coap::request::RequestBuilder;
use coap_lite::{CoapOption, CoapRequest, ContentFormat as LiteCf, RequestType, ResponseType};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use crate::pcap::Capture;
use crate::peer::{ClientRequest, ClientResponse, Peer, PeerError};
use crate::site;
use coaptic::message::{Code, Type};

/// coap-rs backend (tokio server + [`UdpCoAPClient`]).
pub struct CoapRsPeer {
    rt: Runtime,
    capture: Capture,
    stop: Option<oneshot::Sender<()>>,
    addr: Option<SocketAddr>,
    notify: Arc<Mutex<Option<(Vec<String>, Vec<u8>)>>>,
}

impl Default for CoapRsPeer {
    fn default() -> Self {
        Self::new()
    }
}

impl CoapRsPeer {
    /// New peer on a current-thread-friendly multi-thread runtime.
    #[must_use]
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("coap-rs-plugtest")
            .build()
            .expect("tokio runtime");
        Self {
            rt,
            capture: Capture::new(),
            stop: None,
            addr: None,
            notify: Arc::new(Mutex::new(None)),
        }
    }
}

impl Peer for CoapRsPeer {
    fn name(&self) -> &'static str {
        "coap-rs"
    }

    fn start_server(&mut self) -> Result<SocketAddr, PeerError> {
        self.stop_server();
        site::reset();
        let probe = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let addr = probe.local_addr().map_err(|e| e.to_string())?;
        drop(probe);
        let (tx, rx) = oneshot::channel();
        let notify = Arc::clone(&self.notify);
        self.rt.spawn(async move {
            let server = match Server::new_udp(addr) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("coap-rs Server::new_udp: {e}");
                    return;
                }
            };
            let run = server.run(move |req| {
                let n = Arc::clone(&notify);
                async move { handle_request(req, &n) }
            });
            tokio::select! {
                _ = run => {}
                _ = rx => {}
            }
        });
        // Bind retry window.
        std::thread::sleep(Duration::from_millis(40));
        self.stop = Some(tx);
        self.addr = Some(addr);
        Ok(addr)
    }

    fn stop_server(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        self.addr = None;
    }

    fn send_request(
        &mut self,
        dest: SocketAddr,
        req: &ClientRequest,
    ) -> Result<ClientResponse, PeerError> {
        if req.code == Code::EMPTY {
            return raw_ping(dest, req.timeout, &self.capture);
        }
        let dest_s = dest.to_string();
        let built = build_lite_request(req, &dest_s)?;
        let timeout = req.timeout;
        let resp = self
            .rt
            .block_on(async move {
                let client = UdpCoAPClient::new(&dest_s).await?;
                tokio::time::timeout(timeout, client.send(built))
                    .await
                    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "client"))?
            })
            .map_err(|e| PeerError(e.to_string()))?;
        Ok(from_lite(&resp.message))
    }

    fn poll(&mut self, _now_ms: u64) -> Result<(), PeerError> {
        Ok(())
    }

    fn local_addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    fn take_capture(&mut self) -> Capture {
        self.capture.clone()
    }

    fn notify(&mut self, path: &[&str], payload: &[u8]) -> Result<(), PeerError> {
        *self.notify.lock().expect("n") = Some((
            path.iter().map(|s| (*s).to_owned()).collect(),
            payload.to_vec(),
        ));
        Ok(())
    }

    fn supports_dtls(&self) -> bool {
        cfg!(feature = "dtls")
    }
}

impl Drop for CoapRsPeer {
    fn drop(&mut self) {
        self.stop_server();
    }
}

fn handle_request(
    mut request: Box<CoapRequest<SocketAddr>>,
    _notify: &Arc<Mutex<Option<(Vec<String>, Vec<u8>)>>>,
) -> Box<CoapRequest<SocketAddr>> {
    let path = request.get_path();
    let method = *request.get_method();
    let query = request
        .message
        .get_option(CoapOption::UriQuery)
        .map(|vals| {
            vals.iter()
                .filter_map(|v| std::str::from_utf8(v).ok().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(resp) = request.response.as_mut() {
        match (method, path.as_str()) {
            (RequestType::Get, "test") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::TEST_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Put, "test") => {
                resp.set_status(ResponseType::Changed);
            }
            (RequestType::Post, "test") => {
                resp.set_status(ResponseType::Created);
                resp.message
                    .add_option(CoapOption::LocationPath, b"location1".to_vec());
                resp.message
                    .add_option(CoapOption::LocationPath, b"location2".to_vec());
                resp.message
                    .add_option(CoapOption::LocationQuery, b"first=1".to_vec());
                resp.message
                    .add_option(CoapOption::LocationQuery, b"second=2".to_vec());
            }
            (RequestType::Delete, "test") => {
                resp.set_status(ResponseType::Deleted);
            }
            (RequestType::Get, "separate") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::SEP_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Get, "query") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::TEST_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Get, "seg1/seg2/seg3") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::TEST_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Get, "validate") => {
                let etags = request
                    .message
                    .get_option(CoapOption::ETag)
                    .map(|v| v.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                if etags.iter().any(|t| t.as_slice() == b"etag1") {
                    resp.set_status(ResponseType::Valid);
                    resp.message.add_option(CoapOption::ETag, b"etag1".to_vec());
                } else {
                    resp.set_status(ResponseType::Content);
                    resp.message.payload = site::TEST_BODY.to_vec();
                    resp.message.set_content_format(LiteCf::TextPlain);
                    resp.message.add_option(CoapOption::ETag, b"etag1".to_vec());
                }
            }
            (RequestType::Put, "validate") => {
                let if_match = request.message.get_option(CoapOption::IfMatch);
                let if_none = request.message.get_option(CoapOption::IfNoneMatch);
                if if_none.is_some() {
                    resp.set_status(ResponseType::PreconditionFailed);
                } else if let Some(tags) = if_match {
                    if tags.iter().any(|t| t.as_slice() == b"etag1") {
                        resp.set_status(ResponseType::Changed);
                    } else {
                        resp.set_status(ResponseType::PreconditionFailed);
                    }
                } else {
                    resp.set_status(ResponseType::Changed);
                }
            }
            (RequestType::Get, "large") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::large_body();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Put, "large-update") => {
                resp.set_status(ResponseType::Changed);
            }
            (RequestType::Post, "large-create") => {
                resp.set_status(ResponseType::Created);
            }
            (RequestType::Post, "large-post") => {
                resp.set_status(ResponseType::Changed);
                resp.message.payload = site::large_body();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Get, "obs" | "obs-non") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::OBS_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
                resp.message.set_observe_value(0);
            }
            (RequestType::Delete, "obs") => {
                resp.set_status(ResponseType::Deleted);
            }
            (RequestType::Get, ".well-known/core") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::filter_catalog(&query).into_bytes();
                resp.message
                    .set_content_format(LiteCf::try_from(40).unwrap_or(LiteCf::TextPlain));
            }
            (RequestType::Get, "path") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::PATH_LINKS.as_bytes().to_vec();
                resp.message
                    .set_content_format(LiteCf::try_from(40).unwrap_or(LiteCf::TextPlain));
            }
            (RequestType::Get, "path/sub1") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::PATH_SUB1.to_vec();
            }
            (RequestType::Get, "secure") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = site::SECURE_BODY.to_vec();
                resp.message.set_content_format(LiteCf::TextPlain);
            }
            (RequestType::Get, "link1" | "link2" | "link3") => {
                resp.set_status(ResponseType::Content);
                resp.message.payload = b"link1".to_vec();
            }
            _ => {
                resp.set_status(ResponseType::NotFound);
            }
        }
    }
    request
}

fn build_lite_request(
    req: &ClientRequest,
    dest: &str,
) -> Result<CoapRequest<SocketAddr>, PeerError> {
    let method = match req.code {
        Code::GET => RequestType::Get,
        Code::POST => RequestType::Post,
        Code::PUT => RequestType::Put,
        Code::DELETE => RequestType::Delete,
        other => return Err(PeerError(format!("coap-rs: unsupported method {other}"))),
    };
    let path = if req.path.is_empty() {
        String::new()
    } else {
        format!("/{}", req.path.join("/"))
    };
    let payload = if req.payload.is_empty() {
        None
    } else {
        Some(req.payload.clone())
    };
    let query = req
        .query
        .iter()
        .map(|q| q.as_bytes().to_vec())
        .collect::<Vec<_>>();
    let mut options = Vec::new();
    if let Some(cf) = req.content_format {
        options.push((CoapOption::ContentFormat, cf.to_be_bytes().to_vec()));
    }
    if let Some(acc) = req.accept {
        options.push((CoapOption::Accept, acc.to_be_bytes().to_vec()));
    }
    for t in &req.etag {
        options.push((CoapOption::ETag, t.clone()));
    }
    for t in &req.if_match {
        options.push((CoapOption::IfMatch, t.clone()));
    }
    if req.if_none_match {
        options.push((CoapOption::IfNoneMatch, Vec::new()));
    }
    if let Some(obs) = req.observe {
        options.push((CoapOption::Observe, encode_u32(obs)));
    }
    if let Some((num, more, size)) = req.block2 {
        let v = coaptic::message::BlockValue::from_size(num, more, size)
            .map_err(|e| format!("block2: {e:?}"))?;
        options.push((CoapOption::Block2, v.encode().as_bytes().to_vec()));
    }
    let token = match req.token_len {
        Some(0) => Some(Vec::new()),
        Some(n) => Some((0..n).map(|i| 0xC0u8.wrapping_add(i as u8)).collect()),
        None => None,
    };
    Ok(
        RequestBuilder::request_path(&path, method, payload, query, Some(dest.to_owned()))
            .confirmable(req.ty == Type::Confirmable)
            .token(token)
            .options(options)
            .build(),
    )
}

fn encode_u32(n: u32) -> Vec<u8> {
    let b = n.to_be_bytes();
    let start = b.iter().position(|x| *x != 0).unwrap_or(3);
    b[start..].to_vec()
}

fn from_lite(msg: &coap_lite::Packet) -> ClientResponse {
    let code = Code::from_raw(u8::from(msg.header.code));
    let ty = match msg.header.get_type() {
        coap_lite::MessageType::Confirmable => Type::Confirmable,
        coap_lite::MessageType::NonConfirmable => Type::NonConfirmable,
        coap_lite::MessageType::Acknowledgement => Type::Acknowledgement,
        coap_lite::MessageType::Reset => Type::Reset,
    };
    let content_format = msg
        .get_option(CoapOption::ContentFormat)
        .and_then(|v| v.front())
        .map(|b| u16::try_from(uint_from_bytes(b)).unwrap_or(u16::MAX));
    let observe = msg
        .get_option(CoapOption::Observe)
        .and_then(|v| v.front())
        .map(|b| uint_from_bytes(b));
    let etag = msg
        .get_option(CoapOption::ETag)
        .map(|v| v.iter().cloned().collect())
        .unwrap_or_default();
    let location_path = opt_strings(msg, CoapOption::LocationPath);
    let location_query = opt_strings(msg, CoapOption::LocationQuery);
    ClientResponse {
        ty,
        code,
        payload: msg.payload.clone(),
        body: None,
        content_format,
        observe,
        etag,
        location_path,
        location_query,
        rst: ty == Type::Reset && code == Code::EMPTY,
    }
}

fn opt_strings(msg: &coap_lite::Packet, opt: CoapOption) -> Vec<String> {
    msg.get_option(opt)
        .map(|v| {
            v.iter()
                .filter_map(|b| std::str::from_utf8(b).ok().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn uint_from_bytes(b: &[u8]) -> u32 {
    let mut n = 0u32;
    for x in b {
        n = n.saturating_mul(256).saturating_add(u32::from(*x));
    }
    n
}

fn raw_ping(
    dest: SocketAddr,
    timeout: Duration,
    capture: &Capture,
) -> Result<ClientResponse, PeerError> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let local = sock.local_addr().map_err(|e| e.to_string())?;
    // Empty CON, MID 0x5049.
    let wire = [0x40, 0x00, 0x50, 0x49];
    sock.send_to(&wire, dest).map_err(|e| e.to_string())?;
    capture.push(local, dest, &wire, false);
    let mut buf = [0u8; 64];
    let (n, from) = sock.recv_from(&mut buf).map_err(|e| e.to_string())?;
    capture.push(from, local, &buf[..n], false);
    let parsed = coaptic::message::decode(&buf[..n]).map_err(|e| format!("{e:?}"))?;
    if parsed.is_empty_rst() {
        Ok(ClientResponse {
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
        })
    } else {
        Err(PeerError(format!(
            "ping expected RST, got {} {}",
            parsed.ty(),
            parsed.code()
        )))
    }
}

/// Silence unused-import noise when `ContentFormat` try_from needs a path.
#[allow(dead_code)]
static _KEEP: AtomicBool = AtomicBool::new(false);
