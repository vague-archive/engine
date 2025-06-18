//! This library serves two roles.
//!
//! It is the Rust module API to the engine, wrapping the engine's C module API
//! in Rust interfaces. For example, `struct Engine` wraps the callable
//! functions on the engine, while `struct Query` provides an abstraction over
//! querying ECS data.
//!
//! It also contains the type definitions of the `void_public` module. These
//! types include common components and resources, such as `Transform` and
//! `InputState`.
//!
//! The Rust module API and the `void_public` type definitions will likely be
//! separated into different packages in the future.
//!
//! TODO(https://github.com/vaguevoid/engine/issues/385): Separate Rust module
//! API and the `void_public` module.

use std::{
    cmp::Ordering,
    error::Error,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fmt::{Display, Formatter},
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem::{MaybeUninit, size_of},
    num::NonZero,
    ops::{Deref, DerefMut},
    panic::catch_unwind,
    path::{Path, PathBuf},
    ptr,
    slice::{self, from_raw_parts},
};

use bytemuck::{Pod, Zeroable};
use callable::{AsyncCompletion, Callable};
use flatbuffers::{FlatBufferBuilder, Follow, Push, WIPOffset};
use game_module_macro::{Component, Resource};
pub use glam::{
    Mat2, Mat3, Mat4, Quat, Vec2, Vec3, Vec4,
    swizzles::{Vec2Swizzles, Vec3Swizzles, Vec4Swizzles},
};
use serialize::{
    default_camera_aspect_ratio_override, default_camera_clear_color, default_camera_is_enabled,
    default_camera_orthographic_size, default_camera_projection_matrix,
    default_camera_render_order, default_camera_render_target_texture_id,
    default_camera_view_matrix, default_transform_pivot, default_transform_scale,
};
use snapshot::{Deserialize, Serialize};
use system::system_name_generator_c;

use crate::{
    colors::{Color, palette::BLACK},
    coordinate_systems::{
        local_to_world, screen_to_clip, screen_to_view, screen_to_world, set_world_position,
        world_position, world_to_clip, world_to_local, world_to_screen, world_to_view,
    },
};

pub mod callable;
pub mod colors;
pub mod coordinate_systems;
#[allow(clippy::all, clippy::pedantic, warnings, unused)]
pub mod event;
pub mod graphics;
pub mod input;
pub mod linalg;
pub mod material;
pub mod pipeline;
mod serialize;
pub mod system;
pub mod text;

#[macro_export]
macro_rules! concat_bytes {
    ($($s:expr),+) => {{
        $(
            const _: &[u8] = $s; // require constants
        )*
        const LEN: usize = $( $s.len() + )* 0;
        const ARR: [u8; LEN] = {
            use ::std::mem::MaybeUninit;
            let mut arr: [MaybeUninit<u8>; LEN] = [MaybeUninit::zeroed(); LEN];
            let mut base: usize = 0;
            $({
                let mut i = 0;
                while i < $s.len() {
                    arr[base + i] = MaybeUninit::new($s[i]);
                    i += 1;
                }
                base += $s.len();
            })*
            if base != LEN { panic!("invalid length"); }

            unsafe { ::std::mem::transmute(arr) }
        };
        &ARR
    }};
}

/// Returns a `&'static CStr` version of a flatbuffers event name
#[macro_export]
macro_rules! event_name {
    ($event:ident) => {
        unsafe {
            assert!($event::get_fully_qualified_name().is_ascii());
            ::std::ffi::CStr::from_bytes_with_nul_unchecked($crate::concat_bytes!(
                $event::get_fully_qualified_name().as_bytes(),
                &[0]
            ))
        }
    };
}

/// The version of Void which this module is designed to support.
pub const ENGINE_VERSION: u32 = make_api_version(0, 0, 20);

pub const fn make_api_version(major: u32, minor: u32, patch: u32) -> u32 {
    ((major) << 25) | ((minor) << 15) | (patch)
}

pub const fn api_version_major(version: u32) -> u32 {
    version >> 25
}

pub const fn api_version_minor(version: u32) -> u32 {
    (version >> 15) & !(!0 << 10)
}

pub const fn api_version_patch(version: u32) -> u32 {
    version & !(!0 << 15)
}

pub const fn api_version_compatible(version: u32) -> bool {
    api_version_major(ENGINE_VERSION) == api_version_major(version)
        && api_version_minor(ENGINE_VERSION) == api_version_minor(version)
        // consider patch version breaking until public release
        && api_version_patch(ENGINE_VERSION) == api_version_patch(version)
}

/// A handle identifying a component or resource type.
pub type ComponentId = NonZero<u16>;

/// A handle identifying a loaded asset.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Pod, Zeroable, serde::Deserialize,
)]
pub struct AssetId(pub u32);

impl Deref for AssetId {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AssetId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for AssetId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

pub trait EcsType {
    fn id() -> ComponentId;

    /// # Safety
    ///
    /// This function sets a global static. The caller must ensure that no race
    /// conditions occur.
    unsafe fn set_id(id: ComponentId);

    fn string_id() -> &'static CStr;
}

/// A trait representing an ECS Component. All structs which are to be used as
/// a Component must `#[derive(Component)]`.
pub trait Component: EcsType + Copy + Clone + Send + Sync + Sized + 'static {}

/// A trait representing an ECS Resource. All structs which are to be used as
/// a Resource must `#[derive(Resource)]`.
pub trait Resource: EcsType + Send + Sync + Sized + Serialize + Deserialize + 'static {
    fn new() -> Self;
}

/// A repr(C) compatible wrapper of Option<T>. T must implement [`Copy`]. If you
/// need [`FfiOption`] on a non [`Copy`] type, perhaps consider an alternative
/// first. Because [`Drop`] and [`Copy`] are mutually exclusive traits, it is
/// impossible to implement [`Drop`] on this [`FfiOption`], and recreating a version
/// of this struct that can implement [`Drop`] opens a risk of creating memory
/// leaks.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FfiOption<T: Copy> {
    val: MaybeUninit<T>,
    is_some: bool,
}

impl<T: Copy> FfiOption<T> {
    pub fn new(val: Option<T>) -> Self {
        Self {
            is_some: val.is_some(),
            val: val.map_or(MaybeUninit::uninit(), |value| MaybeUninit::new(value)),
        }
    }

    pub fn unwrap(self) -> T {
        match self.is_some {
            true => unsafe { self.val.assume_init_read() },
            false => panic!("called `FfiOption::unwrap()` on a `None` value"),
        }
    }

    pub fn unwrap_or(self, default: T) -> T {
        match self.is_some {
            true => unsafe { self.val.assume_init_read() },
            false => default,
        }
    }

