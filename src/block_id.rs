pub trait SeqId: PartialOrd + Ord + Into<usize> {
    const MIN: Self;
    const MAX: Self;
    fn next(self) -> Self;
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy, Hash)]
#[repr(transparent)]
pub struct BlockId(u16);

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy, Hash)]
#[repr(transparent)]
pub struct StaticId(u16);

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy, Hash)]
#[repr(transparent)]
pub struct DynId(u16);

#[derive(Debug, Clone, Copy)]
pub enum BlockIdVariants {
    Invalid,
    Static(StaticId),
    Dyn(DynId),
}

impl BlockId {
    pub const INVALID: Self = Self(0);
    pub fn into_specialized(self) -> BlockIdVariants {
        if self.0 == 0 {
            BlockIdVariants::Invalid
        } else if self.0 <= StaticId::MAX.0 {
            BlockIdVariants::Static(StaticId(self.0))
        } else {
            BlockIdVariants::Dyn(DynId(self.0))
        }
    }
}

impl From<BlockId> for usize {
    fn from(value: BlockId) -> Self {
        value.0 as usize
    }
}
impl From<StaticId> for usize {
    fn from(value: StaticId) -> Self {
        value.0 as usize
    }
}
impl From<DynId> for usize {
    fn from(value: DynId) -> Self {
        value.0 as usize
    }
}

impl From<u16> for BlockId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<BlockId> for u16 {
    fn from(value: BlockId) -> Self {
        value.0
    }
}
impl From<StaticId> for u16 {
    fn from(value: StaticId) -> Self {
        value.0
    }
}
impl From<DynId> for u16 {
    fn from(value: DynId) -> Self {
        value.0
    }
}

impl SeqId for StaticId {
    const MAX: Self = Self(32767);
    const MIN: Self = Self(1);
    fn next(self) -> Self {
        if self == Self::MAX {
            panic!("overflow")
        };
        Self(self.0 + 1)
    }
}

impl SeqId for DynId {
    const MAX: Self = Self(65535);
    const MIN: Self = Self(32768);
    fn next(self) -> Self {
        if self == Self::MAX {
            panic!("overflow")
        };
        Self(self.0 + 1)
    }
}

impl SeqId for usize {
    const MIN: Self = usize::MIN;
    const MAX: Self = usize::MAX;
    fn next(self) -> Self {
        if self == Self::MAX {
            panic!("overflow")
        };
        self + 1
    }
}

impl From<StaticId> for BlockId {
    fn from(value: StaticId) -> Self {
        Self(value.0)
    }
}
impl From<DynId> for BlockId {
    fn from(value: DynId) -> Self {
        Self(value.0)
    }
}
