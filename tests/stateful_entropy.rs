//! Caller-owned entropy integration; deterministic bytes here are test fixtures only.
use coaptic::app::{Error, IdentityError};
use coaptic::storage::DatagramIo;
use coaptic::{App, BuildError, Endpoint, Request, Response, get, resources::SmallDevice};
use core::cell::Cell;

const PEER: Endpoint = Endpoint::v4([192, 0, 2, 2], 5683);
struct LoopIo {
    wire: [u8; 1152],
    len: usize,
    sends: usize,
    receives: usize,
}
impl Default for LoopIo {
    fn default() -> Self {
        Self {
            wire: [0; 1152],
            len: 0,
            sends: 0,
            receives: 0,
        }
    }
}
impl DatagramIo for LoopIo {
    type Error = ();
    fn recv(&mut self, out: &mut [u8]) -> Result<Option<(usize, Endpoint)>, ()> {
        self.receives += 1;
        if self.len == 0 {
            return Ok(None);
        }
        let n = self.len;
        out[..n].copy_from_slice(&self.wire[..n]);
        self.len = 0;
        Ok(Some((n, PEER)))
    }
    fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, ()> {
        self.sends += 1;
        self.len = bytes.len();
        self.wire[..self.len].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}

#[test]
fn stateful_entropy_refuses_missing_security_and_second_source_before_callback() {
    let calls = Cell::new(0);
    let mut random = |bytes: &mut [u8]| {
        calls.set(calls.get() + 1);
        bytes.fill(1);
        true
    };
    let result =
        App::with_resources(SmallDevice::new()).bind_with_entropy(LoopIo::default(), &mut random);
    assert_eq!(result.err(), Some(BuildError::SecurityRequired));
    for deterministic in [false, true] {
        let builder = App::with_resources(SmallDevice::new()).allow_plaintext();
        let builder = if deterministic {
            builder.deterministic_for_tests()
        } else {
            builder.randomness(|_| panic!("old source must not be sampled"))
        };
        assert_eq!(
            builder
                .bind_with_entropy(LoopIo::default(), &mut random)
                .err(),
            Some(BuildError::RandomnessAlreadyConfigured)
        );
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn stateful_entropy_bind_refusal_releases_borrow_and_allows_retry() {
    let ready = Cell::new(false);
    let mut samples = 0;
    let mut random = |bytes: &mut [u8]| {
        samples += 1;
        bytes.fill(1);
        ready.get()
    };
    let builder = || App::with_resources(SmallDevice::new()).allow_plaintext();
    assert_eq!(
        builder()
            .bind_with_entropy(LoopIo::default(), &mut random)
            .err(),
        Some(BuildError::RandomnessUnavailable)
    );
    ready.set(true);
    let app = builder()
        .bind_with_entropy(LoopIo::default(), &mut random)
        .unwrap();
    assert_eq!(app.transport().transport().sends, 0);
    assert_eq!(app.transport().transport().receives, 0);
    assert_eq!(samples, 2);
}

#[test]
fn stateful_entropy_token_and_jitter_refusal_preserve_capacity_and_recover() {
    let failure = Cell::new(0usize);
    let jitter_reads = Cell::new(0);
    let mut tokens = 0u8;
    let mut random = |bytes: &mut [u8]| {
        if bytes.len() == 4 {
            jitter_reads.set(jitter_reads.get() + 1);
        }
        if failure.get() == bytes.len() {
            return false;
        }
        if bytes.len() == 8 {
            tokens += 1;
            bytes.fill(tokens);
        } else {
            bytes.fill(0);
        }
        if failure.get() == 99 && bytes.len() == 4 {
            bytes.fill(255);
        }
        true
    };
    let mut app = App::with_resources(SmallDevice::new())
        .allow_plaintext()
        .route("hello", get(|_: Request<'_>| Response::content(b"hello")))
        .bind_with_entropy(LoopIo::default(), &mut random)
        .unwrap();
    for fail in [8, 4, 99] {
        failure.set(fail);
        for _ in 0..app.client_capacity() + 1 {
            let before = jitter_reads.get();
            assert_eq!(
                app.get("hello").to(PEER).send(0),
                Err(Error::Identity(IdentityError::RandomnessUnavailable))
            );
            assert_eq!(
                jitter_reads.get() - before,
                if fail == 8 {
                    0
                } else if fail == 99 {
                    8
                } else {
                    1
                }
            );
            assert_eq!(app.transport().transport().sends, 0);
            assert_eq!(app.transport().transport().receives, 0);
            assert_eq!(app.engine_mut().tx_occupied(), 0);
        }
    }
    failure.set(0);
    let call = app.get("hello").to(PEER).send(0).unwrap();
    app.poll(0).unwrap();
    app.poll(1).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap().payload(),
        b"hello"
    );
    assert_eq!(app.engine_mut().rx_occupied(), 0);
}

#[test]
fn stateful_entropy_wrapper_forwards_io_and_default_hook_refuses() {
    let mut io = LoopIo::default();
    assert!(!io.fill_random(&mut [0; 2]));
    let mut counter = 0u8;
    let mut wrapped = coaptic::platform::WithEntropy::new(io, |bytes: &mut [u8]| {
        counter += 1;
        bytes.fill(counter);
        true
    });
    let mut bytes = [0; 4];
    assert!(wrapped.fill_random(&mut bytes));
    assert_eq!(bytes, [1; 4]);
    wrapped.send(PEER, b"data").unwrap();
    assert_eq!(wrapped.recv(&mut bytes).unwrap(), Some((4, PEER)));
    assert_eq!(&bytes, b"data");
    let (io, _) = wrapped.into_parts();
    assert_eq!(io.sends, 1);
    assert_eq!(counter, 1);
}

#[cfg(feature = "alloc")]
#[test]
fn stateful_entropy_allocated_configuration_is_bounded_and_recovers() {
    let resources = SmallDevice::new().allocated(64 * 1024);
    let required = resources.required_heap_bytes().unwrap();
    assert_eq!(
        App::with_resources(SmallDevice::new().allocated(required - 1))
            .allow_plaintext()
            .bind_with_entropy(LoopIo::default(), |b: &mut [u8]| {
                b.fill(0);
                true
            })
            .err(),
        Some(BuildError::HeapBudgetExceeded {
            required,
            limit: required - 1
        })
    );
    let mut counter = 0u8;
    let mut app = App::with_resources(SmallDevice::new().allocated(required))
        .allow_plaintext()
        .route("hello", get(|_: Request<'_>| Response::content(b"hello")))
        .bind_with_entropy(LoopIo::default(), |b: &mut [u8]| {
            counter += 1;
            b.fill(counter);
            true
        })
        .unwrap();
    let call = app.get("hello").to(PEER).send(0).unwrap();
    app.poll(0).unwrap();
    app.poll(1).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap().payload(),
        b"hello"
    );
}

#[cfg(feature = "oscore")]
#[test]
fn stateful_entropy_protected_refusal_and_authenticated_recovery() {
    use coaptic::message::{Code, Message, MessageId, Type, decode};
    use coaptic::oscore::{DeriveParams, SecurityContext};
    let context = |sender, recipient| {
        SecurityContext::derive(DeriveParams {
            master_secret: &[7; 16],
            master_salt: &[],
            sender_id: sender,
            recipient_id: recipient,
            id_context: &[],
        })
        .unwrap()
    };
    let fail = Cell::new(false);
    let mut counter = 0u8;
    let mut app = App::with_resources(SmallDevice::new())
        .oscore(context(&[1], &[2]))
        .bind_with_entropy(LoopIo::default(), |bytes: &mut [u8]| {
            if fail.get() {
                return false;
            }
            counter += 1;
            bytes.fill(counter);
            true
        })
        .unwrap();
    fail.set(true);
    assert_eq!(
        app.get("hello").to(PEER).send(0),
        Err(Error::Identity(IdentityError::RandomnessUnavailable))
    );
    assert_eq!(app.transport().transport().sends, 0);
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    fail.set(false);
    let call = app.get("hello").to(PEER).send(1).unwrap();
    let io = app.transport().transport();
    let outer = decode(&io.wire[..io.len]).unwrap();
    assert!(outer.oscore().is_some());
    let mid = outer.message_id();
    let mut peer = context(&[2], &[1]);
    let mut scratch = [0; 1152];
    let (inner, reference) = peer.unprotect_request(&outer, &mut scratch).unwrap();
    assert_eq!(inner.code(), Code::GET);
    let response = Message::new(
        Type::Acknowledgement,
        Code::CONTENT,
        MessageId::new(mid.get()),
    )
    .with_token(inner.token())
    .with_payload(b"secure");
    let io = app.transport_mut().transport_mut();
    io.len = peer
        .protect_response(&response, reference, &mut io.wire)
        .unwrap();
    app.poll(2).unwrap();
    assert_eq!(
        app.take_response(call).unwrap().unwrap().payload(),
        b"secure"
    );
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}

#[test]
fn stateful_entropy_duplicate_tokens_have_a_finite_budget_and_preserve_live_call() {
    let token_byte = Cell::new(1u8);
    let token_reads = Cell::new(0);
    let mut app = App::profile::<coaptic::profiles::Constrained>()
        .block_wise::<false>()
        .allow_plaintext()
        .bind_with_entropy(LoopIo::default(), |bytes: &mut [u8]| {
            if bytes.len() == 8 {
                token_reads.set(token_reads.get() + 1);
                bytes.fill(token_byte.get());
            } else {
                bytes.fill(0);
            }
            true
        })
        .unwrap();
    let first = app.get("hello").to(PEER).send(0).unwrap();
    let before = token_reads.get();
    assert_eq!(
        app.get("hello").to(PEER).send(0),
        Err(Error::Identity(IdentityError::TokenExhausted))
    );
    assert_eq!(token_reads.get() - before, 8);
    assert_eq!(app.transport().transport().sends, 1);
    assert!(app.cancel(first));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
    token_byte.set(2);
    let next = app.get("hello").to(PEER).send(1).unwrap();
    assert_ne!(first.token(), next.token());
    assert!(app.cancel(next));
    assert_eq!(app.engine_mut().tx_occupied(), 0);
}
