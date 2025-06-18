use std::{
    error::Error,
    ffi::c_char,
    fmt::Display,
    num::NonZero,
    ops::{Deref, DerefMut},
    str::{Utf8Error, from_utf8},
};

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

use crate::{
    AssetId, Component, ComponentId, EcsType, linalg,
    serialize::{
        default_circle_render_num_sides, default_rect_dimensions, default_rect_position,
        default_text_render_alignment, default_text_render_font_size, default_true,
        deserialize_text_render_text_field,
    },
    text::TextAlignment,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub position: Vec2,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendTypes {
    PremultipliedBlendSourceOver,
    Copy,
    UnPremultipliedBlend,
    AdditiveLighten,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    SourceIn,
    SourceOut,
    SourceAtop,
}

#[repr(C)]
#[derive(Component, Debug, bytemuck::Pod, bytemuck::Zeroable, serde::Deserialize)]
pub struct Rect {
    #[serde(default = "default_rect_position")]
    pub position: linalg::Vec2,
    #[serde(default = "default_rect_dimensions")]
    pub dimensions: linalg::Vec2,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect {
            position: linalg::Vec2::from_xy(x, y),
            dimensions: linalg::Vec2::from_xy(w, h),
        }
    }
}

impl Default for Rect {
    fn default() -> Self {
        Rect {
            position: linalg::Vec2::from_xy(0., 0.),
            dimensions: linalg::Vec2::from_xy(1., 1.),
        }
    }
}

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct TextureRender {
    #[serde(skip_deserializing)]
    pub texture_id: TextureId,

    // `uv_region` is the normalized uv rectangle when texturing this Sprite.  Used for spritesheets, animation, etc.
    #[serde(default)]
    pub uv_region: Rect,

    #[serde(default = "default_true")]
    pub visible: bool,
}

impl TextureRender {
    pub fn new(texture_id: TextureId) -> Self {
        Self {
            texture_id,
            uv_region: Rect::default(),
            visible: true,
        }
    }
}

impl Default for TextureRender {
    fn default() -> Self {
        Self {
            texture_id: TextureId::default(),
            uv_region: Rect::default(),
            visible: true,
        }
    }
}

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct ColorRender {
    #[serde(default = "default_true")]
    pub visible: bool,
}

impl Default for ColorRender {
    fn default() -> Self {
        Self { visible: true }
    }
}

pub(crate) const TEXT_RENDER_SIZE: usize = 256;

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct TextRender {
    #[serde(deserialize_with = "deserialize_text_render_text_field")]
    pub text: [u8; TEXT_RENDER_SIZE],
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub bounds_size: crate::linalg::Vec2,
    #[serde(default = "default_text_render_font_size")]
    pub font_size: f32,
    #[serde(default = "default_text_render_alignment")]
    pub alignment: TextAlignment,
}

impl Default for TextRender {
    fn default() -> Self {
        Self {
            text: [0; TEXT_RENDER_SIZE],
            visible: true,
            bounds_size: Default::default(),
            font_size: 1.0,
            alignment: Default::default(),
        }
    }
}

impl TextRender {
    pub fn new(string: &str, font_size: f32) -> Self {
        Self {
            text: Self::str_to_u8array::<TEXT_RENDER_SIZE>(string),
            font_size,
            ..Default::default()
        }
    }

    pub fn str_to_u8array<const N: usize>(string: &str) -> [u8; N] {
        let mut output_array = [0; N];
        string
            .as_bytes()
            .iter()
            .take(N)
            .enumerate()
            .for_each(|(index, byte)| output_array[index] = *byte);

        output_array
    }

    // TODO Modify components dealing with text to have a length so we aren't trimming \0s.
    // <https://github.com/vaguevoid/engine/issues/359>
    pub fn u8array_to_str(u8_slice: &[u8]) -> Result<&str, Box<dyn Error + Send + Sync>> {
        from_utf8(u8_slice)
            .map(|str| str.trim_matches('\0'))
            .map_err(|err| err.into())
    }

    pub fn get_text(&self) -> Result<&str, Utf8Error> {
        from_utf8(&self.text).map(|str| str.trim_matches('\0'))
    }
}

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct CircleRender {
    #[serde(default = "default_circle_render_num_sides")]
    pub num_sides: u32,
    #[serde(default = "default_true")]
    pub visible: bool,
}

impl Default for CircleRender {
    fn default() -> Self {
        Self {
            num_sides: 32,
            visible: true,
        }
    }
}

impl CircleRender {
    pub fn new(num_sides: u32) -> Self {
        Self {
            num_sides,
            ..Default::default()
        }
    }
}

