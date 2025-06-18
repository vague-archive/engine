use std::{
    fmt::{Debug, Formatter, Result},
    mem::transmute,
    num::{NonZero, NonZeroU32},
};

#[cfg(feature = "state_snapshots")]
mod serialize;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(C)]
pub struct EntityId {
    pub id: NonZeroU32,
    pub lifecycle: u32,
}

impl Debug for EntityId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.id)
    }
}

impl EntityId {
    pub const MIN: Self = Self {
        id: unsafe { NonZeroU32::new_unchecked(1) },
        lifecycle: 0,
    };

    pub const MAX: Self = Self {
        id: unsafe { NonZeroU32::new_unchecked(u32::MAX) },
        lifecycle: 0,
    };

    #[track_caller]
    pub fn new(id: u32, lifecycle: u32) -> Self {
        #[cfg(debug_assertions)]
        return Self {
            id: NonZeroU32::new(id).expect("EntityId may not be 0"),
            lifecycle,
        };

        #[cfg(not(debug_assertions))]
        return Self {
            id: unsafe { NonZeroU32::new_unchecked(id) },
            lifecycle,
        };
    }

    #[inline]
    pub fn as_index(&self) -> usize {
        self.id.get().try_into().unwrap()
    }
}

impl From<void_public::EntityId> for EntityId {
    #[inline]
    fn from(value: void_public::EntityId) -> Self {
        unsafe { transmute(value) }
    }
}

impl From<EntityId> for void_public::EntityId {
    fn from(value: EntityId) -> Self {
        unsafe { transmute(value) }
    }
}

impl From<NonZero<u64>> for EntityId {
    #[inline]
    fn from(value: NonZero<u64>) -> Self {
        unsafe { transmute(value) }
    }
}

impl From<EntityId> for NonZero<u64> {
    fn from(value: EntityId) -> Self {
        unsafe { transmute(value) }
    }
}

impl TryFrom<u64> for EntityId {
    type Error = <NonZero<u64> as TryFrom<u64>>::Error;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        NonZero::try_from(value).map(EntityId::from)
    }
}

pub enum ParentType {
    Parent(EntityId),
    Root,
}
