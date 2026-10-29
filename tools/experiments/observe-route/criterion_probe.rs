//! Private hosted cfg(test) child `crate::app::observe_route_probe`.
//!
//! Parent supplies Criterion =0.5.1 (default-features=false), OSCORE, and frozen
//! 176c337d350a4f601e4c2a02b9bdfdcaf0792a92 copies of App::poll and poll_engine
//! named poll_scan_oracle / poll_engine_scan_oracle. In those copies ONLY the
//! poll_engine entry and its due-notification Site call are renamed; that Site
//! call uses observe_source_scan_oracle, the original linear lookup. No Site
//! selector field, production instrumentation, or public API is required.
//!
//! Four processes: ROUND=1 baseline,candidate; ROUND=2 candidate,baseline.
//! Before those arms, run baseline/candidate with OBSERVE_ROUTE_FIXTURES_ONLY=1
//! and compare the exact eight sorted receipts. This mode returns before any
//! Criterion construction or metric-directory checks.
//! BINDING_PROBE_OUTPUT is the absolute per-round directory. Parent pins CPU0,
//! provides an external four-minute process watchdog, checks frozen source and
//! the eight identical OBSERVE_ROUTE_FIXTURE receipts before accepting samples.
//! This source also checks a cooperative four-minute deadline outside clocks.
//!
//! Criterion 0.5.1 iter_batched_ref / PerIteration uses a batch of exactly one:
//! setup (including RefCell borrowing) precedes measurement.start(); routine is
//! between start/end; output and input drop after end. Input drop verifies the
//! packet and authenticates/consumes it through the independent client App.
//! Warmup elapsed time includes setup/verification. Per-iteration timer and
//! symmetric wrapper/black_box overhead remain in the observations; no subtraction.
//! Parent verified src/bencher.rs:324-345 and src/lib.rs:233-237 at
//! https://raw.githubusercontent.com/bheisler/criterion.rs/0.5.1/src/bencher.rs
//! and https://raw.githubusercontent.com/bheisler/criterion.rs/0.5.1/src/lib.rs.
//!
//! One live fixed pair, one shared single-packet link; no fixture bank. Epochs
//! end after at most 4096 notifications INCLUDING prelude/calibration/warmup.
//! First notification is at virtual time zero, then exactly 3000 ms per NON.
//! Its actual confirmation deadline must be 86_400_000 ms; last-first is at
//! most 12_285_000 ms. No clock, sequence, replay, or shallow App-state reset:
//! deregister, collect, drain, destroy, then recreate with distinct epoch keys.
//! Fixed preludes use their own key domain and exactly 4096 notifications each.
//! Dynamic sampled counts need not have identical whole-arm wire hashes.
//!
//! Logical concrete pair+Rc allocation is asserted <=128 KiB. This is not an
//! emitted-stack or whole-memory claim. Use the ordinary test-thread stack;
//! no thread/stack override, native transport, growth, or capability expansion.
//! No performance gate or acceptance decision is implemented here.

use super::{App, AppStore, Call, Error, Request, Response, get};
use crate::message::{Code, ContentFormat, MessageId, ObserveTransmission, Type, decode};
use crate::oscore::{DeriveParams, OscoreHeader, SecurityContext};
use crate::resources::SmallDevice;
use crate::storage::{
    DatagramIo, DatagramTransport, Endpoint, Metrics, ObserveInterest, ObserveNotifyHold, SlotId,
    profiles,
};
use criterion::{BatchSize, Criterion, black_box};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell, RefMut},
    env,
    fmt::Write as _,
    mem::size_of,
    path::PathBuf,
    rc::Rc,
    string::String,
    time::{Duration, Instant},
};

const WIRE: usize = 1152;
const EPOCH_NOTIFICATIONS: u32 = 4096;
const STEP_MS: u64 = 3000;
const CONFIRM_MS: u64 = 86_400_000;
const LAST_FIRST_MS: u64 = 12_285_000;
const MAX_AGE: u32 = 86_400;
const MAX_ITERATIONS: u64 = 8_000_000;
const MAX_EPOCHS: u64 = 2048;
const MAX_SETUP_WORK: u64 = 80_000_000;
const HOST_LIMIT: Duration = Duration::from_secs(240);
const SERVER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
const CLIENT: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
const PAYLOAD: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const PATHS: [&str; 32] = [
    "r00", "r01", "r02", "r03", "r04", "r05", "r06", "r07", "r08", "r09", "r10", "r11", "r12",
    "r13", "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", "r22", "r23", "r24", "r25",
    "r26", "r27", "r28", "r29", "r30", "r31",
];
const NAMES: [&str; 8] = [
    "routes4-first-unprotected",
    "routes4-last-unprotected",
    "routes32-first-unprotected",
    "routes32-last-unprotected",
    "routes4-first-oscore",
    "routes4-last-oscore",
    "routes32-first-oscore",
    "routes32-last-oscore",
];

