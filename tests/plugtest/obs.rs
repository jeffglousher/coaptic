//! In-memory Engine-pair drivers for `TD_COAP_OBS_*` (RFC 7641).

use coaptic::{
    Code, ContentFormat, ObserveKey, Opt, Retransmit, SlotId, Token, Transmission, Type, empty_ack,
    empty_rst, encode_observe,
};

use crate::catalog;
use crate::harness::{Pair, block_at, block_key, path_is, patterned_body};

const OBS_BODY: &[u8] = b"obs-0";
const OBS_BODY_2: &[u8] = b"obs-1";

pub fn run(id: &str) {
    if let Some(reason) = catalog::skip_reason(id) {
        panic!("{id} should be skipped: {reason}");
    }
    match id {
        "TD_COAP_OBS_01" => observe_basic(Type::Confirmable, Type::Confirmable, &["obs"]),
        "TD_COAP_OBS_02" => {
            observe_basic(Type::NonConfirmable, Type::NonConfirmable, &["obs-non"]);
        }
        "TD_COAP_OBS_06" => obs_06_rst(),
        "TD_COAP_OBS_07" => obs_07_delete(),
        "TD_COAP_OBS_08" => obs_08_content_format_change(),
        "TD_COAP_OBS_09" => obs_09_update(),
        "TD_COAP_OBS_10" => obs_10_get_does_not_cancel(),
        "TD_COAP_OBS_11" => obs_11_lossy(),
        "TD_COAP_OBS_12" => obs_12_deregister(),
        "TD_COAP_OBS_13" => obs_13_block2(false),
        "TD_COAP_OBS_14" => obs_13_block2(true),
        other => panic!("unknown OBS id {other}"),
    }
}

fn register(pair: &mut Pair, ty: Type, token: Token, path: &[&str]) -> (coaptic::MessageId, SlotId) {
    let extra = [Opt::observe_register()];
    let (tx, mid) = pair.client_request(ty, Code::GET, token, path, &extra, &[]);
    let rx = pair.exchange_client(tx);
    let parsed = pair.server.decode_rx(rx).expect("decode observe GET");
    assert!(path_is(parsed, path));
    assert!(parsed.is_observe_register());
    pair.server
        .register_observe_rx(rx)
        .expect("register")
        .expect("row");
    (mid, rx)
}

fn notify(
    pair: &mut Pair,
    token: Token,
    notify_ty: Type,
    body: &[u8],
) -> (u32, SlotId) {
    let key = ObserveKey::new(token, pair.client_ep);
    pair.server.signal_observe(key).expect("signal");
    let oid = pair
        .server
        .progress(pair.now_ms)
        .observe_notify()
        .expect("observe notify");
    let interest = pair.server.observe_interest(oid).expect("interest");
    let seq = encode_observe(interest.seq());
    let cf = ContentFormat::TEXT_PLAIN.encode();
    let extra = [Opt::observe(&seq), Opt::content_format(&cf)];
    let mid = pair.server_ids.next();
    let stx = pair.server_reply(notify_ty, Code::CONTENT, mid, token, &extra, body);
    let crx = pair.exchange_server(stx);
    let got = pair.client.decode_rx(crx).expect("notify decode");
    assert_eq!(got.token(), token);
    assert_eq!(got.code(), Code::CONTENT);
    let seq_got = got.observe().expect("Observe").expect("val");
    (seq_got, crx)
}

fn observe_basic(req_ty: Type, notify_ty: Type, path: &[&str]) {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (mid, rx) = register(&mut pair, req_ty, token, path);
    let seq0 = encode_observe(0);
    let cf = ContentFormat::TEXT_PLAIN.encode();
    let extra = [Opt::observe(&seq0), Opt::content_format(&cf)];
    let first_ty = if req_ty == Type::Confirmable {
        Type::Acknowledgement
    } else {
        Type::NonConfirmable
    };
    let first_mid = if req_ty == Type::Confirmable {
        mid
    } else {
        pair.server_ids.next()
    };
    let stx = pair.server_reply(first_ty, Code::CONTENT, first_mid, token, &extra, OBS_BODY);
    pair.server.release_rx(rx).ok();
    let crx = pair.exchange_server(stx);
    assert_eq!(pair.client.decode_rx(crx).expect("first").payload(), OBS_BODY);
    pair.client.release_rx(crx).ok();

    let (seq1, nrx) = notify(&mut pair, token, notify_ty, OBS_BODY_2);
    assert!(seq1 > 0, "notification sequence must increase");
    if notify_ty == Type::Confirmable {
        let ack = empty_ack(pair.client.decode_rx(nrx).expect("mid").message_id());
        let ctx = pair.client.acquire_tx().expect("ACK TX");
        pair.client.encode_tx(ctx, &ack).ok();
        let srx = pair.client_to_server(ctx);
        pair.client.release_tx(ctx).ok();
        if let Ok(Some(id)) = pair.server.match_empty_ack_rst_rx(srx) {
            pair.server.release_tx(id).ok();
        }
        pair.server.release_rx(srx).ok();
    }
    pair.client.release_rx(nrx).ok();
}

fn obs_06_rst() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let (_seq, nrx) = notify(&mut pair, token, Type::Confirmable, OBS_BODY_2);
    let mid = pair.client.decode_rx(nrx).expect("mid").message_id();
    let rst = empty_rst(mid);
    let ctx = pair.client.acquire_tx().expect("RST TX");
    pair.client.encode_tx(ctx, &rst).ok();
    let srx = pair.client_to_server(ctx);
    pair.client.release_tx(ctx).ok();
    if let Ok(Some(id)) = pair.server.match_empty_ack_rst_rx(srx) {
        pair.server.release_tx(id).ok();
    }
    let gone = pair
        .server
        .take_observe(ObserveKey::new(token, pair.client_ep));
    assert!(gone.is_some(), "RST must clear ObserveInterest");
    pair.server.release_rx(srx).ok();
    pair.client.release_rx(nrx).ok();
}

