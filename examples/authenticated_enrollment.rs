//! Host fixture for commissioning, restart refresh and revocation.
//!
//! Run with `--fixture` and features `std,edhoc`. Public test keys and an
//! in-memory preapproved ownership registry exercise real EDHOC and OSCORE.
//! The fixture's separate latest-anchor field can detect record-only rollback;
//! rolling back the complete fixture defeats it. It supplies no production
//! ownership service, power-loss durability, private-key storage or monotonic
//! provider. Install the bootstrap pin through an authenticated commissioning
//! path and replace the provider before deployment.

use coaptic::Endpoint;
use coaptic::message::MessageId;
use coaptic::provisioning::{
    AuthenticatedPolicyRequest, AuthorityConnection, AuthorityResponder, BootstrapAuthority,
    EnrollmentPolicy, Identity, Initiator, OnlinePolicyAuthority, PeerTrust, PolicyQuery,
    PolicySnapshot, Principal, Responder, Session, TrustAnchor, TrustError, TrustRecord,
    TrustRecordParts,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureError {
    Approval,
    Conflict,
    Unavailable,
    Rollback,
    Exhausted,
}

struct ApprovedRegistration {
    device: Principal,
    policy: EnrollmentPolicy,
}

struct FixtureAuthority {
    approval: ApprovedRegistration,
    latest: TrustAnchor,
    snapshot: Option<PolicySnapshot>,
}

impl FixtureAuthority {
    fn new(device: &Identity, application: &Identity) -> Self {
        let owner = [0xa5; 32];
        let mut permissions = blake3::Hasher::new_derive_key(
            "https://github.com/jeffglousher/coaptic fixture resource policy v1",
        );
        permissions.update(&owner);
        permissions.update(device.peer().principal().fingerprint());
        permissions.update(b"PUT /telemetry;GET /configuration");
        let policy = EnrollmentPolicy::new(
            application.peer(),
            owner,
            *permissions.finalize().as_bytes(),
        )
        .unwrap();
        Self {
            approval: ApprovedRegistration {
                device: device.peer().principal(),
                policy,
            },
            latest: TrustAnchor::genesis([0x51; 32]).unwrap(),
            snapshot: None,
        }
    }

    fn revoke(&mut self) -> Result<(), FixtureError> {
        let old = self.snapshot.as_ref().ok_or(FixtureError::Unavailable)?;
        if old.record().anchor() != self.latest {
            return Err(FixtureError::Rollback);
        }
        let mut parts = old.record().parts();
        parts.generation = parts
            .generation
            .checked_add(1)
            .ok_or(FixtureError::Exhausted)?;
        parts.enabled = false;
        let record = TrustRecord::from_parts(parts).map_err(|_| FixtureError::Conflict)?;
        let snapshot = PolicySnapshot::new(record, *old.owner(), *old.resource_policy()).unwrap();
        self.latest = snapshot.record().anchor();
        self.snapshot = Some(snapshot);
        Ok(())
    }
}

impl OnlinePolicyAuthority for FixtureAuthority {
    type Error = FixtureError;

    fn resolve(
        &mut self,
        request: &AuthenticatedPolicyRequest,
    ) -> Result<PolicySnapshot, FixtureError> {
        if request.principal() != &self.approval.device {
            return Err(FixtureError::Approval);
        }
        match request.query() {
            PolicyQuery::Enroll { expected, policy } => {
                if policy.peer().principal() != self.approval.policy.peer().principal()
                    || policy.owner() != self.approval.policy.owner()
                    || policy.resource_policy() != self.approval.policy.resource_policy()
                {
                    return Err(FixtureError::Approval);
                }
                if *expected != self.latest || expected.parts().1 != 0 || self.snapshot.is_some() {
                    return Err(FixtureError::Conflict);
                }
                let record = TrustRecord::from_parts(TrustRecordParts {
                    version: 1,
                    store_id: self.latest.parts().0,
                    generation: 1,
                    local_principal: *request.principal().fingerprint(),
                    peer_public_key: policy.peer().public_key(),
                    peer_kid: policy.peer().kid(),
                    enabled: true,
                })
                .map_err(|_| FixtureError::Conflict)?;
                let snapshot =
                    PolicySnapshot::new(record, *policy.owner(), *policy.resource_policy())
                        .unwrap();
                self.latest = snapshot.record().anchor();
                self.snapshot = Some(snapshot.clone());
                Ok(snapshot)
            }
            PolicyQuery::Refresh { expected, .. } => {
                if expected.parts().0 != self.latest.parts().0 {
                    return Err(FixtureError::Conflict);
                }
                let snapshot = self.snapshot.as_ref().ok_or(FixtureError::Unavailable)?;
                if snapshot.record().anchor() != self.latest {
                    return Err(FixtureError::Rollback);
                }
                Ok(snapshot.clone())
            }
        }
    }
}

fn fixture_identity(value: u8, kid: u8) -> Identity {
    let mut scalar = [0; 32];
    scalar[31] = value;
    Identity::from_private_key(scalar, kid).unwrap()
}

fn entropy(bytes: &mut [u8]) -> bool {
    getrandom::fill(bytes).is_ok()
}

fn sessions(device: &Identity, authority: &Identity) -> (Session, Session) {
    let (initiator, m1) = Initiator::start(device, authority.peer(), entropy).unwrap();
    let (responder, m2) =
        Responder::receive_message_1(authority, device.peer(), &m1, entropy).unwrap();
    let (initiator, m3) = initiator
        .receive_message_2(&m2, |principal| *principal == authority.peer().principal())
        .unwrap();
    let (server, m4) = responder
        .receive_message_3(&m3, |principal| *principal == device.peer().principal())
        .unwrap();
    let client = initiator
        .receive_message_4(&m4, |principal| *principal == authority.peer().principal())
        .unwrap();
    (client, server)
}

fn exchange(
    device: &Identity,
    authority_identity: &Identity,
    bootstrap: &BootstrapAuthority,
    query: PolicyQuery,
    authority: &mut FixtureAuthority,
    local_mirror: &mut Option<PolicySnapshot>,
) -> PeerTrust {
    let device_endpoint = Endpoint::v4([127, 0, 0, 1], 56830);
    let authority_endpoint = Endpoint::v4([127, 0, 0, 1], 56831);
    let (client, server) = sessions(device, authority_identity);
    let clock = std::time::Instant::now();
    let pending = AuthorityConnection::from_session(device, bootstrap, authority_endpoint, client)
        .unwrap()
        .begin(query, MessageId::new(1), 0, 1000, entropy)
        .unwrap();
    let reply = AuthorityResponder::from_session(authority_identity, device_endpoint, server)
        .unwrap()
        .serve(
            device_endpoint,
            pending
                .datagram(clock.elapsed().as_millis() as u64)
                .unwrap()
                .1,
            authority,
        )
        .unwrap();
    pending
        .accept(
            device,
            authority_endpoint,
            clock.elapsed().as_millis() as u64,
            reply.datagram().1,
        )
        .unwrap()
        .persist_and_restore(device, |snapshot| {
            *local_mirror = Some(snapshot.clone());
            Ok::<_, FixtureError>(())
        })
        .unwrap()
}

fn main() {
    if std::env::args().nth(1).as_deref() != Some("--fixture") {
        eprintln!(
            "Usage: cargo run --features std,edhoc --example authenticated_enrollment -- --fixture"
        );
        std::process::exit(2);
    }
    let device = fixture_identity(1, 1);
    let authority_identity = fixture_identity(2, 2);
    let application_identity = fixture_identity(3, 3);
    let bootstrap = BootstrapAuthority::new(authority_identity.peer());
    let mut authority = FixtureAuthority::new(&device, &application_identity);
    let query = PolicyQuery::Enroll {
        expected: authority.latest,
        policy: authority.approval.policy.clone(),
    };
    let mut mirror = None;
    {
        let enrolled = exchange(
            &device,
            &authority_identity,
            &bootstrap,
            query,
            &mut authority,
            &mut mirror,
        );
        assert_eq!(
            enrolled.grant(&device).unwrap().principal(),
            &application_identity.peer().principal()
        );
    }
    let query = PolicyQuery::refresh(mirror.as_ref().unwrap());
    {
        let restored = exchange(
            &device,
            &authority_identity,
            &bootstrap,
            query,
            &mut authority,
            &mut mirror,
        );
        assert!(restored.grant(&device).is_ok());
    }
    authority.revoke().unwrap();
    let query = PolicyQuery::refresh(mirror.as_ref().unwrap());
    let revoked = exchange(
        &device,
        &authority_identity,
        &bootstrap,
        query,
        &mut authority,
        &mut mirror,
    );
    assert_eq!(revoked.grant(&device), Err(TrustError::Unauthorized));
    println!(
        "{{\"fixture\":true,\"enrollment\":\"pass\",\"restart_refresh\":\"pass\",\"revocation\":\"pass\",\"generation\":{},\"production_provider_qualified\":false,\"host_object_bytes\":{{\"authority_connection\":{},\"pending_policy\":{},\"protected_response\":{},\"policy_snapshot\":{},\"verified_policy\":{}}}}}",
        revoked.checkpoint().anchor().parts().1,
        core::mem::size_of::<AuthorityConnection>(),
        core::mem::size_of::<coaptic::provisioning::PendingPolicy>(),
        core::mem::size_of::<coaptic::provisioning::ProtectedPolicyResponse>(),
        core::mem::size_of::<PolicySnapshot>(),
        core::mem::size_of::<coaptic::provisioning::VerifiedPolicy>()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        device: &Identity,
        authority_identity: &Identity,
        query: PolicyQuery,
        authority: &mut FixtureAuthority,
    ) -> Result<(), FixtureError> {
        let (client, server) = sessions(device, authority_identity);
        let endpoint = Endpoint::v4([127, 0, 0, 1], 5683);
        let pending = AuthorityConnection::from_session(
            device,
            &BootstrapAuthority::new(authority_identity.peer()),
            endpoint,
            client,
        )
        .unwrap()
        .begin(query, MessageId::new(1), 0, 1000, entropy)
        .unwrap();
        AuthorityResponder::from_session(authority_identity, endpoint, server)
            .unwrap()
            .serve(endpoint, pending.datagram(0).unwrap().1, authority)
            .map(|_| ())
            .map_err(|error| match error {
                coaptic::provisioning::PolicyServeError::Provider(error) => error,
                _ => FixtureError::Conflict,
            })
    }

    #[test]
    fn possession_without_approved_registration_cannot_claim_ownership() {
        let device = fixture_identity(1, 1);
        let authority_identity = fixture_identity(2, 2);
        let other = fixture_identity(3, 3);
        let mut authority = FixtureAuthority::new(&device, &authority_identity);
        let query = PolicyQuery::Enroll {
            expected: authority.latest,
            policy: authority.approval.policy.clone(),
        };
        assert_eq!(
            request(&other, &authority_identity, query, &mut authority),
            Err(FixtureError::Approval)
        );
        assert!(authority.snapshot.is_none());
    }

    #[test]
    fn approval_substitution_and_duplicate_claim_are_refused() {
        let device = fixture_identity(1, 1);
        let authority_identity = fixture_identity(2, 2);
        let mut authority = FixtureAuthority::new(&device, &authority_identity);
        let genesis = authority.latest;
        let approved = authority.approval.policy.clone();
        let substituted = EnrollmentPolicy::new(
            authority_identity.peer(),
            [9; 32],
            *approved.resource_policy(),
        )
        .unwrap();
        assert_eq!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::Enroll {
                    expected: genesis,
                    policy: substituted
                },
                &mut authority
            ),
            Err(FixtureError::Approval)
        );
        assert!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::Enroll {
                    expected: genesis,
                    policy: approved.clone()
                },
                &mut authority
            )
            .is_ok()
        );
        assert_eq!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::Enroll {
                    expected: genesis,
                    policy: approved
                },
                &mut authority
            ),
            Err(FixtureError::Conflict)
        );
    }

    #[test]
    fn unavailable_state_and_record_only_rollback_never_synthesize_genesis() {
        let device = fixture_identity(1, 1);
        let authority_identity = fixture_identity(2, 2);
        let mut authority = FixtureAuthority::new(&device, &authority_identity);
        let genesis = authority.latest;
        assert_eq!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::recover_initial_commit(genesis).unwrap(),
                &mut authority
            ),
            Err(FixtureError::Unavailable)
        );
        assert!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::Enroll {
                    expected: genesis,
                    policy: authority.approval.policy.clone()
                },
                &mut authority
            )
            .is_ok()
        );
        let old_snapshot = authority.snapshot.clone().unwrap();
        authority.revoke().unwrap();
        authority.snapshot = Some(old_snapshot.clone());
        assert_eq!(
            request(
                &device,
                &authority_identity,
                PolicyQuery::refresh(&old_snapshot),
                &mut authority
            ),
            Err(FixtureError::Rollback)
        );
        assert_eq!(authority.latest.parts().1, 2);
    }
}