    pub fn borrow(&self) -> Option<&T> {
        match self.is_some {
            true => Some(unsafe { self.val.assume_init_ref() }),
            false => None,
        }
    }

    pub fn borrow_mut(&mut self) -> Option<&mut T> {
        match self.is_some {
            true => Some(unsafe { self.val.assume_init_mut() }),
            false => None,
        }
    }

    pub fn set(&mut self, val: Option<T>) {
        self.is_some = val.is_some();
        self.val = val.map_or(MaybeUninit::uninit(), |value| MaybeUninit::new(value));
    }
}

impl<T: Copy> From<Option<T>> for FfiOption<T> {
    fn from(value: Option<T>) -> Self {
        Self::new(value)
    }
}

impl<T: Copy + Display> Display for FfiOption<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_some {
            write!(f, "FfiSome({})", unsafe { self.val.assume_init_ref() })
        } else {
            write!(f, "FfiNone")
        }
    }
}

impl<T: Copy + PartialEq> PartialEq for FfiOption<T> {
    fn eq(&self, other: &Self) -> bool {
        if !self.is_some && !other.is_some {
            // if both are None we are equal
            return true;
        }

        if !self.is_some || !other.is_some {
            // if one is None and the other is Some, we are not equal
            return false;
        }

        if self.is_some && other.is_some {
            unsafe { return self.val.assume_init_ref().eq(other.val.assume_init_ref()) }
        }

        false
    }
}

impl<T: Copy + PartialEq + Eq> Eq for FfiOption<T> {}

impl<T: Copy + Hash> Hash for FfiOption<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.is_some {
            unsafe { self.val.assume_init_ref().hash(state) };
        } else {
            self.is_some.hash(state);
        }
    }
}

impl<T: Copy + PartialOrd> PartialOrd for FfiOption<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if !self.is_some && !other.is_some {
            // if both are None we are equal
            return Some(Ordering::Equal);
        }

        if !self.is_some || !other.is_some {
            // if one is None and the other is not, we are not equal
            return None;
        }

        if self.is_some && other.is_some {
            unsafe {
                return self
                    .val
                    .assume_init_ref()
                    .partial_cmp(self.val.assume_init_ref());
            }
        }

        None
    }
}

/// A handle representing an entity.
#[repr(transparent)]
#[derive(
    Component,
    Debug,
    Hash,
    PartialEq,
    Eq,
    serde::Deserialize,
    snapshot::Deserialize,
    snapshot::Serialize,
)]
pub struct EntityId(NonZero<u64>);

impl EntityId {
    pub fn new(value: NonZero<u64>) -> Self {
        Self(value)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ButtonState {
    pub pressed: bool,
    pub pressed_this_frame: bool,
    pub released_this_frame: bool,
}

/// Screen position in range `[0, 1]`, where top-left is `(0, 0)`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ScreenPosition {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct Cursor {
    pub position: ScreenPosition,
    /// Change in position from last frame (applies even when the cursor is out of frame).
    pub delta_position: ScreenPosition,
}

/// A resource containing values which are constant for the whole frame.
#[repr(C)]
#[derive(Resource, Default, Clone, Copy)]
pub struct FrameConstants {
    pub delta_time: f32,
    pub frame_rate: f32,

    /// The number of times the engine update loop been invoked since the last loaded tape. If no tapes
    /// have been loaded, this is the number since the application started.
    pub tick_count: u64,
}

#[repr(C)]
#[derive(Resource, Clone, Copy)]
pub struct FrameConfig {
    /// The frame delta time will be clamped to this maximum (seconds)
    pub max_delta_time: f32,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            max_delta_time: 0.4,
        }
    }
}

/// A component representing a 3D transform.
#[repr(C)]
#[derive(Component, Debug, bytemuck::Pod, bytemuck::Zeroable, serde::Deserialize)]
pub struct Transform {
    #[serde(default)]
    pub position: linalg::Vec3,
    #[serde(default = "default_transform_scale")]
    pub scale: linalg::Vec2,
    #[serde(default)]
    pub skew: linalg::Vec2,
    #[serde(default = "default_transform_pivot")]
    pub pivot: linalg::Vec2,
    #[serde(default)]
    pub rotation: f32,
    #[serde(skip)]
    pub _padding: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Default::default(),
            rotation: 0.0,
            // <https://github.com/vaguevoid/engine/issues/311>
            // "Normal" scaling is currently 100. by 100. for sprites and 1. by
            // 1. for text. This is not ideal, and will be fixed with ^^ ticket.
            scale: linalg::Vec2::new(Vec2::new(1., 1.)),
            skew: linalg::Vec2::new(Vec2::new(0.0, 0.0)),
            pivot: linalg::Vec2::new(Vec2::new(0.5, 0.5)),
            _padding: 0.,
        }
    }
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        Self {
            position: linalg::Vec3::new(position),
            ..Default::default()
        }
    }

    pub fn from_scale_rotation_translation(
        scale: &Vec2,
        rotation: f32,
        translation: &Vec2,
    ) -> Self {
        Self {
            position: translation.extend(0.0).into(),
            scale: (*scale).into(),
            rotation,
            ..Default::default()
        }
    }

    pub fn from_rotation_translation(rotation: f32, translation: &Vec2) -> Self {
        Self {
            position: translation.extend(0.0).into(),
            rotation,
            ..Default::default()
        }
    }

    pub fn from_translation(translation: &Vec2) -> Self {
        Self {
            position: translation.extend(0.0).into(),
            ..Default::default()
        }
    }

    /// Set the world-space position of this `Transform` using the given `local_to_world` matrix
    pub fn set_world_position(&mut self, world_position: &Vec2, local_to_world: &LocalToWorld) {
        set_world_position(self, local_to_world, world_position);
    }
}

#[repr(C)]
#[derive(Component, Debug, bytemuck::Pod, bytemuck::Zeroable, serde::Deserialize)]
pub struct LocalToWorld(linalg::Mat4);

impl Default for LocalToWorld {
    fn default() -> Self {
        Self(linalg::Mat4::new(Mat4::IDENTITY))
    }
}

impl Display for LocalToWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in 0..4 {
            writeln!(f, "{}", self.row(row as usize))?;
        }
        Ok(())
    }
}

impl Deref for LocalToWorld {
    type Target = linalg::Mat4;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LocalToWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<linalg::Mat4> for LocalToWorld {
    fn from(value: linalg::Mat4) -> Self {
        Self(value)
    }
}

impl From<LocalToWorld> for linalg::Mat4 {
    fn from(value: LocalToWorld) -> Self {
        value.0
    }
}

impl From<Mat4> for LocalToWorld {
    fn from(value: Mat4) -> Self {
        Self(linalg::Mat4::from(value))
    }
}

impl From<LocalToWorld> for Mat4 {
    fn from(value: LocalToWorld) -> Self {
        *value.0
    }
}

impl LocalToWorld {
    /// Convert the given `world-position` into the local coordinate of this entity.
    #[inline]
    pub fn world_to_local(&self, world_position: &Vec2) -> Vec2 {
        world_to_local(world_position, self)
    }