type Profile = profiles::WithObserve<profiles::Constrained, 1>;
type ProbeApp<const N: usize> = App<Profile, Io, N, false, AppStore<Profile, false>, 0, 1, 4, 1>;

std::thread_local! {
    static SOURCE_CALLS: Cell<u32> = const { Cell::new(0) };
}

fn representation() -> Response<'static> {
    Response::content(&PAYLOAD)
        .content_format(ContentFormat::OCTET_STREAM)
        .max_age(MAX_AGE)
}

fn read(_: Request<'_>) -> Response<'static> {
    representation().observe(0)
}

fn snapshot() -> Response<'static> {
    SOURCE_CALLS.with(|count| count.set(count.get().checked_add(1).expect("source count")));
    representation()
}

#[derive(Clone, Copy)]
struct Frame {
    bytes: [u8; WIRE],
    len: usize,
    from_server: bool,
}

#[derive(Default)]
struct Link {
    packet: Option<Frame>,
}

struct Io {
    link: Rc<RefCell<Link>>,
    server: bool,
    sends: u64,
    receives: u64,
}

impl DatagramTransport for Io {
    type Error = &'static str;
}

impl DatagramIo for Io {
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        let mut link = self.link.borrow_mut();
        if link
            .packet
            .as_ref()
            .is_none_or(|p| p.from_server == self.server)
        {
            return Ok(None);
        }
        let packet = link.packet.take().expect("checked packet");
        if packet.len > out.len() {
            return Err("short RX scratch");
        }
        out[..packet.len].copy_from_slice(&packet.bytes[..packet.len]);
        self.receives += 1;
        Ok(Some((
            packet.len,
            if self.server { CLIENT } else { SERVER },
        )))
    }

    fn send(&mut self, peer: Endpoint, wire: &[u8]) -> Result<usize, Self::Error> {
        if peer.physical() != if self.server { CLIENT } else { SERVER } {
            return Err("unexpected destination");
        }
        let mut link = self.link.borrow_mut();
        if link.packet.is_some() || wire.len() > WIRE {
            return Err("single packet link full or oversized");
        }
        let mut packet = Frame {
            bytes: [0; WIRE],
            len: wire.len(),
            from_server: self.server,
        };
        packet.bytes[..wire.len()].copy_from_slice(wire);
        link.packet = Some(packet);
        self.sends += 1;
        Ok(wire.len())
    }
}

// All limits and accounting run outside Criterion's measurement interval.
// A work unit is an explicit route/bind/key setup, prepare, signal, packet
// inspection/authentication, client/server setup poll, validation, or teardown.
// Every notification (including calibration and warmup) consumes an iteration.
struct Budget {
    start: Instant,
    iterations: Cell<u64>,
    verified: Cell<u64>,
    epochs: Cell<u64>,
    registrations: Cell<u64>,
    cancellations: Cell<u64>,
    setup_work: Cell<u64>,
    setup_polls: Cell<u64>,
}

impl Budget {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            iterations: Cell::new(0),
            verified: Cell::new(0),
            epochs: Cell::new(0),
            registrations: Cell::new(0),
            cancellations: Cell::new(0),
            setup_work: Cell::new(0),
            setup_polls: Cell::new(0),
        }
    }

    fn charge(&self, work: u64) {
        assert!(
            self.start.elapsed() < HOST_LIMIT,
            "STOP: four-minute host limit"
        );
        let total = self
            .setup_work
            .get()
            .checked_add(work)
            .expect("work overflow");
        assert!(total <= MAX_SETUP_WORK, "STOP: setup-work limit");
        self.setup_work.set(total);
    }

    fn iteration(&self) {
        self.charge(1);
        let total = self.iterations.get() + 1;
        assert!(total <= MAX_ITERATIONS, "STOP: iteration limit");
        self.iterations.set(total);
    }

    fn epoch(&self, routes: usize, protected: bool) {
        self.charge(routes as u64 + 3 + if protected { 2 } else { 0 });
        let total = self.epochs.get() + 1;
        assert!(total <= MAX_EPOCHS, "STOP: epoch limit");
        self.epochs.set(total);
    }

    fn setup_poll(&self) {
        self.charge(1);
        self.setup_polls.set(self.setup_polls.get() + 1);
    }
}

