use std::{
    error::Error,
    ffi::c_char,
    fmt::Display,
    num::NonZero,
    ops::{Deref, DerefMut},
};

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

impl From<crate::event::graphics::TextAlignment> for TextAlignment {
    fn from(value: crate::event::graphics::TextAlignment) -> Self {
        match value {
            crate::event::graphics::TextAlignment::Left => TextAlignment::Left,
            crate::event::graphics::TextAlignment::Center => TextAlignment::Center,
            crate::event::graphics::TextAlignment::Right => TextAlignment::Right,
            _ => unreachable!(),
        }
    }
}

#[repr(transparent)]
#[derive(
    Clone,
    Copy,
    Debug,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Deserialize,
    serde::Serialize,
    snapshot::Deserialize,
    snapshot::Serialize,
)]
pub struct TextId(pub NonZero<u32>);

impl TryFrom<u32> for TextId {
    type Error = Box<dyn Error + Send + Sync>;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        NonZero::new(value)
            .ok_or("Value for TextId is 0, which is not allowed".into())
            .map(TextId)
    }
}

impl Deref for TextId {
    type Target = NonZero<u32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TextId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<NonZero<u32>> for TextId {
    fn from(value: NonZero<u32>) -> Self {
        Self(value)
    }
}

impl Display for TextId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A Hash value of the current version of a text
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Pod, Zeroable)]
pub struct TextHash([u8; 8]);

impl<'a, I: Iterator<Item = &'a u8>> From<I> for TextHash {
    fn from(value: I) -> Self {
        let mut array = [0; 8];
        for (index, byte) in value.into_iter().enumerate().take(8) {
            array[index] = *byte;
        }
        Self(array)
    }
}

impl Deref for TextHash {
    type Target = [u8; 8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for TextHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl TextHash {
    pub const fn create_empty() -> TextHash {
        Self([0; 8])
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextType {
    Pending,
    Engine,
    Loaded,
    Failed,
}

#[repr(C)]
#[derive(Debug)]
pub struct PendingText {
    pub text_path: *const c_char,
    pub id: TextId,
    pub set_up_watcher: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct EngineText {
    pub text_path: *const c_char,
    pub format: *const c_char,
    pub raw_text: *const c_char,
    pub id: TextId,
}

#[repr(C)]
#[derive(Debug)]
pub struct LoadedText {
    pub version: TextHash,
    pub text_path: *const c_char,
    pub format: *const c_char,
    pub raw_text: *const c_char,
    pub id: TextId,
    pub watcher_set_up: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct FailedText {
    pub failure_reason: *const c_char,
    pub text_path: *const c_char,
    pub id: TextId,
}

#[repr(u32)]
#[derive(Debug)]
pub enum CreatePendingTextStatus {
    Success,
    OutputPendingTextNull,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextTypeByIdStatus {
    Success,
    TextAssetManagerNull,
    TextTypeNull,
    TextIdNotFound,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextByIdStatus {
    Success,
    TextAssetManagerNull,
    OutputTextNull,
    TextIdNotFound,
    TextTypeIncorrect,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextTypeByPathStatus {
    Success,
    TextPathNull,
    TextAssetManagerNull,
    TextTypeNull,
    TextPathNotFound,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextByPathStatus {
    Success,
    TextPathNull,
    TextAssetManagerNull,
    OutputTextNull,
    TextIdNotFound,
    TextTypeIncorrect,
}

#[repr(u32)]
#[derive(Debug)]
pub enum LoadTextStatus {
    Success,
    TextAssetManagerNull,
    OutputPendingTextNull,
    LoadTextError,
}

#[repr(u32)]
#[derive(Debug)]
pub enum LoadTextByPendingTextStatus {
    Success,
    PendingTextNull,
    TextAssetManagerNull,
    LoadTextError,
}
