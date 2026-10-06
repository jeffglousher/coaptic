//! Independent server fixtures; no request-path logging or shared driver code.
#![forbid(unsafe_code)]
use coap_lite::{CoapOption, MessageClass, MessageType, Packet, ResponseType};
use coaptic::storage::Capacities;
use coaptic::{App, ContentFormat, Request, Response, get};
use std::{
    env,
    net::UdpSocket,
    sync::{Arc, OnceLock},
    time::Instant,
};

static BODY: OnceLock<&'static [u8]> = OnceLock::new();

fn representation(_: Request<'_>) -> Response<'static> {
    Response::content(BODY.get().expect("initialized fixture"))
        .content_format(ContentFormat::OCTET_STREAM)
        .etag(b"fixture")
}

fn coaptic_server(address: &str, bytes: usize) -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(address)?;
    socket.set_nonblocking(true)?;
    let capacities = Capacities {
        rx_datagram_slots: 4,
        rx_datagram_bytes: 1472,
        tx_datagram_slots: 4,
        tx_datagram_bytes: 1472,
        dedup_entries: 8,
        observe_entries: 4,
        rx_body_slots: Some(1),
        rx_body_bytes: Some(bytes.div_ceil(1024) * 1024),
        tx_body_slots: Some(2),
        tx_body_bytes: Some(bytes.div_ceil(1024) * 1024),
    };
    let mut app = App::builder()
        .routes::<1>()
        .block_wise::<true>()
        .randomness(|buffer| getrandom::fill(buffer).is_ok())
        .route("bench", get(representation))
        .bind_alloc(socket, capacities)?;
    let started = Instant::now();
    loop {
        app.poll(started.elapsed().as_millis() as u64)?;
    }
}

fn codec_server(address: &str, body: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(address)?;
    let mut bytes = [0u8; 65_535];
    loop {
        let (count, peer) = socket.recv_from(&mut bytes)?;
        let request = match Packet::from_bytes(&bytes[..count]) {
            Ok(packet) => packet,
            Err(_) => continue,
        };
        if request.header.code != MessageClass::Request(coap_lite::RequestType::Get) {
            continue;
        }
        let block = request
            .get_option(CoapOption::Block2)
            .and_then(|values| values.front())
            .map(|v| {
                v.iter()
                    .fold(0usize, |value, byte| value << 8 | usize::from(*byte))
            });
        let number = block.unwrap_or(0) >> 4;
        let size = if let Some(value) = block {
            1usize << ((value & 7).min(6) + 4)
        } else {
            1024
        };
        let offset = number * size;
        if offset >= body.len() {
            continue;
        }
        let end = (offset + size).min(body.len());
        let mut response = Packet::new();
        response
            .header
            .set_type(if request.header.get_type() == MessageType::Confirmable {
                MessageType::Acknowledgement
            } else {
                MessageType::NonConfirmable
            });
        response.header.message_id = request.header.message_id;
        response.header.code = MessageClass::Response(ResponseType::Content);
        response.set_token(request.get_token().to_vec());
        response.payload = body[offset..end].to_vec();
        response.set_option(CoapOption::ContentFormat, [vec![42]].into());
        if block.is_some() || body.len() > 1024 {
            let raw = number << 4
                | usize::from(end < body.len()) << 3
                | (size.trailing_zeros() as usize - 4);
            let value = (raw as u32).to_be_bytes();
            let first = value.iter().position(|byte| *byte != 0).unwrap_or(4);
            response.set_option(CoapOption::Block2, [value[first..].to_vec()].into());
        }
        socket.send_to(&response.to_bytes()?, peer)?;
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 {
        return Err(
            "usage: bench-rust-server coaptic|coap-rs|coap-lite-codec HOST PORT BYTES".into(),
        );
    }
    let size: usize = args[4].parse()?;
    if !(1..=1_048_576).contains(&size) {
        return Err("fixture size".into());
    }
    let body: &'static [u8] = Box::leak(
        (0..size)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    BODY.set(body).map_err(|_| "fixture initialization")?;
    let address = format!("{}:{}", args[2], args[3]);
    match args[1].as_str() {
        "coaptic" => coaptic_server(&address, size),
        "coap-lite-codec" => codec_server(&address, body),
        "coap-rs" => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(async {
                let server = coap::Server::new_udp(&address)?;
                let body: Arc<[u8]> = Arc::from(body);
                server
                    .run(
                        move |mut request: Box<coap_lite::CoapRequest<std::net::SocketAddr>>| {
                            let body = Arc::clone(&body);
                            async move {
                                if let Some(response) = request.response.as_mut() {
                                    response.message.header.code =
                                        MessageClass::Response(ResponseType::Content);
                                    response.message.payload = body.to_vec();
                                    response
                                        .message
                                        .set_option(CoapOption::ContentFormat, [vec![42]].into());
                                }
                                request
                            }
                        },
                    )
                    .await?;
                Ok(())
            }),
        _ => Err("unknown implementation".into()),
    }
}
