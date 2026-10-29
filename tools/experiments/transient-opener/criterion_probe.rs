//! Ephemeral cfg(test) child of oscore::protect; never a production module.
//! Adapted from 3474c106:tools/experiments/binding-only/criterion_probe.rs.
//! Parent supplies Criterion =0.5.1 (default-features=false), the exact original
//! 869d435 bound-open method as baseline_unprotect_bound_response_wire and FREE
//! helper as baseline_unprotect_response_candidates. Only the oracle method's
//! helper call is renamed: both original searches and one-shot AEAD stay intact.
//! Invoke ROUND=1 baseline,candidate; ROUND=2 candidate,baseline, sharing each
//! round's absolute BINDING_PROBE_OUTPUT directory. No argument parsing here.
//! Repeated read-only ciphertext opens measure authentication, NOT replay admission.

extern crate std;

use super::*;
use crate::message::{BlockValue, decode};
use crate::oscore::DeriveParams;
use criterion::Criterion;
use std::{env, hint::black_box, path::PathBuf, time::Duration};

type Context = BoundedSecurityContext<4>;
const WIRE: usize = 256;
const SENTINEL: u8 = 0xa5;
const NAMES: [&str; 7] = [
    "normal-no-piv",
    "first-slot-no-piv",
    "latest-observe-piv",
    "fallback-piv",
    "fallback-no-piv",
    "rejected-piv",
    "rejected-no-piv",
];

struct Fixture {
    ctx: Context,
    tokens: [Token; 4],
    wires: [[u8; WIRE]; 4],
    lengths: [usize; 4],
    expected: [[u8; WIRE]; 4],
    expected_lengths: [usize; 4],
    accepted: [Option<RequestRef>; 4],
    bindings: [(RequestRef, Option<RequestRef>); 4],
    intermediate: Option<RequestRef>,
}

fn pair(kind: u8) -> (Context, Context) {
    // Public fixed keys, distinct per kind and identical across all four arms.
    // Each encryption within one fixture consumes a fresh request/server nonce.
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
    // Four distinct full-width tokens sharing the same seven-byte prefix.
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
    for i in 0..4 {
        for j in 0..i {
            assert_ne!(refs[i], refs[j]);
            assert_ne!(tokens[i], tokens[j]);
        }
    }
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
        intermediate: None,
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

fn fallback(piv: bool) -> Fixture {
    let (mut client, mut server) = pair(if piv { b'f' } else { b'g' });
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
        intermediate: Some(intermediate),
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
        fixture.lengths[i] = if piv {
            // One retained server: every encryption consumes a fresh server PIV.
            server.protect_response_with_piv(&plain, reference, &mut fixture.wires[i])
        } else {
            // Latest/original/intermediate request refs each used exactly once.
            server.protect_response(&plain, reference, &mut fixture.wires[i])
        }
        .unwrap();
    }
    // Copy then tamper: no re-encryption under an already used nonce.
    fixture.wires[3] = fixture.wires[1];
    fixture.lengths[3] = fixture.lengths[1];
    fixture.wires[3][fixture.lengths[3] - 1] ^= 1;
    fixture
}

// Views borrow stable arrays; all parsing, fixture construction and checks are untimed.
fn views(fixture: &Fixture) -> [ParsedMessage<'_>; 4] {
    core::array::from_fn(|i| decode(&fixture.wires[i][..fixture.lengths[i]]).unwrap())
}

fn assert_plain(wire: &[u8], fixture: &Fixture, i: usize) {
    let expected = &fixture.expected[i][..fixture.expected_lengths[i]];
    assert_eq!(wire, expected);
    let inner = decode(wire).unwrap();
    let expected = decode(expected).unwrap();
    assert_eq!(inner.token(), expected.token());
    assert_eq!(inner.code(), Code::CONTENT);
    assert_eq!(inner.message_id(), expected.message_id());
    assert_eq!(inner.ty(), expected.ty());
    assert_eq!(inner.payload(), expected.payload());
    assert_eq!(inner.payload().len(), 32);
}