    /// Convert the given `local_position` of this entity into world-space.
    #[inline]
    pub fn local_to_world(&self, local_position: &Vec2) -> Vec2 {
        local_to_world(local_position, self)
    }

    /// Returns the world-space position of this entity.
    #[inline]
    pub fn world_position(&self) -> Vec2 {
        world_position(self)
    }
}

#[derive(Debug, Copy, Clone, serde::Deserialize)]
pub struct Viewport {
    /// A normalized value indicating the start x position of the viewport relative to the window.
    pub x: f32,
    /// A normalized value indicating the start y position of the viewport relative to the window.
    pub y: f32,
    /// A normalized value indicating the percentage of the window width this viewport represents.
    pub width: f32,
    /// A normalized value indicating the percentage of the window height this viewport represents.
    pub height: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl Viewport {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A component representing a 3D camera.
#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct Camera {
    /// The matrix that converts from world-space to camera space (view-space)
    #[serde(default = "default_camera_view_matrix")]
    pub view_matrix: linalg::Mat4,

    /// The matrix that converts from camera-space into homogenous clip space
    #[serde(default = "default_camera_projection_matrix")]
    pub projection_matrix: linalg::Mat4,

    /// The clear color of this camera
    #[serde(default = "default_camera_clear_color")]
    pub clear_color: Color,

    /// The normalized ratio of the viewport that this camera will render to.
    /// [x, y, width, height]
    //
    //     |----------------| 1,1
    //     |                |
    //     |                |
    //     |                |
    // 0,0 |----------------|
    #[serde(default)]
    pub viewport_ratio: Viewport,

    /// An optional aspect ratio (width/height) for the virtual camera. If `None`, the camera will
    /// default to the aspect ratio of the window.
    #[serde(default = "default_camera_aspect_ratio_override")]
    pub aspect_ratio_override: FfiOption<f32>,

    /// The camera's render target texture id if assigned. If `None` will render to `ColorMSAA`
    #[serde(default = "default_camera_render_target_texture_id")]
    pub render_target_texture_id: FfiOption<u32>,

    /// The scale factor for the orthographic view volume. Increasing this value has the effect of
    /// "zooming" in the camera.
    #[serde(default = "default_camera_orthographic_size")]
    pub orthographic_size: f32,

    /// The render priority for this camera. Cameras with a higher `render_order` will be rendered
    /// first.
    #[serde(default = "default_camera_render_order")]
    pub render_order: i32,

    /// If this camera will render
    #[serde(default = "default_camera_is_enabled")]
    pub is_enabled: bool,
    // Note: There is 7 bytes of padding due to 16-byte alignment. We could explicitly declare
    // the padding, but then we'd have to either expose it publicly or force a constructor function
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            view_matrix: linalg::Mat4::new(Mat4::IDENTITY),
            projection_matrix: linalg::Mat4::new(Mat4::IDENTITY),
            clear_color: BLACK,
            viewport_ratio: Viewport::default(),
            aspect_ratio_override: FfiOption::new(None),
            render_target_texture_id: FfiOption::new(None),
            orthographic_size: 1.0f32,
            render_order: 0,
            is_enabled: true,
        }
    }
}

impl Camera {
    /// Convert the given world-space `position` into the local coordinate system of this `Camera`
    pub fn world_to_view(&self, position: &Vec3) -> Vec3 {
        world_to_view(position, self)
    }

    /// Convert the given world-space `position` into the homogenous clip space this `Camera`
    pub fn world_to_clip(&self, position: &Vec3) -> Vec3 {
        world_to_clip(position, self)
    }

    /// Convert the given world-space `position` into the screen space of this `Camera` at the given
    /// `screen_dimensions` in pixels.
    pub fn world_to_screen(&self, position: &Vec3, screen_dimensions: &Vec2) -> Vec2 {
        world_to_screen(position, screen_dimensions, self)
    }

    /// Convert the given screen-space `screen_position` of this `Camera` and `screen_dimensions`
    /// into a world-space position.
    pub fn screen_to_world(&self, screen_position: &Vec2, screen_dimensions: &Vec2) -> Vec3 {
        screen_to_world(screen_position, screen_dimensions, self)
    }

    /// Convert the given screen-space `screen_position` using the given `screen_dimensions` into its
    /// homogenous clip space equivalent.
    pub fn screen_to_clip(&self, screen_position: &Vec2, screen_dimensions: &Vec2) -> Vec3 {
        screen_to_clip(screen_position, screen_dimensions)
    }

    /// Convert the given screen-space `screen_position` using the `screen_dimensions`
    /// into the local coordinate system of this `Camera`.
    pub fn screen_to_view(&self, screen_position: &Vec2, screen_dimensions: &Vec2) -> Vec3 {
        screen_to_view(screen_position, screen_dimensions, self)
    }
}

/// A resource representing the game window size (in pixels).
#[repr(C)]
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Aspect {
    pub width: f32,
    pub height: f32,
}

// TODO, replace PathBuf with something more platform agnostic.
// <https://github.com/vaguevoid/engine/issues/360>
/// This is a reference to an asset's path relative to it's project's folder path.
#[repr(C)]
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssetPath(PathBuf);

impl AssetPath {
    pub fn as_c_string(&self) -> CString {
        CString::new(self.0.to_string_lossy().into_owned()).unwrap_or_else(|_| {
            CString::new("Asset Path could not be converted to CString").unwrap()
        })
    }
}

impl<P: AsRef<Path>> From<P> for AssetPath {
    fn from(value: P) -> Self {
        Self(value.as_ref().to_path_buf())
    }
}

impl Deref for AssetPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for AssetPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// A generic storage for data from any component type. This type is used to be
/// able to collect various types of component data at runtime to eventually
/// transform to [`ComponentRef`] and pass into [`Engine::spawn`].
///
/// It is NOT recommend to use this struct manually -- use the
/// [`ComponentBuilder`] to automatically collect components.
#[repr(C)]
#[derive(Debug)]
pub struct ComponentData {
    /// `component_id` is an `Option` to enforce an explicit check which ensures
    /// that the component ids is valid and non-zero.
    component_id: Option<ComponentId>,
    component_data: Vec<MaybeUninit<u8>>,
}

impl ComponentData {
    pub fn new(component_id: ComponentId, component_data: Vec<MaybeUninit<u8>>) -> Self {
        Self {
            component_id: Some(component_id),
            component_data,
        }
    }

