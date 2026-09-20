//! Isolated coap-rs peer, using a modern test-only DTLS transport.
#![forbid(unsafe_code)]
macro_rules! conn_as_any {
    () => {
        fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
            self
        }
    };
}
#[path = "../../../tools/interop/coap_dtls.rs"]
mod coap_dtls;
#[path = "../../../tools/interop/dtls_listener.rs"]
mod dtls_listener;
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
            let listener =
                dtls_listener::BoundedListener::bind(a.address(), config(&a.key)).await?;
            let _ = listener.addr().await?;
            Server::from_listeners(vec![Box::new(coap_dtls::Server(listener))])
        } else {
            Server::new_udp(a.address())?
        };
        let counter = Arc::new(AtomicU32::new(0));
        support::ready(
            "coap-rs",
            "coap 0.28.1 / webrtc-dtls 0.12.0",
            a.port,
            a.dtls,
        );
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
            let transport = coap_dtls::Client::connect(a.address(), config(&a.key)).await?;
            let connection = Arc::clone(&transport.0);
            let result = CoAPClient::from_transport(transport).send(request).await;
            // Stop timing before orderly shutdown, matching the other Rust peer.
            let elapsed = start.elapsed();
            let _ = tokio::time::timeout(Duration::from_secs(1), connection.close()).await;
            let response = result?;
            support::response(
                response.message.header.code.into(),
                &response.message.payload,
                elapsed,
            );
            return Ok::<(), Error>(());
        } else {
            coap::client::UdpCoAPClient::new(a.address())
                .await?
                .send(request)
                .await?
        };
        support::response(
            response.message.header.code.into(),
            &response.message.payload,
            start.elapsed(),
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
