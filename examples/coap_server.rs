//! Taste of [`coaptic::App`]: routes, [`State`], one `poll` loop.
//!
//! ```text
//! cargo run --example coap_server --features std
//! ```
//!
//! Client GET `/sensors/temp` → 2.05 Content. PUT `/leds/0` then GET
//! reflects shared state. `/.well-known/core` is link-format from
//! registered paths. Engine slots stay off the happy path (`CALLER.md`).

use std::net::UdpSocket;
use std::time::Instant;

use coaptic::{
    App, Code, ContentFormat, DatagramIo, Endpoint, Ids, Opt, OptionsBuilder, ParsedMessage, Reply,
    Request, State, Token, decode, encode, get, profiles,
};

struct Sensors {
    temp_c: i16,
    led_on: bool,
}

fn get_temp(State(s): State<&mut Sensors>, _req: Request<'_>) -> Reply {
    let _ = s.temp_c;
    Reply::content(b"21.5").content_format(ContentFormat::TEXT_PLAIN)
}

fn get_led(State(s): State<&mut Sensors>, _: Request<'_>) -> Reply {
    Reply::content(if s.led_on { b"on" } else { b"off" })
}

fn put_led(State(s): State<&mut Sensors>, req: Request<'_>) -> Reply {
    s.led_on = req.payload() == b"1";
    Reply::changed()
}

fn now_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let origin = Instant::now();

    let socket = UdpSocket::bind("127.0.0.1:0")?;
    let mut client = UdpSocket::bind("127.0.0.1:0")?;
    let server_ep = Endpoint::from(socket.local_addr()?);
    let timeout = std::time::Duration::from_millis(200);
    socket.set_read_timeout(Some(timeout))?;
    client.set_read_timeout(Some(timeout))?;

    let mut app = App::profile::<profiles::Default>()
        .block_wise(true)
        .state(Sensors {
            temp_c: 215,
            led_on: false,
        })
        .route(&["sensors", "temp"], get(get_temp))
        .route(&["leds", "0"], get(get_led).put(put_led))
        .well_known_core()
        .bind(socket)?;

    send_client(&mut client, server_ep, Code::GET, &["sensors", "temp"], &[]);
    app.poll(now_ms(origin))?;
    expect_reply(&mut client, Code::CONTENT, Some(b"21.5"))?;

    send_client(&mut client, server_ep, Code::PUT, &["leds", "0"], b"1");
    app.poll(now_ms(origin))?;
    expect_reply(&mut client, Code::CHANGED, None)?;

    send_client(&mut client, server_ep, Code::GET, &["leds", "0"], &[]);
    app.poll(now_ms(origin))?;
    expect_reply(&mut client, Code::CONTENT, Some(b"on"))?;

    send_client(
        &mut client,
        server_ep,
        Code::GET,
        &[".well-known", "core"],
        &[],
    );
    app.poll(now_ms(origin))?;
    expect_reply(
        &mut client,
        Code::CONTENT,
        Some(b"</sensors/temp>,</leds/0>"),
    )?;

    eprintln!("coap_server: 2.05 \"21.5\"; PUT led; GET on; well-known/core");
    Ok(())
}

fn send_client<T: DatagramIo>(
    client: &mut T,
    server: Endpoint,
    code: Code,
    path: &[&str],
    payload: &[u8],
) where
    T::Error: core::fmt::Debug,
{
    let token = Token::mint(2, &[0xC0, 0xA1]).expect("token");
    let mut ids = Ids::new(0x1000);
    let mut opts = OptionsBuilder::<4>::new();
    for segment in path {
        opts.push(Opt::uri_path(segment)).expect("path room");
    }
    let msg = ids
        .con(code, token)
        .with_options(opts.as_slice())
        .with_payload(payload);
    let mut wire = [0u8; 1472];
    let n = encode(&msg, &mut wire).expect("encode");
    client.send(server, &wire[..n]).expect("client send");
}

fn expect_reply<T: DatagramIo>(
    client: &mut T,
    code: Code,
    payload: Option<&[u8]>,
) -> Result<(), Box<dyn std::error::Error>>
where
    T::Error: core::fmt::Debug + std::error::Error + 'static,
{
    let mut buf = [0u8; 1472];
    let n = match DatagramIo::recv(client, &mut buf)? {
        Some((n, _)) => n,
        None => return Err("client expected a response".into()),
    };
    let parsed: ParsedMessage<'_> = decode(&buf[..n])?;
    assert_eq!(parsed.code(), code);
    if let Some(expected) = payload {
        assert_eq!(parsed.payload(), expected);
    }
    Ok(())
}