    pub fn component_id(&self) -> Option<ComponentId> {
        self.component_id
    }
}

impl<C: Component> From<C> for ComponentData {
    fn from(value: C) -> Self {
        let ptr = (&value as *const C).cast::<MaybeUninit<u8>>();
        let size = size_of::<C>();

        Self {
            component_id: Some(C::id()),
            component_data: unsafe { from_raw_parts(ptr, size).to_vec() },
        }
    }
}

/// A generic reference to a component. This type is necessary to pass
/// components to `Engine::spawn()`.
///
/// It is NOT recommended to use this struct manually -- use the `bundle!()`
/// macro to automatically convert components.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ComponentRef<'a> {
    /// `component_id` is an `Option` to enforce an explicit check which ensures
    /// that the component ids is valid and non-zero.
    pub component_id: Option<ComponentId>,
    pub component_size: usize,
    pub component_val: *const c_void,
    marker: PhantomData<&'a MaybeUninit<u8>>,
}

impl ComponentRef<'_> {
    pub fn new(component_id: ComponentId, component_val: &[MaybeUninit<u8>]) -> Self {
        Self {
            component_id: Some(component_id),
            component_size: component_val.len(),
            component_val: component_val.as_ptr().cast::<c_void>(),
            marker: PhantomData,
        }
    }
}

impl<'a, C> From<&'a C> for ComponentRef<'a>
where
    C: Component,
{
    fn from(value: &'a C) -> Self {
        Self {
            component_id: Some(C::id()),
            component_size: size_of::<C>(),
            component_val: (value as *const C).cast::<c_void>(),
            marker: PhantomData,
        }
    }
}

impl<'a, C> From<&'a mut C> for ComponentRef<'a>
where
    C: Component,
{
    fn from(value: &'a mut C) -> Self {
        (&*value).into()
    }
}

impl<'a> From<&'a ComponentData> for ComponentRef<'a> {
    fn from(value: &ComponentData) -> Self {
        Self {
            component_id: value.component_id,
            component_size: value.component_data.len(),
            component_val: value.component_data.as_ptr().cast(),
            marker: PhantomData,
        }
    }
}

/// Converts a list of component references into the type which `Engine::spawn()` accepts.
#[macro_export]
macro_rules! bundle {
    ($($c:expr),* $(,)?) => (
        &[$(::void_public::ComponentRef::from($c)),*]
    )
}

/// Converts a list of component references into a type which [`ComponentBuilder`] uses
#[macro_export]
macro_rules! bundle_for_builder {
    ($($c:expr),* $(,)?) => {
        [$(::void_public::ComponentData::from($c)),*]
    };
}

/// This is a convenience struct for constructing runtime [`Component`]
/// collections of type erased [`Component`] data. This is only recommended for
/// cases where you must create dynamic components at runtime, as this creates
/// an unnecessary allocation that just using `Engine::spawn(bundles!(/*
/// components here */))` does not. It is recommended that you construct this
/// struct with the `bundle_for_builder` macro, like this...
///
/// # Example
///
/// ```ignore
/// use void_public::{AssetId, bundle_for_builder, colors::palette, ComponentBuilder, graphics::TextureRender, Transform};
///
/// let component_builder: ComponentBuilder = bundle_for_builder!(Transform::default(), palette::RED, TextureRender { asset_id: AssetId(0), visible: true}).into();
/// ```
#[derive(Debug)]
pub struct ComponentBuilder(Vec<ComponentData>);

impl<const N: usize> From<[ComponentData; N]> for ComponentBuilder {
    fn from(value: [ComponentData; N]) -> Self {
        Self(value.into())
    }
}

impl ComponentBuilder {
    pub fn add_component<C: Component>(&mut self, component: C) {
        self.0.push(component.into());
    }

    /// Add multiple components to an existing builder. It is strongly
    /// recommended that you use the [`bundle_for_builder`] macro with this
    /// method, like this...
    ///
    /// # Example
    ///
    /// ```ignore
    /// use void_public::{AssetId, bundle_for_builder, colors::palette, ComponentBuilder, graphics::TextureRender, Transform};
    ///
    /// let mut component_builder: ComponentBuilder = bundle_for_builder!(TextureRender { asset_id: AssetId(0), visible: true}).into();
    /// component_builder.add_components(bundle_for_builder!(Transform::default(), palette::RED));
    /// ```
    pub fn add_components<const N: usize>(&mut self, components: [ComponentData; N]) {
        self.0.extend(components);
    }

    /// Outputs the [`ComponentRef`]s for the underlying [`ComponentData`] in
    /// the builder, likely will be used with [`Engine::spawn`]
    ///
    /// # Example
    ///
    /// ```ignore
    /// Engine::spawn(component_builder.component_refs().as_slice());
    /// ```
    pub fn build(&self) -> Vec<ComponentRef<'_>> {
        self.0.iter().map(ComponentRef::from).collect()
    }
}

pub struct Engine;

impl Engine {
    /// Loads a string containing scene data into the engine
    pub fn load_scene(scene_str: &CStr) {
        unsafe {
            _LOAD_SCENE.unwrap_unchecked()(scene_str.as_ptr());
        }
    }

    /// Spawns an entity with the specified components.
    ///
    /// Returns the `EntityId` of the new entity.
    ///
    /// NOTE: commands are deferred until the end of the frame, so the spawned
    /// entity will not be iterated by queries on the frame it is spawned.
    pub fn spawn(components: &[ComponentRef<'_>]) -> EntityId {
        unsafe {
            _SPAWN.unwrap_unchecked()(components.as_ptr(), components.len())
                .expect("could not spawn entity")
        }
    }

    /// Despawns an entity with the specified `EntityId`.
    ///
    /// NOTE: commands are deferred until the end of the frame, so the despawned
    /// entity will still be iterated by queries on the frame it is despawned.
    pub fn despawn(entity_id: EntityId) {
        unsafe {
            _DESPAWN.unwrap_unchecked()(entity_id);
        }
    }

    /// Adds components to an existing entity.
    ///
    /// NOTE: commands are deferred until the end of the frame, so the new
    /// components will not be iterated by queries on the frame that they are
    /// added.
    pub fn add_components(entity_id: EntityId, components: &[ComponentRef<'_>]) {
        unsafe {
            _ADD_COMPONENTS_FN.unwrap_unchecked()(entity_id, components.as_ptr(), components.len());
        }
    }

    /// Removes components from an existing entity.
    ///
    /// NOTE: commands are deferred until the end of the frame, so the removed
    /// components will still be iterated by queries on the frame that they are
    /// removed.
    pub fn remove_components(entity_id: EntityId, component_ids: &[ComponentId]) {
        unsafe {
            _REMOVE_COMPONENTS_FN.unwrap_unchecked()(
                entity_id,
                component_ids.as_ptr(),
                component_ids.len(),
            );
        }
    }

    /// Returns the label associated with an entity via the provided closure.
    /// If no label is associated with the entity, `None` is passed to the
    /// closure. The closure is always run, even if no label is associated with
    /// the entity.
    ///
    /// This function returns a value via a closure rather than a direct return
    /// type, because the lifetime of the label is not guaranteed to be valid
    /// indefinitely.
    pub fn entity_label<F, R>(entity_id: EntityId, f: F) -> R
    where
        F: FnOnce(Option<&CStr>) -> R,
    {
        unsafe {
            let ptr = _ENTITY_LABEL_FN.unwrap_unchecked()(entity_id);

            let label = if ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(ptr))
            };

            f(label)
        }
    }

