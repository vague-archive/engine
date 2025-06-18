use std::{
    collections::HashMap,
    error::Error,
    ffi::{CStr, CString, NulError},
    fmt::Display,
    path::PathBuf,
};

use void_public::{
    AssetPath, EventWriter,
    event::graphics::{NewTexture, NewTextureBuilder},
    graphics::{TextureHash, TextureId, TextureType},
};

type FfiPendingTexture = void_public::graphics::PendingTexture;

pub type TextureFailure = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
#[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
pub struct TextureAssetManager {
    /// This is used in situation where potentially large numbers of textures
    /// can be queued up, for example when loading a scene.
    pub(crate) batched_textures: HashMap<TextureId, Texture>,
    /// Similiar to `user_asset_path_to_id`, holds `asset_path` to `id`
    /// relationship for batched textures.
    pub(crate) batched_asset_path_to_id: HashMap<AssetPath, TextureId>,
    /// New [`Texture`]'s require a [`TextureId`], this stores the next available one.
    pub(crate) next_texture_id: TextureId,
    pub(crate) textures: HashMap<TextureId, Texture>,
    /// Storage of all the user's [`AssetPath`]'s mapped to the [`TextureId`].
    /// [`AssetPath`]s must be unique.
    pub(crate) user_asset_path_to_id: HashMap<AssetPath, TextureId>,
    /// Storage of all the internal engine [`AssetPath`]'s mapped to the
    /// [`TextureId`]. [`AssetPath`]s must be unique. These maps are seperate to
    /// avoid name collisions.
    pub(crate) engine_asset_path_to_id: HashMap<AssetPath, TextureId>,
}

// Reserved TextureIds must match order in TextureAssetManager TextureAssetManager::default()
pub const WHITE_TEXTURE_TEXTURE_ID: TextureId = TextureId(0);
pub const MISSING_TEXTURE_TEXTURE_ID: TextureId = TextureId(1);

impl Default for TextureAssetManager {
    /// By default we reserve a white texture and missing texture
    fn default() -> Self {
        let mut textures = HashMap::new();
        let white_texture = EngineTexture::new(
            Self::white_texture_id(),
            &"white_texture".into(),
            2,
            2,
            false,
        );
        let missing_texture = EngineTexture::new(
            Self::missing_texture_id(),
            &"missing_texture".into(),
            2,
            2,
            false,
        );

        let mut engine_asset_path_to_id = HashMap::new();
        engine_asset_path_to_id.insert(white_texture.texture_path().clone(), white_texture.id());
        engine_asset_path_to_id
            .insert(missing_texture.texture_path().clone(), missing_texture.id());

        textures.insert(Self::white_texture_id(), white_texture.into());
        textures.insert(Self::missing_texture_id(), missing_texture.into());

        Self {
            batched_textures: HashMap::new(),
            batched_asset_path_to_id: HashMap::new(),
            next_texture_id: TextureId(2),
            textures,
            user_asset_path_to_id: HashMap::new(),
            engine_asset_path_to_id,
        }
    }
}

impl TextureAssetManager {
    pub const fn white_texture_id() -> TextureId {
        WHITE_TEXTURE_TEXTURE_ID
    }

    /// This should only be used internally, as our renderer assumes this means
    /// the texture has not been loaded, or there was a failure during load, and
    /// so we skip some caching while we wait for it load
    pub const fn missing_texture_id() -> TextureId {
        MISSING_TEXTURE_TEXTURE_ID
    }

    /// Gives the caller the next available [`TextureId`]. Currently
    /// [`TextureId`]'s should not be relied upon as consistently applied to the
    /// same asset. The [`AssetPath`] is the human readable, consistent value
    /// for a given [`Texture`]
    pub fn register_next_texture_id(&mut self) -> TextureId {
        let next_texture_id = self.next_texture_id;
        self.next_texture_id = TextureId(*next_texture_id + 1);
        next_texture_id
    }

    /// Hashes a slice of [`u8`]s into a [`TextureHash`]. This is a no op in
    /// production, as this currently is only used for development and tapes
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub fn generate_hash(data: &[u8]) -> TextureHash {
        #[cfg(not(debug_assertions))]
        {
            TextureHash::create_empty()
        }
        #[cfg(debug_assertions)]
        {
            use std::hash::Hasher;

            use ahash::AHasher;

            let mut hasher = AHasher::default();
            hasher.write(data);
            let hash = hasher.finish();
            hash.to_le_bytes().iter().into()
        }
    }

