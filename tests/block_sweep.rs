//! Combinatorial Block / Q-Block SZX × 1..=25 sweep.
//!
//! Dynamic loops, not a hard-coded case list. Policy:
//! `knowledge/block-testing.md`. Default body 4096 is not the ceiling —
//! this uses [`harness::SweepProfile`] (32 × 1024). Classic Block1/Block2
//! assembly and Q-Block windowed send/receive are all exercised. Gaps and
//! overflows fail the test.
//!
//! ```text
//! cargo test --test block_sweep
//! cargo test --test block_sweep --all-features
//! ```

#![allow(clippy::too_many_lines)]

mod harness;

use coaptic::{
    BlockKey, BlockTransfer, BlockTransferError, BlockValue, Code, Endpoint, MessageId, Token, Type,
};
use harness::{
    BLOCK_COUNTS, SWEEP_BODY_BYTES, SZX_SIZES, block_at, block_slice, build_engine, patterned_body,
    release_bodies,
};

fn token_for(tag: u8) -> Token {
    Token::new(&[tag, 0x5a]).expect("token")
}

fn key_for(tag: u8) -> BlockKey {
    BlockKey::new(token_for(tag), Endpoint::v4([198, 51, 100, tag], 5683))
}

fn body_len(blocks: u32, size: u16) -> usize {
    (blocks as usize)
        .checked_mul(usize::from(size))
        .expect("body length")
}

fn assert_body_eq(got: Option<&[u8]>, want: &[u8], ctx: &str) {
    let got = got.unwrap_or_else(|| panic!("{ctx}: missing body payload"));
    assert_eq!(
        got,
        want,
        "{ctx}: assembled {} bytes, expected {}",
        got.len(),
        want.len()
    );
}

/// Classic incoming Block1 / Block2: in-order NUM 0..n-1.
fn sweep_classic_incoming(block1: bool) {
    let mut engine = Box::new(build_engine());
    assert!(
        engine.capacities().rx_body_bytes.unwrap_or(0) >= body_len(25, 1024),
        "SweepProfile body must hold 25 × 1024"
    );
    for size in SZX_SIZES {
        for blocks in BLOCK_COUNTS {
            let want = patterned_body(body_len(blocks, size));
            let key = key_for(if block1 { 0x11 } else { 0x12 });
            let mut last_complete = false;
            for num in 0..blocks {
                let chunk = block_slice(&want, num, size);
                let more = num + 1 < blocks;
                let block = block_at(num, more, size);
                let progress = if block1 {
                    engine.apply_block1(key, block, chunk, Some(want.len() as u32))
                } else {
                    engine.apply_block2(key, block, chunk, Some(want.len() as u32))
                };
                let progress = progress.unwrap_or_else(|e| {
                    panic!(
                        "classic incoming {} SZX={size} blocks={blocks} NUM={num}: {e:?}",
                        if block1 { "Block1" } else { "Block2" }
                    )
                });
                last_complete = progress.complete();
                if !more {
                    assert!(
                        progress.complete(),
                        "classic incoming SZX={size} blocks={blocks}: M=0 not complete"
                    );
                    assert_body_eq(
                        engine.rx_body_payload(progress.id()),
                        &want,
                        &format!(
                            "classic incoming {} SZX={size} blocks={blocks}",
                            if block1 { "Block1" } else { "Block2" }
                        ),
                    );
                }
            }
            assert!(
                last_complete,
                "classic incoming SZX={size} blocks={blocks}: never completed"
            );
            release_bodies(&mut engine, key);
        }
    }
}

/// Classic outgoing Block1 / Block2: slice + encode each NUM.
fn sweep_classic_outgoing(block1: bool) {
    let mut engine = Box::new(build_engine());
    for size in SZX_SIZES {
        let szx = BlockValue::from_size(0, false, size).expect("szx").szx();
        for blocks in BLOCK_COUNTS {
            let want = patterned_body(body_len(blocks, size));
            let key = key_for(if block1 { 0x21 } else { 0x22 });
            let body_id = if block1 {
                engine.start_block1(key, &want, szx)
            } else {
                engine.start_block2(key, &want, szx)
            }
            .unwrap_or_else(|e| {
                panic!(
                    "classic outgoing {} start SZX={size} blocks={blocks}: {e:?}",
                    if block1 { "Block1" } else { "Block2" }
                )
            });
            let mut issued_complete = false;
            for num in 0..blocks {
                let tx = engine.acquire_tx().expect("TX for outgoing block");
                let issued = if block1 {
                    engine.encode_block1_tx(
                        body_id,
                        tx,
                        Type::Confirmable,
                        Code::PUT,
                        MessageId::new(num as u16),
                    )
                } else {
                    engine.encode_block2_tx(
                        body_id,
                        tx,
                        Type::Acknowledgement,
                        Code::CONTENT,
                        MessageId::new(num as u16),
                    )
                }
                .unwrap_or_else(|e| {
                    panic!(
                        "classic outgoing {} encode SZX={size} blocks={blocks} NUM={num}: {e:?}",
                        if block1 { "Block1" } else { "Block2" }
                    )
                });
                assert_eq!(issued.block().num(), num);
                assert_eq!(issued.block().more(), num + 1 < blocks);
                let parsed = engine.decode_tx(tx).expect("decode outgoing");
                let opt = if block1 {
                    parsed.block1().expect("Block1").expect("val")
                } else {
                    parsed.block2().expect("Block2").expect("val")
                };
                assert_eq!(opt.num(), num);
                assert_eq!(parsed.payload(), block_slice(&want, num, size));
                issued_complete = issued.complete();
                engine.release_tx(tx).expect("release TX");
            }
            assert!(
                issued_complete,
                "classic outgoing SZX={size} blocks={blocks}: last not complete"
            );
            release_bodies(&mut engine, key);
            // TX slots already released per issued block.
        }
    }
}

