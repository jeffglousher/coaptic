//! Builder and capacity errors.

/// Failure to construct an [`Engine`](crate::Engine).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildError {
    /// Builder sizes do not match the [`Storage`](crate::Storage) being moved in.
    SizeMismatch,
    /// Enabled body slot bytes are not a multiple of 1024 (max Block/Q-Block SZX).
    BodyBytesNotMultipleOf1024,
    /// `.block_wise(false)` but storage or the builder includes body pools.
    UnexpectedBodyPools,
    /// `.block_wise(true)` but storage has no body pools.
    MissingBodyPools,
    /// Enabled body slot count or bytes is zero. Zero is not “disabled.”
    ZeroBodyCapacity,
    /// Some body capacity fields are set and others are not.
    IncompleteBodyCapacities,
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SizeMismatch => f.write_str("storage sizes do not match the builder"),
            Self::BodyBytesNotMultipleOf1024 => {
                f.write_str("body slot bytes must be a multiple of 1024")
            }
            Self::UnexpectedBodyPools => {
                f.write_str("body pools are present but block-wise is disabled")
            }
            Self::MissingBodyPools => {
                f.write_str("block-wise is enabled but storage has no body pools")
            }
            Self::ZeroBodyCapacity => {
                f.write_str("enabled body capacity must not be zero slots or zero bytes")
            }
            Self::IncompleteBodyCapacities => {
                f.write_str("body capacity fields must be all set or all absent")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BuildError {}