    pub fn get_texture_by_id(&self, texture_id: TextureId) -> Option<&Texture> {
        self.textures
            .get(&texture_id)
            .or_else(|| self.batched_textures.get(&texture_id))
    }

    pub fn get_texture_by_path(&self, texture_path: &AssetPath) -> Option<&Texture> {
        match self
            .user_asset_path_to_id
            .get(texture_path)
            .or_else(|| self.batched_asset_path_to_id.get(texture_path))
        {
            Some(texture_id) => self.get_texture_by_id(*texture_id),
            None => None,
        }
    }

    pub fn are_all_ids_loaded<'a, I>(&self, ids: I) -> bool
    where
        I: IntoIterator<Item = &'a TextureId>,
    {
        ids.into_iter().all(|id| {
            let Some(texture) = self.textures.get(id) else {
                return false;
            };

            matches!(texture.texture_type(), TextureType::Loaded)
        })
    }

    /// Creates a new [`PendingTexture`], and sends the [`NewTexture`] event to
    /// load the texture. Like
    /// [`TextureAssetManager::load_texture_by_pending_texture`], each platform should
    /// implement [`EventReader`] for [`NewTexture`], handle loading, and then
    /// call [`TextureAssetManager::insert_loaded_texture`].
    ///
    /// # Errors
    ///
    /// Will error if the input [`AssetPath`] cannot be parsed. Will _not_ error
    /// if [`AssetPath`] already exists, but will remove that asset as it
    /// considers this API to "reload" that texture.
    pub fn load_texture<'a>(
        &'a mut self,
        texture_path: &AssetPath,
        insert_in_atlas: bool,
        new_texture_event_writer: &EventWriter<NewTexture<'_>>,
    ) -> Result<&'a PendingTexture, TextureFailure> {
        let pending_texture_id = self.register_next_texture_id();
        let pending_texture =
            PendingTexture::new(pending_texture_id, texture_path, insert_in_atlas);

        self.load_texture_by_pending_texture(&pending_texture, new_texture_event_writer)?;
        Ok(self
            .get_texture_by_id(pending_texture_id)
            .unwrap()
            .as_pending_texture()
            .unwrap())
    }

    /// Stores the [`PendingTexture`] and sends the [`NewTexture`] event to load
    /// the texture. Each platform should implement [`EventReader`]s for
    /// [`NewTexture`], handling loading, and then call
    /// [`TextureAssetManager::insert_loaded_texture`].
    ///
    /// # Errors
    ///
    /// Will error if the input [`AssetPath`] cannot be parsed. Will _not_ error
    /// if [`AssetPath`] already exists, but will remove that asset as it
    /// considers this API to "reload" that texture.
    pub fn load_texture_by_pending_texture(
        &mut self,
        pending_texture: &PendingTexture,
        new_texture_event_writer: &EventWriter<NewTexture<'_>>,
    ) -> Result<(), TextureFailure> {
        self.textures.remove(&pending_texture.id());
        let Some(path_as_str) = pending_texture.texture_path().as_os_str().to_str() else {
            let error_text = format!("Path {} is invalid", &pending_texture.texture_path());
            let failed_texture = FailedTexture::new(
                pending_texture.id,
                pending_texture.texture_path(),
                &error_text,
            );
            self.textures
                .insert(failed_texture.id(), failed_texture.into());

            return Err(error_text.into());
        };

        new_texture_event_writer.write_builder(|builder| {
            let path_as_str = builder.create_string(path_as_str);
            let mut new_texture_builder = NewTextureBuilder::new(builder);
            new_texture_builder.add_id(*pending_texture.id());
            new_texture_builder.add_asset_path(path_as_str);
            new_texture_builder.add_insert_in_atlas(pending_texture.insert_in_atlas());
            new_texture_builder.finish()
        });

        self.textures
            .insert(pending_texture.id(), pending_texture.clone().into());

        self.user_asset_path_to_id
            .insert(pending_texture.texture_path.clone(), pending_texture.id());

        Ok(())
    }
}

#[cfg(feature = "internal_features")]
impl TextureAssetManager {
    /// Used for internal batching of large numbers of [`PendingTexture`]s,
    /// [`TextureAssetManager::trigger_batched_textures`] should be called after all
    /// textures batched
    pub fn add_to_batched_textures(&mut self, pending_texture: PendingTexture) {
        self.batched_asset_path_to_id
            .insert(pending_texture.texture_path().clone(), pending_texture.id());
        self.batched_textures
            .insert(pending_texture.id(), pending_texture.into());
    }