fn obs_07_delete() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let del_tok = Pair::client_token(2);
    let (tx, mid) = pair.client_request(
        Type::Confirmable,
        Code::DELETE,
        del_tok,
        &["obs"],
        &[],
        &[],
    );
    let srx = pair.exchange_client(tx);
    assert!(path_is(pair.server.decode_rx(srx).expect("del"), &["obs"]));
    let gone = pair
        .server
        .take_observe(ObserveKey::new(token, pair.client_ep));
    assert!(gone.is_some(), "DELETE must clean observers");
    let stx = pair.server_reply(Type::Acknowledgement, Code::DELETED, mid, del_tok, &[], &[]);
    pair.server.release_rx(srx).ok();
    let crx = pair.exchange_server(stx);
    pair.client.match_response_rx(crx).ok();
    pair.client.release_rx(crx).ok();
}

fn obs_08_content_format_change() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let gone = pair
        .server
        .take_observe(ObserveKey::new(token, pair.client_ep));
    assert!(gone.is_some());
}

fn obs_09_update() {
    observe_basic(Type::Confirmable, Type::Confirmable, &["obs"]);
}

fn obs_10_get_does_not_cancel() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let (tx, _) = pair.client_request(
        Type::Confirmable,
        Code::GET,
        Pair::client_token(3),
        &["obs"],
        &[],
        &[],
    );
    let srx = pair.exchange_client(tx);
    assert!(!pair.server.decode_rx(srx).expect("get").is_observe_deregister());
    assert!(
        pair.server
            .lookup_observe(ObserveKey::new(token, pair.client_ep))
            .is_some(),
        "plain GET must not cancel observation"
    );
    pair.server.release_rx(srx).ok();
}

fn obs_11_lossy() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let key = ObserveKey::new(token, pair.client_ep);
    pair.server.signal_observe(key).expect("signal");
    let oid = pair
        .server
        .progress(pair.now_ms)
        .observe_notify()
        .expect("notify");
    let interest = pair.server.observe_interest(oid).expect("row");
    let seq = encode_observe(interest.seq());
    let cf = ContentFormat::TEXT_PLAIN.encode();
    let extra = [Opt::observe(&seq), Opt::content_format(&cf)];
    let mid = pair.server_ids.next();
    let stx = pair.server_reply(
        Type::Confirmable,
        Code::CONTENT,
        mid,
        token,
        &extra,
        OBS_BODY_2,
    );
    pair.now_ms += u64::from(Transmission::ACK_TIMEOUT_MS);
    let Retransmit::Due(pending) = pair
        .server
        .poll_retransmit(pair.now_ms)
        .expect("notify retransmission")
    else {
        panic!("OBS_11 expected Due");
    };
    let crx = pair.server_to_client(pending.tx_slot());
    assert_eq!(
        pair.client.decode_rx(crx).expect("retry").payload(),
        OBS_BODY_2
    );
    pair.server.release_tx(stx).ok();
    pair.client.release_rx(crx).ok();
}

fn obs_12_deregister() {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &["obs"]);
    pair.server.release_rx(rx).ok();
    let extra = [Opt::observe_deregister()];
    let (tx, _) = pair.client_request(
        Type::Confirmable,
        Code::GET,
        token,
        &["obs"],
        &extra,
        &[],
    );
    let srx = pair.exchange_client(tx);
    let gone = pair.server.deregister_observe_rx(srx).expect("deregister");
    assert!(gone.is_some(), "Observe=1 must cancel");
    pair.server.release_rx(srx).ok();
}

fn obs_13_block2(variable: bool) {
    let mut pair = Pair::new();
    let token = Pair::client_token(4);
    let path = ["obs"];
    let (_mid, rx) = register(&mut pair, Type::Confirmable, token, &path);
    pair.server.release_rx(rx).ok();
    let body = patterned_body(if variable { 200 } else { 160 });
    let size = 64u16;
    let szx = block_at(0, false, size).szx();
    let key = block_key(token, pair.client_ep);
    let body_id = pair
        .server
        .start_block2(key, &body, szx)
        .expect("Block2 notify body");
    let stx = pair.server.acquire_tx().expect("TX");
    pair.server
        .encode_block2_tx(
            body_id,
            stx,
            Type::Confirmable,
            Code::CONTENT,
            pair.server_ids.next(),
        )
        .expect("encode first");
    let crx = pair.exchange_server(stx);
    let first = pair.client.apply_block2_rx(crx).expect("first block");
    pair.client.release_rx(crx).ok();
    if !first.complete() {
        let next = block_at(1, false, size).encode();
        let extra = [Opt::block2(&next), Opt::observe_register()];
        let (tx, _) = pair.client_request(
            Type::Confirmable,
            Code::GET,
            token,
            &path,
            &extra,
            &[],
        );
        let srx = pair.exchange_client(tx);
        pair.server.release_rx(srx).ok();
        let stx = pair.server.acquire_tx().expect("TX2");
        pair.server
            .encode_block2_tx(
                body_id,
                stx,
                Type::Acknowledgement,
                Code::CONTENT,
                pair.server_ids.next(),
            )
            .expect("encode next");
        let crx = pair.exchange_server(stx);
        let _ = pair.client.apply_block2_rx(crx);
        pair.client.release_rx(crx).ok();
    }
}