fn context(server: bool, cell: u8, phase: u8, epoch: u32) -> SecurityContext {
    // Injective test-only key material across (cell, prelude/sampling, epoch).
    // Separate processes intentionally reproduce the fixed functional prelude.
    let mut secret = *b"observe-route-v1";
    secret[8] = cell;
    secret[9] = phase;
    secret[10..14].copy_from_slice(&epoch.to_be_bytes());
    SecurityContext::derive(DeriveParams {
        master_secret: &secret,
        master_salt: &[],
        sender_id: if server { &[2] } else { &[1] },
        recipient_id: if server { &[1] } else { &[2] },
        id_context: &[],
    })
    .expect("derive epoch context")
}

fn quiet_metrics(metrics: Metrics) {
    assert_eq!(metrics.token_capacity_refused, 0);
    assert_eq!(metrics.admission_rx_refused, 0);
    assert_eq!(metrics.admission_tx_refused, 0);
    assert_eq!(metrics.admission_refusal_sent, 0);
    assert_eq!(metrics.admission_refusal_unsent, 0);
    assert_eq!(metrics.hard_rx_exhausted, 0);
    assert_eq!(metrics.hard_tx_exhausted, 0);
    assert_eq!(metrics.rx_error, 0);
    assert_eq!(metrics.tx_fail, 0);
    assert_eq!(metrics.con_retransmit, 0);
    assert_eq!(metrics.give_up, 0);
    assert_eq!(metrics.saturated, 0);
    assert_eq!(metrics.nstart_reject, 0);
    assert_eq!(metrics.block1_assemble, 0);
    assert_eq!(metrics.block2_assemble, 0);
    assert_eq!(metrics.empty_ack, 0);
    assert_eq!(metrics.empty_rst, 0);
}

fn empty_protocol<const N: usize>(app: &mut ProbeApp<N>, observers: usize) {
    let usage = app.resource_usage();
    assert_eq!(usage.storage.rx_datagram.occupied, 0, "queued/held RX");
    assert_eq!(usage.storage.tx_datagram.occupied, 0, "retained TX");
    assert_eq!(usage.storage.rx_datagram.capacity, 2);
    assert_eq!(usage.storage.tx_datagram.capacity, 2);
    assert_eq!(usage.storage.rx_body, None, "bodies/Q disabled");
    assert_eq!(usage.storage.tx_body, None, "bodies/Q disabled");
    assert_eq!(usage.storage.observe.capacity, 1);
    assert_eq!(usage.storage.observe.occupied, observers);
    assert_eq!(usage.storage.dedup.occupied, 0, "NON has no reply cache");
    assert_eq!(usage.storage.dedup.capacity, 4);
    assert_eq!(usage.deferred.capacity, 0);
    assert_eq!(usage.deferred.occupied, 0);
    assert_eq!(usage.client_requests.capacity, 1);
    assert_eq!(usage.client_replies.capacity, 1);
    assert_eq!(usage.client_cooldowns, 0);
    assert_eq!(usage.revoked_associations, 0);
    assert!(app.deferred.rows.is_empty());
    assert!(app.dedup_closed.is_none());
    assert!(matches!(
        app.oscore_checkpoint,
        super::oscore::CheckpointState::Volatile
    ));
    for index in 0..2 {
        let id = SlotId::from_index(index);
        assert!(app.engine.pending_con(id).is_none(), "pending CON/RTO");
        assert!(
            app.engine.exchange_entry(id).is_none(),
            "pending exchange/Q"
        );
    }
    quiet_metrics(app.metrics());
}

struct Pair<const N: usize> {
    server: ProbeApp<N>,
    client: ProbeApp<4>,
    link: Rc<RefCell<Link>>,
    call: Call,
    path: &'static str,
    protected: bool,
    now: u64,
    sequence: u32,
    last_mid: MessageId,
    before: Metrics,
    before_sends: u64,
    before_receives: u64,
    sender_sequence: u64,
    awaiting: bool,
}

const fn pair_bytes<const N: usize>() -> usize {
    size_of::<Pair<N>>() + size_of::<RefCell<Link>>() + 2 * size_of::<usize>()
}

const _: () = {
    assert!(pair_bytes::<4>() <= 128 * 1024);
    assert!(pair_bytes::<32>() <= 128 * 1024);
    assert!(STEP_MS == ObserveTransmission::NON_TIMEOUT_MS as u64);
    assert!(CONFIRM_MS == ObserveTransmission::CONFIRM_INTERVAL_MS);
    assert!(LAST_FIRST_MS == (EPOCH_NOTIFICATIONS as u64 - 1) * STEP_MS);
};

