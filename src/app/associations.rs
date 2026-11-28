//! Bounded association ownership; packet and body pools remain shared by App.

use crate::message::{OwnedToken, Token};
use crate::storage::Endpoint;

#[cfg(feature = "oscore")]
use crate::oscore::BoundedSecurityContext;

/// Identity of an installed association in its owning App.
///
/// The owner and epoch come from authenticated provisioning policy. This handle
/// is local routing metadata, not proof of authentication or authorization.
/// Handles belong to the App that issued them and must not survive its restart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Association {
    pub(super) route: Endpoint,
    pub(super) owner: [u8; 32],
    pub(super) epoch: u64,
}

impl Association {
    /// Exact UDP endpoint bound at installation.
    pub const fn endpoint(self) -> Endpoint {
        self.route.physical()
    }

    /// Authenticated owner supplied at installation.
    pub const fn owner(&self) -> &[u8; 32] {
        &self.owner
    }

    /// Caller-owned cryptographic/policy epoch, not the local slot index.
    pub const fn epoch(self) -> u64 {
        self.epoch
    }
}

#[cfg(feature = "oscore")]
pub(super) struct Entry<const REQUESTS: usize, K: OwnedToken = Token> {
    pub context: BoundedSecurityContext<REQUESTS, K>,
    pub association: Option<Association>,
    pub revoked: bool,
}

// Holds the selected slot exclusively while the caller moves the already
// checked context. Dropping it leaves the slot, generation and checkpoint alone.
#[cfg(feature = "oscore")]
pub(crate) struct VacantAssociation<'a, const R: usize, K: OwnedToken> {
    row: &'a mut Option<Entry<R, K>>,
    generation: &'a mut u32,
    checkpoint: &'a mut super::oscore::CheckpointState,
    association: Association,
}

pub(crate) struct Store<const REQUESTS: usize, const ASSOCIATIONS: usize, K: OwnedToken = Token> {
    #[cfg(feature = "oscore")]
    pub(super) rows: [Option<Entry<REQUESTS, K>>; ASSOCIATIONS],
    #[cfg(feature = "oscore")]
    pub(super) generation: u32,
    marker: core::marker::PhantomData<K>,
}

impl<const R: usize, const A: usize, K: OwnedToken> Store<R, A, K> {
    #[cfg(feature = "oscore")]
    pub fn is_some(&self) -> bool {
        self.generation != 0 || self.rows.first().is_some_and(Option::is_some)
    }
    // Until a managed association is installed, the only possible entry is
    // the primary in slot zero. A nonzero generation survives removal.
    #[cfg(feature = "oscore")]
    fn lookup_len(&self) -> usize {
        if self.generation == 0 { A.min(1) } else { A }
    }

    #[cfg(feature = "oscore")]
    fn lookup_rows(&self) -> &[Option<Entry<R, K>>] {
        &self.rows[..self.lookup_len()]
    }

    pub const fn new() -> Self {
        Self {
            #[cfg(feature = "oscore")]
            rows: [const { None }; A],
            #[cfg(feature = "oscore")]
            generation: 0,
            marker: core::marker::PhantomData,
        }
    }

