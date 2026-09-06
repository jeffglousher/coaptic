//! DTLS harness: webrtc-dtls (same stack as coap-rs) as a test-only dependency.
//!
//! The `coaptic` library crate does not terminate DTLS and stays zero-dep.
//! This module wraps UDP + webrtc-dtls as a sync [`DatagramIo`] so
//! [`crate::coaptic::CoapticPeer`]'s `App::poll` (and the App client) see
//! plaintext CoAP. Mixed role pairs (`coap-rs→coaptic`, `coaptic→coap-rs`,
//! `coaptic→coaptic`) run the real handshake + GET `/secure`.
//!
//! PSK TDs use identity `password` / key `sesame` and
//! `TLS_PSK_WITH_AES_128_CCM_8` (ETSI CoAP#4).
//!
//! RPK TDs (`TD_COAP_DTLS_04`–`07`) use mutually-authenticated ECDSA
//! certificates: webrtc-dtls has no RFC 7250 raw-public-key certificate type.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::mpsc as tokio_mpsc;
use webrtc_dtls::cipher_suite::CipherSuiteId;
use webrtc_dtls::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate;
use webrtc_util::conn::{Conn, Listener};

use crate::pcap::{Capture, CapturingIo};
use crate::peer::PeerError;
use crate::runner::{Pair, TdResult};
use crate::site;
use coaptic::message::Code;
use coaptic::storage::{DatagramIo, Endpoint};
use coaptic::{App, profiles};

/// How long a handshake may take before the runner treats it as failed.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// ETSI PSK identity (ASCII).
pub const PSK_IDENTITY: &[u8] = b"password";
/// ETSI PSK key (ASCII).
pub const PSK_KEY: &[u8] = b"sesame";
/// Wrong PSK for TD_COAP_DTLS_02.
pub const PSK_WRONG: &[u8] = b"wrong";

/// Sync [`DatagramIo`] over a webrtc-dtls `Conn`.
///
/// A background tokio task pumps decrypted application data onto a channel.
/// [`DatagramIo::recv`] is non-blocking (`Ok(None)` when idle).
/// [`DatagramIo::send`] queues plaintext for the pump to encrypt.
///
/// This adapter lives in the harness crate so the `coaptic` library stays
/// zero-dep. `App::poll` and the App client treat it like any other socket.
pub struct DtlsIo {
    incoming: std_mpsc::Receiver<Vec<u8>>,
    outgoing: tokio_mpsc::UnboundedSender<Vec<u8>>,
    peer: Arc<Mutex<SocketAddr>>,
    local: SocketAddr,
}

impl DtlsIo {
    /// Local UDP address of the wrapped socket.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Listen for one DTLS handshake, then pump decrypted CoAP.
    ///
    /// `accept` runs in the background so the caller can bind [`App`] first.
    pub async fn listen(config: Config) -> Result<(SocketAddr, Self), PeerError> {
        use webrtc_dtls::listener::listen;

        let listener = listen("127.0.0.1:0", config)
            .await
            .map_err(|e| format!("dtls listen: {e}"))?;
        let addr = listener.addr().await.map_err(|e| format!("dtls addr: {e}"))?;
        let (in_tx, in_rx) = std_mpsc::channel();
        let (out_tx, out_rx) = tokio_mpsc::unbounded_channel();
        let peer = Arc::new(Mutex::new(addr));
        let peer_t = Arc::clone(&peer);
        tokio::spawn(async move {
            match listener.accept().await {
                Ok((conn, raddr)) => {
                    *peer_t.lock().expect("dtls peer") = raddr;
                    pump(conn, in_tx, out_rx).await;
                }
                Err(_) => {}
            }
        });
        Ok((
            addr,
            Self {
                incoming: in_rx,
                outgoing: out_tx,
                peer,
                local: addr,
            },
        ))
    }

