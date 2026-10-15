/// Fixed EDHOC method-3 profile with pinned CCS credentials and mandatory M4.
///
/// The choice is local policy, never inferred from an unauthenticated error.
/// Both profiles derive the RFC 8613 AES-CCM-16-64-128 OSCORE context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CipherSuite {
    /// Suite 2: eight-byte EDHOC MACs and AES-CCM authentication tags.
    Suite2,
    /// Suite 3: sixteen-byte EDHOC MACs and AES-CCM authentication tags.
    Suite3,
}

impl CipherSuite {
    pub(super) const fn label(self) -> u8 {
        match self {
            Self::Suite2 => 2,
            Self::Suite3 => 3,
        }
    }

    pub(super) const fn tag_len(self) -> usize {
        match self {
            Self::Suite2 => 8,
            Self::Suite3 => 16,
        }
    }

    pub(super) fn selected(suites: &[u8]) -> Option<Self> {
        let (&selected, preferred) = suites.split_last()?;
        if preferred.iter().any(|suite| matches!(suite, 2 | 3)) {
            return None;
        }
        match selected {
            2 => Some(Self::Suite2),
            3 => Some(Self::Suite3),
            _ => None,
        }
    }

    pub(super) const fn lakers(self) -> super::lakers::EDHOCSuite {
        match self {
            Self::Suite2 => super::lakers::EDHOCSuite::CipherSuite2,
            Self::Suite3 => super::lakers::EDHOCSuite::CipherSuite3,
        }
    }
}
