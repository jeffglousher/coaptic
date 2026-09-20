//! Process-isolated Coaptic peer. No coap-rs types or dependencies.
#![forbid(unsafe_code)]
mod dtls;
#[path = "../../../tools/interop/support.rs"]
mod support;
use coaptic::storage::{DatagramIo, Endpoint};
use coaptic::{App, Code, ContentFormat, Method, Request, Response, get, profiles};
use std::{
    net::UdpSocket,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};
use support::{Args, Error};
use webrtc_util::conn::Listener;
static METHOD_RESOURCE: std::sync::Mutex<support::MethodResource> =
    std::sync::Mutex::new(support::MethodResource::new());
static COUNTER: AtomicU32 = AtomicU32::new(0);
fn count(_: Request<'_>) -> Response<'static> {
    Response::content_copy(COUNTER.load(Ordering::SeqCst).to_string().as_bytes())
}
fn increment(_: Request<'_>) -> Response<'static> {
    COUNTER.fetch_add(1, Ordering::SeqCst);
    Response::changed()
}
fn method_resource(request: Request<'_>) -> Response<'static> {
    let method = request.method().expect("routed method").code().as_raw();
    let (code, bytes) = METHOD_RESOURCE.lock().expect("fixture lock").respond(
        method,
        request.payload(),
        request.content_format() == Some(Ok(ContentFormat::OCTET_STREAM)),
    );
    Response::new(Code::from_raw(code)).payload_copy(&bytes)
}
enum Io {
    Udp(UdpSocket),
    Dtls(dtls::DtlsIo),
}
impl DatagramIo for Io {
    type Error = std::io::Error;
    fn recv(&mut self, b: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        match self {
            Self::Udp(s) => DatagramIo::recv(s, b),
            Self::Dtls(s) => s.recv(b),
        }
    }
    fn send(&mut self, d: Endpoint, b: &[u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Udp(s) => DatagramIo::send(s, d, b),
            Self::Dtls(s) => s.send(d, b),
        }
    }
}
async fn run() -> Result<(), Error> {
    let a = Args::parse()?;
    let start = Instant::now();
    if a.server && a.dtls {
        let listener =
            dtls::listener::BoundedListener::bind(a.address(), dtls::psk_config(a.key.as_bytes()))
                .await?;
        support::ready("coaptic", "webrtc-dtls 0.12.0", a.port, "dtls");
        loop {
            let (conn, peer) = listener.accept().await?;
            tokio::spawn(async move {
                // CoAP exchange/dedup/Observe state belongs to this authenticated
                // association. Resource contents (COUNTER) remain shared.
                let Ok(mut app) = fixture(Io::Dtls(dtls::DtlsIo::accepted(conn, peer))) else {
                    return;
                };
                let start = Instant::now();
                loop {
                    if app.poll(start.elapsed().as_millis() as u64 + 1).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            });
        }
    }
    let io = if a.dtls {
        Io::Dtls(
            dtls::DtlsIo::connect(
                a.address(),
                dtls::psk_config(a.key.as_bytes()),
                Duration::from_millis(a.timeout),
            )
            .await?,
        )
    } else {
        let socket = UdpSocket::bind(if a.server {
            a.address()
        } else {
            let mut address = a.address();
            address.set_port(0);
            address
        })?;
        socket.set_nonblocking(true)?;
        Io::Udp(socket)
    };
    let mut app = fixture(io)?;
    if a.oscore {
        // Public RFC 8613 C.1 fixture material, never production credentials.
        let mut secret = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        if a.key != "sesame" {
            secret[0] ^= 0xff;
        }
        let mut context = coaptic::oscore::SecurityContext::derive(coaptic::oscore::DeriveParams {
            master_secret: &secret,
            master_salt: &[0x9e, 0x7c, 0xa9, 0x22, 0x23, 0x78, 0x63, 0x40],
            sender_id: if a.server { &[1] } else { &[] },
            recipient_id: if a.server { &[] } else { &[1] },
            id_context: &[],
        })
        .map_err(|e| format!("OSCORE derivation: {e:?}"))?;
        context
            .set_sender_seq(a.sequence)
            .map_err(|e| format!("OSCORE sequence: {e:?}"))?;
        app.set_oscore(context);
    }
    if a.server {
        support::ready(
            "coaptic",
            if a.oscore {
                "coaptic OSCORE"
            } else {
                "coaptic UDP"
            },
            a.port,
            if a.oscore { "oscore" } else { "udp" },
        );
        loop {
            app.poll(start.elapsed().as_millis() as u64 + 1)
                .map_err(|e| format!("poll: {e}"))?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let path = match a.path.as_str() {
        "test" => &["test"][..],
        "large" => &["large"][..],
        "counter" => &["counter"][..],
        "methods" => &["methods"][..],
        _ => &["missing"][..],
    };
    let method = match a.method {
        1 => Method::Get,
        2 => Method::Post,
        3 => Method::Put,
        4 => Method::Delete,
        5 => Method::Fetch,
        6 => Method::Patch,
        _ => Method::IPatch,
    };
    let mut outgoing = app
        .request(method, path)
        .payload(&a.payload)
        .to(Endpoint::from(a.address()));
    if a.path == "methods" && matches!(a.method, 2 | 3 | 5 | 6 | 7) {
        outgoing = outgoing.content_format(ContentFormat::OCTET_STREAM);
    }
    let call = outgoing.send(1).map_err(|e| format!("send: {e}"))?;
    while start.elapsed() < Duration::from_millis(a.timeout) {
        app.poll(start.elapsed().as_millis() as u64 + 1)
            .map_err(|e| format!("poll: {e}"))?;
        let response = app.take_response(call).transpose()?.map(|r| {
            (
                r.code().as_raw(),
                r.body().unwrap_or(r.payload()).to_vec(),
                start.elapsed(),
            )
        });
        if let Some((code, body, elapsed)) = response {
            // Flush CloseNotify before the runtime exits. Response timing ends
            // at assembly; host timing includes this bounded session shutdown.
            if let Io::Dtls(io) = app.transport_mut() {
                io.close().await?;
            }
            support::response(code, &body, elapsed);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("request timed out".into())
}
fn fixture(io: Io) -> Result<App<profiles::Default, Io, 4, true>, Error> {
    App::profile::<profiles::Default>()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .block_wise::<true>()
        .routes::<4>()
        .route(
            "/test",
            get(|_: Request<'_>| Response::content(support::BODY)),
        )
        .route(
            "/large",
            get(|_: Request<'_>| Response::content(&support::LARGE)),
        )
        .route("/counter", get(count).post(increment))
        .route(
            "/methods",
            get(method_resource)
                .post(method_resource)
                .put(method_resource)
                .delete(method_resource)
                .fetch(method_resource)
                .patch(method_resource)
                .ipatch(method_resource),
        )
        .bind(io)
        .map_err(|e| format!("bind: {e:?}").into())
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    support::finish(run().await)
}