    /// Triggers the batched textures, not currently working
    pub fn trigger_batched_textures(&mut self) {
        // TODO, this is currently broken, cannot trigger this
        // Engine::set_system_enabled(c"gpu_assets::process_batched_textures", true);
    }

    pub fn drain_batched_textures(
        &mut self,
    ) -> std::collections::hash_map::Drain<'_, TextureId, Texture> {
        self.batched_asset_path_to_id.clear();
        self.batched_textures.drain()
    }

    pub fn get_engine_texture_id_from_path(&self, texture_path: &AssetPath) -> Option<TextureId> {
        self.engine_asset_path_to_id.get(texture_path).copied()
    }

    /// Engine Textures are special internal textures, such as atlas textures
    /// and render targets. This adds them, but should only be used within
    /// engine
    ///
    /// # Errors
    ///
    /// * Fails if [`TextureId`] already exists
    /// * Fails if [`AssetPath`] is already used within engine textures
    pub fn insert_engine_texture(
        &mut self,
        engine_texture: &EngineTexture,
    ) -> Result<(), TextureFailure> {
        if self.textures.contains_key(&engine_texture.id()) {
            return Err(format!(
                "Texture id {} already exists, cannot insert internal texture",
                engine_texture.id()
            )
            .into());
        }

        if let Some(existent_texture_id) = self
            .engine_asset_path_to_id
            .get(engine_texture.texture_path())
        {
            return Err(format!("Texture path {} already exists on id {existent_texture_id}, cannot insert internal texture", engine_texture.texture_path()).into());
        }

        self.textures
            .insert(engine_texture.id(), engine_texture.clone().into());
        self.engine_asset_path_to_id
            .insert(engine_texture.texture_path().clone(), engine_texture.id());
        Ok(())
    }

    /// Updates an engine texture at a given [`TextureId`]
    ///
    /// # Errors
    ///
    /// * Fails if [`TextureId`] is not found
    /// * Fails if [`Texture`] at [`TextureId`] is not [`TextureType::Engine`]
    pub fn update_engine_texture(
        &mut self,
        id: TextureId,
        width: usize,
        height: usize,
    ) -> Result<(), TextureFailure> {
        let Some(engine_texture) = self.textures.get_mut(&id) else {
            return Err(
                format!("Id {id} does not exist, could not update internal texture").into(),
            );
        };

        let Texture::Engine(engine_texture) = engine_texture else {
            return Err(format!(
                "Id {id} is not an internal texture, could not update internal texture"
            )
            .into());
        };

        engine_texture.width = width;
        engine_texture.height = height;

        Ok(())
    }

    /// Updates a [`Texture`], likely a [`PendingTexture`], to
    /// [`FailedTexture`]. This is likely done based on a message from the
    /// platform failing to load the [`Texture`]
    pub fn replace_failed_texture(&mut self, failed_texture: &FailedTexture) {
        self.textures.remove(&failed_texture.id());

        self.textures
            .insert(failed_texture.id(), failed_texture.clone().into());
        self.user_asset_path_to_id
            .insert(failed_texture.texture_path.clone(), failed_texture.id());
    }

    /// Updates a [`Texture`], likely a [`PendingTexture`], to
    /// [`LoadedTexture`]. This is likely done based on a message from the
    /// platform failing to load the [`Texture`]
    pub fn replace_loaded_texture(&mut self, loaded_texture: &LoadedTexture) {
        self.textures.remove(&loaded_texture.id());

        self.textures
            .insert(loaded_texture.id(), loaded_texture.clone().into());
        self.user_asset_path_to_id
            .insert(loaded_texture.texture_path.clone(), loaded_texture.id());
    }

    /// Directly inserts a loaded texture, outside of typical platform messaging workflow.
    ///
    /// # Errors
    ///
    /// * Fails if [`TextureId`] already exists
    /// * Fails if [`Texture`] at [`TextureId`] already is a [`TextureType::Loaded`]
    pub fn insert_loaded_texture(
        &mut self,
        loaded_texture: &LoadedTexture,
    ) -> Result<(), TextureFailure> {
        if self.textures.contains_key(&loaded_texture.id()) {
            return Err(format!(
                "Id {} already exists, cannot insert loaded texture",
                loaded_texture.id()
            )
            .into());
        }
        if let Some(existent_texture_id) = self
            .user_asset_path_to_id
            .get(loaded_texture.texture_path())
        {
            return Err(format!("Texture path {} already exists on id {existent_texture_id}, cannot load loaded texture", loaded_texture.texture_path()).into());
        }

        self.textures
            .insert(loaded_texture.id(), loaded_texture.clone().into());
        self.user_asset_path_to_id
            .insert(loaded_texture.texture_path.clone(), loaded_texture.id());

        Ok(())
    }
}

