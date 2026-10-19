//! Complete an unpaired host after a device committed but its reply was lost.
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
        let prior = state.checkpoint().clone();
        state
            .pair(challenge, policy, now_ms, |_, _| Ok::<_, Error>(()))
            .map_err(|_| Error::Expired)?;
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

/// One immutable device snapshot and the exact unpaired destination host record.
pub struct PeerRecovery {
    device: PinnedPeer,
    record: PairingRecord,
    host: PairingRecord,
}

impl PeerRecovery {
    /// Both records must come from trusted custody, never unauthenticated network
    /// discovery. Refuse a revoked device, another host, or any existing host claim.
    pub fn new(
        device: PinnedPeer,
        record: PairingRecord,
        host: &PairingRecord,
    ) -> Result<Self, Error> {
        let policy = record.policy().ok_or(Error::Input)?;
        if !record.enabled()
            || record.local_principal() != device.principal().fingerprint()
            || policy.peer().principal().fingerprint() != host.local_principal()
            || host.policy().is_some()
            || host.enabled()
        {
            return Err(Error::Input);
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
        Request::for_pairing(challenge, &reciprocal, Action::Pair)
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
        host.pair(challenge, request.pairing_policy()?, now_ms, commit)
            .map_err(|_| Error::Persistence)
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
