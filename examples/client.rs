//! README client; explicitly plaintext on localhost.

use coaptic::{App, Endpoint};
use std::{
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let mut client = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_plaintext()
        .bind(socket)?;

    let peer = Endpoint::v4([127, 0, 0, 1], 5683);
    let clock = Instant::now();
    let call = client.get("sensors/temp").to(peer).send(0)?;
    let mut body = [0; 1472];
    loop {
        client.poll(clock.elapsed().as_millis() as u64)?;
        if let Some(reply) = client.take_response_into(call, &mut body)? {
            let reply = reply?;
            if !reply.code().is_success() {
                return Err(format!("peer replied {}", reply.code()).into());
            }
            println!("{}", String::from_utf8_lossy(reply.payload()));
            return Ok(());
        }
        if clock.elapsed() >= Duration::from_secs(5) {
            return Err("request timed out".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
