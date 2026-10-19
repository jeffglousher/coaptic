//! Repair the exact destination host after a device committed but its reply was lost.
//! Read the device over the trusted physical channel and authenticate its current
//! owner before calling `commit`. This never sends another device update, revives
//! a former owner, or repairs missing/corrupt identity or pairing storage.

use crate::{Action, Error, Request};
use coaptic::provisioning::{
    EnrollmentPolicy, LocalPairing, PairingChallenge, PairingRecord, PinnedPeer,
};

/// Validated reciprocal transition held only in memory until the device confirms
/// its update. Its challenge is consumed before any device mutation, so waiting
/// for the device reply cannot expire an otherwise valid host transition.
pub struct PreparedHostPairing {
    prior: PairingRecord,
    next: PairingRecord,
}

impl PreparedHostPairing {
    pub fn prepare(
        state: &mut LocalPairing,
        challenge: &PairingChallenge,
        policy: EnrollmentPolicy,
        now_ms: u64,
    ) -> Result<Self, Error> {
        if state.checkpoint().policy().is_some() {
            return Err(Error::Input);
        }
        Self::prepare_update(state, challenge, policy, false, now_ms)
    }

    /// Stage an ordinary same-owner grant or revocation without persisting it.
    /// Authentication and exact remote-policy binding remain the caller's job.
    pub fn prepare_update(
        state: &mut LocalPairing,
        challenge: &PairingChallenge,
        policy: EnrollmentPolicy,
        revoke: bool,
        now_ms: u64,
    ) -> Result<Self, Error> {
        let prior = state.checkpoint().clone();
        if revoke {
            let current = prior.policy().ok_or(Error::Input)?;
            if current.owner() != policy.owner()
                || current.peer().principal() != policy.peer().principal()
                || current.resource_policy() != policy.resource_policy()
            {
                return Err(Error::Input);
            }
            state
                .revoke(challenge, policy.owner(), now_ms, |_, _| Ok::<_, Error>(()))
                .map_err(|_| Error::Expired)?;
        } else {
            state
                .pair(challenge, policy, now_ms, |_, _| Ok::<_, Error>(()))
                .map_err(|_| Error::Expired)?;
        }
        Ok(Self {
            prior,
            next: state.checkpoint().clone(),
        })
    }

    /// Persist only after validating the device's exact committed response.
    /// The caller keeps its exclusive host-store lock across prepare and commit.
    pub fn commit(
        self,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), Error>,
    ) -> Result<(), Error> {
        commit(&self.prior, &self.next)
    }
}

/// One immutable device snapshot and the exact destination host record.
pub struct PeerRecovery {
    device: PinnedPeer,
    record: PairingRecord,
    host: PairingRecord,
}

impl PeerRecovery {
    /// Both records must come from trusted custody, never unauthenticated network
    /// discovery. Existing claims must name this device and the same owner.
    /// A revoked device can only revoke an exact existing reciprocal grant.
    pub fn new(
        device: PinnedPeer,
        record: PairingRecord,
        host: &PairingRecord,
    ) -> Result<Self, Error> {
        let policy = record.policy().ok_or(Error::Input)?;
        if record.local_principal() != device.principal().fingerprint()
            || policy.peer().principal().fingerprint() != host.local_principal()
        {
            return Err(Error::Input);
        }
        match host.policy() {
            Some(prior)
                if prior.peer().principal() == device.principal()
                    && prior.owner() == policy.owner()
                    && (host.enabled() != record.enabled()
                        || prior.resource_policy() != policy.resource_policy())
                    && (record.enabled()
                        || prior.resource_policy() == policy.resource_policy()) => {}
            None if record.enabled() && !host.enabled() => {}
            _ => return Err(Error::Input),
        }
        Ok(Self {
            device,
            record,
            host: host.clone(),
        })
    }

    /// Require a fresh host challenge and approval by the device's current owner.
    /// The resulting ordinary host-pair request binds its exact reciprocal policy.
    pub fn request(&self, challenge: &PairingChallenge, owner: [u8; 32]) -> Result<Request, Error> {
        let policy = self.record.policy().ok_or(Error::Input)?;
        if policy.owner() != &owner
            || challenge.device().principal().fingerprint() != self.host.local_principal()
            || challenge.expected()
                != (
                    *self.host.store(),
                    self.host.revision(),
                    self.host.commitment(),
                )
        {
            return Err(Error::Input);
        }
        let reciprocal =
            EnrollmentPolicy::new(self.device.clone(), owner, *policy.resource_policy())
                .map_err(|_| Error::Input)?;
        Request::for_pairing(
            challenge,
            &reciprocal,
            if self.record.enabled() {
                Action::Pair
            } else {
                Action::Revoke
            },
        )
    }

    /// Consume the hardware-approved request only after inspecting the physical
    /// device again. Any changed device/host record or transaction refuses before
    /// storage. An uncertain host write still requires verified store recovery.
    pub fn commit(
        &self,
        request: &Request,
        challenge: &PairingChallenge,
        observed: (&PinnedPeer, &PairingRecord),
        host: &mut LocalPairing,
        now_ms: u64,
        commit: impl FnOnce(&PairingRecord, &PairingRecord) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if observed.0.principal() != self.device.principal()
            || observed.1.commitment() != self.record.commitment()
            || host.checkpoint().commitment() != self.host.commitment()
            || request.commitment()? != self.request(challenge, request.owner)?.commitment()?
        {
            return Err(Error::Input);
        }
        if self.record.enabled() {
            host.pair(challenge, request.pairing_policy()?, now_ms, commit)
                .map_err(|_| Error::Persistence)
        } else {
            host.revoke(challenge, &request.owner, now_ms, commit)
                .map_err(|_| Error::Persistence)
        }
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
