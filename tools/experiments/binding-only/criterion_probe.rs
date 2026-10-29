//! Ephemeral cfg(test) child of oscore::protect; never a production module.
//! Parent supplies Criterion =0.5.1 (default-features=false), response_bindings,
//! the actual bound-open entrypoint, and the exact 869d435 observe_request body
//! under the test-only name baseline_observe_request. No argument parsing here.
//! Invoke once per arm: ROUND=1 baseline,candidate; ROUND=2 candidate,baseline.
//! BINDING_PROBE_OUTPUT is the exact round directory, shared by its two arms.
//! Repeated immutable opens measure authentication, not replay admission.

extern crate std;

use super::*;
use crate::message::{BlockValue, decode};
use crate::oscore::DeriveParams;
use criterion::Criterion;
use std::{env, hint::black_box, path::PathBuf, time::Duration};

type Context = BoundedSecurityContext<4>;
type Bindings = Option<(RequestRef, Option<RequestRef>)>;
const WIRE: usize = 256;
const NAMES: [&str; 4] = [
    "normal-entry",
    "fallback-entry",
    "normal-lookup",
    "first-slot-entry",
];

// Preserve the two baseline searches without an intervening optimization barrier.
fn baseline_bindings(ctx: &Context, token: Token) -> Bindings {
    let request = ctx.lookup(token)?;
    let original = ctx
        .baseline_observe_request(token)
        .filter(|old| *old != request);
    Some((request, original))
}

struct Fixture {
    ctx: Context,
    tokens: [Token; 4],
    wires: [[u8; WIRE]; 4],
    lengths: [usize; 4],
    expected: [[u8; WIRE]; 4],
    expected_lengths: [usize; 4],
    accepted: [Option<RequestRef>; 4],
    bindings: [(RequestRef, Option<RequestRef>); 4],
}

fn pair(kind: u8) -> (Context, Context) {
    // Separate public fixture keys by kind; deterministically identical in every arm.
    // Each distinct encryption within a fixture uses a fresh request/server nonce.
    let label = [b'p', b'r', b'o', b'b', b'e', kind];
    let derive = |sender_id, recipient_id| {
        Context::derive(DeriveParams {
            master_secret: &[0x51; 16],
            master_salt: &[0xa7; 8],
            sender_id,
            recipient_id,
            id_context: &label,
        })
        .unwrap()
    };
    (derive(&[1], &[2]), derive(&[2], &[1]))
}

fn tokens() -> [Token; 4] {
    // Same seven-byte prefix makes all four full-width token comparisons useful.
    core::array::from_fn(|i| Token::new(&[9, 8, 7, 6, 5, 4, 3, i as u8]).unwrap())
}