    /// Associates an entity with a label. An entity may only have one label,
    /// and all labels must be unique (entities cannot share identical labels).
    pub fn set_entity_label(entity_id: EntityId, label: &CStr) {
        unsafe {
            _SET_ENTITY_LABEL_FN.unwrap_unchecked()(entity_id, label.as_ptr());
        }
    }

    /// Clears an entity's label.
    pub fn clear_entity_label(entity_id: EntityId) {
        unsafe {
            _SET_ENTITY_LABEL_FN.unwrap_unchecked()(entity_id, ptr::null());
        }
    }

    pub fn call<'a, F: Callable>(parameters: impl Into<F::Parameters<'a>>)
    where
        F::Parameters<'a>: Push,
    {
        let mut builder = FlatBufferBuilder::new();
        let parameters = builder.push(parameters.into());
        builder.finish_minimal(parameters);
        let parameter_data = builder.finished_data();

        unsafe {
            _CALL_FN.unwrap_unchecked()(
                F::id(),
                parameter_data.as_ptr().cast(),
                parameter_data.len(),
            );
        }
    }

    pub fn call_with_builder<'a, F: Callable>(
        parameters: impl FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<F::Parameters<'a>>,
    ) {
        let mut builder = FlatBufferBuilder::new();
        let parameters = parameters(&mut builder);
        builder.finish_minimal(parameters);
        let parameter_data = builder.finished_data();

        unsafe {
            _CALL_FN.unwrap_unchecked()(
                F::id(),
                parameter_data.as_ptr().cast(),
                parameter_data.len(),
            );
        }
    }

    pub fn call_async<'a, F: AsyncCompletion>(
        parameters: impl Into<<F::Function as Callable>::Parameters<'a>>,
        user_data: impl Into<F::UserData<'a>>,
    ) where
        <F::Function as Callable>::Parameters<'a>: Push,
        F::UserData<'a>: Push,
    {
        let mut builder = FlatBufferBuilder::new();
        let parameters = builder.push(parameters.into());
        builder.finish_minimal(parameters);
        let parameter_data = builder.finished_data();

        let mut builder = FlatBufferBuilder::new();
        let user_data = builder.push(user_data.into());
        builder.finish_minimal(user_data);
        let user_data = builder.finished_data();

        unsafe {
            _CALL_ASYNC_FN.unwrap_unchecked()(
                F::id(),
                parameter_data.as_ptr().cast(),
                parameter_data.len(),
                user_data.as_ptr().cast(),
                user_data.len(),
            );
        }
    }

    pub fn call_async_with_builder<'a, F: AsyncCompletion>(
        parameters: impl FnOnce(
            &mut FlatBufferBuilder<'a>,
        ) -> WIPOffset<<F::Function as Callable>::Parameters<'a>>,
        user_data: impl FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<F::UserData<'a>>,
    ) {
        let mut builder = FlatBufferBuilder::new();
        let parameters = parameters(&mut builder);
        builder.finish_minimal(parameters);
        let parameter_data = builder.finished_data();

        let mut builder = FlatBufferBuilder::new();
        let user_data = user_data(&mut builder);
        builder.finish_minimal(user_data);
        let user_data = builder.finished_data();

        unsafe {
            _CALL_ASYNC_FN.unwrap_unchecked()(
                F::id(),
                parameter_data.as_ptr().cast(),
                parameter_data.len(),
                user_data.as_ptr().cast(),
                user_data.len(),
            );
        }
    }

    /// Assign a new parent to the given entity
    ///
    /// This assignment is deferred until the next data sync point (currently the end of cpu system updates).
    /// Until that point, the entity's parent will be unchanged.
    pub fn set_parent_deferred(
        entity_id: EntityId,
        parent_data: Option<EntityId>,
        keep_world_space_transform: bool,
    ) {
        unsafe {
            _SET_PARENT_FN.unwrap_unchecked()(entity_id, parent_data, keep_world_space_transform);
        }
    }

    /// Get the parent id of the given entity. If the requested entity has no parent,
    /// (or if the given entity doesn't exist) this function will return `None`.
    pub fn get_parent(entity_id: EntityId) -> Option<EntityId> {
        unsafe {
            let mut out_parent_id: u64 = 0;
            if _GET_PARENT_FN.unwrap_unchecked()(entity_id, &mut out_parent_id) {
                if out_parent_id > 0 {
                    Some(EntityId(
                        NonZero::<u64>::new(out_parent_id).unwrap_unchecked(),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }

    fn set_fully_qualified_system_enabled(fully_qualified_system_name: &CStr, enabled: bool) {
        unsafe {
            (_SET_SYSTEM_ENABLED_FN).unwrap_unchecked()(
                fully_qualified_system_name.as_ptr(),
                enabled,
            );
        }
    }

    /// Allows a system to be turned off or on. The `set_system_enabled` macro
    /// is generally preferred, but that only accepts system functions as
    /// parameters, so if you need to pass in the system name by it's &`CStr`
    /// value, this is the correct API.
    pub fn set_system_enabled(
        system_name: &CStr,
        enabled: bool,
        module_name_function: ModuleNameFn,
    ) {
        Self::set_fully_qualified_system_enabled(
            &system_name_generator_c(
                unsafe { CStr::from_ptr(module_name_function()) },
                system_name,
            ),
            enabled,
        );
    }
}

pub type ModuleNameFn = unsafe extern "C" fn() -> *const c_char;

pub struct EventReader<T> {
    handle: *const c_void,
    marker: PhantomData<T>,
}

impl<'a, T: Follow<'a> + 'a> EventReader<T> {
    /// # Safety
    ///
    /// `EventReader` should only be constructed from a valid pointer retrieved
    /// from a corresponding `EventReader` parameter in an ECS system's FFI function.
    pub unsafe fn new(handle: *const c_void) -> Self {
        Self {
            handle,
            marker: PhantomData,
        }
    }

    pub fn get(&self, index: usize) -> Option<T::Inner> {
        unsafe {
            let ptr = _EVENT_GET_FN.unwrap_unchecked()(self.handle, index);

            if ptr.is_null() {
                None
            } else {
                let len = ptr.read() as usize;
                // data immediately follows `len`, offset by 1 to get data
                let data = slice::from_raw_parts(ptr.offset(1).cast(), len);
                Some(flatbuffers::root_unchecked::<T>(data))
            }
        }
    }

    pub fn iter(&'a self) -> impl Iterator<Item = T::Inner> {
        let count = unsafe { _EVENT_COUNT_FN.unwrap_unchecked()(self.handle) };

        (0..count).map(|i| unsafe {
            let ptr = _EVENT_GET_FN.unwrap_unchecked()(self.handle, i);
            let len = ptr.read() as usize;
            // data immediately follows `len`, offset by 1 to get data
            let data = slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len);
            flatbuffers::root_unchecked::<T>(data)
        })
    }
}

impl<'a, T: Follow<'a> + 'a> IntoIterator for &'a EventReader<T> {
    type Item = T::Inner;

    type IntoIter = EventReaderIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        EventReaderIter { i: 0, reader: self }
    }
}

pub struct EventReaderIter<'a, T> {
    i: usize,
    reader: &'a EventReader<T>,
}

