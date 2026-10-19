use coaptic::app::diagnostics::{Reason, Recorder, Snapshot};
use coaptic::app::{AppStorage, AsyncCallError};
use coaptic::storage::{DatagramIo, DatagramTransport, Endpoint};
use coaptic::{App, CallFailure, Error, profiles};

#[derive(Default)]
struct Io {
    fail: bool,
    sends: usize,
}
impl DatagramTransport for Io {
    type Error = ();
}
impl DatagramIo for Io {
    fn recv(&mut self, _: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        Ok(None)
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        if self.fail {
            return Err(());
        }
        self.sends += 1;
        Ok(bytes.len())
    }
}
const PEER: Endpoint = Endpoint::v4([127, 0, 0, 1], 5683);

fn exercise<S: AppStorage<Token = coaptic::message::Token>>(
    mut app: App<profiles::Default, Io, 8, false, S, 0, 1>,
) {
    let mut recorder = Recorder::default();
    let idle = app.diagnostic_snapshot(0);
    assert_eq!(idle.usage.client_requests.capacity, 1);
    assert_eq!(idle.usage.storage.rx_body, None);
    assert_eq!(idle.next_poll_ms, None);
    recorder.sample(&idle);
    let call = app.get("x").to(PEER).non().deadline(10).send(0).unwrap();
    let full = app.diagnostic_snapshot(1);
    assert_eq!(full.at_ms, 1);
    assert_eq!(full.usage.client_requests.occupied, 1);
    assert_eq!(full.usage.client_replies.occupied, 0);
    assert_eq!(full.next_timeout_ms, Some(10));
    assert_eq!(full.next_poll_ms, Some(10));
    assert_eq!(app.diagnostic_snapshot(1), full);
    assert_eq!(app.transport().sends, 1);
    recorder.sample(&full);
    let error = app.get("other").to(PEER).non().send(1).unwrap_err();
    recorder.record(Reason::from_error(&error));
    assert_eq!(recorder.count(Reason::Capacity), 1);
    app.poll(10).unwrap();
    let completed = app.diagnostic_snapshot(10);
    assert_eq!(completed.usage.client_replies.occupied, 1);
    assert_eq!(completed.usage.completed_replies, 1);
    let failure = app.take_response(call).unwrap().unwrap_err();
    assert_eq!(Reason::from_call_failure(&failure), Reason::Deadline);
    recorder.record(Reason::from_call_failure(&failure));
    let recovered = app.diagnostic_snapshot(11);
    assert_eq!(recovered.usage.client_requests.occupied, 0);
    assert_eq!(recovered.usage.client_replies.occupied, 0);
    recorder.sample(&recovered);
    assert_eq!(recorder.samples(), 3);
    assert_eq!(
        recorder.sampled_maxima().unwrap().client_requests.occupied,
        1
    );

    let mut slots = Vec::new();
    while let Some(slot) = app.engine_mut().acquire_tx() {
        slots.push(slot);
    }
    let physical_full = app.diagnostic_snapshot(11);
    assert_eq!(
        physical_full.usage.storage.tx_datagram.occupied,
        physical_full.usage.storage.tx_datagram.capacity
    );
    assert_eq!(
        Reason::from_error(&app.get("x").to(PEER).non().send(11).unwrap_err()),
        Reason::Capacity
    );
    for slot in slots {
        app.engine_mut().release_tx(slot).unwrap();
    }

    let fresh = app.get("x").to(PEER).non().send(12).unwrap();
    assert!(app.cancel(fresh));
    recorder.record(Reason::from_call_failure(&CallFailure::Cancelled));
    let before = app.diagnostic_snapshot(12);
    recorder.reset();
    assert_eq!(recorder, Recorder::default());
    assert_eq!(app.diagnostic_snapshot(12), before);
    app.reset_metrics();
    assert_eq!(app.metrics(), coaptic::Metrics::ZERO);
    assert_eq!(app.diagnostic_snapshot(12).usage, before.usage);
    assert_eq!(before.usage.completed_replies, 1);
    assert_eq!(
        app.take_response(fresh).unwrap().unwrap_err(),
        CallFailure::Cancelled
    );

    app.transport_mut().fail = true;
    let error = app.get("x").to(PEER).non().send(13).unwrap_err();
    assert_eq!(Reason::from_error(&error), Reason::Transport);
    assert_eq!(
        app.diagnostic_snapshot(13).usage.client_requests.occupied,
        0
    );
    app.transport_mut().fail = false;
    let call = app.get("x").to(PEER).non().send(14).unwrap();
    assert!(app.cancel(call));
    assert!(core::mem::size_of::<Recorder>() <= 512);
    assert!(core::mem::size_of::<Snapshot>() <= 768);
}

#[test]
fn fixed_diagnostics_full_refusal_reset_drop_and_recovery() {
    exercise(
        App::profile::<profiles::Default>()
            .block_wise::<false>()
            .client_calls::<1>()
            .deterministic_for_tests()
            .allow_unprotected()
            .bind(Io::default())
            .unwrap(),
    );
}

#[cfg(feature = "alloc")]
#[test]
fn allocated_diagnostics_full_refusal_reset_drop_and_recovery() {
    let mut capacities = coaptic::storage::Capacities::from_profile::<profiles::Default>();
    capacities.rx_body_slots = None;
    capacities.rx_body_bytes = None;
    capacities.tx_body_slots = None;
    capacities.tx_body_bytes = None;
    exercise(
        App::profile::<profiles::Default>()
            .block_wise::<false>()
            .client_calls::<1>()
            .deterministic_for_tests()
            .allow_unprotected()
            .bind_alloc(Io::default(), capacities)
            .unwrap(),
    );
}

#[test]
fn surfaced_errors_keep_security_delivery_and_protocol_categories_distinct() {
    assert_eq!(
        Reason::from_async_io(&coaptic::app::AsyncIoError::<(), ()>::ClockAfterSend(())),
        Reason::DeliveryUncertain
    );
    assert_eq!(
        Reason::from_async_io(&coaptic::app::AsyncIoError::<(), ()>::ClockBeforeSend(())),
        Reason::LocalState
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::DeadlineElapsed),
        Reason::Deadline
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::CommitAfterSend),
        Reason::DeliveryUncertain
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::Backoff { until_ms: 1 }),
        Reason::Backoff
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::FetchContentFormatRequired),
        Reason::Protocol
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::ObserveMethodUnsupported),
        Reason::Unsupported
    );
    assert_eq!(
        Reason::from_error(&Error::<()>::DeferredUnknown),
        Reason::LocalState
    );
    assert_eq!(
        Reason::from_async_call(&AsyncCallError::<(), ()>::ClockAfterSend(())),
        Reason::DeliveryUncertain
    );
    assert_eq!(
        Reason::from_async_call(&AsyncCallError::<(), ()>::ProgressRequired { at_ms: 0 }),
        Reason::ProgressRequired
    );
    #[cfg(feature = "oscore")]
    {
        assert_eq!(
            Reason::from_error(&Error::<()>::OscoreCheckpointFailed),
            Reason::Security
        );
        assert_eq!(
            Reason::from_call_failure(&CallFailure::AssociationRevoked),
            Reason::Security
        );
    }
}