fn issue(
    client: &mut Context,
    server: &mut Context,
    token: Token,
    mid: u16,
    options: &[Opt<'_>],
) -> RequestRef {
    let request = Message::new(Type::Confirmable, Code::GET, MessageId::new(mid))
        .with_token(token)
        .with_options(options);
    let mut wire = [0; WIRE];
    let mut scratch = [0; WIRE];
    let n = client.protect_request(&request, &mut wire).unwrap();
    let (_, reference) = server
        .unprotect_request(&decode(&wire[..n]).unwrap(), &mut scratch)
        .unwrap();
    assert_eq!(client.lookup(token), Some(reference));
    reference
}

fn assert_full(ctx: &mut Context, reference: RequestRef) {
    assert_eq!(
        ctx.remember(Token::new(b"missing!").unwrap(), reference),
        Err(Error::Saturated)
    );
}

fn normal() -> Fixture {
    let (mut client, mut server) = pair(b'n');
    let tokens = tokens();
    let refs = core::array::from_fn::<_, 4, _>(|i| {
        issue(&mut client, &mut server, tokens[i], 10 + i as u16, &[])
    });
    assert_full(&mut client, refs[0]);
    let mut fixture = Fixture {
        ctx: client,
        tokens,
        wires: [[0; WIRE]; 4],
        lengths: [0; 4],
        expected: [[0; WIRE]; 4],
        expected_lengths: [0; 4],
        accepted: refs.map(Some),
        bindings: refs.map(|reference| (reference, None)),
    };
    for (i, reference) in refs.into_iter().enumerate() {
        let payload = [0x40 + i as u8; 32];
        let plain = Message::new(
            Type::Acknowledgement,
            Code::CONTENT,
            MessageId::new(10 + i as u16),
        )
        .with_token(tokens[i])
        .with_payload(&payload);
        fixture.expected_lengths[i] = plain.encode(&mut fixture.expected[i]).unwrap();
        // Exactly one no-PIV encryption per distinct request nonce.
        fixture.lengths[i] = server
            .protect_response(&plain, reference, &mut fixture.wires[i])
            .unwrap();
    }
    fixture
}

fn fallback() -> Fixture {
    let (mut client, mut server) = pair(b'f');
    let tokens = tokens();
    for (i, &token) in tokens[..3].iter().enumerate() {
        issue(&mut client, &mut server, token, 20 + i as u16, &[]);
    }
    let token = tokens[3];
    let original = issue(
        &mut client,
        &mut server,
        token,
        23,
        &[Opt::observe_register(), Opt::uri_path("obs")],
    );
    let mut downloads = [original; 2];
    for (i, reference) in downloads.iter_mut().enumerate() {
        let block = BlockValue::from_size(i as u32 + 1, false, 16)
            .unwrap()
            .encode();
        *reference = issue(
            &mut client,
            &mut server,
            token,
            24 + i as u16,
            &[Opt::uri_path("obs"), Opt::block2(&block)],
        );
    }
    let [intermediate, latest] = downloads;
    assert_ne!(original, intermediate);
    assert_ne!(intermediate, latest);
    assert_ne!(original, latest);
    assert_full(&mut client, latest);
    let mut fixture = Fixture {
        ctx: client,
        tokens: [token; 4],
        wires: [[0; WIRE]; 4],
        lengths: [0; 4],
        expected: [[0; WIRE]; 4],
        expected_lengths: [0; 4],
        accepted: [Some(latest), Some(original), None, None],
        bindings: [(latest, Some(original)); 4],
    };
    for (i, reference) in [latest, original, intermediate].into_iter().enumerate() {
        let payload = [0x60 + i as u8; 32];
        let plain = Message::new(
            Type::NonConfirmable,
            Code::CONTENT,
            MessageId::new(40 + i as u16),
        )
        .with_token(token)
        .with_payload(&payload);
        fixture.expected_lengths[i] = plain.encode(&mut fixture.expected[i]).unwrap();
        // Each encryption consumes a fresh server PIV, including rejected input.
        fixture.lengths[i] = server
            .protect_response_with_piv(&plain, reference, &mut fixture.wires[i])
            .unwrap();
    }
    // Tamper a copy; do not re-encrypt a different plaintext with an old nonce.
    fixture.wires[3] = fixture.wires[1];
    fixture.lengths[3] = fixture.lengths[1];
    fixture.wires[3][fixture.lengths[3] - 1] ^= 1;
    fixture
}

// Parsed views live at the call site, borrowing already-stable fixed arrays.
fn views(fixture: &Fixture) -> [ParsedMessage<'_>; 4] {
    core::array::from_fn(|i| decode(&fixture.wires[i][..fixture.lengths[i]]).unwrap())
}

fn verify(fixture: &Fixture, parsed: &[ParsedMessage<'_>; 4]) {
    for (i, protected) in parsed.iter().enumerate() {
        assert_eq!(
            baseline_bindings(&fixture.ctx, fixture.tokens[i]),
            Some(fixture.bindings[i])
        );
        assert_eq!(
            fixture.ctx.response_bindings(fixture.tokens[i]),
            Some(fixture.bindings[i])
        );
        let mut old_scratch = [0; WIRE];
        let mut new_scratch = [0; WIRE];
        let old = fixture
            .ctx
            .baseline_unprotect_bound_response_wire(protected, &mut old_scratch);
        let new = fixture
            .ctx
            .unprotect_bound_response_wire(protected, &mut new_scratch);
        match fixture.accepted[i] {
            Some(reference) => {
                let (old_wire, old_ref) = old.unwrap();
                let (new_wire, new_ref) = new.unwrap();
                let expected = &fixture.expected[i][..fixture.expected_lengths[i]];
                assert_eq!(old_wire, expected);
                assert_eq!(new_wire, expected);
                assert_eq!(old_ref, reference);
                assert_eq!(new_ref, reference);
                let expected = decode(expected).unwrap();
                for wire in [old_wire, new_wire] {
                    let inner = decode(wire).unwrap();
                    assert_eq!(inner.token(), expected.token());
                    assert_eq!(inner.code(), Code::CONTENT);
                    assert_eq!(inner.message_id(), expected.message_id());
                    assert_eq!(inner.ty(), expected.ty());
                    assert_eq!(inner.payload(), expected.payload());
                    assert_eq!(inner.payload().len(), 32);
                }
            }
            None => {
                assert_eq!(old, Err(Error::Decrypt));
                assert_eq!(new, Err(Error::Decrypt));
            }
        }
    }
    let absent = Token::new(b"missing!").unwrap();
    assert_eq!(baseline_bindings(&fixture.ctx, absent), None);
    assert_eq!(fixture.ctx.response_bindings(absent), None);
}

fn verify_fallback(fixture: &Fixture, parsed: &[ParsedMessage<'_>; 4]) {
    verify(fixture, parsed);
    let (latest, original) = fixture.bindings[1];
    let original = original.unwrap();
    let mut scratch = [0; WIRE];
    assert_eq!(
        super::unprotect_response_candidates(&fixture.ctx, &parsed[1], latest, None, &mut scratch),
        Err(Error::Decrypt)
    );
    let (wire, accepted) = super::unprotect_response_candidates(
        &fixture.ctx,
        &parsed[1],
        latest,
        Some(original),
        &mut scratch,
    )
    .unwrap();
    assert_eq!(accepted, original);
    assert_eq!(wire, &fixture.expected[1][..fixture.expected_lengths[1]]);
}

fn entry(
    criterion: &mut Criterion,
    name: &str,
    ctx: &Context,
    parsed: &[ParsedMessage<'_>],
    candidate: bool,
) {
    assert!(parsed.len().is_power_of_two());
    let mask = parsed.len() - 1;
    let mut index = 0;
    let mut scratch = [0; WIRE];
    // Select the arm outside the timed closure; no function-pointer indirection.
    if candidate {
        criterion.bench_function(name, |b| {
            b.iter(|| {
                let (ctx, input) = black_box((ctx, &parsed[index]));
                index = (index + 1) & mask;
                let (wire, accepted) = ctx
                    .unprotect_bound_response_wire(input, &mut scratch)
                    .unwrap();
                black_box((wire, accepted));
            });
        });
    } else {
        criterion.bench_function(name, |b| {
            b.iter(|| {
                let (ctx, input) = black_box((ctx, &parsed[index]));
                index = (index + 1) & mask;
                let (wire, accepted) = ctx
                    .baseline_unprotect_bound_response_wire(input, &mut scratch)
                    .unwrap();
                black_box((wire, accepted));
            });
        });
    }
}

fn lookup(criterion: &mut Criterion, ctx: &Context, tokens: &[Token; 4], candidate: bool) {
    let mut index = 0;
    if candidate {
        criterion.bench_function(NAMES[2], |b| {
            b.iter(|| {
                let (ctx, token) = black_box((ctx, tokens[index]));
                index = (index + 1) & 3;
                black_box(ctx.response_bindings(token));
            });
        });
    } else {
        criterion.bench_function(NAMES[2], |b| {
            b.iter(|| {
                let (ctx, token) = black_box((ctx, tokens[index]));
                index = (index + 1) & 3;
                black_box(baseline_bindings(ctx, token));
            });
        });
    }
}

fn fixture_receipts(label: &str, fixture: &Fixture) {
    use std::fmt::Write as _;
    for i in 0..4 {
        let mut hex = std::string::String::new();
        for byte in &fixture.wires[i][..fixture.lengths[i]] {
            write!(&mut hex, "{byte:02x}").unwrap();
        }
        std::println!("BINDING_FIXTURE {label} {i} {hex}");
    }
}

#[test]
#[ignore = "ephemeral hosted Criterion probe; requires explicit ROUND, ARM, BINDING_PROBE_OUTPUT"]
fn criterion_probe() {
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
    if first {
        for name in NAMES {
            assert!(
                !output.join(name).exists(),
                "first arm requires fresh metric directories"
            );
        }
    } else {
        for name in NAMES {
            assert!(
                output.join(name).join("first/estimates.json").is_file(),
                "run first arm first"
            );
        }
    }
    let normal = normal();
    let fallback = fallback();
    let normal_views = views(&normal);
    let fallback_views = views(&fallback);
    verify(&normal, &normal_views);
    verify_fallback(&fallback, &fallback_views);

    fixture_receipts("normal", &normal);
    fixture_receipts("fallback", &fallback);

    let criterion = Criterion::default()
        .without_plots()
        .with_output_color(false)
        .sample_size(30)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(1))
        .nresamples(10_000)
        .confidence_level(0.95)
        .noise_threshold(0.02)
        .output_directory(&output);
    let mut criterion = if first {
        criterion.save_baseline("first".into())
    } else {
        criterion.retain_baseline("first".into(), true)
    };
    entry(
        &mut criterion,
        NAMES[0],
        &normal.ctx,
        &normal_views,
        candidate,
    );
    // Reuse slot zero to guard the constrained first-hit path separately.
    entry(
        &mut criterion,
        NAMES[3],
        &normal.ctx,
        &normal_views[..1],
        candidate,
    );
    // Only original-bound response [1] is timed: latest fails, original succeeds.
    entry(
        &mut criterion,
        NAMES[1],
        &fallback.ctx,
        &fallback_views[1..2],
        candidate,
    );
    lookup(&mut criterion, &normal.ctx, &normal.tokens, candidate);
    verify(&normal, &normal_views);
    verify_fallback(&fallback, &fallback_views);
    criterion.final_summary();
}
