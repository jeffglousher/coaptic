//! Packet headroom within configured pools; no separate reserve buffers.
use super::*;

pub(super) const RX_HEADROOM: usize = 1;
pub(super) const TX_HEADROOM: usize = 1;

pub(super) fn free_tx<S: Storage>(engine: &mut Engine<S>) -> usize {
    engine
        .capacities()
        .tx_datagram_slots
        .saturating_sub(engine.tx_occupied())
}
pub(super) fn free_rx<S: Storage>(engine: &mut Engine<S>) -> usize {
    engine
        .capacities()
        .rx_datagram_slots
        .saturating_sub(engine.rx_occupied())
}

// Ordinary ownership may outlive this pass (CON, cached reply, or caller state).
pub(super) fn ordinary_tx<S: Storage>(engine: &mut Engine<S>) -> Option<SlotId> {
    if free_tx(engine) <= TX_HEADROOM {
        Metrics::inc(&mut engine.metrics_mut().admission_tx_refused);
        return None;
    }
    engine.acquire_tx()
}

// Callers must send-and-release: never promote this slot to pending/cache state.
pub(super) fn control_tx<S: Storage>(engine: &mut Engine<S>) -> Option<SlotId> {
    engine.acquire_tx()
}

pub(super) enum RequestAdmission {
    Ready,
    Continuation,
    Refused,
}

pub(super) fn admit_request<S: AppStorage>(
    engine: &mut Engine<S>,
    parsed: &ParsedMessage<'_, S::Token>,
    peer: Endpoint,
    options: &RequestOptions<'_>,
) -> RequestAdmission {
    // Existing body continuations and cancellation release or advance already
    // admitted work. Their individual send paths still enforce TX ownership.
    let rx_free = free_rx(engine);
    let tx_free = free_tx(engine);
    if rx_free >= RX_HEADROOM && tx_free > TX_HEADROOM {
        return RequestAdmission::Ready;
    }
    let existing = ((options.block2.is_some() || options.q_block2.is_some())
        && response_body_for(engine, BlockKey::new(parsed.token(), peer)).is_some())
        || (options.observe == Some(Ok(1))
            && engine
                .lookup_observe(ObserveKey::new(parsed.token(), peer))
                .is_some())
        || ((options.block1.is_some() || options.q_block1.is_some())
            && engine.admitted_upload(parsed, peer));
    if existing {
        RequestAdmission::Continuation
    } else if refuse_at_headroom(engine, rx_free, tx_free) {
        RequestAdmission::Refused
    } else {
        RequestAdmission::Ready
    }
}

pub(super) fn refuse_dispatch<S: AppStorage>(engine: &mut Engine<S>) -> bool {
    let rx_free = free_rx(engine);
    let tx_free = free_tx(engine);
    refuse_at_headroom(engine, rx_free, tx_free)
}

fn refuse_at_headroom<S: AppStorage>(
    engine: &mut Engine<S>,
    rx_free: usize,
    tx_free: usize,
) -> bool {
    if rx_free < RX_HEADROOM {
        Metrics::inc(&mut engine.metrics_mut().admission_rx_refused);
        return true;
    }
    !ensure_tx_headroom(engine, tx_free)
}

fn ensure_tx_headroom<S: Storage + DedupSlots>(engine: &mut Engine<S>, mut tx_free: usize) -> bool {
    for _ in 0..=TX_HEADROOM {
        if tx_free > TX_HEADROOM {
            return true;
        }
        if !evict_one_dedup_pin(engine) {
            break;
        }
        tx_free = free_tx(engine);
    }
    if tx_free <= TX_HEADROOM {
        Metrics::inc(&mut engine.metrics_mut().admission_tx_refused);
        return false;
    }
    true
}

// Admission and eviction share one free-slot check. A failed eviction cannot
// create packet headroom. Check again only after actually releasing a pin.
pub(super) fn acquire_tx_or_evict<S: Storage + DatagramSlots + DedupSlots>(
    engine: &mut Engine<S>,
) -> Option<SlotId> {
    let free = free_tx(engine);
    ensure_tx_headroom(engine, free)
        .then(|| engine.acquire_tx())
        .flatten()
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_control_response_async, bound(crate::app::async_server::ProtocolIo), calls(finish_send => finish_send_async))]
pub(super) fn send_control_response<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    response: &Response<'_, S::Token>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + DedupSlots,
    T: DatagramIo,
{
    let ty = match meta.ty {
        Type::Confirmable => Type::Acknowledgement,
        Type::NonConfirmable => Type::NonConfirmable,
        _ => return Ok(()),
    };
    let mid = if ty == Type::NonConfirmable {
        ids.next_for(engine, io, now_ms)?
    } else {
        meta.mid
    };
    let tx = control_tx(engine).ok_or(Error::Saturated)?;
    if let Err(error) = encode_response::<S, T::Error, REQUESTS>(
        engine,
        meta.dest,
        tx,
        ty,
        mid,
        meta.token,
        response,
        response.payload(),
        None,
        meta.block1,
        oscore,
        meta.oscore,
    ) {
        let _ = engine.release_tx(tx);
        return Err(error);
    }
    if meta.ty == Type::Confirmable {
        // Only copy into existing inline history; this packet must never be pinned.
        let entry = engine.access_tx(tx).ok().and_then(|access| {
            let bytes = access.as_bytes();
            (bytes.len() <= DedupEntry::REPLAY_MAX).then(|| {
                DedupEntry::new(meta.mid, meta.dest)
                    .with_due_ms(dedup_due_ms(now_ms))
                    .with_replay(bytes)
            })
        });
        if let Some(entry) = entry {
            note_dedup_store(engine, entry, meta.request, dedup_closed);
        } else {
            remember_empty_ack(
                engine,
                meta.dest,
                meta.mid,
                now_ms,
                meta.request,
                dedup_closed,
            );
        }
    }
    finish_send(engine, io, tx, meta.dest, None)
}

#[allow(clippy::too_many_arguments)]
#[coaptic_io::dual_io(send_refusal_async, bound(crate::app::async_server::ProtocolIo), calls(send_control_response => send_control_response_async, send_empty_ack => send_empty_ack_async))]
pub(super) fn send_refusal<S, T, const REQUESTS: usize>(
    engine: &mut Engine<S>,
    io: &mut T,
    ids: &mut AppIds,
    now_ms: u64,
    meta: SendResponse<'_, S::Token>,
    oscore: &mut oscore::Field<REQUESTS, S::Token>,
    dedup_closed: &mut Option<DedupClosed>,
) -> Result<(), Error<T::Error>>
where
    S: Storage + DatagramSlots + PendingCons + DedupSlots,
    T: DatagramIo,
{
    if meta.no_response.suppresses(Code::SERVICE_UNAVAILABLE) {
        return if meta.ty == Type::Confirmable {
            send_empty_ack(engine, io, meta.dest, meta.mid)
        } else {
            Ok(())
        };
    }
    // Count a pending attempt as unsent so dropping an async send is visible.
    Metrics::inc(&mut engine.metrics_mut().admission_refusal_unsent);
    let outcome = send_control_response(
        engine,
        io,
        ids,
        now_ms,
        meta,
        &Response::new(Code::SERVICE_UNAVAILABLE).max_age(1),
        oscore,
        dedup_closed,
    );
    if outcome.is_ok() {
        let metrics = engine.metrics_mut();
        metrics.admission_refusal_unsent = metrics.admission_refusal_unsent.wrapping_sub(1);
        Metrics::inc(&mut engine.metrics_mut().admission_refusal_sent);
    }
    outcome
}