/// A Hash value of the current version of a texture
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Pod, Zeroable)]
pub struct TextureHash([u8; 8]);

impl<'a, I: Iterator<Item = &'a u8>> From<I> for TextureHash {
    fn from(value: I) -> Self {
        let mut array = [0; 8];
        for (index, byte) in value.into_iter().enumerate().take(8) {
            array[index] = *byte;
        }
        Self(array)
    }
}

impl Deref for TextureHash {
    type Target = [u8; 8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for TextureHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl TextureHash {
    pub const fn create_empty() -> TextureHash {
        Self([0; 8])
    }
}

/// A handle identifying a texture
#[repr(transparent)]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Pod,
    Zeroable,
    serde::Deserialize,
)]
pub struct TextureId(pub u32);

impl AsRef<TextureId> for TextureId {
    fn as_ref(&self) -> &TextureId {
        self
    }
}

impl Deref for TextureId {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TextureId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for TextureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for TextureId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

// Reserved TextureIds must match order in [`TextureAssetManager::default()`].
pub const WHITE_TEXTURE_TEXTURE_ID: TextureId = TextureId(0);
pub const MISSING_TEXTURE_TEXTURE_ID: TextureId = TextureId(1);

pub type ParticleEffectHandle = Option<NonZero<u64>>;

/// A component representing a `ParticleEffect` registered with `ParticleEffectManager`.  An entity can only have
/// one `ParticleRender` at a time.
#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct ParticleRender {
    descriptor_id: AssetId,
    handle: ParticleEffectHandle,
    visible: bool,
}

pub trait ParticleManager {
    fn next_effect_handle(&mut self) -> ParticleEffectHandle;
}

impl ParticleRender {
    pub fn new(descriptor_id: AssetId, visible: bool) -> ParticleRender {
        ParticleRender {
            descriptor_id,
            handle: None,
            visible,
        }
    }

    pub fn handle(&self) -> ParticleEffectHandle {
        self.handle
    }

    pub fn set_visibility(&mut self, new_visibility: bool) {
        self.visible = new_visibility;
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn descriptor_id(&self) -> AssetId {
        self.descriptor_id
    }

    /// `init_effect` should only be used by a complete `ParticleManager` (ex: `GpuWeb::ParticleEffectManager`) that manages simulation
    /// and rendering for this component.  This is a temporary solution until we refactor `ParticleEffectManager` into
    /// more ECS-friendly systems.  See: <https://github.com/vaguevoid/engine/issues/350>
    pub fn init_effect<P: ParticleManager>(&mut self, particle_manager: &mut P) {
        self.handle = particle_manager.next_effect_handle();
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct PendingTexture {
    pub texture_path: *const c_char,
    pub id: TextureId,
    pub insert_in_atlas: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct EngineTexture {
    pub texture_path: *const c_char,
    pub id: TextureId,
    pub width: u32,
    pub height: u32,
    pub in_atlas: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct LoadedTexture {
    pub version: TextureHash,
    pub texture_path: *const c_char,
    pub format_type: *const c_char,
    pub id: TextureId,
    pub width: u32,
    pub height: u32,
    pub in_atlas: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct FailedTexture {
    pub texture_path: *const c_char,
    pub failure_reason: *const c_char,
    pub id: TextureId,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureType {
    Pending,
    Engine,
    Loaded,
    Failed,
}

#[repr(u32)]
#[derive(Debug)]
pub enum CreatePendingTexture {
    Success,
    OutputPendingTextureNull,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextureTypeByIdStatus {
    Success,
    TextureAssetManagerNull,
    TextureTypeNull,
    TextureIdNotFound,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextureByIdStatus {
    Success,
    TextureAssetManagerNull,
    OutputTextureNull,
    TextureIdNotFound,
    TextureTypeIncorrect,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextureTypeByPathStatus {
    Success,
    TexturePathNull,
    TextureAssetManagerNull,
    TextureTypeNull,
    TexturePathNotFound,
}

#[repr(u32)]
#[derive(Debug)]
pub enum GetTextureByPathStatus {
    Success,
    TexturePathNull,
    TextureAssetManagerNull,
    OutputTextureNull,
    TextureIdNotFound,
    TextureTypeIncorrect,
}

#[repr(u32)]
#[derive(Debug)]
pub enum LoadTextureStatus {
    Success,
    TextureAssetManagerNull,
    OutputPendingTextureNull,
    LoadTextureError,
}

#[repr(u32)]
#[derive(Debug)]
pub enum LoadTextureByPendingTextureStatus {
    Success,
    PendingTextureNull,
    TextureAssetManagerNull,
    LoadTextureError,
}