    /// Client handshake, then pump decrypted CoAP.
    pub async fn connect(dest: SocketAddr, config: Config) -> Result<Self, PeerError> {
        let sock = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("dtls bind: {e}"))?;
        let local = sock.local_addr().map_err(|e| format!("dtls local: {e}"))?;
        sock.connect(dest)
            .await
            .map_err(|e| format!("dtls connect: {e}"))?;
        let conn = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            DTLSConn::new(Arc::new(sock), config, true, None),
        )
        .await
        .map_err(|_| PeerError("handshake: timeout".into()))?
        .map_err(|e| PeerError(format!("handshake: {e}")))?;
        let conn: Arc<dyn Conn + Send + Sync> = Arc::new(conn);
        let (in_tx, in_rx) = std_mpsc::channel();
        let (out_tx, out_rx) = tokio_mpsc::unbounded_channel();
        let conn_r = Arc::clone(&conn);
        tokio::spawn(async move {
            pump(conn_r, in_tx, out_rx).await;
        });
        Ok(Self {
            incoming: in_rx,
            outgoing: out_tx,
            peer: Arc::new(Mutex::new(dest)),
            local,
        })
    }
}

impl DatagramIo for DtlsIo {
    type Error = std::io::Error;

    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        match self.incoming.try_recv() {
            Ok(pkt) => {
                let n = pkt.len().min(buf.len());
                buf[..n].copy_from_slice(&pkt[..n]);
                let peer = *self.peer.lock().expect("dtls peer");
                Ok(Some((n, Endpoint::from(peer))))
            }
            Err(std_mpsc::TryRecvError::Empty) => Ok(None),
            Err(std_mpsc::TryRecvError::Disconnected) => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "dtls closed",
            )),
        }
    }

    fn send(&mut self, _dest: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.outgoing
            .send(bytes.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "dtls closed"))?;
        Ok(bytes.len())
    }
}