fn verify(fixture: &Fixture, parsed: &[ParsedMessage<'_>; 4], piv: bool) {
    for (i, protected) in parsed.iter().enumerate() {
        let (latest, original) = fixture.bindings[i];
        assert_eq!(fixture.ctx.lookup(fixture.tokens[i]), Some(latest));
        assert_eq!(fixture.ctx.observe_request(fixture.tokens[i]), original);
        assert_eq!(parse_protected(protected).unwrap().piv.is_some(), piv);
        let mut old_scratch = [SENTINEL; WIRE];
        let mut new_scratch = [SENTINEL; WIRE];
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
                assert_plain(old_wire, fixture, i);
                assert_plain(new_wire, fixture, i);
                assert_eq!(old_ref, reference);
                assert_eq!(new_ref, reference);
                let n = fixture.expected_lengths[i];
                assert_eq!(&old_scratch[n..], &[SENTINEL; WIRE][n..]);
                assert_eq!(&new_scratch[n..], &[SENTINEL; WIRE][n..]);
            }
            None => {
                assert_eq!(old, Err(Error::Decrypt));
                assert_eq!(new, Err(Error::Decrypt));
                assert_eq!(old_scratch, [SENTINEL; WIRE]);
                assert_eq!(new_scratch, [SENTINEL; WIRE]);
            }
        }
        // Include every caller-output byte, not only decoded fields.
        assert_eq!(old_scratch, new_scratch);
    }
    let absent = Token::new(b"missing!").unwrap();
    assert_eq!(fixture.ctx.lookup(absent), None);
    assert_eq!(fixture.ctx.observe_request(absent), None);
}

fn single<'a>(
    baseline: bool,
    fixture: &Fixture,
    protected: &ParsedMessage<'_>,
    reference: RequestRef,
    out: &'a mut [u8],
) -> Result<(&'a [u8], RequestRef), Error> {
    if baseline {
        super::baseline_unprotect_response_candidates(&fixture.ctx, protected, reference, None, out)
    } else {
        super::unprotect_response_candidates(&fixture.ctx, protected, reference, None, out)
    }
}