impl<const N: usize> Pair<N> {
    fn new(
        cell: usize,
        phase: u8,
        epoch: u32,
        budget: &Budget,
        receipt: Option<&mut Receipt>,
    ) -> Self {
        assert!(N == 4 || N == 32);
        budget.epoch(N, cell >= 4);
        let link = Rc::new(RefCell::new(Link::default()));
        let io = |server| Io {
            link: Rc::clone(&link),
            server,
            sends: 0,
            receives: 0,
        };
        let mut builder =
            App::with_resources(SmallDevice::new().observe_entries::<1>().routes::<N>())
                .deterministic_for_tests()
                .allow_unprotected();
        for path in &PATHS[..N] {
            builder
                .try_route(*path, get(read).observe(snapshot))
                .expect("populate exact Site");
        }
        let mut server: ProbeApp<N> = builder.bind(io(true)).expect("bind server");
        let mut client: ProbeApp<4> =
            App::with_resources(SmallDevice::new().observe_entries::<1>())
                .deterministic_for_tests()
                .allow_unprotected()
                .bind(io(false))
                .expect("bind verifier");
        let protected = cell >= 4;
        if protected {
            server
                .set_oscore(&mut Some(context(true, cell as u8, phase, epoch)))
                .expect("server key");
            client
                .set_oscore(&mut Some(context(false, cell as u8, phase, epoch)))
                .expect("verifier key");
        }
        let path = PATHS[if cell % 2 == 0 { 0 } else { N - 1 }];
        budget.charge(1);
        let call = client
            .get(path)
            .observe()
            .non()
            .to(SERVER)
            .send(0)
            .expect("register NON");
        budget.setup_poll();
        server.poll(0).expect("registration server poll");
        let mut pair = Self {
            server,
            client,
            link,
            call,
            path,
            protected,
            now: 0,
            sequence: 0,
            last_mid: MessageId::new(0),
            before: Metrics::ZERO,
            before_sends: 0,
            before_receives: 0,
            sender_sequence: 0,
            awaiting: false,
        };
        assert_eq!(pair.server.site.len(), N);
        assert!(pair.server.site.is_full());
        let frame = pair.consume(Some(0), false, budget);
        if let Some(receipt) = receipt {
            receipt.record(&frame, true);
        }
        let row = pair.interest();
        assert_eq!(row.seq(), 0);
        assert_eq!(
            row.confirm_due_ms(),
            None,
            "registration must not start a hold"
        );
        assert_eq!(row.notify_hold(), None);
        assert_eq!(row.resource(), pair.server.site.observe_resource(&[path]));
        assert_eq!(row.oscore().is_some(), protected);
        assert_eq!(pair.server.oscore().is_some(), protected);
        assert_eq!(pair.client.oscore().is_some(), protected);
        budget.registrations.set(budget.registrations.get() + 1);
        pair.idle();
        pair
    }

    fn interest(&self) -> ObserveInterest {
        self.server
            .engine
            .observe_interest(SlotId::from_index(0))
            .expect("one server observer")
    }

    fn idle(&mut self) {
        assert!(self.link.borrow().packet.is_none(), "link not drained");
        empty_protocol(&mut self.server, 1);
        empty_protocol(&mut self.client, 1);
        let server = self.server.resource_usage();
        let client = self.client.resource_usage();
        assert_eq!(server.client_requests.occupied, 0);
        assert_eq!(server.client_replies.occupied, 0);
        assert_eq!(client.client_requests.occupied, 1);
        assert_eq!(client.completed_replies, 0);
        assert_eq!(server.security.expect("OSCORE compiled").capacity, 1);
        assert_eq!(
            server.security.unwrap().occupied,
            usize::from(self.protected)
        );
        assert_eq!(
            client.security.unwrap().occupied,
            usize::from(self.protected)
        );
        assert_eq!(self.server.site.len(), N);
        let row = self.interest();
        assert!(!row.key().is_client());
        assert_eq!(row.token(), self.call.token());
        assert_eq!(row.endpoint(), CLIENT);
        assert_eq!(row.seq(), self.sequence);
        assert!(!row.is_pending());
        assert!(!row.is_deleted());
        assert!(!row.q_block2());
        assert_eq!(row.block2_max_szx(), None);
    }

    fn prepare(&mut self, budget: &Budget) {
        budget.iteration();
        assert!(!self.awaiting);
        assert!(
            self.sequence < EPOCH_NOTIFICATIONS,
            "STOP: epoch must be replaced"
        );
        self.idle();
        self.now = u64::from(self.sequence) * STEP_MS;
        let row = self.interest();
        assert!(!row.is_notify_held(self.now), "NON hold must have elapsed");
        assert!(!row.must_confirm(self.now));
        assert!(!row.is_lifetime_due(self.now));
        if self.sequence != 0 {
            assert_eq!(row.confirm_due_ms(), Some(CONFIRM_MS));
            assert!(self.now < row.confirm_due_ms().unwrap());
        }
        assert!(self.now <= LAST_FIRST_MS);
        assert!(
            self.client.take_response(self.call).is_none(),
            "previous notification collected"
        );
        budget.charge(1);
        assert_eq!(
            self.server.signal(&[self.path]),
            1,
            "exactly one signaled observer"
        );
        let row = self.interest();
        assert!(row.is_pending());
        assert_eq!(
            self.server.next_poll_at(self.now),
            Some(self.now),
            "already eligible"
        );
        SOURCE_CALLS.with(|count| count.set(0));
        self.before = self.server.metrics();
        self.before_sends = self.server.io.sends;
        self.before_receives = self.server.io.receives;
        self.sender_sequence = self.server.oscore().map_or(0, |ctx| ctx.sender_seq());
        self.awaiting = true;
    }

