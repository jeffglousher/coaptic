//! Modern DTLS socket adapter owned only by the Coaptic executable.
use coaptic::storage::{DatagramIo, Endpoint};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use tokio::sync::mpsc as tmpsc;
use webrtc_dtls::{cipher_suite::CipherSuiteId, config::Config, conn::DTLSConn};
use webrtc_util::conn::{Conn, Listener};
type Routes = Arc<Mutex<HashMap<SocketAddr, tmpsc::Sender<Vec<u8>>>>>;
pub struct DtlsIo {
    incoming: mpsc::Receiver<(SocketAddr, Vec<u8>)>,
    routes: Routes,
}
impl DtlsIo {
    pub async fn listen_at(addr: SocketAddr, config: Config) -> Result<(SocketAddr, Self), String> {
        let listener = webrtc_dtls::listener::listen(addr, config)
            .await
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(64);
        let routes: Routes = Arc::default();
        let map = routes.clone();
        tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(2), listener.accept()).await {
                    Ok(Ok((conn, peer))) => attach(conn, peer, tx.clone(), map.clone()),
                    _ => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        });
        Ok((
            addr,
            Self {
                incoming: rx,
                routes,
            },
        ))
    }
    pub async fn connect(
        addr: SocketAddr,
        config: Config,
        timeout: Duration,
    ) -> Result<Self, String> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        socket.connect(addr).await.map_err(|e| e.to_string())?;
        let conn =
            tokio::time::timeout(timeout, DTLSConn::new(Arc::new(socket), config, true, None))
                .await
                .map_err(|_| "DTLS handshake timeout")?
                .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(64);
        let routes: Routes = Arc::default();
        attach(Arc::new(conn), addr, tx, routes.clone());
        Ok(Self {
            incoming: rx,
            routes,
        })
    }
}
fn attach(
    conn: Arc<dyn Conn + Send + Sync>,
    peer: SocketAddr,
    tx: mpsc::SyncSender<(SocketAddr, Vec<u8>)>,
    routes: Routes,
) {
    let (out, mut rx) = tmpsc::channel::<Vec<u8>>(64);
    {
        let mut map = routes.lock().expect("routes");
        if map.len() >= 128 {
            return;
        }
        map.insert(peer, out);
    }
    tokio::spawn(async move {
        let mut buf = [0; 4096];
        loop {
            tokio::select! {
                received=tokio::time::timeout(Duration::from_secs(5),conn.recv(&mut buf))=>match received {
                    Ok(Ok(n)) if n>0=> {if tx.try_send((peer,buf[..n].to_vec())).is_err(){break;}}, _=>break
                },
                outgoing=rx.recv()=>match outgoing {Some(bytes)=>{if conn.send(&bytes).await.is_err(){break;}},None=>break}
            }
        }
        routes.lock().expect("routes").remove(&peer);
        let _ = conn.close().await;
    });
}
impl DatagramIo for DtlsIo {
    type Error = std::io::Error;
    fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        match self.incoming.try_recv() {
            Ok((peer, bytes)) => {
                if bytes.len() > buf.len() {
                    return Err(std::io::Error::other("oversize DTLS datagram"));
                }
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok(Some((bytes.len(), peer.into())))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(_) => Err(std::io::Error::other("DTLS receiver closed")),
        }
    }
    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        let addr = std::net::SocketAddr::from(peer);
        self.routes
            .lock()
            .expect("routes")
            .get(&addr)
            .ok_or_else(|| std::io::Error::other("unknown DTLS session"))?
            .try_send(bytes.to_vec())
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(bytes.len())
    }
}
pub fn psk_config(key: &[u8]) -> Config {
    let key = key.to_vec();
    Config {
        psk: Some(Arc::new(move |_| Ok(key.clone()))),
        psk_identity_hint: Some(b"password".to_vec()),
        cipher_suites: vec![CipherSuiteId::Tls_Psk_With_Aes_128_Ccm_8],
        ..Default::default()
    }
}