    #[cfg(feature = "oscore")]
    pub(super) fn vacant<'a, U: OwnedToken>(
        &'a mut self,
        endpoint: Endpoint,
        owner: [u8; 32],
        epoch: u64,
        context: &BoundedSecurityContext<R, U>,
        checkpoint: &'a mut super::oscore::CheckpointState,
    ) -> Result<VacantAssociation<'a, R, K>, super::OscoreInstallError> {
        use super::OscoreInstallError as E;
        let physical = endpoint.physical();
        let mut vacant = None;
        for row in &mut self.rows {
            let Some(entry) = row else {
                vacant.get_or_insert(row);
                continue;
            };
            // A primary can only occupy slot zero, so Busy precedes conflicts.
            let association = entry.association.as_ref().ok_or(E::Busy)?;
            if (association.owner == owner && association.epoch == epoch)
                || (association.endpoint() == physical
                    && entry.context.recipient_id() == context.recipient_id()
                    && entry.context.id_context() == context.id_context())
            {
                return Err(E::Ambiguous);
            }
        }
        let row = vacant.ok_or(E::Capacity)?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(E::GenerationExhausted)?;
        Ok(VacantAssociation {
            row,
            generation: &mut self.generation,
            checkpoint,
            association: Association {
                route: physical.with_association(generation),
                owner,
                epoch,
            },
        })
    }

    #[cfg(feature = "oscore")]
    pub fn set_primary(&mut self, context: BoundedSecurityContext<R, K>) {
        assert!(A > 0, "OSCORE requires association capacity");
        self.rows[0] = Some(Entry {
            context,
            association: None,
            revoked: false,
        });
    }

    #[cfg(feature = "oscore")]
    pub fn as_ref(&self) -> Option<&BoundedSecurityContext<R, K>> {
        self.rows
            .first()?
            .as_ref()
            .filter(|row| row.association.is_none() && !row.revoked)
            .map(|row| &row.context)
    }

    #[cfg(feature = "oscore")]
    pub fn as_mut(&mut self) -> Option<&mut BoundedSecurityContext<R, K>> {
        self.rows
            .first_mut()?
            .as_mut()
            .filter(|row| row.association.is_none() && !row.revoked)
            .map(|row| &mut row.context)
    }
}