impl<'a, T: Follow<'a> + 'a> Iterator for EventReaderIter<'a, T> {
    type Item = T::Inner;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(val) = self.reader.get(self.i) {
            self.i += 1;
            Some(val)
        } else {
            None
        }
    }
}

pub struct EventWriter<T> {
    handle: *const c_void,
    marker: PhantomData<T>,
}

impl<T> EventWriter<T> {
    /// # Safety
    ///
    /// `EventWriter` should only be constructed from a valid pointer retrieved
    /// from a corresponding `EventWriter` parameter in an ECS system's FFI function.
    pub unsafe fn new(handle: *const c_void) -> Self {
        Self {
            handle,
            marker: PhantomData,
        }
    }

    #[inline]
    pub fn write(&self, event: T)
    where
        T: Push,
    {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let event = builder.push(event);
        builder.finish_minimal(event);

        let data = builder.finished_data();

        unsafe {
            _EVENT_SEND_FN.unwrap_unchecked()(self.handle, data.as_ptr(), data.len());
        }
    }

    pub fn write_builder<'a, F>(&self, f: F)
    where
        T: Follow<'a>,
        F: FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<T>,
    {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let val = f(&mut builder);
        builder.finish_minimal(val);

        let data = builder.finished_data();

        unsafe {
            _EVENT_SEND_FN.unwrap_unchecked()(self.handle, data.as_ptr(), data.len());
        }
    }
}

/// A query is essentially an iterator over a number of entities, based on the specified
/// template components. For example, a query of type `Query<&Transform>` will iterate over
/// all the entities with a Transform component, and provide access to their `Transform` component.
///
/// Generic `Q` specifies the components to include in this query. Components *must* be references.
/// If the query specifies more than one component, `Q` should be a tuple (i.e. `Query<(&A, &B)>`).
#[repr(C)]
pub struct Query<Q> {
    query_handle: *mut c_void,
    marker: PhantomData<Q>,
}

unsafe impl<Q> Send for Query<Q> {}
unsafe impl<Q> Sync for Query<Q> {}

impl<Q> Query<Q> {
    /// # Safety
    ///
    /// `Query` should only be constructed from a valid pointer retrieved
    /// from a corresponding `Query` parameter in an ECS system's FFI function.
    pub unsafe fn new(query_handle: *mut c_void) -> Self {
        Self {
            query_handle,
            marker: PhantomData,
        }
    }

    /// Returns the number of entities matched by this query.
    pub fn len(&self) -> usize {
        unsafe { _QUERY_LEN_FN.unwrap_unchecked()(self.query_handle) }
    }