    fn consume(&mut self, sequence: Option<u32>, notification: bool, budget: &Budget) -> Frame {
        budget.charge(1);
        let frame = self.link.borrow().packet.expect("missing server packet");
        assert!(frame.from_server);
        let outer = decode(&frame.bytes[..frame.len]).expect("well-formed wire");
        assert_eq!(outer.ty(), Type::NonConfirmable, "unexpected CON/ACK/RST");
        assert_eq!(outer.token(), self.call.token());
        assert_eq!(
            outer.message_id().get(),
            self.last_mid.get() + 1,
            "exact deterministic MID"
        );
        assert_eq!(outer.oscore().is_some(), self.protected);
        let mut scratch = [0; WIRE];
        let inner = if self.protected {
            budget.charge(1);
            let ctx = self.client.oscore().expect("independent verifier context");
            let (wire, binding) = ctx
                .unprotect_bound_response_wire(&outer, &mut scratch)
                .expect("notification/response AEAD authentication");
            assert_eq!(ctx.lookup(self.call.token()), Some(binding));
            let header = OscoreHeader::parse(outer.oscore().unwrap()).expect("OSCORE header");
            if notification {
                let piv = header.piv.expect("notification has fresh PIV");
                assert_eq!(piv.seq(), self.sender_sequence);
                assert_eq!(
                    piv.seq(),
                    u64::from(sequence.expect("notification sequence")),
                    "registration used PIV zero; notifications advance by one"
                );
                ctx.notification_fresh(self.call.token(), Some(piv))
                    .expect("fresh notification");
            } else if sequence.is_some() {
                assert_eq!(header.piv.expect("NON registration fresh PIV").seq(), 0);
            } else {
                assert_eq!(
                    header.piv, None,
                    "one response under cancellation request binding"
                );
            }
            decode(wire).expect("authenticated inner")
        } else {
            outer
        };
        assert_eq!(inner.code(), Code::CONTENT);
        assert_eq!(inner.ty(), Type::NonConfirmable);
        assert_eq!(inner.token(), self.call.token());
        assert_eq!(inner.message_id(), outer.message_id());
        // Protected Inner Observe is empty. Ordering is the authenticated PIV,
        // whereas the server relation still advances its local 24-bit sequence.
        let wire_sequence = if self.protected {
            sequence.map(|_| 0)
        } else {
            sequence
        };
        assert_eq!(
            inner.observe().transpose().expect("Observe encoding"),
            wire_sequence
        );
        assert_eq!(inner.payload(), PAYLOAD);
        assert_eq!(
            inner.content_format(),
            Some(Ok(ContentFormat::OCTET_STREAM))
        );
        assert_eq!(inner.max_age(), Some(Ok(MAX_AGE)));
        assert!(inner.block1().is_none());
        assert!(inner.block2().is_none());
        budget.setup_poll();
        let receives = self.client.io.receives;
        let sends = self.client.io.sends;
        self.client.poll(self.now).expect("verifier App poll");
        assert_eq!(self.client.io.receives, receives + 1);
        assert_eq!(
            self.client.io.sends, sends,
            "NON generates no control packet"
        );
        assert!(
            self.link.borrow().packet.is_none(),
            "exactly one packet consumed"
        );
        budget.charge(1);
        let response = self
            .client
            .take_response(self.call)
            .expect("client admitted response")
            .expect("successful response");
        assert_eq!(response.code(), Code::CONTENT);
        assert_eq!(response.payload(), PAYLOAD);
        assert_eq!(response.payload_src_len(), PAYLOAD.len());
        assert!(!response.payload_truncated());
        assert!(response.body().is_none());
        assert_eq!(response.ty(), Some(Type::NonConfirmable));
        assert_eq!(response.token(), Some(self.call.token()));
        assert_eq!(response.message_id(), Some(outer.message_id()));
        assert_eq!(response.peer(), Some(SERVER));
        assert_eq!(response.observe_seq(), wire_sequence);
        assert_eq!(response.format(), Some(ContentFormat::OCTET_STREAM));
        assert_eq!(response.max_age_secs(), Some(MAX_AGE));
        if self.protected && notification {
            let piv = OscoreHeader::parse(outer.oscore().unwrap())
                .unwrap()
                .piv
                .unwrap();
            assert_eq!(
                self.client
                    .oscore()
                    .unwrap()
                    .notification_fresh(self.call.token(), Some(piv)),
                Err(crate::oscore::Error::Replay),
                "App committed authenticated notification admission"
            );
        }
        self.last_mid = outer.message_id();
        frame
    }