/// Incoming Q-Block: reverse-NUM within each MAX_PAYLOADS window.
fn sweep_q_incoming(q_block1: bool) {
    let mut engine = Box::new(build_engine());
    let window = u32::from(BlockTransfer::MAX_PAYLOADS);
    for size in SZX_SIZES {
        for blocks in BLOCK_COUNTS {
            let want = patterned_body(body_len(blocks, size));
            let key = key_for(if q_block1 { 0x31 } else { 0x32 });
            let mut last_complete = false;
            let mut base = 0u32;
            while base < blocks {
                let end = (base + window).min(blocks);
                let mut nums: Vec<u32> = (base..end).collect();
                nums.reverse();
                for num in nums {
                    let chunk = block_slice(&want, num, size);
                    let more = num + 1 < blocks;
                    let block = block_at(num, more, size);
                    let progress = if q_block1 {
                        engine.apply_q_block1(key, block, chunk, Some(want.len() as u32))
                    } else {
                        engine.apply_q_block2(key, block, chunk, Some(want.len() as u32))
                    };
                    let progress = progress.unwrap_or_else(|e| {
                        panic!(
                            "Q incoming {} SZX={size} blocks={blocks} NUM={num}: {e:?}",
                            if q_block1 { "Q-Block1" } else { "Q-Block2" }
                        )
                    });
                    last_complete = progress.complete();
                }
                base = end;
            }
            assert!(
                last_complete,
                "Q incoming SZX={size} blocks={blocks}: never completed"
            );
            let id = engine
                .lookup_rx_body(key)
                .unwrap_or_else(|| panic!("Q incoming SZX={size} blocks={blocks}: no body"));
            assert_body_eq(
                engine.rx_body_payload(id),
                &want,
                &format!(
                    "Q incoming {} SZX={size} blocks={blocks}",
                    if q_block1 { "Q-Block1" } else { "Q-Block2" }
                ),
            );
            release_bodies(&mut engine, key);
        }
    }
}

/// Outgoing Q-Block: issue a window, Continue, next window.
fn sweep_q_outgoing(q_block1: bool) {
    let mut engine = Box::new(build_engine());
    let window = u32::from(BlockTransfer::MAX_PAYLOADS);
    for size in SZX_SIZES {
        let szx = BlockValue::from_size(0, false, size).expect("szx").szx();
        for blocks in BLOCK_COUNTS {
            let want = patterned_body(body_len(blocks, size));
            let key = key_for(if q_block1 { 0x41 } else { 0x42 });
            let body_id = if q_block1 {
                engine.start_q_block1(key, &want, szx)
            } else {
                engine.start_q_block2(key, &want, szx)
            }
            .unwrap_or_else(|e| {
                panic!(
                    "Q outgoing {} start SZX={size} blocks={blocks}: {e:?}",
                    if q_block1 { "Q-Block1" } else { "Q-Block2" }
                )
            });
            let mut issued = 0u32;
            let mut last_complete = false;
            while issued < blocks {
                let window_end = (issued + window).min(blocks);
                while issued < window_end {
                    let tx = engine.acquire_tx().expect("TX Q outgoing");
                    let out = if q_block1 {
                        engine.encode_q_block1_tx(
                            body_id,
                            tx,
                            Type::NonConfirmable,
                            Code::PUT,
                            MessageId::new(issued as u16),
                        )
                    } else {
                        engine.encode_q_block2_tx(
                            body_id,
                            tx,
                            Type::NonConfirmable,
                            Code::CONTENT,
                            MessageId::new(issued as u16),
                        )
                    }
                    .unwrap_or_else(|e| {
                        panic!(
                            "Q outgoing {} encode SZX={size} blocks={blocks} NUM={issued}: {e:?}",
                            if q_block1 { "Q-Block1" } else { "Q-Block2" }
                        )
                    });
                    assert_eq!(out.block().num(), issued);
                    let parsed = engine.decode_tx(tx).expect("decode Q");
                    assert_eq!(parsed.payload(), block_slice(&want, issued, size));
                    last_complete = out.complete();
                    engine.release_tx(tx).expect("release TX");
                    issued += 1;
                }
                if last_complete {
                    break;
                }
                // Continue: Q-Block1 uses last NUM of the window; Q-Block2 uses next base.
                if q_block1 {
                    engine
                        .ack_q_block1(body_id, issued - 1)
                        .unwrap_or_else(|e| {
                            panic!(
                                "Q-Block1 Continue SZX={size} blocks={blocks} num={}: {e:?}",
                                issued - 1
                            )
                        });
                } else {
                    engine.ack_q_block2(body_id, issued).unwrap_or_else(|e| {
                        panic!("Q-Block2 Continue SZX={size} blocks={blocks} num={issued}: {e:?}")
                    });
                }
            }
            assert!(
                last_complete,
                "Q outgoing SZX={size} blocks={blocks}: last not complete"
            );
            release_bodies(&mut engine, key);
            // TX slots already released per issued block.
        }
    }
}

