use super::*;
use coaptic::provisioning::{Identity, PinnedPeer};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    path: std::path::PathBuf,
    principal: Principal,
    authority: Authority,
    anchor: TrustAnchor,
}
impl Fixture {
    fn new() -> Self {
        let principal = Identity::from_private_key([1; 32], 1)
            .unwrap()
            .peer()
            .principal();
        let anchor = TrustAnchor::from_parts([3; 32], 1, [4; 32]).unwrap();
        Self {
            path: std::env::temp_dir().join(format!(
                "coaptic-receipt-lookup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            principal,
            authority: Authority::new(Policy {
                anchor,
                principal,
                resource: [5; 32],
                enabled: true,
            }),
            anchor,
        }
    }
    fn revoke(&self) {
        self.authority
            .replace(Policy {
                anchor: TrustAnchor::from_parts([3; 32], 2, [8; 32]).unwrap(),
                principal: self.principal,
                resource: [5; 32],
                enabled: false,
            })
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn snapshot(journal: &mut Journal) -> Vec<u8> {
    // Windows byte-range locks also exclude separately opened readers. Inspect
    // through the actual locked owner and leave its stream position unchanged.
    let position = journal.file.stream_position().unwrap();
    journal.file.rewind().unwrap();
    let mut bytes = Vec::new();
    journal.file.read_to_end(&mut bytes).unwrap();
    journal.file.seek(SeekFrom::Start(position)).unwrap();
    bytes
}

#[test]
fn revoked_lookup_is_exact_and_never_changes_effects_or_file_bytes() {
    let fixture = Fixture::new();
    let mut journal = Journal::create(&fixture.path, fixture.authority.clone(), 2).unwrap();
    let operation =
        TelemetryOperation::new(OperationId::new([6; 16]), [5; 32], Some(60), b"committed")
            .unwrap();
    let receipt = journal
        .commit(&fixture.anchor, &fixture.principal, &operation)
        .unwrap();
    let original = snapshot(&mut journal);
    fixture.revoke();
    assert!(matches!(
        journal.commit(&fixture.anchor, &fixture.principal, &operation),
        Err(ReceiptError::Unauthorized)
    ));
    assert_eq!(
        journal
            .lookup_receipt(
                &fixture.principal,
                operation.id(),
                operation.resource(),
                operation.digest()
            )
            .unwrap(),
        Some(receipt)
    );
    assert_eq!(
        journal
            .lookup_receipt(
                &fixture.principal,
                OperationId::new([7; 16]),
                operation.resource(),
                operation.digest()
            )
            .unwrap(),
        None
    );
    let other: PinnedPeer = Identity::from_private_key([2; 32], 2).unwrap().peer();
    assert_eq!(
        journal
            .lookup_receipt(
                &other.principal(),
                operation.id(),
                operation.resource(),
                operation.digest()
            )
            .unwrap(),
        None
    );
    for (resource, digest) in [
        ([9; 32], *operation.digest()),
        (*operation.resource(), [9; 32]),
    ] {
        assert!(matches!(
            journal.lookup_receipt(&fixture.principal, operation.id(), &resource, &digest),
            Err(ReceiptError::Conflict)
        ));
    }
    assert_eq!(journal.effects().unwrap(), 1);
    assert_eq!(snapshot(&mut journal), original);
    drop(journal);
    let mut reopened = Journal::open(&fixture.path, fixture.authority.clone()).unwrap();
    assert_eq!(
        reopened
            .lookup_receipt(
                &fixture.principal,
                operation.id(),
                operation.resource(),
                operation.digest()
            )
            .unwrap(),
        Some(receipt)
    );
    assert_eq!(snapshot(&mut reopened), original);
}

#[test]
fn ambiguous_commit_cannot_return_metadata_until_durable_recovery() {
    let fixture = Fixture::new();
    let mut journal = Journal::create(&fixture.path, fixture.authority.clone(), 1).unwrap();
    let operation =
        TelemetryOperation::new(OperationId::new([6; 16]), [5; 32], None, b"retained").unwrap();
    journal.set_checkpoint(|stage| {
        if stage == Stage::AfterSync {
            Err(std::io::Error::other("lost acknowledgement"))
        } else {
            Ok(())
        }
    });
    assert!(
        journal
            .commit(&fixture.anchor, &fixture.principal, &operation)
            .is_err()
    );
    assert!(matches!(
        journal.lookup_receipt(
            &fixture.principal,
            operation.id(),
            operation.resource(),
            operation.digest()
        ),
        Err(ReceiptError::Persistence(Error::NeedsRecovery))
    ));
    fixture.revoke();
    drop(journal);
    let original = std::fs::read(&fixture.path).unwrap();
    let mut recovered = Journal::open(&fixture.path, fixture.authority.clone()).unwrap();
    let receipt = recovered
        .lookup_receipt(
            &fixture.principal,
            operation.id(),
            operation.resource(),
            operation.digest(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(receipt.sequence(), 1);
    assert_eq!(recovered.effects().unwrap(), 1);
    assert_eq!(snapshot(&mut recovered), original);
}