    /// Returns whether this query does not match any entities.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an immutable reference to a set of components in this query.
    ///
    /// `index` is the index in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the index is out of bounds).
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(components) = query.get(0) {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get(&self, index: usize) -> Option<QueryComponentsRef<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRef {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Returns a mutable reference to a set of components in this query.
    ///
    /// `index` is the index in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the index is out of bounds).
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(mut query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(mut components) = query.get_mut(0) {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get_mut(&mut self, index: usize) -> Option<QueryComponentsRefMut<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRefMut {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Returns an immutable reference to a set of components in this query.
    ///
    /// `entity_id` is the entity in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the entity does not exist in this query).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(components) = query.get_entity(entity_id) {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get_entity(&self, entity_id: EntityId) -> Option<QueryComponentsRef<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_ENTITY_FN.unwrap_unchecked()(
                self.query_handle,
                entity_id,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRef {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Returns a mutable reference to a set of components in this query.
    ///
    /// `entity_id` is the entity in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the entity does not exist in this query).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(mut query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(mut components) = query.get_entity(entity_id) {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get_entity_mut(&mut self, entity_id: EntityId) -> Option<QueryComponentsRefMut<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_ENTITY_FN.unwrap_unchecked()(
                self.query_handle,
                entity_id,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRefMut {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Returns an immutable reference to a set of components in this query.
    ///
    /// `label` is the label of an entity in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the label does not exist for
    /// any entities in the query).
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(components) = query.get_label(c"main_door") {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get_label(&self, label: &CStr) -> Option<QueryComponentsRef<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_LABEL_FN.unwrap_unchecked()(
                self.query_handle,
                label.as_ptr(),
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRef {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Returns an mutable reference to a set of components in this query.
    ///
    /// `label` is the label of an entity in this query to look up.
    ///
    /// Returns `None` if the lookup failed (i.e. the label does not exist for
    /// any entities in the query).
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(query: Query<(&mut Transform, &Color)>) {
    ///     if let Some(components) = query.get_label(c"main_door") {
    ///         let (transform, color) = components.unpack();
    ///     }
    /// }
    /// ```
    pub fn get_label_mut(&mut self, label: &CStr) -> Option<QueryComponentsRefMut<'_, Q>> {
        let mut component_ptrs = MaybeUninit::<Q>::uninit();

        let res = unsafe {
            _QUERY_GET_LABEL_FN.unwrap_unchecked()(
                self.query_handle,
                label.as_ptr(),
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res == 0 {
            Some(QueryComponentsRefMut {
                q: unsafe { component_ptrs.assume_init() },
                marker: PhantomData,
            })
        } else {
            None
        }
    }

    /// Iterates over all entities in this query by calling the provided function once per entity.
    ///
    /// This function only runs on a single thread. Prefer `par_for_each` where possible
    /// (see `par_for_each` docs for details).
    ///
    /// The parameters of the function will match the order and mutability of the query template.
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(mut query: Query<(&mut Transform, &Color)>) {
    ///     query.for_each(|(transform, color)| {
    ///
    ///     });
    /// }
    /// ```
    pub fn for_each<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut Q),
    {
        unsafe extern "C" fn callback<Q, F: FnMut(&mut Q)>(
            entity_data: *mut *const c_void,
            user_data: *mut c_void,
        ) -> c_int {
            let callback = || unsafe {
                let entity_data = entity_data.cast::<Q>().as_mut().unwrap_unchecked();
                let f = user_data.cast::<F>().as_mut().unwrap_unchecked();
                f(entity_data);
            };

            match catch_unwind(callback) {
                Ok(..) => ForEachResult::Continue as i32,
                Err(..) => ForEachResult::Error as i32,
            }
        }

        unsafe {
            let f_ptr: *mut F = &mut f as *mut _;
            _QUERY_FOR_EACH_FN.unwrap_unchecked()(
                self.query_handle,
                callback::<Q, F>,
                f_ptr.cast(),
            );
        }
    }

    /// Iterates over all entities in this query by calling the provided function once per entity.
    ///
    /// This version of for-each will be run in parallel and can provide significant performance improvements.
    /// This version should be the default, unless it is necessary to mutate captured state.
    ///
    /// The parameters of the function will match the order and mutability of the query template.
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(mut query: Query<(&mut Transform, &Color)>) {
    ///     query.par_for_each(|(transform, color)| {
    ///
    ///     });
    /// }
    /// ```
    pub fn par_for_each<F>(&mut self, f: F)
    where
        F: Fn(&mut Q) + Send + Sync,
    {
        unsafe extern "C" fn callback<Q, F: Fn(&mut Q)>(
            entity_data: *mut *const c_void,
            user_data: *const c_void,
        ) -> c_int {
            let callback = || unsafe {
                let entity_data = entity_data.cast::<Q>().as_mut().unwrap_unchecked();
                let f = user_data.cast::<F>().as_ref().unwrap_unchecked();
                f(entity_data);
            };

            match catch_unwind(callback) {
                Ok(..) => ForEachResult::Continue as i32,
                Err(..) => ForEachResult::Error as i32,
            }
        }

        unsafe {
            let f: *const F = &f as *const _;
            _QUERY_PAR_FOR_EACH_FN.unwrap_unchecked()(
                self.query_handle,
                callback::<Q, F>,
                f.cast(),
            );
        }
    }

    /// Creates an immutable [`Iterator`] over a query
    ///
    /// # Examples
    ///
    /// ```
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(query: Query<(&Transform, &Color)>) {
    ///     let object_location_descriptions = query.iter().map(|scene_object_ref| {
    ///         let (transform, _) = scene_object_ref.unpack();
    ///         format!("Object is at {:?}", transform)
    ///     }).collect::<Vec<_>>();
    /// }
    /// ```
    pub fn iter(&self) -> QueryIter<'_, Q> {
        QueryIter::new(self.query_handle, self.len())
    }

    /// Creates an mutable [`Iterator`] over a query
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ops::ControlFlow;
    /// use void_public::{Transform, Query, colors::Color};
    ///
    /// fn my_system(mut query: Query<(&mut Transform, &Color)>) {
    ///     let expected_color = Color::new(0., 0., 1., 1.);
    ///     query.iter_mut().try_for_each(|mut scene_object_ref| {
    ///         let (transform, color) = scene_object_ref.unpack();
    ///         if **color == expected_color {
    ///             transform.position.x += 5.5;
    ///             ControlFlow::Break(())
    ///         } else {
    ///             ControlFlow::Continue(())
    ///         }
    ///     });
    /// }
    /// ```
    pub fn iter_mut(&mut self) -> QueryIterMut<'_, Q> {
        QueryIterMut::new(self.query_handle, self.len())
    }
}

#[derive(Debug)]
pub struct QueryIter<'a, Q> {
    query_handle: *const c_void,
    index: usize,
    reverse_index: usize,
    marker: PhantomData<&'a Q>,
}

impl<Q> QueryIter<'_, Q> {
    fn new(query_handle: *const c_void, len: usize) -> Self {
        Self {
            query_handle,
            index: 0,
            reverse_index: len,
            marker: PhantomData,
        }
    }
}

impl<'a, Q> Iterator for QueryIter<'a, Q> {
    type Item = QueryComponentsRef<'a, Q>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index > self.reverse_index {
            return None;
        }
        let mut component_ptrs = MaybeUninit::<Q>::uninit();
        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                self.index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res != 0 {
            return None;
        }

        self.index += 1;

        Some(QueryComponentsRef {
            q: unsafe { component_ptrs.assume_init() },
            marker: PhantomData,
        })
    }
}

impl<Q> DoubleEndedIterator for QueryIter<'_, Q> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index > self.reverse_index {
            return None;
        }
        self.reverse_index -= 1;
        let mut component_ptrs = MaybeUninit::<Q>::uninit();
        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                self.reverse_index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res != 0 {
            return None;
        }

        Some(QueryComponentsRef {
            q: unsafe { component_ptrs.assume_init() },
            marker: PhantomData,
        })
    }
}

impl<Q> ExactSizeIterator for QueryIter<'_, Q> {
    fn len(&self) -> usize {
        self.reverse_index - self.index
    }
}

#[derive(Debug)]
pub struct QueryIterMut<'a, Q> {
    query_handle: *mut c_void,
    index: usize,
    reverse_index: usize,
    marker: PhantomData<&'a mut Q>,
}

impl<Q> QueryIterMut<'_, Q> {
    fn new(query_handle: *mut c_void, len: usize) -> Self {
        Self {
            query_handle,
            index: 0,
            reverse_index: len,
            marker: PhantomData,
        }
    }
}

impl<'a, Q> Iterator for QueryIterMut<'a, Q> {
    type Item = QueryComponentsRefMut<'a, Q>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index > self.reverse_index {
            return None;
        }
        let mut component_ptrs = MaybeUninit::<Q>::uninit();
        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                self.index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res != 0 {
            return None;
        }

        self.index += 1;

        Some(QueryComponentsRefMut {
            q: unsafe { component_ptrs.assume_init() },
            marker: PhantomData,
        })
    }
}

impl<Q> DoubleEndedIterator for QueryIterMut<'_, Q> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index > self.reverse_index {
            return None;
        }
        self.reverse_index -= 1;
        let mut component_ptrs = MaybeUninit::<Q>::uninit();
        let res = unsafe {
            _QUERY_GET_FN.unwrap_unchecked()(
                self.query_handle,
                self.reverse_index,
                (&mut component_ptrs as *mut MaybeUninit<Q>).cast(),
            )
        };

        if res != 0 {
            return None;
        }

        Some(QueryComponentsRefMut {
            q: unsafe { component_ptrs.assume_init() },
            marker: PhantomData,
        })
    }
}

