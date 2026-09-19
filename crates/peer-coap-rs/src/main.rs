//! Isolated coap-rs peer, owning its old DTLS stack.
#![forbid(unsafe_code)]
#[path = "../../../tools/interop/support.rs"]
mod support;
use coap::{Server, client::CoAPClient, request::RequestBuilder};
use coap_lite::{MessageClass, RequestType, ResponseType};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use support::{Args, Error};
use webrtc_dtls::{cipher_suite::CipherSuiteId, config::Config};
use webrtc_util::conn::Listener;
fn config(key: &str) -> Config {
    let key = key.as_bytes().to_vec();
    Config {
        psk: Some(Arc::new(move |_| Ok(key.clone()))),
        psk_identity_hint: Some(b"password".to_vec()),
        cipher_suites: vec![CipherSuiteId::Tls_Psk_With_Aes_128_Ccm_8],
        ..Default::default()
    }
}
async fn run() -> Result<(), Error> {
    let a = Args::parse()?;
    let start = Instant::now();
    if a.server {
        let server = if a.dtls {
            let listener = webrtc_dtls::listener::listen(a.address(), config(&a.key)).await?;
            let _ = listener.addr().await?;
            Server::from_listeners(vec![Box::new(TimedListener(listener))])
        } else {
            Server::new_udp(a.address())?
        };
        let counter = Arc::new(AtomicU32::new(0));
        support::ready("coap-rs", "coap 0.28.1 / webrtc-dtls 0.8.0", a.port, a.dtls);
        server
            .run(
                move |mut req: Box<coap_lite::CoapRequest<std::net::SocketAddr>>| {
                    let counter = Arc::clone(&counter);
                    async move {
                        let path = req.get_path();
                        let method = *req.get_method();
                        if let Some(r) = req.response.as_mut() {
                            let (code, body) = match (method, path.as_str()) {
                                (RequestType::Get, "test") => {
                                    (ResponseType::Content, support::BODY.to_vec())
                                }
                                (RequestType::Get, "large") => {
                                    (ResponseType::Content, support::LARGE.to_vec())
                                }
                                (RequestType::Get, "counter") => (
                                    ResponseType::Content,
                                    counter.load(Ordering::SeqCst).to_string().into_bytes(),
                                ),
                                (RequestType::Post, "counter") => {
                                    counter.fetch_add(1, Ordering::SeqCst);
                                    (ResponseType::Changed, vec![])
                                }
                                _ => (ResponseType::NotFound, vec![]),
                            };
                            r.message.header.code = MessageClass::Response(code);
                            r.message.payload = body;
                        }
                        req
                    }
                },
            )
            .await?;
        return Ok(());
    }
    let operation = async {
        let request = RequestBuilder::request_path(
            &format!("/{}", a.path),
            if a.post {
                RequestType::Post
            } else {
                RequestType::Get
            },
            None,
            vec![],
            None,
        )
        .build();
        let response = if a.dtls {
            CoAPClient::from_udp_dtls_config(coap::dtls::UdpDtlsConfig {
                config: config(&a.key),
                dest_addr: a.address(),
            })
            .await?
            .send(request)
            .await?
        } else {
            coap::client::UdpCoAPClient::new(a.address())
                .await?
                .send(request)
                .await?
        };
        support::response(
            response.message.header.code.into(),
            &response.message.payload,
            start,
        );
        Ok::<(), Error>(())
    };
    tokio::time::timeout(Duration::from_millis(a.timeout), operation).await??;
    Ok(())
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    support::finish(run().await)
}

// Bound the old peer's serialized handshake accept so a bad PSK cannot stall
// subsequent clients indefinitely. This wrapper owns only old util 0.8 types.
struct TimedListener<T>(T);
#[async_trait::async_trait]
impl<T: Listener + Send + Sync> Listener for TimedListener<T> {
    async fn accept(
        &self,
    ) -> webrtc_util::Result<(
        Arc<dyn webrtc_util::conn::Conn + Send + Sync>,
        std::net::SocketAddr,
    )> {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), self.0.accept()).await {
                Ok(Ok(connection)) => return Ok(connection),
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    }
    async fn close(&self) -> webrtc_util::Result<()> {
        self.0.close().await
    }
    async fn addr(&self) -> webrtc_util::Result<std::net::SocketAddr> {
        self.0.addr().await
    }
}