    fn finish(&mut self, budget: &Budget) -> Frame {
        budget.charge(1);
        assert!(self.awaiting);
        SOURCE_CALLS.with(|count| assert_eq!(count.get(), 1, "one due source call"));
        let mut expected = self.before;
        expected.progress += 1;
        expected.tx_ok += 1;
        expected.observe_notify += 1;
        assert_eq!(
            self.server.metrics(),
            expected,
            "only one complete notification poll"
        );
        assert_eq!(self.server.io.sends, self.before_sends + 1);
        assert_eq!(
            self.server.io.receives, self.before_receives,
            "timed poll had no ingress"
        );
        if self.protected {
            assert_eq!(
                self.server.oscore().unwrap().sender_seq(),
                self.sender_sequence + 1
            );
        }
        let next = self.interest().seq();
        assert_eq!(next, self.sequence + 1, "actual server sequence");
        assert_eq!((next.wrapping_sub(self.sequence)) & 0x00ff_ffff, 1);
        let frame = self.consume(Some(next), true, budget);
        self.sequence = next;
        self.awaiting = false;
        let row = self.interest();
        assert_eq!(
            row.confirm_due_ms(),
            Some(CONFIRM_MS),
            "actual first-NON deadline"
        );
        assert!(self.now < row.confirm_due_ms().unwrap());
        assert_eq!(
            row.notify_hold(),
            Some(ObserveNotifyHold::non(self.now, self.last_mid))
        );
        budget.verified.set(budget.verified.get() + 1);
        self.idle();
        frame
    }

    fn close(&mut self, budget: &Budget) -> Frame {
        budget.charge(1);
        assert!(!self.awaiting);
        self.idle();
        // A real cancellation after the final NON hold, using the retained Call.
        self.now = u64::from(self.sequence) * STEP_MS;
        assert!(!self.interest().is_notify_held(self.now));
        assert_eq!(
            self.client
                .get(self.path)
                .deregister_call(self.call)
                .non()
                .to(SERVER)
                .send(self.now)
                .expect("real NON deregistration"),
            self.call
        );
        budget.setup_poll();
        self.server.poll(self.now).expect("server deregistration");
        let frame = self.consume(None, false, budget);
        assert_eq!(self.server.metrics().observe_cancel, 1);
        budget.setup_poll();
        self.server.poll(self.now).expect("server drain");
        budget.setup_poll();
        self.client.poll(self.now).expect("client drain");
        empty_protocol(&mut self.server, 0);
        empty_protocol(&mut self.client, 0);
        for usage in [self.server.resource_usage(), self.client.resource_usage()] {
            assert_eq!(usage.client_requests.occupied, 0);
            assert_eq!(usage.client_replies.occupied, 0);
        }
        assert!(self.link.borrow().packet.is_none());
        assert!(!super::oscore::has_live_requests(&self.server.oscore));
        assert!(!super::oscore::has_live_requests(&self.client.oscore));
        budget.cancellations.set(budget.cancellations.get() + 1);
        frame
    }
}