/// Enum representing all possible states of a texture
#[derive(Clone, Debug)]
pub enum Texture {
    Pending(PendingTexture),
    Loaded(LoadedTexture),
    Engine(EngineTexture),
    Failed(FailedTexture),
}

impl Texture {
    pub fn id(&self) -> TextureId {
        match self {
            Self::Pending(pending_texture) => pending_texture.id(),
            Self::Loaded(loaded_texture) => loaded_texture.id(),
            Self::Engine(engine_texture) => engine_texture.id(),
            Self::Failed(failed_texture) => failed_texture.id(),
        }
    }

    pub fn path(&self) -> &AssetPath {
        match self {
            Self::Pending(pending_texture) => pending_texture.texture_path(),
            Self::Loaded(loaded_texture) => loaded_texture.texture_path(),
            Self::Engine(engine_texture) => engine_texture.texture_path(),
            Self::Failed(failed_texture) => failed_texture.texture_path(),
        }
    }

    pub const fn texture_type(&self) -> TextureType {
        match self {
            Self::Pending(_) => PendingTexture::texture_type(),
            Self::Loaded(_) => LoadedTexture::texture_type(),
            Self::Engine(_) => EngineTexture::texture_type(),
            Self::Failed(_) => FailedTexture::texture_type(),
        }
    }

    pub fn as_pending_texture(&self) -> Option<&PendingTexture> {
        if let Self::Pending(pending_texture) = self {
            Some(pending_texture)
        } else {
            None
        }
    }

    pub fn as_engine_texture(&self) -> Option<&EngineTexture> {
        if let Self::Engine(engine_texture) = self {
            Some(engine_texture)
        } else {
            None
        }
    }

    pub fn as_loaded_texture(&self) -> Option<&LoadedTexture> {
        if let Self::Loaded(loaded_texture) = self {
            Some(loaded_texture)
        } else {
            None
        }
    }

    pub fn as_failed_texture(&self) -> Option<&FailedTexture> {
        if let Self::Failed(failed_texture) = self {
            Some(failed_texture)
        } else {
            None
        }
    }
}

/// A [`Texture`] that is being loaded on the platform
#[derive(Clone, Debug)]
pub struct PendingTexture {
    id: TextureId,
    texture_path: AssetPath,
    insert_in_atlas: bool,
}

impl PendingTexture {
    pub fn new(id: TextureId, texture_path: &AssetPath, insert_in_atlas: bool) -> Self {
        Self {
            id,
            texture_path: texture_path.clone(),
            insert_in_atlas,
        }
    }

    pub fn id(&self) -> TextureId {
        self.id
    }

    pub fn texture_path(&self) -> &AssetPath {
        &self.texture_path
    }

    pub const fn texture_type() -> TextureType {
        TextureType::Pending
    }

    pub fn insert_in_atlas(&self) -> bool {
        self.insert_in_atlas
    }
}

impl From<PendingTexture> for Texture {
    fn from(value: PendingTexture) -> Self {
        Self::Pending(value)
    }
}

impl From<&FfiPendingTexture> for PendingTexture {
    fn from(value: &FfiPendingTexture) -> Self {
        let texture_path = PathBuf::from(
            unsafe { CStr::from_ptr(value.texture_path) }
                .to_string_lossy()
                .as_ref(),
        );
        Self::new(value.id, &texture_path.into(), value.insert_in_atlas)
    }
}

impl From<FfiPendingTexture> for PendingTexture {
    fn from(value: FfiPendingTexture) -> Self {
        (&value).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatType {
    Jpeg,
    Png,
    Unimplemented(String),
}

impl Display for FormatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string_representation = match self {
            FormatType::Jpeg => "jpeg",
            FormatType::Png => "png",
            FormatType::Unimplemented(description) => description,
        };
        write!(f, "{}", string_representation)
    }
}

