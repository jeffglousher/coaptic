//! Caller-owned file persistence across process termination.
//!
//! The fixture trusts its local storage and tests commit ordering, sender range
//! reservation, replay refusal and storage-error refusal. It does not qualify
//! flash power-loss behavior or protection against replacement of durable files.
#![cfg(feature = "oscore")]

use coaptic::message::{Code, Message, MessageId, Token, Type, decode};
use coaptic::oscore::{DeriveParams, Error, ReplayCheckpoint, SecurityContext};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CONTEXT: &[u8; 8] = b"persist1";

fn context(server: bool) -> SecurityContext {
    SecurityContext::derive(DeriveParams {
        master_secret: b"qualification fixture secret",
        master_salt: &[],
        sender_id: if server { &[2] } else { &[1] },
        recipient_id: if server { &[1] } else { &[2] },
        id_context: CONTEXT,
    })
    .unwrap()
}

fn read_state(path: &Path) -> std::io::Result<(ReplayCheckpoint, u64)> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() != 28 || &bytes[..8] != CONTEXT {
        return Err(std::io::Error::other("invalid durable context record"));
    }
    let left = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let bitmap = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let sender = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let checkpoint = ReplayCheckpoint::from_parts(left, bitmap)
        .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    if sender > 1 << 40 {
        return Err(std::io::Error::other("invalid durable sender bound"));
    }
    Ok((checkpoint, sender))
}

fn write_state(path: &Path, checkpoint: ReplayCheckpoint, sender: u64) -> std::io::Result<()> {
    let (left, bitmap) = checkpoint.parts();
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(CONTEXT)?;
    file.write_all(&left.to_le_bytes())?;
    file.write_all(&bitmap.to_le_bytes())?;
    file.write_all(&sender.to_le_bytes())?;
    file.sync_all()
}

fn packet(sequence: u64) -> Vec<u8> {
    let mut sender = context(false);
    sender.set_sender_seq(sequence).unwrap();
    let plain = Message::new(Type::Confirmable, Code::POST, MessageId::new(10))
        .with_token(Token::new(&[1]).unwrap())
        .with_payload(b"effect");
    let mut bytes = [0u8; 1280];
    let length = sender.protect_request(&plain, &mut bytes).unwrap();
    bytes[..length].to_vec()
}

#[test]
fn persistent_replay_worker() {
    let Some(directory) = std::env::var_os("COAPTIC_REPLAY_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let phase = std::env::var("COAPTIC_REPLAY_PHASE").unwrap();
    let state_path = directory.join("state");
    let (checkpoint, sender_bound) = match read_state(&state_path) {
        Ok(state) => state,
        Err(_) => std::process::exit(74),
    };
    let mut recipient = context(true);
    recipient.restore_replay(checkpoint).unwrap();
    recipient.restore_sender_reservation(sender_bound).unwrap();
    if phase == "sender-storage-error" {
        let reservation = recipient.reserve_sender_sequences(32, |_| {
            File::open(&state_path)?.write_all(b"cannot reserve")
        });
        assert!(matches!(
            reservation,
            Err(coaptic::oscore::SenderReservationError::Persistence(_))
        ));
        assert_eq!(recipient.sender_seq(), sender_bound);
        assert_eq!(recipient.sender_reservation_end(), Some(sender_bound));
        let request = Message::con(Code::POST, MessageId::new(11), Token::EMPTY);
        assert_eq!(
            recipient.protect_request(&request, &mut [0; 128]),
            Err(Error::SequenceUnreserved)
        );
        std::process::exit(76);
    }
    let reserved = recipient
        .reserve_sender_sequences(32, |end| write_state(&state_path, checkpoint, end))
        .unwrap();
    let bytes = fs::read(directory.join("packet")).unwrap();
    let protected = decode(&bytes).unwrap();
    let mut plain = [0u8; 1280];
    let request = match recipient.unprotect_request(&protected, &mut plain) {
        Ok((message, request)) => {
            assert_eq!(message.payload(), b"effect");
            request
        }
        Err(Error::Replay) => std::process::exit(73),
        Err(error) => panic!("unexpected authentication error: {error:?}"),
    };
    if phase == "before-commit" {
        std::process::exit(70);
    }
    if phase == "storage-error" {
        let mut file = File::open(&state_path).unwrap();
        assert!(file.write_all(b"cannot commit").is_err());
        std::process::exit(75);
    }
    write_state(&state_path, recipient.replay_checkpoint(), reserved).unwrap();
    if phase == "after-commit" {
        std::process::exit(71);
    }
    let mut effects = OpenOptions::new()
        .append(true)
        .create(true)
        .open(directory.join("effects"))
        .unwrap();
    effects.write_all(b"effect\n").unwrap();
    effects.sync_all().unwrap();
    let response = Message::new(Type::Acknowledgement, Code::CHANGED, MessageId::new(10))
        .with_token(Token::new(&[1]).unwrap());
    let mut wire = [0u8; 1280];
    let length = recipient
        .protect_response_with_piv(&response, request, &mut wire)
        .unwrap();
    assert_eq!(recipient.sender_seq(), sender_bound + 1);
    fs::write(
        directory.join(format!("response-{sender_bound}")),
        &wire[..length],
    )
    .unwrap();
}

fn worker(directory: &Path, phase: &str) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "persistent_replay_worker", "--nocapture"])
        .env("COAPTIC_REPLAY_DIRECTORY", directory)
        .env("COAPTIC_REPLAY_PHASE", phase)
        .output()
        .unwrap()
}

struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn durable_replay_barrier_survives_restart_and_refuses_storage_errors() {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).unwrap();
    let root = Directory(
        std::env::temp_dir().join(format!("coaptic-replay-{}", u64::from_le_bytes(random))),
    );
    fs::create_dir(&root.0).unwrap();
    for (phase, exit, expected_effects) in [
        ("before-commit", 70, 1),
        ("storage-error", 75, 1),
        ("sender-storage-error", 76, 1),
        ("after-commit", 71, 0),
        ("complete", 0, 1),
    ] {
        let directory = root.0.join(phase);
        fs::create_dir(&directory).unwrap();
        write_state(
            &directory.join("state"),
            context(true).replay_checkpoint(),
            0,
        )
        .unwrap();
        fs::write(directory.join("packet"), packet(0)).unwrap();
        let first = worker(&directory, phase);
        assert_eq!(first.status.code(), Some(exit), "{phase}: {first:?}");
        if phase != "complete" {
            assert!(!directory.join("effects").exists());
        }
        let second = worker(&directory, "complete");
        let expected_exit = if matches!(phase, "after-commit" | "complete") {
            73
        } else {
            0
        };
        assert_eq!(
            second.status.code(),
            Some(expected_exit),
            "{phase}: {second:?}"
        );
        let effects = fs::read(directory.join("effects")).unwrap_or_default();
        assert_eq!(effects.len(), expected_effects * 7);
        assert_eq!(worker(&directory, "complete").status.code(), Some(73));
        let fresh_sender_bound = read_state(&directory.join("state")).unwrap().1;
        let mut client = context(false);
        client.set_sender_seq(1).unwrap();
        let token = Token::new(&[1]).unwrap();
        let request = Message::con(Code::POST, MessageId::new(10), token).with_payload(b"effect");
        let mut wire = [0; 1280];
        let length = client.protect_request(&request, &mut wire).unwrap();
        fs::write(directory.join("packet"), &wire[..length]).unwrap();
        let fresh = worker(&directory, "complete");
        assert!(fresh.status.success(), "{fresh:?}");
        assert_eq!(
            fs::read(directory.join("effects")).unwrap().len(),
            (expected_effects + 1) * 7
        );
        let response = fs::read(directory.join(format!("response-{fresh_sender_bound}"))).unwrap();
        let response = decode(&response).unwrap();
        let mut plain = [0; 1280];
        let response = client
            .unprotect_response(&response, client.lookup(token).unwrap(), &mut plain)
            .unwrap();
        assert_eq!(response.code(), Code::CHANGED);
        assert!(response.payload().is_empty());
        if phase == "complete" {
            assert!(directory.join("response-0").exists());
        }
        let saved = fs::read(directory.join("state")).unwrap();
        fs::write(directory.join("state"), &saved[..27]).unwrap();
        assert_eq!(worker(&directory, "complete").status.code(), Some(74));
        let mut wrong_context = saved;
        wrong_context[0] ^= 1;
        fs::write(directory.join("state"), wrong_context).unwrap();
        assert_eq!(worker(&directory, "complete").status.code(), Some(74));
    }
    println!(
        "persistent replay: five commit phases, replay after restart, guarded sender reservations, protected response recovery, truncated and wrong-context storage refusal"
    );
}