fn verify_fallback(fixture: &Fixture, parsed: &[ParsedMessage<'_>; 4], piv: bool) {
    verify(fixture, parsed, piv);
    let (latest, original) = fixture.bindings[0];
    let original = original.unwrap();
    let intermediate = fixture.intermediate.unwrap();
    // Evidence from BOTH helpers: original response really needs fallback;
    // obsolete intermediate is genuine, but both retained bindings reject it.
    for baseline in [true, false] {
        for (i, reference) in [latest, original, intermediate].into_iter().enumerate() {
            let mut scratch = [SENTINEL; WIRE];
            let (wire, accepted) =
                single(baseline, fixture, &parsed[i], reference, &mut scratch).unwrap();
            assert_plain(wire, fixture, i);
            assert_eq!(accepted, reference);
            let n = fixture.expected_lengths[i];
            assert_eq!(&scratch[n..], &[SENTINEL; WIRE][n..]);
        }
        for (i, reference) in [
            (0, original),
            (1, latest),
            (2, latest),
            (2, original),
            (3, latest),
            (3, original),
        ] {
            let mut scratch = [SENTINEL; WIRE];
            assert_eq!(
                single(baseline, fixture, &parsed[i], reference, &mut scratch),
                Err(Error::Decrypt)
            );
            assert_eq!(scratch, [SENTINEL; WIRE]);
        }
    }
}

fn entry<const CANDIDATE: bool>(
    criterion: &mut Criterion,
    name: &str,
    ctx: &Context,
    parsed: &[ParsedMessage<'_>],
) {
    assert!(parsed.len().is_power_of_two());
    let mask = parsed.len() - 1;
    let mut index = 0;
    let mut scratch = [0; WIRE];
    // Compile-time arm outside closure; black_box only inputs and full outputs.
    // Nothing separates the baseline's two searches or its AEAD setup.
    if CANDIDATE {
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

fn rejected<const CANDIDATE: bool>(
    criterion: &mut Criterion,
    name: &str,
    ctx: &Context,
    parsed: &[ParsedMessage<'_>],
) {
    assert_eq!(parsed.len(), 2);
    let mut index = 0;
    let mut scratch = [0; WIRE];
    if CANDIDATE {
        criterion.bench_function(name, |b| {
            b.iter(|| {
                let (ctx, input) = black_box((ctx, &parsed[index]));
                index ^= 1;
                let error = ctx
                    .unprotect_bound_response_wire(input, &mut scratch)
                    .unwrap_err();
                black_box((error, &scratch[..]));
            });
        });
    } else {
        criterion.bench_function(name, |b| {
            b.iter(|| {
                let (ctx, input) = black_box((ctx, &parsed[index]));
                index ^= 1;
                let error = ctx
                    .baseline_unprotect_bound_response_wire(input, &mut scratch)
                    .unwrap_err();
                black_box((error, &scratch[..]));
            });
        });
    }
}

fn measure<const CANDIDATE: bool>(
    criterion: &mut Criterion,
    normal: &Fixture,
    normal_views: &[ParsedMessage<'_>; 4],
    piv: &Fixture,
    piv_views: &[ParsedMessage<'_>; 4],
    no_piv: &Fixture,
    no_piv_views: &[ParsedMessage<'_>; 4],
) {
    entry::<CANDIDATE>(criterion, NAMES[0], &normal.ctx, normal_views);
    entry::<CANDIDATE>(criterion, NAMES[1], &normal.ctx, &normal_views[..1]);
    entry::<CANDIDATE>(criterion, NAMES[2], &piv.ctx, &piv_views[..1]);
    entry::<CANDIDATE>(criterion, NAMES[3], &piv.ctx, &piv_views[1..2]);
    entry::<CANDIDATE>(criterion, NAMES[4], &no_piv.ctx, &no_piv_views[1..2]);
    rejected::<CANDIDATE>(criterion, NAMES[5], &piv.ctx, &piv_views[2..]);
    rejected::<CANDIDATE>(criterion, NAMES[6], &no_piv.ctx, &no_piv_views[2..]);
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
    let normal = normal();
    let piv = fallback(true);
    let no_piv = fallback(false);
    let normal_views = views(&normal);
    let piv_views = views(&piv);
    let no_piv_views = views(&no_piv);
    verify(&normal, &normal_views, false);
    verify_fallback(&piv, &piv_views, true);
    verify_fallback(&no_piv, &no_piv_views, false);
    let retained = [normal.wires, piv.wires, no_piv.wires];
    // Exactly twelve unique labeled lines, deterministically identical in all arms.
    fixture_receipts("normal", &normal);
    fixture_receipts("fallback-piv", &piv);
    fixture_receipts("fallback-no-piv", &no_piv);

    // Aes128Ccm is private to sibling aead: parent may inject its own layout test.
    // No production accessor or substitute alias is introduced for layout reporting.
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
    if candidate {
        measure::<true>(
            &mut criterion,
            &normal,
            &normal_views,
            &piv,
            &piv_views,
            &no_piv,
            &no_piv_views,
        );
    } else {
        measure::<false>(
            &mut criterion,
            &normal,
            &normal_views,
            &piv,
            &piv_views,
            &no_piv,
            &no_piv_views,
        );
    }
    verify(&normal, &normal_views, false);
    verify_fallback(&piv, &piv_views, true);
    verify_fallback(&no_piv, &no_piv_views, false);
    assert_eq!(retained, [normal.wires, piv.wires, no_piv.wires]);
    criterion.final_summary();
}