pub(crate) trait ContextStorage<const REQUESTS: usize, K: OwnedToken = Token> {
    #[cfg(feature = "oscore")]
    fn context(&self, namespace: u32) -> Option<&BoundedSecurityContext<REQUESTS, K>>;
    #[cfg(feature = "oscore")]
    fn context_mut(&mut self, namespace: u32) -> Option<&mut BoundedSecurityContext<REQUESTS, K>>;
    #[cfg(feature = "oscore")]
    fn resolve(&self, peer: Endpoint) -> Result<Endpoint, crate::oscore::Error>;
    #[cfg(feature = "oscore")]
    fn inbound_route(
        &self,
        peer: Endpoint,
        parsed: &crate::message::ParsedMessage<'_, K>,
    ) -> Result<Endpoint, crate::oscore::Error>;
    #[cfg(feature = "oscore")]
    fn has_live_requests(&self) -> bool;
    #[cfg(feature = "oscore")]
    fn checkpoints(&self, commit: &mut super::oscore::CheckpointCommit<'_>) -> bool;
    fn association(&self, peer: Endpoint) -> Option<Association>;
    #[cfg(feature = "oscore")]
    fn is_some(&self) -> bool;
}

impl<const R: usize, const A: usize, K: OwnedToken> ContextStorage<R, K> for Store<R, A, K> {
    #[cfg(feature = "oscore")]
    fn context(&self, namespace: u32) -> Option<&BoundedSecurityContext<R, K>> {
        self.lookup_rows()
            .iter()
            .flatten()
            .find(|row| {
                !row.revoked && row.association.map_or(0, |a| a.route.association()) == namespace
            })
            .map(|row| &row.context)
    }
    #[cfg(feature = "oscore")]
    fn context_mut(&mut self, namespace: u32) -> Option<&mut BoundedSecurityContext<R, K>> {
        let len = self.lookup_len();
        self.rows[..len]
            .iter_mut()
            .flatten()
            .find(|row| {
                !row.revoked && row.association.map_or(0, |a| a.route.association()) == namespace
            })
            .map(|row| &mut row.context)
    }
    #[cfg(feature = "oscore")]
    fn resolve(&self, peer: Endpoint) -> Result<Endpoint, crate::oscore::Error> {
        if !self.is_some() && self.generation == 0 {
            return Ok(peer.physical());
        }
        let mut found = None;
        for row in self
            .lookup_rows()
            .iter()
            .flatten()
            .filter(|row| !row.revoked)
        {
            let route = row.association.map_or(peer.physical(), |a| a.route);
            if route.physical() == peer.physical()
                && (peer.association() == 0 || peer.association() == route.association())
                && found.replace(route).is_some()
            {
                return Err(crate::oscore::Error::Context);
            }
        }
        found.ok_or(crate::oscore::Error::Context)
    }
    #[cfg(feature = "oscore")]
    fn inbound_route(
        &self,
        peer: Endpoint,
        parsed: &crate::message::ParsedMessage<'_, K>,
    ) -> Result<Endpoint, crate::oscore::Error> {
        use crate::oscore::{Error, OscoreHeader};
        if !self.is_some() && self.generation == 0 {
            return Ok(peer.physical());
        }
        if self
            .lookup_rows()
            .iter()
            .flatten()
            .any(|row| !row.revoked && row.association.is_none())
        {
            return Ok(peer.physical());
        }
        let mut options = parsed.get_options(crate::message::OptionNumber::OSCORE);
        let option = options.next();
        if options.next().is_some() {
            return Err(Error::Header);
        }
        let header = option
            .map(|value| OscoreHeader::parse(value.value()))
            .transpose()?;
        let mut found = None;
        for row in self
            .lookup_rows()
            .iter()
            .flatten()
            .filter(|row| !row.revoked)
        {
            let route = row.association.map_or(peer.physical(), |a| a.route);
            if route.physical() != peer.physical() {
                continue;
            }
            let matches = if parsed.code().is_request() {
                header
                    .as_ref()
                    .is_some_and(|h| row.context.matches_recipient(h.kid, h.kid_context))
                    || (header.is_none() && row.association.is_none())
            } else {
                row.context.lookup(parsed.token()).is_some()
            };
            if matches && found.replace(route).is_some() {
                return Err(Error::Context);
            }
        }
        found.ok_or(Error::Context)
    }
    #[cfg(feature = "oscore")]
    fn has_live_requests(&self) -> bool {
        self.lookup_rows()
            .iter()
            .flatten()
            .any(|row| row.context.has_live_requests())
    }
    #[cfg(feature = "oscore")]
    fn checkpoints(&self, commit: &mut super::oscore::CheckpointCommit<'_>) -> bool {
        self.lookup_rows()
            .iter()
            .flatten()
            .all(|row| commit(row.association, row.context.replay_checkpoint()))
    }
    fn association(&self, peer: Endpoint) -> Option<Association> {
        #[cfg(feature = "oscore")]
        {
            self.lookup_rows()
                .iter()
                .flatten()
                .filter(|row| !row.revoked)
                .filter_map(|row| row.association)
                .find(|a| a.route == peer)
        }
        #[cfg(not(feature = "oscore"))]
        {
            let _ = peer;
            None
        }
    }
    #[cfg(feature = "oscore")]
    fn is_some(&self) -> bool {
        Store::is_some(self)
    }
}

#[cfg(feature = "oscore")]
impl<const R: usize, K: OwnedToken> ContextStorage<R, K> for Option<BoundedSecurityContext<R, K>> {
    fn context(&self, namespace: u32) -> Option<&BoundedSecurityContext<R, K>> {
        (namespace == 0).then_some(self.as_ref()).flatten()
    }
    fn context_mut(&mut self, namespace: u32) -> Option<&mut BoundedSecurityContext<R, K>> {
        if namespace == 0 { self.as_mut() } else { None }
    }
    fn resolve(&self, peer: Endpoint) -> Result<Endpoint, crate::oscore::Error> {
        Ok(peer)
    }
    fn inbound_route(
        &self,
        peer: Endpoint,
        _: &crate::message::ParsedMessage<'_, K>,
    ) -> Result<Endpoint, crate::oscore::Error> {
        Ok(peer)
    }
    fn has_live_requests(&self) -> bool {
        self.as_ref().is_some_and(|ctx| ctx.has_live_requests())
    }
    fn checkpoints(&self, commit: &mut super::oscore::CheckpointCommit<'_>) -> bool {
        self.as_ref()
            .is_some_and(|ctx| commit(None, ctx.replay_checkpoint()))
    }
    fn association(&self, _: Endpoint) -> Option<Association> {
        None
    }
    fn is_some(&self) -> bool {
        self.is_some()
    }
}

#[cfg(feature = "oscore")]
impl<const R: usize, K: OwnedToken> VacantAssociation<'_, R, K> {
    pub(crate) fn insert(self, context: BoundedSecurityContext<R, K>) -> Association {
        *self.row = Some(Entry {
            context,
            association: Some(self.association),
            revoked: false,
        });
        *self.generation = self.association.route.association();
        self.checkpoint.invalidate();
        self.association
    }
}
