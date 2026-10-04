//! UDP listener for a coap-rs fixture that issues empty ACKs before `/separate`.

use async_trait::async_trait;
use coap::server::{Listener, Responder, TransportRequestSender};
use coap_lite::{CoapOption, MessageClass, MessageType, Packet, RequestType};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct SeparateListener {
    socket: tokio::net::UdpSocket,
    response_rx: UnboundedReceiver<(Vec<u8>, SocketAddr)>,
    response_tx: UnboundedSender<(Vec<u8>, SocketAddr)>,
}

impl SeparateListener {
    pub async fn bind(address: SocketAddr) -> std::io::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(address).await?;
        let (response_tx, response_rx) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            socket,
            response_rx,
            response_tx,
        })
    }
}

struct SeparateResponder {
    address: SocketAddr,
    tx: UnboundedSender<(Vec<u8>, SocketAddr)>,
}

#[async_trait]
impl Responder for SeparateResponder {
    async fn respond(&self, response: Vec<u8>) {
        let _ = self.tx.send((response, self.address));
    }
    fn address(&self) -> SocketAddr {
        self.address
    }
}

#[async_trait]
impl Listener for SeparateListener {
    async fn listen(
        mut self: Box<Self>,
        sender: TransportRequestSender,
    ) -> std::io::Result<tokio::task::JoinHandle<std::io::Result<()>>> {
        Ok(tokio::spawn(async move {
            let mut buffer = [0u8; 2048];
            loop {
                tokio::select! {
                    datagram = self.socket.recv_from(&mut buffer) => {
                        let (size, source) = datagram?;
                        if let Ok(packet) = Packet::from_bytes(&buffer[..size]) {
                            let separate = packet.get_option(CoapOption::UriPath)
                                .is_some_and(|parts| parts.len() == 1 && parts.front().is_some_and(|path| path == b"separate"));
                            if separate && packet.header.get_type() == MessageType::Confirmable
                                && packet.header.code == MessageClass::Request(RequestType::Get) {
                                let mid = packet.header.message_id.to_be_bytes();
                                self.socket.send_to(&[0x60, 0, mid[0], mid[1]], source).await?;
                            }
                        }
                        sender.send((buffer[..size].to_vec(), Arc::new(SeparateResponder {
                            address: source, tx: self.response_tx.clone(),
                        }))).map_err(|_| std::io::Error::other("server receiver closed"))?;
                    }
                    response = self.response_rx.recv() => {
                        let Some((bytes, destination)) = response else { return Ok(()) };
                        self.socket.send_to(&bytes, destination).await?;
                    }
                }
            }
        }))
    }
}
