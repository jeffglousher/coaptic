//! Host interoperability fixture using public, fixed test identities.
//! Never install these identities in a deployed application. Output includes
//! the freshly derived test session keys for independent comparison.

use std::io::{self, BufRead};

use coaptic::provisioning::{Identity, Initiator, Message, Responder, Session};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn input(lines: &mut impl Iterator<Item = io::Result<String>>) -> Result<Message, String> {
    let line = lines
        .next()
        .ok_or("peer closed input")?
        .map_err(|e| e.to_string())?;
    let line = line.trim();
    if line.len() > Message::CAPACITY * 2 || line.len() % 2 != 0 || !line.is_ascii() {
        return Err("invalid hex length".into());
    }
    let bytes = (0..line.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&line[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Message::from_slice(&bytes).map_err(|e| format!("{e:?}"))
}

fn output(number: u8, message: &Message) {
    println!(
        "{{\"message\":{number},\"hex\":\"{}\"}}",
        hex(message.as_bytes())
    );
}

fn identity(value: u8, kid: u8) -> Identity {
    let mut bytes = [0; 32];
    bytes[31] = value;
    Identity::from_private_key(bytes, kid).expect("fixed test identity")
}

fn finish(session: Session) {
    let (context, principal) = session.into_parts();
    println!(
        "{{\"complete\":true,\"sender_key\":\"{}\",\"recipient_key\":\"{}\",\"common_iv\":\"{}\",\"sender_id\":\"{}\",\"recipient_id\":\"{}\",\"principal\":\"{}\",\"method\":3,\"suite\":2,\"message4\":true}}",
        hex(context.sender_key()),
        hex(context.recipient_key()),
        hex(context.common_iv()),
        hex(context.sender_id()),
        hex(context.recipient_id()),
        hex(principal.fingerprint()),
    );
}

fn run() -> Result<(), String> {
    let role = std::env::args()
        .nth(1)
        .ok_or("expected initiator or responder")?;
    let client = identity(1, 0);
    let server = identity(2, 1);
    let mut lines = io::stdin().lock().lines();
    let entropy = |bytes: &mut [u8]| getrandom::fill(bytes).is_ok();
    if role == "initiator" {
        let (state, message) =
            Initiator::start(&client, server.peer(), entropy).map_err(|e| format!("{e:?}"))?;
        output(1, &message);
        let (state, message) = state
            .receive_message_2(&input(&mut lines)?, |p| *p == server.peer().principal())
            .map_err(|e| format!("{e:?}"))?;
        output(3, &message);
        let session = state
            .receive_message_4(&input(&mut lines)?, |p| *p == server.peer().principal())
            .map_err(|e| format!("{e:?}"))?;
        finish(session);
    } else if role == "responder" {
        println!("{{\"ready\":true}}");
        let (state, message) =
            Responder::receive_message_1(&server, client.peer(), &input(&mut lines)?, entropy)
                .map_err(|e| format!("{e:?}"))?;
        output(2, &message);
        let (session, message) = state
            .receive_message_3(&input(&mut lines)?, |p| *p == client.peer().principal())
            .map_err(|e| format!("{e:?}"))?;
        output(4, &message);
        finish(session);
    } else {
        return Err("expected initiator or responder".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