impl<Q> ExactSizeIterator for QueryIterMut<'_, Q> {
    fn len(&self) -> usize {
        self.reverse_index - self.index
    }
}

impl<'a, Q> IntoIterator for &'a Query<Q> {
    type Item = QueryComponentsRef<'a, Q>;

    type IntoIter = QueryIter<'a, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, Q> IntoIterator for &'a mut Query<Q> {
    type Item = QueryComponentsRefMut<'a, Q>;

    type IntoIter = QueryIterMut<'a, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// A safe wrapper type allowing immutable access to query components `Q`.
#[derive(Debug)]
pub struct QueryComponentsRef<'a, Q> {
    q: Q,
    marker: PhantomData<&'a Q>,
}

impl<Q> QueryComponentsRef<'_, Q> {
    pub fn unpack(&self) -> &Q {
        &self.q
    }
}

impl<Q> Deref for QueryComponentsRef<'_, Q> {
    type Target = Q;

    fn deref(&self) -> &Self::Target {
        &self.q
    }
}

/// A safe wrapper type allowing mutable access to query components `Q`.
#[derive(Debug)]
pub struct QueryComponentsRefMut<'a, Q> {
    q: Q,
    marker: PhantomData<&'a mut Q>,
}

impl<Q> QueryComponentsRefMut<'_, Q> {
    pub fn unpack(&mut self) -> &mut Q {
        &mut self.q
    }
}

impl<Q> Deref for QueryComponentsRefMut<'_, Q> {
    type Target = Q;

    fn deref(&self) -> &Self::Target {
        &self.q
    }
}

impl<Q> DerefMut for QueryComponentsRefMut<'_, Q> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.q
    }
}

#[repr(i32)]
#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForEachResult {
    Continue = 0,
    Break = 1,
    Error = 2,
}

impl TryFrom<i32> for ForEachResult {
    type Error = Box<dyn Error + Send + Sync>;

    fn try_from(value: i32) -> Result<Self, <Self as TryFrom<i32>>::Error> {
        match value {
            0 => Ok(Self::Continue),
            1 => Ok(Self::Break),
            2 => Ok(Self::Error),
            _ => Err(format!("invalid ForEachResult error code '{value}'").into()),
        }
    }
}

// Global callback functions. When adding a function here, make sure to also
// update the codegen crate responsible for generating the FFI boilerplate code,
// as well as the `get_module_api_proc_addr` function in the `c_api` mod.

pub static mut _LOAD_SCENE: Option<unsafe extern "C" fn(scene_str: *const c_char)> = None;

// spawning
pub static mut _SPAWN: Option<
    unsafe extern "C" fn(*const ComponentRef<'_>, usize) -> Option<EntityId>,
> = None;

pub static mut _DESPAWN: Option<unsafe extern "C" fn(EntityId)> = None;

pub static mut _ADD_COMPONENTS_FN: Option<
    unsafe extern "C" fn(EntityId, *const ComponentRef<'_>, usize),
> = None;

pub static mut _REMOVE_COMPONENTS_FN: Option<
    unsafe extern "C" fn(EntityId, *const ComponentId, usize),
> = None;

pub static mut _ENTITY_LABEL_FN: Option<unsafe extern "C" fn(EntityId) -> *const c_char> = None;

pub static mut _SET_ENTITY_LABEL_FN: Option<unsafe extern "C" fn(EntityId, *const c_char)> = None;

// events
pub static mut _EVENT_COUNT_FN: Option<unsafe extern "C" fn(*const c_void) -> usize> = None;

pub static mut _EVENT_GET_FN: Option<unsafe extern "C" fn(*const c_void, usize) -> *const u64> =
    None;

pub static mut _EVENT_SEND_FN: Option<unsafe extern "C" fn(*const c_void, *const u8, usize)> = None;

// queries
pub static mut _QUERY_LEN_FN: Option<unsafe extern "C" fn(*const c_void) -> usize> = None;

pub static mut _QUERY_GET_FN: Option<
    unsafe extern "C" fn(*const c_void, usize, *mut *const c_void) -> i32,
> = None;

pub static mut _QUERY_GET_ENTITY_FN: Option<
    unsafe extern "C" fn(*mut c_void, EntityId, *mut *const c_void) -> i32,
> = None;

pub static mut _QUERY_GET_LABEL_FN: Option<
    unsafe extern "C" fn(*mut c_void, *const c_char, *mut *const c_void) -> i32,
> = None;

pub static mut _QUERY_FOR_EACH_FN: Option<
    unsafe extern "C" fn(
        *mut c_void,
        unsafe extern "C" fn(*mut *const c_void, *mut c_void) -> c_int,
        *mut c_void,
    ),
> = None;

pub static mut _QUERY_PAR_FOR_EACH_FN: Option<
    unsafe extern "C" fn(
        *mut c_void,
        unsafe extern "C" fn(*mut *const c_void, *const c_void) -> c_int,
        *const c_void,
    ),
> = None;

pub static mut _CALL_FN: Option<unsafe extern "C" fn(ComponentId, *const c_void, usize)> = None;

pub static mut _CALL_ASYNC_FN: Option<
    unsafe extern "C" fn(ComponentId, *const c_void, usize, *const c_void, usize),
> = None;

// parentage
pub static mut _SET_PARENT_FN: Option<unsafe extern "C" fn(EntityId, Option<EntityId>, bool)> =
    None;

pub static mut _GET_PARENT_FN: Option<unsafe extern "C" fn(EntityId, *mut u64) -> bool> = None;

// system meta
pub static mut _SET_SYSTEM_ENABLED_FN: Option<unsafe extern "C" fn(*const c_char, bool)> = None;

// ENGINE INTERNALS - NOT COPIED TO RELEASE HEADERS

#[repr(C)]
#[derive(Debug)]
pub enum ComponentType {
    AsyncCompletion,
    Component,
    Resource,
}

#[repr(C)]
#[derive(Debug)]
pub enum ArgType {
    Completion,
    DataAccessMut,
    DataAccessRef,
    EventReader,
    EventWriter,
    Query,
}

/// `FfiVec` is intended to be used when transferring a Rust side Vec to C via
/// FFI. Because [`Vec::into_raw_parts`] isn't stable, we manually create our
/// own in the form of this struct. This memory should not be freed on the C
/// side, but instead should be passed into a Rust provided free function which
/// will call [`Vec::from_raw_parts`] with the parameters in this struct and
/// then free the memory.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FfiVec<T> {
    pub ptr: *mut T,
    pub len: usize,
    pub capacity: usize,
}