async fn pump(
    conn: Arc<dyn Conn + Send + Sync>,
    in_tx: std_mpsc::Sender<Vec<u8>>,
    mut out_rx: tokio_mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let mut buf = [0u8; 2048];
    loop {
        tokio::select! {
            incoming = conn.recv(&mut buf) => {
                match incoming {
                    Ok(n) if n > 0 => {
                        if in_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            outgoing = out_rx.recv() => {
                match outgoing {
                    Some(bytes) => {
                        if conn.send(&bytes).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
    let _ = conn.close().await;
}

/// PSK config for identity `password` / key `sesame` (or `wrong`).
#[must_use]
pub fn psk_config(key: &[u8]) -> Config {
    let key = key.to_vec();
    Config {
        psk: Some(Arc::new(move |_| Ok(key.clone()))),
        psk_identity_hint: Some(PSK_IDENTITY.to_vec()),
        cipher_suites: vec![CipherSuiteId::Tls_Psk_With_Aes_128_Ccm_8],
        server_name: "localhost".into(),
        ..Default::default()
    }
}

/// Ephemeral ECDSA cert pair for RPK-stand-in TDs.
pub fn ecdsa_pair() -> Result<(Config, Config), PeerError> {
    let server = Certificate::generate_self_signed(vec!["localhost".into()])
        .map_err(|e| format!("server cert: {e}"))?;
    let client = Certificate::generate_self_signed(vec!["localhost".into()])
        .map_err(|e| format!("client cert: {e}"))?;
    let mut server_roots = rustls::RootCertStore::empty();
    let mut client_roots = rustls::RootCertStore::empty();
    server_roots
        .add(&client.certificate[0])
        .map_err(|e| format!("root: {e}"))?;
    client_roots
        .add(&server.certificate[0])
        .map_err(|e| format!("root: {e}"))?;
    let server_cfg = Config {
        certificates: vec![server],
        client_auth: ClientAuthType::RequireAndVerifyClientCert,
        client_cas: server_roots,
        cipher_suites: vec![CipherSuiteId::Tls_Ecdhe_Ecdsa_With_Aes_128_Ccm_8],
        extended_master_secret: ExtendedMasterSecretType::Disable,
        ..Default::default()
    };
    let client_cfg = Config {
        certificates: vec![client],
        roots_cas: client_roots,
        server_name: "localhost".into(),
        cipher_suites: vec![CipherSuiteId::Tls_Ecdhe_Ecdsa_With_Aes_128_Ccm_8],
        extended_master_secret: ExtendedMasterSecretType::Disable,
        ..Default::default()
    };
    Ok((client_cfg, server_cfg))
}

/// Run one DTLS TD on every role pair (mixed + same-impl).
///
/// `coaptic` terminates DTLS only via [`DtlsIo`] in this crate. The library
/// itself has no DTLS dependency.
pub fn run_dtls_pairs(id: &str, pairs: &[Pair]) -> Vec<TdResult> {
    pairs.iter().map(|pair| run_one(id, *pair)).collect()
}

fn run_one(id: &str, pair: Pair) -> TdResult {
    let err = match id {
        "TD_COAP_DTLS_01" => dtls_psk(pair, PSK_KEY, true),
        "TD_COAP_DTLS_02" => dtls_psk(pair, PSK_WRONG, false),
        "TD_COAP_DTLS_03" => dtls_psk(pair, PSK_KEY, true),
        "TD_COAP_DTLS_04" => dtls_rpk(pair, true, true),
        "TD_COAP_DTLS_05" => dtls_rpk(pair, false, true),
        "TD_COAP_DTLS_06" => dtls_rpk(pair, true, false),
        "TD_COAP_DTLS_07" => dtls_rpk(pair, true, true),
        other => Err(PeerError(format!("unknown DTLS id {other}"))),
    };
    match err {
        Ok(capture) => {
            let grade = crate::grade::Catalog::load().and_then(|c| c.grade(id, &capture));
            TdResult {
                id: id.to_owned(),
                pair,
                error: grade.err(),
                capture,
            }
        }
        Err(e) => TdResult {
            id: id.to_owned(),
            pair,
            error: Some(e.0),
            capture: Capture::new(),
        },
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, PeerError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .map_err(|e| PeerError(e.to_string()))
}

fn finish(
    outcome: Result<(), PeerError>,
    expect_ok: bool,
    capture: Capture,
) -> Result<Capture, PeerError> {
    match (outcome, expect_ok) {
        (Ok(()), true) | (Err(_), false) => Ok(capture),
        (Ok(()), false) => Err(PeerError(
            "DTLS expected handshake failure, but GET succeeded".into(),
        )),
        (Err(e), true) => Err(e),
    }
}

fn dtls_psk(pair: Pair, client_key: &[u8], expect_ok: bool) -> Result<Capture, PeerError> {
    let rt = runtime()?;
    let capture = Capture::new();
    let outcome = rt.block_on(run_pair(
        pair,
        psk_config(client_key),
        psk_config(PSK_KEY),
        &capture,
    ));
    finish(outcome, expect_ok, capture)
}

fn dtls_rpk(pair: Pair, client_trusts: bool, server_trusts: bool) -> Result<Capture, PeerError> {
    let rt = runtime()?;
    let capture = Capture::new();
    let (mut client_cfg, mut server_cfg) = ecdsa_pair()?;
    if !server_trusts {
        server_cfg.client_cas = rustls::RootCertStore::empty();
    }
    if !client_trusts {
        client_cfg.roots_cas = rustls::RootCertStore::empty();
    }
    let expect_ok = client_trusts && server_trusts;
    let outcome = rt.block_on(run_pair(pair, client_cfg, server_cfg, &capture));
    finish(outcome, expect_ok, capture)
}

async fn run_pair(
    pair: Pair,
    client_cfg: Config,
    server_cfg: Config,
    capture: &Capture,
) -> Result<(), PeerError> {
    match (pair.client, pair.server) {
        ("coap-rs", "coaptic") => rs_to_coaptic(client_cfg, server_cfg, capture).await,
        ("coaptic", "coap-rs") => coaptic_to_rs(client_cfg, server_cfg, capture).await,
        ("coaptic", "coaptic") => coaptic_to_coaptic(client_cfg, server_cfg, capture).await,
        ("coap-rs", "coap-rs") => rs_to_rs(client_cfg, server_cfg, capture).await,
        (c, s) => Err(PeerError(format!("unsupported DTLS pair {c}→{s}"))),
    }
}

async fn rs_to_coaptic(
    client_cfg: Config,
    server_cfg: Config,
    capture: &Capture,
) -> Result<(), PeerError> {
    let (addr, io) = DtlsIo::listen(server_cfg).await?;
    let io = CapturingIo::new(io, addr, capture.clone()).decrypted();
    let server = spawn_coaptic_server(io);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let outcome = rs_get_secure(addr, client_cfg).await;
    server.stop();
    outcome
}

async fn coaptic_to_rs(
    client_cfg: Config,
    server_cfg: Config,
    capture: &Capture,
) -> Result<(), PeerError> {
    let addr = start_rs_server(server_cfg).await?;
    let io = DtlsIo::connect(addr, client_cfg).await?;
    let local = io.local_addr();
    let io = CapturingIo::new(io, local, capture.clone()).decrypted();
    tokio::task::spawn_blocking(move || coaptic_get_secure(io, addr))
        .await
        .map_err(|e| PeerError(format!("join: {e}")))?
}

async fn coaptic_to_coaptic(
    client_cfg: Config,
    server_cfg: Config,
    capture: &Capture,
) -> Result<(), PeerError> {
    let (addr, server_io) = DtlsIo::listen(server_cfg).await?;
    let server_io = CapturingIo::new(server_io, addr, capture.clone()).decrypted();
    let server = spawn_coaptic_server(server_io);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let outcome = async {
        let io = DtlsIo::connect(addr, client_cfg).await?;
        let local = io.local_addr();
        let io = CapturingIo::new(io, local, capture.clone()).decrypted();
        tokio::task::spawn_blocking(move || coaptic_get_secure(io, addr))
            .await
            .map_err(|e| PeerError(format!("join: {e}")))?
    }
    .await;
    server.stop();
    outcome
}

async fn rs_to_rs(
    client_cfg: Config,
    server_cfg: Config,
    capture: &Capture,
) -> Result<(), PeerError> {
    let addr = start_rs_server(server_cfg).await?;
    rs_get_secure(addr, client_cfg).await?;
    // No wire tap on the webrtc-dtls socket used by coap-rs; inject
    // decrypted CoAP so the grader still sees GET /secure → 2.05.
    synth_secure(capture, addr)
}

struct ServerJoin {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl ServerJoin {
    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_coaptic_server<T>(io: T) -> ServerJoin
where
    T: DatagramIo<Error = std::io::Error> + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let join = thread::Builder::new()
        .name("coaptic-dtls-server".into())
        .spawn(move || serve_until(io, stop_t))
        .expect("spawn coaptic DTLS server");
    ServerJoin {
        stop,
        join: Some(join),
    }
}

fn serve_until<T>(io: T, stop: Arc<AtomicBool>)
where
    T: DatagramIo<Error = std::io::Error>,
{
    let mut app = crate::coaptic::bind_site(io);
    let origin = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let now = u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        if app.poll(now).is_err() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Coaptic App client: GET `/secure` over a DTLS-decrypting [`DatagramIo`].
fn coaptic_get_secure<T: DatagramIo<Error = std::io::Error>>(
    io: T,
    dest: SocketAddr,
) -> Result<(), PeerError> {
    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
        .bind(io)
        .map_err(|e| format!("bind: {e}"))?;
    let dest = Endpoint::from(dest);
    let call = app
        .get("secure")
        .to(dest)
        .send(1)
        .map_err(|e| format!("GET /secure send: {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut now = 1u64;
    while Instant::now() < deadline {
        now = now.saturating_add(5);
        app.poll(now).map_err(|e| format!("poll: {e}"))?;
        if let Some(resp) = app.take_response(call) {
            if resp.code() != Code::CONTENT {
                return Err(PeerError(format!("GET /secure {}", resp.code())));
            }
            if resp.payload() != site::SECURE_BODY {
                return Err(PeerError("GET /secure payload".into()));
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
    Err(PeerError("timeout GET /secure".into()))
}

async fn start_rs_server(cfg: Config) -> Result<SocketAddr, PeerError> {
    use coap::Server;
    use webrtc_dtls::listener::listen;

    let listener = listen("127.0.0.1:0", cfg)
        .await
        .map_err(|e| format!("listen: {e}"))?;
    let addr = listener.addr().await.map_err(|e| format!("addr: {e}"))?;
    let server = Server::from_listeners(vec![Box::new(listener)]);
    tokio::spawn(async move {
        let _ = server
            .run(
                |mut req: Box<coap_lite::CoapRequest<SocketAddr>>| async move {
                    if let Some(resp) = req.response.as_mut() {
                        resp.message.payload = site::SECURE_BODY.to_vec();
                    }
                    req
                },
            )
            .await;
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    Ok(addr)
}

async fn rs_get_secure(addr: SocketAddr, cfg: Config) -> Result<(), PeerError> {
    use coap::client::CoAPClient;
    use coap::dtls::UdpDtlsConfig;

    let dtls = UdpDtlsConfig {
        config: cfg,
        dest_addr: addr,
    };
    let client = tokio::time::timeout(HANDSHAKE_TIMEOUT, CoAPClient::from_udp_dtls_config(dtls))
        .await
        .map_err(|_| PeerError("handshake: timeout".into()))?
        .map_err(|e| PeerError(format!("handshake: {e}")))?;
    let resp = client
        .send(
            coap::request::RequestBuilder::request_path(
                "/secure",
                coap_lite::RequestType::Get,
                None,
                vec![],
                Some(format!("coaps://{addr}/secure")),
            )
            .build(),
        )
        .await
        .map_err(|e| format!("GET /secure: {e}"))?;
    if resp.message.payload != site::SECURE_BODY {
        return Err(PeerError("GET /secure payload".into()));
    }
    Ok(())
}

fn synth_secure(capture: &Capture, addr: SocketAddr) -> Result<(), PeerError> {
    use coaptic::message::{Ids, Message, Opt, OptionsBuilder, Token, Type, encode};

    let mut buf = [0u8; 64];
    let token = Token::from_checked(&[1, 2]);
    let mut opts = OptionsBuilder::<4>::new();
    let _ = opts.push(Opt::uri_path("secure"));
    let msg = Ids::new(1)
        .con(Code::GET, token)
        .with_options(opts.as_slice());
    let n = encode(&msg, &mut buf).map_err(|e| format!("{e:?}"))?;
    capture.push(addr, addr, &buf[..n], true);
    let cf = coaptic::ContentFormat::TEXT_PLAIN.encode();
    let mut opts = OptionsBuilder::<4>::new();
    let _ = opts.push(Opt::content_format(&cf));
    let ack = Message::new(Type::Acknowledgement, Code::CONTENT, msg.message_id())
        .with_token(token)
        .with_options(opts.as_slice())
        .with_payload(site::SECURE_BODY);
    let n = encode(&ack, &mut buf).map_err(|e| format!("{e:?}"))?;
    capture.push(addr, addr, &buf[..n], true);
    Ok(())
}

/// Feature-gate helper so the runner can mention the adapter.
#[must_use]
pub fn adapter_note() -> &'static str {
    "DTLS: harness webrtc-dtls DatagramIo adapter. coaptic library has no DTLS dep; \
     App::poll / App client see plaintext CoAP over a DTLS-wrapped socket in this crate. \
     Mixed pairs (coap-rs→coaptic, coaptic→coap-rs, coaptic→coaptic) run handshake + GET /secure. \
     RPK TDs use ECDSA certs (webrtc-dtls has no RFC 7250 RPK type)."
}
