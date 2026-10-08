use super::*;
use crate::provisioning::{Identity, TrustRecord};

fn identity(value: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, value).unwrap()
}

fn policy() -> (Identity, PeerTrust, TrustGrant) {
    let local = identity(1);
    let trust = PeerTrust::install(
        &local,
        identity(2).peer(),
        TrustAnchor::genesis([42; 32]).unwrap(),
        |_, _, _| Ok::<_, ()>(()),
    )
    .unwrap();
    let grant = trust.grant(&local).unwrap();
    (local, trust, grant)
}

struct Store {
    anchor: TrustAnchor,
    principal: Principal,
    resource: [u8; 32],
    receipt: Option<Receipt>,
    effects: usize,
    lose_ack: bool,
    invalid_receipt: bool,
}

impl Store {
    fn new(grant: &TrustGrant) -> Self {
        Self {
            anchor: grant.anchor(),
            principal: *grant.principal(),
            resource: [7; 32],
            receipt: None,
            effects: 0,
            lose_ack: false,
            invalid_receipt: false,
        }
    }
}

impl ReceiptStore for Store {
    type Error = &'static str;

    fn commit(
        &mut self,
        expected: &TrustAnchor,
        principal: &Principal,
        operation: &TelemetryOperation<'_>,
    ) -> Result<Receipt, ReceiptError<Self::Error>> {
        if *expected != self.anchor
            || *principal != self.principal
            || *operation.resource() != self.resource
        {
            return Err(ReceiptError::Unauthorized);
        }
        if let Some(receipt) = self.receipt {
            return if receipt.id() == operation.id() && receipt.digest() == operation.digest() {
                Ok(receipt)
            } else {
                Err(ReceiptError::Conflict)
            };
        }
        let digest = if self.invalid_receipt {
            [0; 32]
        } else {
            *operation.digest()
        };
        let receipt = Receipt::from_parts(operation.id(), digest, 1).unwrap();
        self.receipt = Some(receipt);
        self.effects += 1;
        if self.lose_ack {
            self.lose_ack = false;
            Err(ReceiptError::Persistence("committed acknowledgement lost"))
        } else {
            Ok(receipt)
        }
    }
}

#[test]
fn lost_acknowledgement_recovers_same_receipt_without_repeating_effect() {
    let (_identity, trust, grant) = policy();
    let mut store = Store::new(&grant);
    store.lose_ack = true;
    let operation = TelemetryOperation::new(
        OperationId::new([4; 16]),
        [7; 32],
        Some(42),
        b"\0\xfftelemetry",
    )
    .unwrap();
    assert!(matches!(
        commit_telemetry(&trust, &grant, &operation, &mut store),
        Err(ReceiptError::Persistence(_))
    ));
    let receipt = commit_telemetry(&trust, &grant, &operation, &mut store).unwrap();
    assert_eq!(store.effects, 1);
    assert_eq!(receipt, store.receipt.unwrap());
    assert_eq!(Receipt::decode(&receipt.encode()), Some(receipt));
}

#[test]
fn id_reuse_with_changed_payload_format_or_resource_is_refused() {
    let (_identity, trust, grant) = policy();
    let mut store = Store::new(&grant);
    let id = OperationId::new([4; 16]);
    let original = TelemetryOperation::new(id, [7; 32], None, b"value").unwrap();
    commit_telemetry(&trust, &grant, &original, &mut store).unwrap();
    for (resource, format, payload) in [
        ([7; 32], None, b"other".as_slice()),
        ([7; 32], Some(0), b"value".as_slice()),
        ([8; 32], None, b"value".as_slice()),
    ] {
        let changed = TelemetryOperation::new(id, resource, format, payload).unwrap();
        assert!(matches!(
            commit_telemetry(&trust, &grant, &changed, &mut store),
            Err(ReceiptError::Conflict | ReceiptError::Unauthorized)
        ));
    }
    assert_eq!(store.effects, 1);
}

#[test]
fn local_and_authoritative_revocation_both_refuse_effects_and_receipts() {
    let (_identity, mut trust, grant) = policy();
    let mut store = Store::new(&grant);
    let operation =
        TelemetryOperation::new(OperationId::new([4; 16]), [7; 32], None, b"value").unwrap();
    let mut parts = trust.checkpoint().parts();
    parts.generation += 1;
    parts.enabled = false;
    store.anchor = TrustRecord::from_parts(parts).unwrap().anchor();
    assert_eq!(
        commit_telemetry(&trust, &grant, &operation, &mut store),
        Err(ReceiptError::Unauthorized)
    );
    trust.revoke(|_, _, _| Ok::<_, ()>(())).unwrap();
    assert_eq!(
        commit_telemetry(&trust, &grant, &operation, &mut store),
        Err(ReceiptError::Unauthorized)
    );
    assert_eq!(store.effects, 0);
}

#[test]
fn provider_receipt_must_match_complete_operation() {
    let (_identity, trust, grant) = policy();
    let mut store = Store::new(&grant);
    store.invalid_receipt = true;
    let operation =
        TelemetryOperation::new(OperationId::new([4; 16]), [7; 32], None, b"value").unwrap();
    assert_eq!(
        commit_telemetry(&trust, &grant, &operation, &mut store),
        Err(ReceiptError::InvalidReceipt)
    );
}

#[test]
fn cross_principal_and_capacity_failures_precede_effects() {
    let (_identity, trust, grant) = policy();
    let mut store = Store::new(&grant);
    store.principal = identity(3).peer().principal();
    let operation =
        TelemetryOperation::new(OperationId::new([4; 16]), [7; 32], None, b"value").unwrap();
    assert_eq!(
        commit_telemetry(&trust, &grant, &operation, &mut store),
        Err(ReceiptError::Unauthorized)
    );
    assert_eq!(store.effects, 0);
    assert!(matches!(
        TelemetryOperation::new(OperationId::new([4; 16]), [7; 32], None, &[0; 1025]),
        Err(ReceiptError::Capacity)
    ));
}

#[test]
fn incomplete_or_zero_sequence_receipts_are_refused() {
    let receipt = Receipt::from_parts(OperationId::new([4; 16]), [8; 32], 1)
        .unwrap()
        .encode();
    for len in 0..56 {
        assert!(Receipt::decode(&receipt[..len]).is_none());
    }
    let mut extra = [0; 57];
    extra[..56].copy_from_slice(&receipt);
    assert!(Receipt::decode(&extra).is_none());
    extra[48..56].fill(0);
    assert!(Receipt::decode(&extra[..56]).is_none());
}