// Result validation belongs to Criterion's untimed output drop, including errors
// returned after a send. Packet success alone must not hide a poll error.
struct PollResult(Result<(), Error<&'static str>>);

impl Drop for PollResult {
    fn drop(&mut self) {
        assert!(self.0.is_ok(), "poll failed: {:?}", self.0);
    }
}

#[inline(never)]
fn candidate_poll<const N: usize>(server: &mut ProbeApp<N>, now: u64) -> PollResult {
    PollResult(server.poll(now))
}

#[inline(never)]
fn baseline_poll<const N: usize>(server: &mut ProbeApp<N>, now: u64) -> PollResult {
    PollResult(server.poll_scan_oracle(now))
}

// All server output is hashed outside clocks, with an unambiguous big-endian
// u64 length before every complete datagram. Four representative packets also
// appear in full: registration, first/last notification, cancellation. No wire
// bank is retained. SHA-256 already belongs to the required OSCORE feature.
struct Receipt {
    wire: String,
    digest: Sha256,
    packets: u64,
    bytes: u64,
}

impl Receipt {
    fn new() -> Self {
        Self {
            wire: String::new(),
            digest: Sha256::new(),
            packets: 0,
            bytes: 0,
        }
    }

    fn record(&mut self, frame: &Frame, include_wire: bool) {
        self.digest.update((frame.len as u64).to_be_bytes());
        self.digest.update(&frame.bytes[..frame.len]);
        self.packets += 1;
        self.bytes += frame.len as u64;
        if include_wire {
            write!(&mut self.wire, "{}:", frame.len).unwrap();
            for byte in &frame.bytes[..frame.len] {
                write!(&mut self.wire, "{byte:02x}").unwrap();
            }
            self.wire.push(';');
        }
    }

    fn finish(mut self) -> String {
        assert_eq!(self.packets, u64::from(EPOCH_NOTIFICATIONS) + 2);
        write!(
            &mut self.wire,
            "packets={} bytes={} sha256=",
            self.packets, self.bytes
        )
        .unwrap();
        for byte in self.digest.finalize() {
            write!(&mut self.wire, "{byte:02x}").unwrap();
        }
        self.wire
    }
}

fn prelude<const N: usize, const CANDIDATE: bool>(cell: usize, budget: &Budget) {
    let mut receipt = Receipt::new();
    let mut pair = Pair::<N>::new(cell, 0, 0, budget, Some(&mut receipt));
    for index in 0..EPOCH_NOTIFICATIONS {
        pair.prepare(budget);
        if CANDIDATE {
            drop(candidate_poll(&mut pair.server, pair.now));
        } else {
            drop(baseline_poll(&mut pair.server, pair.now));
        }
        let wire = pair.finish(budget);
        receipt.record(&wire, index == 0 || index == EPOCH_NOTIFICATIONS - 1);
    }
    assert_eq!(pair.now, LAST_FIRST_MS);
    receipt.record(&pair.close(budget), true);
    std::println!("OBSERVE_ROUTE_FIXTURE {} {}", NAMES[cell], receipt.finish());
}

struct Fixture<'a, const N: usize> {
    pair: Option<Pair<N>>,
    cell: usize,
    epoch: u32,
    budget: &'a Budget,
}

impl<const N: usize> Fixture<'_, N> {
    fn prepare(&mut self) {
        if self
            .pair
            .as_ref()
            .is_some_and(|pair| pair.sequence == EPOCH_NOTIFICATIONS)
        {
            self.close();
            self.epoch = self.epoch.checked_add(1).expect("epoch counter");
        }
        if self.pair.is_none() {
            self.pair = Some(Pair::new(self.cell, 1, self.epoch, self.budget, None));
        }
        self.pair.as_mut().unwrap().prepare(self.budget);
    }

    fn close(&mut self) {
        if let Some(mut pair) = self.pair.take() {
            pair.close(self.budget);
            // Drop this full owner before binding its successor. No cloned App,
            // retained fixture bank, key reuse, or sequence/reset shortcut.
            drop(pair);
        }
    }
}

struct Prepared<'a, const N: usize> {
    pair: RefMut<'a, Pair<N>>,
    budget: &'a Budget,
}

impl<const N: usize> Drop for Prepared<'_, N> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.pair.finish(self.budget);
        }
    }
}

fn measure<const N: usize, const CANDIDATE: bool>(
    criterion: &mut Criterion,
    cell: usize,
    budget: &Budget,
) {
    let fixture = RefCell::new(Fixture::<N> {
        pair: None,
        cell,
        epoch: 0,
        budget,
    });
    // Selection is outside both measurement closures. Borrowing, all setup,
    // source-count reset, and all eligibility assertions precede the clock.
    if CANDIDATE {
        criterion.bench_function(NAMES[cell], |b| {
            b.iter_batched_ref(
                || {
                    let mut fixture = fixture.borrow_mut();
                    fixture.prepare();
                    Prepared {
                        pair: RefMut::map(fixture, |fixture| fixture.pair.as_mut().unwrap()),
                        budget,
                    }
                },
                |input| {
                    let pair = &mut *input.pair;
                    candidate_poll(black_box(&mut pair.server), black_box(pair.now))
                },
                BatchSize::PerIteration,
            );
        });
    } else {
        criterion.bench_function(NAMES[cell], |b| {
            b.iter_batched_ref(
                || {
                    let mut fixture = fixture.borrow_mut();
                    fixture.prepare();
                    Prepared {
                        pair: RefMut::map(fixture, |fixture| fixture.pair.as_mut().unwrap()),
                        budget,
                    }
                },
                |input| {
                    let pair = &mut *input.pair;
                    baseline_poll(black_box(&mut pair.server), black_box(pair.now))
                },
                BatchSize::PerIteration,
            );
        });
    }
    fixture.borrow_mut().close();
    budget.charge(1);
}