impl AsRef<str> for FormatType {
    fn as_ref(&self) -> &str {
        match self {
            FormatType::Jpeg => "jpeg",
            FormatType::Png => "png",
            FormatType::Unimplemented(description) => description,
        }
    }
}

impl TryFrom<FormatType> for CString {
    type Error = NulError;

    fn try_from(value: FormatType) -> Result<Self, Self::Error> {
        (&value).try_into()
    }
}

impl TryFrom<&FormatType> for CString {
    type Error = NulError;

    fn try_from(value: &FormatType) -> Result<Self, Self::Error> {
        CString::new(value.as_ref())
    }
}

/// An internal, engine only texture. It's [`AssetPath`]'s are unique, seperate
/// from user [`LoadedTexture`]s. This is used for things like Atlas textures
/// and Render Targets
#[derive(Clone, Debug)]
pub struct EngineTexture {
    id: TextureId,
    texture_path: AssetPath,
    pub(crate) width: usize,
    pub(crate) height: usize,
    in_atlas: bool,
}

impl EngineTexture {
    pub fn new(
        id: TextureId,
        texture_path: &AssetPath,
        width: usize,
        height: usize,
        in_atlas: bool,
    ) -> Self {
        Self {
            id,
            texture_path: texture_path.clone(),
            width,
            height,
            in_atlas,
        }
    }

    pub fn id(&self) -> TextureId {
        self.id
    }

    pub fn texture_path(&self) -> &AssetPath {
        &self.texture_path
    }

    pub const fn texture_type() -> TextureType {
        TextureType::Engine
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn in_atlas(&self) -> bool {
        self.in_atlas
    }
}

impl From<EngineTexture> for Texture {
    fn from(value: EngineTexture) -> Self {
        Self::Engine(value)
    }
}

#[derive(Clone, Debug)]
pub struct LoadedTexture {
    id: TextureId,
    texture_path: AssetPath,
    pub(crate) version: TextureHash,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) format_type: FormatType,
    in_atlas: bool,
}

impl LoadedTexture {
    pub fn new(
        id: TextureId,
        texture_path: &AssetPath,
        version: &TextureHash,
        width: usize,
        height: usize,
        format_type: FormatType,
        in_atlas: bool,
    ) -> Self {
        Self {
            id,
            texture_path: texture_path.clone(),
            version: *version,
            width,
            height,
            format_type,
            in_atlas,
        }
    }

    pub fn id(&self) -> TextureId {
        self.id
    }

    pub fn texture_path(&self) -> &AssetPath {
        &self.texture_path
    }

    pub const fn texture_type() -> TextureType {
        TextureType::Loaded
    }

    pub fn version(&self) -> &TextureHash {
        &self.version
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn format_type(&self) -> &FormatType {
        &self.format_type
    }

    pub fn in_atlas(&self) -> bool {
        self.in_atlas
    }
}

impl From<LoadedTexture> for Texture {
    fn from(value: LoadedTexture) -> Self {
        Self::Loaded(value)
    }
}

/// This represents a [`Texture`] that has failed to load from the platform.
#[derive(Clone, Debug)]
pub struct FailedTexture {
    id: TextureId,
    texture_path: AssetPath,
    failure_reason: String,
}

impl FailedTexture {
    pub fn new(id: TextureId, texture_path: &AssetPath, failure_reason: &str) -> Self {
        Self {
            id,
            texture_path: texture_path.clone(),
            failure_reason: failure_reason.to_string(),
        }
    }

    pub fn id(&self) -> TextureId {
        self.id
    }

    pub fn texture_path(&self) -> &AssetPath {
        &self.texture_path
    }

    pub const fn texture_type() -> TextureType {
        TextureType::Failed
    }

    pub fn failure_reason(&self) -> &str {
        &self.failure_reason
    }
}

impl From<FailedTexture> for Texture {
    fn from(value: FailedTexture) -> Self {
        Self::Failed(value)
    }
}

#[allow(
    clippy::derivable_impls,
    clippy::missing_safety_doc,
    clippy::needless_lifetimes,
    elided_lifetimes_in_paths,
    explicit_outlives_requirements,
    unused_extern_crates,
    unused_imports,
    unsafe_op_in_unsafe_fn
)]
pub mod events {
    include!(concat!(env!("OUT_DIR"), "/texture_events_generated.rs"));
}
