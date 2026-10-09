use static_assertions::{const_assert, const_assert_eq};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
#[repr(transparent)]
pub struct BlockId(u16);

// invariants
const_assert!(BlockId::INVALID.into_raw() < BlockId::MIN_VALID.into_raw());
const_assert!(BlockId::MIN_VALID.into_raw() < BlockId::MAX.into_raw());
const_assert_eq!(BlockId::MAX.next().into_raw(), BlockId::INVALID.into_raw());
const_assert_eq!(
    BlockId::INVALID.next().into_raw(),
    BlockId::INVALID.into_raw()
);

impl BlockId {
    pub const INVALID: Self = Self(0);
    pub const MIN_VALID: Self = Self(1);
    pub const MAX: Self = Self(u16::MAX);

    #[inline]
    pub const fn next(self) -> Self {
        if self.0 == 0u16 || self.0 == u16::MAX {
            Self::INVALID
        } else {
            Self(self.0 + 1)
        }
    }

    #[inline]
    pub const fn from_raw(u: u16) -> Self {
        Self(u)
    }

    #[inline]
    pub const fn into_raw(self) -> u16 {
        self.0
    }
}

impl From<BlockId> for usize {
    #[inline]
    fn from(value: BlockId) -> Self {
        value.into_raw() as usize
    }
}

impl From<u16> for BlockId {
    #[inline]
    fn from(value: u16) -> Self {
        BlockId::from_raw(value)
    }
}

impl From<BlockId> for u16 {
    #[inline]
    fn from(value: BlockId) -> Self {
        value.into_raw()
    }
}