fn preludes<const CANDIDATE: bool>(budget: &Budget) {
    // All eight deterministic full-entry preludes finish before ANY sampling.
    prelude::<4, CANDIDATE>(0, budget);
    prelude::<4, CANDIDATE>(1, budget);
    prelude::<32, CANDIDATE>(2, budget);
    prelude::<32, CANDIDATE>(3, budget);
    prelude::<4, CANDIDATE>(4, budget);
    prelude::<4, CANDIDATE>(5, budget);
    prelude::<32, CANDIDATE>(6, budget);
    prelude::<32, CANDIDATE>(7, budget);
}

fn run<const CANDIDATE: bool>(criterion: &mut Criterion, budget: &Budget) {
    preludes::<CANDIDATE>(budget);
    measure::<4, CANDIDATE>(criterion, 0, budget);
    measure::<4, CANDIDATE>(criterion, 1, budget);
    measure::<32, CANDIDATE>(criterion, 2, budget);
    measure::<32, CANDIDATE>(criterion, 3, budget);
    measure::<4, CANDIDATE>(criterion, 4, budget);
    measure::<4, CANDIDATE>(criterion, 5, budget);
    measure::<32, CANDIDATE>(criterion, 6, budget);
    measure::<32, CANDIDATE>(criterion, 7, budget);
}

#[test]
#[ignore = "private hosted Criterion; requires ROUND, ARM, BINDING_PROBE_OUTPUT and CPU0 watchdog runner"]
fn observe_route_criterion_probe() {
    let budget = Budget::new();
    let round = match env::var("ROUND").expect("ROUND=1|2").as_str() {
        "1" => 1,
        "2" => 2,
        _ => panic!("exactly two rounds: ROUND=1|2"),
    };
    let candidate = match env::var("ARM").expect("ARM=baseline|candidate").as_str() {
        "baseline" => false,
        "candidate" => true,
        _ => panic!("ARM=baseline|candidate"),
    };
    let output = PathBuf::from(env::var_os("BINDING_PROBE_OUTPUT").expect("BINDING_PROBE_OUTPUT"));
    assert!(
        output.is_absolute(),
        "round output directory must be absolute"
    );
    let first = (round == 1 && !candidate) || (round == 2 && candidate);
    let fixtures_only = match env::var("OBSERVE_ROUTE_FIXTURES_ONLY") {
        Err(env::VarError::NotPresent) => false,
        Ok(value) if value == "1" => true,
        _ => panic!("OBSERVE_ROUTE_FIXTURES_ONLY must be absent or 1"),
    };
    std::println!(
        "OBSERVE_ROUTE_LAYOUT pair4={} pair32={} bound=131072 fixed SmallDevice/Constrained Observe=1 Calls=1 Deferred=0 Requests=4 Associations=1 blockwise=false payload=32",
        pair_bytes::<4>(),
        pair_bytes::<32>()
    );
    if fixtures_only {
        if candidate {
            preludes::<true>(&budget);
        } else {
            preludes::<false>(&budget);
        }
        assert_eq!(budget.iterations.get(), 8 * u64::from(EPOCH_NOTIFICATIONS));
        report(&budget);
        return;
    }
    for name in NAMES {
        if first {
            assert!(
                !output.join(name).exists(),
                "first arm requires fresh metric directories"
            );
        } else {
            assert!(
                output.join(name).join("first/estimates.json").is_file(),
                "run first arm first"
            );
        }
    }
    let criterion = Criterion::default()
        .without_plots()
        .with_output_color(false)
        .sample_size(30)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500))
        .nresamples(10_000)
        .confidence_level(0.95)
        .noise_threshold(0.02)
        .output_directory(&output);
    let mut criterion = if first {
        criterion.save_baseline("first".into())
    } else {
        criterion.retain_baseline("first".into(), true)
    };
    if candidate {
        run::<true>(&mut criterion, &budget);
    } else {
        run::<false>(&mut criterion, &budget);
    }
    criterion.final_summary();
    report(&budget);
}

fn report(budget: &Budget) {
    budget.charge(1);
    assert_eq!(budget.iterations.get(), budget.verified.get());
    assert_eq!(budget.epochs.get(), budget.registrations.get());
    assert_eq!(budget.epochs.get(), budget.cancellations.get());
    std::println!(
        "OBSERVE_ROUTE_WORK iterations={} verified={} epochs={} registrations={} cancellations={} setup_polls={} setup_work={} elapsed_ms={} limits_iterations={} epochs={} setup_work={} host_ms=240000",
        budget.iterations.get(),
        budget.verified.get(),
        budget.epochs.get(),
        budget.registrations.get(),
        budget.cancellations.get(),
        budget.setup_polls.get(),
        budget.setup_work.get(),
        budget.start.elapsed().as_millis(),
        MAX_ITERATIONS,
        MAX_EPOCHS,
        MAX_SETUP_WORK
    );
}