#[test]
fn sweep_classic_block1_incoming() {
    sweep_classic_incoming(true);
}

#[test]
fn sweep_classic_block2_incoming() {
    sweep_classic_incoming(false);
}

#[test]
fn sweep_classic_block1_outgoing() {
    sweep_classic_outgoing(true);
}

#[test]
fn sweep_classic_block2_outgoing() {
    sweep_classic_outgoing(false);
}

#[test]
fn sweep_q_block1_incoming_windowed() {
    sweep_q_incoming(true);
}

#[test]
fn sweep_q_block2_incoming_windowed() {
    sweep_q_incoming(false);
}

#[test]
fn sweep_q_block1_outgoing_windowed() {
    sweep_q_outgoing(true);
}

#[test]
fn sweep_q_block2_outgoing_windowed() {
    sweep_q_outgoing(false);
}

#[test]
fn sweep_axes_are_dynamic() {
    assert_eq!(SZX_SIZES, [16, 32, 64, 128, 256, 512, 1024]);
    assert_eq!(*BLOCK_COUNTS.start(), 1);
    assert_eq!(*BLOCK_COUNTS.end(), 25);
    const {
        assert!(
            SWEEP_BODY_BYTES >= 25 * 1024,
            "Default 4096 is not the ceiling"
        );
        assert!(SWEEP_BODY_BYTES % 1024 == 0);
    }
}

#[test]
fn overflow_beyond_sweep_capacity_fails_loudly() {
    let mut engine = Box::new(build_engine());
    let key = key_for(0xee);
    let chunk = [0x5au8; 1024];
    let max_blocks = SWEEP_BODY_BYTES / 1024;
    for n in 0..max_blocks {
        let more = true;
        let block = block_at(n as u32, more, 1024);
        engine
            .apply_block1(key, block, &chunk, None)
            .unwrap_or_else(|e| panic!("fits NUM={n}: {e:?}"));
    }
    let extra = block_at(max_blocks as u32, false, 1024);
    assert_eq!(
        engine.apply_block1(key, extra, &chunk[..1], None),
        Err(BlockTransferError::Overflow),
        "NUM={} must overflow SweepProfile body {}",
        max_blocks,
        SWEEP_BODY_BYTES
    );
}

#[cfg(feature = "alloc")]
#[test]
fn alloc_memory_holds_25_by_1024() {
    use coaptic::{AllocMemory, Capacities, Engine, EngineBuilder};

    let caps = Capacities {
        rx_datagram_slots: 8,
        rx_datagram_bytes: 1472,
        tx_datagram_slots: 8,
        tx_datagram_bytes: 1472,
        dedup_entries: 8,
        observe_entries: 4,
        rx_body_slots: Some(2),
        rx_body_bytes: Some(32 * 1024),
        tx_body_slots: Some(2),
        tx_body_bytes: Some(32 * 1024),
    };
    let mut engine: Engine<AllocMemory> = EngineBuilder::new()
        .rx_datagram(8, 1472)
        .tx_datagram(8, 1472)
        .dedup(8)
        .observe(4)
        .rx_body(2, 32 * 1024)
        .tx_body(2, 32 * 1024)
        .block_wise(true)
        .build_alloc(caps)
        .expect("AllocMemory sweep capacities");
    let want = patterned_body(25 * 1024);
    let key = key_for(0xA1);
    for num in 0..25u32 {
        let chunk = block_slice(&want, num, 1024);
        let more = num + 1 < 25;
        engine
            .apply_block1(
                key,
                block_at(num, more, 1024),
                chunk,
                Some(want.len() as u32),
            )
            .unwrap_or_else(|e| panic!("AllocMemory Block1 NUM={num}: {e:?}"));
    }
    let id = engine.lookup_rx_body(key).expect("assembled");
    assert_eq!(engine.rx_body_payload(id), Some(want.as_slice()));
}
