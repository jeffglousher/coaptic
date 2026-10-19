//! Host interoperability fixture using public, fixed test identities.
//! Never install these identities in a deployed application. Output includes
//! the freshly derived test session keys for independent comparison.

use std::io::{self, BufRead};

use coaptic::provisioning::{
    CipherSuite, ConnectionId, Ead, EadItem, Identity, Initiator, Message, Responder, Session,
};

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

#[derive(Default)]
struct EadReceipts {
    counts: [u8; 4],
    authenticated: u8,
    invalid: bool,
}
impl EadReceipts {
    fn receive(&mut self, stage: u8, authenticated: bool, item: EadItem<'_>) -> bool {
        let index = usize::from(stage - 1);
        self.counts[index] += 1;
        if authenticated {
            self.authenticated |= 1 << index;
        }
        let correct = match (self.counts[index], item.label, item.value) {
            (1, -32768, Some(value)) => value == [stage],
            (2, 32769, None) => true,
            (3, 32770, Some(value)) => value.is_empty(),
            (_, -32771, _) => false,
            _ => {
                self.invalid = true;
                false
            }
        };
        correct && item.label == -32768
    }
}
fn outgoing_ead(enabled: bool, stage: u8, critical_stage: u8) -> Ead {
    let mut ead = Ead::new();
    if enabled {
        // Four ordered items fit the existing 64-byte aggregate bound.
        ead.push(0, Some(&[stage, 0])).expect("padding");
        ead.push(
            if critical_stage == stage {
                -32771
            } else {
                -32768
            },
            Some(&[stage]),
        )
        .expect("critical item");
        ead.push(32769, None).expect("absent elective value");
        ead.push(32770, Some(&[])).expect("empty elective value");
    }
    ead
}
fn finish(
    session: Session,
    suite: u8,
    receipts: EadReceipts,
    ead: bool,
    initiator: bool,
) -> Result<(), String> {
    let expected_counts = if !ead {
        [0; 4]
    } else if initiator {
        [0, 3, 0, 3]
    } else {
        [3, 0, 3, 0]
    };
    let expected_auth = if !ead {
        0
    } else if initiator {
        0b1010
    } else {
        0b0100
    };
    if receipts.invalid
        || receipts.counts != expected_counts
        || receipts.authenticated != expected_auth
    {
        return Err("EAD callback order/value/authentication mismatch".into());
    }
    let context_bytes = core::array::from_fn::<_, 256, _>(|index| index as u8);
    let mut exports = Vec::new();
    for (label, context_len, length) in [
        (32768, 0, 16),
        (65535, 23, 33),
        (32768, 24, 64),
        (32768, 255, 16),
        (32768, 256, 16),
    ] {
        let mut output = [0; 64];
        session
            .exporter()
            .export(label, &context_bytes[..context_len], &mut output[..length])
            .map_err(|e| format!("exporter: {e:?}"))?;
        exports.push(format!(
            "{{\"label\":{label},\"context\":\"{}\",\"length\":{length},\"hex\":\"{}\"}}",
            hex(&context_bytes[..context_len]),
            hex(&output[..length])
        ));
    }
    let (context, principal) = session.into_parts();
    println!(
        "{{\"complete\":true,\"sender_key\":\"{}\",\"recipient_key\":\"{}\",\"common_iv\":\"{}\",\"sender_id\":\"{}\",\"recipient_id\":\"{}\",\"principal\":\"{}\",\"method\":3,\"suite\":{suite},\"message4\":true,\"exports\":[{}],\"ead_received\":{:?},\"ead_authenticated\":{}}}",
        hex(context.sender_key()),
        hex(context.recipient_key()),
        hex(context.common_iv()),
        hex(context.sender_id()),
        hex(context.recipient_id()),
        hex(principal.fingerprint()),
        exports.join(","),
        receipts.counts,
        receipts.authenticated,
    );
    Ok(())
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let role = args.next().ok_or("expected initiator or responder")?;
    let mut suite = 2;
    let mut ead = false;
    let mut critical_stage = 0;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--suite" => {
                suite = args
                    .next()
                    .ok_or("missing suite")?
                    .parse::<u8>()
                    .map_err(|_| "invalid suite")?
            }
            "--ead" => ead = true,
            "--critical-stage" => {
                critical_stage = args
                    .next()
                    .ok_or("missing stage")?
                    .parse::<u8>()
                    .map_err(|_| "invalid stage")?
            }
            _ => return Err("unknown fixture option".into()),
        }
    }
    if critical_stage > 4 || (critical_stage != 0 && !ead) {
        return Err("invalid critical stage".into());
    }
    let selected = match suite {
        2 => CipherSuite::Suite2,
        3 => CipherSuite::Suite3,
        _ => return Err("unsupported fixture suite".into()),
    };
    let client = identity(1, 0);
    let server = identity(2, 1);
    let mut receipts = EadReceipts::default();
    let mut lines = io::stdin().lock().lines();
    let entropy = |bytes: &mut [u8]| getrandom::fill(bytes).is_ok();
    if role == "initiator" {
        let (state, message) = Initiator::start_with_suite_and_ead(
            &client,
            server.peer(),
            ConnectionId::new(0).expect("fixture CID"),
            selected,
            &outgoing_ead(ead, 1, critical_stage),
            entropy,
        )
        .map_err(|e| format!("{e:?}"))?;
        output(1, &message);
        let (state, message) = state
            .receive_message_2_with_ead(
                &input(&mut lines)?,
                &outgoing_ead(ead, 3, critical_stage),
                |principal, item| {
                    receipts.receive(2, *principal == server.peer().principal(), item)
                },
                |p| *p == server.peer().principal(),
            )
            .map_err(|e| format!("{e:?}"))?;
        output(3, &message);
        let session = state
            .receive_message_4_with_ead(
                &input(&mut lines)?,
                |principal, item| {
                    receipts.receive(4, *principal == server.peer().principal(), item)
                },
                |p| *p == server.peer().principal(),
            )
            .map_err(|e| format!("{e:?}"))?;
        finish(session, suite, receipts, ead, true)?;
    } else if role == "responder" {
        println!("{{\"ready\":true}}");
        let first = input(&mut lines)?;
        // This fixture offers exactly one preferred-encoded suite; retain that
        // explicit control rather than misreporting the accepted peer profile.
        if first.as_bytes().get(..2) != Some(&[3, suite]) {
            return Err("fixture suite mismatch".into());
        }
        let (state, message) = Responder::receive_message_1_with_ead(
            &server,
            client.peer(),
            ConnectionId::new(1).expect("fixture CID"),
            &first,
            &outgoing_ead(ead, 2, critical_stage),
            |item| receipts.receive(1, false, item),
            entropy,
        )
        .map_err(|e| format!("{e:?}"))?;
        output(2, &message);
        let (session, message) = state
            .receive_message_3_with_ead(
                &input(&mut lines)?,
                &outgoing_ead(ead, 4, critical_stage),
                |principal, item| {
                    receipts.receive(3, *principal == client.peer().principal(), item)
                },
                |p| *p == client.peer().principal(),
            )
            .map_err(|e| format!("{e:?}"))?;
        output(4, &message);
        finish(session, suite, receipts, ead, false)?;
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
