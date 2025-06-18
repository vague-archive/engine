use std::{
    cmp::Ordering,
    ops::{Deref, DerefMut},
};

const DEFAULT_SIZE: usize = 4;

/// The intent of this struct is to force a fixed size slice of data that is determined at runtime.
/// In this case, it is for fixed size arrays of uniform values
#[derive(Debug, Clone)]
pub struct FixedSizeVec<T: Copy>(pub Box<[T]>);

impl<T: Copy> FixedSizeVec<T> {
    pub fn new(data: &[T]) -> Self {
        Self(data.into())
    }
}

impl<T: Copy + Default> Default for FixedSizeVec<T> {
    fn default() -> Self {
        Self::new(&[T::default(); DEFAULT_SIZE])
    }
}

impl<T: Copy + Default> FixedSizeVec<T> {
    pub fn new_empty(len: usize) -> Self {
        Self(vec![T::default(); len].into())
    }
}

impl<T: Copy + PartialEq> PartialEq for FixedSizeVec<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl<T: Copy + PartialOrd> PartialOrd for FixedSizeVec<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (*self.0).iter().partial_cmp(&(*other.0)) {
            Some(Ordering::Equal) => Some(Ordering::Equal),
            ord => ord,
        }
    }
}

impl<T: Copy> Deref for FixedSizeVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Copy> DerefMut for FixedSizeVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
