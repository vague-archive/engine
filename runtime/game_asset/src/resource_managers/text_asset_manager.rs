use std::{
    collections::HashMap,
    error::Error,
    ffi::{CStr, CString, NulError},
    fmt::Display,
    num::NonZero,
    path::PathBuf,
};

use void_public::{
    AssetPath, EventWriter,
    event::graphics::{NewText, NewTextBuilder},
    text::{TextHash, TextId, TextType},
};

use crate::ecs_module::TextAssetManager;

type FfiPendingText = void_public::text::PendingText;

pub type TextFailure = Box<dyn Error + Send + Sync>;

// The reserved TextIds must match order in TextAssetManager TextAssetManager::default().
pub const MISSING_TEXT_ID: TextId = TextId(unsafe { NonZero::new_unchecked(1) });

impl Default for TextAssetManager {
    fn default() -> Self {
        let mut text = HashMap::new();
        let missing_text = EngineText::new(
            Self::missing_text_id(),
            &"missing_text".into(),
            FormatType::Text,
            "lorem ipsum",
        );

        let mut engine_asset_path_to_id = HashMap::new();
        engine_asset_path_to_id.insert(missing_text.text_path().clone(), missing_text.id());

        text.insert(Self::missing_text_id(), missing_text.into());

        Self {
            batched_text: HashMap::new(),
            batched_asset_path_to_id: HashMap::new(),
            next_text_id: TextId(unsafe { NonZero::<u32>::new_unchecked(2) }),
            text,
            user_asset_path_to_id: HashMap::new(),
            engine_asset_path_to_id,
        }
    }
}

impl TextAssetManager {
    pub const fn missing_text_id() -> TextId {
        MISSING_TEXT_ID
    }
    /// Gives the caller the next available [`TextId`]. Currently [`TextId`]'s
    /// should not be relied upon as consistently applied to the same asset. The
    /// [`AssetPath`] is the human readable, consistent value for a given
    /// [`Text`]
    pub fn register_next_text_id(&mut self) -> TextId {
        let next_text_id = self.next_text_id;
        self.next_text_id = TextId(next_text_id.checked_add(1).unwrap());
        next_text_id
    }

    /// Hashes a slice of [`u8`]s into a [`TextHash`]. This is a no op in
    /// production, as this currently is only used for development and tapes
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub fn generate_hash(data: &[u8]) -> TextHash {
        #[cfg(not(debug_assertions))]
        {
            TextHash::create_empty()
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

    pub fn get_text_by_id(&self, text_id: TextId) -> Option<&Text> {
        self.text
            .get(&text_id)
            .or_else(|| self.batched_text.get(&text_id))
    }

    pub fn get_text_by_path(&self, text_path: &AssetPath) -> Option<&Text> {
        match self
            .user_asset_path_to_id
            .get(text_path)
            .or_else(|| self.batched_asset_path_to_id.get(text_path))
        {
            Some(text_id) => self.get_text_by_id(*text_id),
            None => None,
        }
    }

    pub fn are_all_ids_loaded<'a, I>(&self, ids: I) -> bool
    where
        I: IntoIterator<Item = &'a TextId>,
    {
        ids.into_iter().all(|id| {
            let Some(text) = self.text.get(id) else {
                return false;
            };

            matches!(text.text_type(), TextType::Loaded)
        })
    }

    /// Creates a new [`PendingText`], and sends the [`NewText`] event to
    /// load the text. Like
    /// [`TextAssetManager::load_text_by_pending_text`], each platform should
    /// implement [`EventReader`] for [`NewText`], handle loading, and then
    /// call [`TextAssetManager::insert_loaded_text`].
    ///
    /// # Errors
    ///
    /// Will error if the input [`AssetPath`] cannot be parsed. Will _not_ error
    /// if [`AssetPath`] already exists, but will remove that asset as it
    /// considers this API to "reload" that text.
    pub fn load_text<'a>(
        &'a mut self,
        text_path: &AssetPath,
        set_up_watcher: bool,
        new_text_event_writer: &EventWriter<NewText<'_>>,
    ) -> Result<&'a PendingText, TextFailure> {
        let pending_text_id = self.register_next_text_id();
        let pending_text = PendingText::new(pending_text_id, text_path, set_up_watcher);

        self.load_text_by_pending_text(&pending_text, new_text_event_writer)?;
        Ok(self
            .get_text_by_id(pending_text_id)
            .unwrap()
            .as_pending_text()
            .unwrap())
    }

    /// Stores the [`PendingText`] and sends the [`NewText`] event to load
    /// the text. Each platform should implement [`EventReader`]s for
    /// [`NewText`], handling loading, and then call
    /// [`TextAssetManager::insert_loaded_text`].
    ///
    /// # Errors
    ///
    /// Will error if the input [`AssetPath`] cannot be parsed. Will _not_ error
    /// if [`AssetPath`] already exists, but will remove that asset as it
    /// considers this API to "reload" that text.
    pub fn load_text_by_pending_text(
        &mut self,
        pending_text: &PendingText,
        new_text_event_writer: &EventWriter<NewText<'_>>,
    ) -> Result<(), TextFailure> {
        self.text.remove(&pending_text.id());
        let Some(path_as_str) = pending_text.text_path().as_os_str().to_str() else {
            let error_text = format!("Path {} is invalid", &pending_text.text_path());
            let failed_text =
                FailedText::new(pending_text.id(), pending_text.text_path(), &error_text);
            self.text.insert(failed_text.id(), failed_text.into());

            return Err(error_text.into());
        };

        new_text_event_writer.write_builder(|builder| {
            let path_as_str = builder.create_string(path_as_str);
            let mut new_text_builder = NewTextBuilder::new(builder);
            new_text_builder.add_id((*pending_text.id()).into());
            new_text_builder.add_asset_path(path_as_str);
            new_text_builder.add_set_up_watcher(pending_text.set_up_watcher());
            new_text_builder.finish()
        });

        self.text
            .insert(pending_text.id(), pending_text.clone().into());

        self.user_asset_path_to_id
            .insert(pending_text.text_path().clone(), pending_text.id());

        Ok(())
    }
}

#[cfg(feature = "internal_features")]
impl TextAssetManager {
    /// Used for internal batching of large numbers of [`PendingText`]s,
    /// [`TextAssetManager::trigger_batched_text`] should be called after all text
    /// batched
    pub fn add_to_batched_text(&mut self, pending_text: PendingText) {
        self.batched_asset_path_to_id
            .insert(pending_text.text_path().clone(), pending_text.id());
        self.batched_text
            .insert(pending_text.id(), pending_text.into());
    }

    /// Triggers the batched text, not currently working
    pub fn trigger_batched_text(&mut self) {
        // TODO, this is currently broken, cannot trigger this
        // Engine::set_system_enabled(c"gpu_assets::process_batched_text", true);
    }

    pub fn drain_batched_text(&mut self) -> std::collections::hash_map::Drain<'_, TextId, Text> {
        self.batched_asset_path_to_id.clear();
        self.batched_text.drain()
    }

    pub fn get_engine_text_id_from_path(&self, text_path: &AssetPath) -> Option<TextId> {
        self.engine_asset_path_to_id.get(text_path).copied()
    }

    /// Engine Text are special internal text, such as shader fragments and
    /// material definitions. This adds them, but should only be used within
    /// engine.
    ///
    /// # Errors
    ///
    /// * Fails if [`TextId`] already exists
    /// * Fails if [`AssetPath`] is already used within engine text
    pub fn insert_engine_text(&mut self, engine_text: &EngineText) -> Result<(), TextFailure> {
        if self.text.contains_key(&engine_text.id()) {
            return Err(format!(
                "Text id {} already exists, cannot insert internal text",
                engine_text.id()
            )
            .into());
        }

        if let Some(existent_text_id) = self.engine_asset_path_to_id.get(engine_text.text_path()) {
            return Err(format!(
                "Text path {} already exists on id {existent_text_id}, cannot insert internal text",
                engine_text.text_path()
            )
            .into());
        }

        self.text
            .insert(engine_text.id(), engine_text.clone().into());
        self.engine_asset_path_to_id
            .insert(engine_text.text_path().clone(), engine_text.id());
        Ok(())
    }

    /// Updates an engine text at a given [`TextId`]
    ///
    /// # Errors
    ///
    /// * Fails if [`TextId`] is not found
    /// * Fails if [`Text`] at [`TextId`] is not [`TextType::Engine`]
    pub fn update_engine_text(&mut self, id: TextId, raw_text: &str) -> Result<(), TextFailure> {
        let Some(engine_text) = self.text.get_mut(&id) else {
            return Err(format!("Id {id} does not exist, could not update internal text").into());
        };

        let Text::Engine(engine_text) = engine_text else {
            return Err(
                format!("Id {id} is not an internal text, could not update internal text").into(),
            );
        };

        engine_text.raw_text = raw_text.to_string();

        Ok(())
    }

    /// Updates a [`Text`], likely a [`PendingText`], to
    /// [`FailedText`]. This is likely done based on a message from the
    /// platform failing to load the [`Text`]
    pub fn replace_failed_text(&mut self, failed_text: &FailedText) {
        self.text.remove(&failed_text.id());

        self.text
            .insert(failed_text.id(), failed_text.clone().into());
        self.user_asset_path_to_id
            .insert(failed_text.text_path.clone(), failed_text.id());
    }

    /// Updates a [`Text`], likely a [`PendingText`], to
    /// [`LoadedText`]. This is likely done based on a message from the
    /// platform failing to load the [`Text`]
    pub fn replace_loaded_text(&mut self, loaded_text: &LoadedText) {
        self.text.remove(&loaded_text.id());

        self.text
            .insert(loaded_text.id(), loaded_text.clone().into());
        self.user_asset_path_to_id
            .insert(loaded_text.text_path().clone(), loaded_text.id());
    }

    /// Directly inserts a loaded text, outside of typical platform messaging workflow.
    ///
    /// # Errors
    ///
    /// * Fails if [`TextId`] already exists
    /// * Fails if [`Text`] at [`TextId`] already is a [`TextType::Loaded`]
    pub fn insert_loaded_text(&mut self, loaded_text: &LoadedText) -> Result<(), TextFailure> {
        if self.text.contains_key(&loaded_text.id()) {
            return Err(format!(
                "Id {} already exists, cannot insert loaded text",
                loaded_text.id()
            )
            .into());
        }
        if let Some(existent_text_id) = self.user_asset_path_to_id.get(loaded_text.text_path()) {
            return Err(format!(
                "Texture path {} already exists on id {existent_text_id}, cannot load loaded text",
                loaded_text.text_path()
            )
            .into());
        }

        self.text
            .insert(loaded_text.id(), loaded_text.clone().into());
        self.user_asset_path_to_id
            .insert(loaded_text.text_path.clone(), loaded_text.id());

        Ok(())
    }
}

/// Enum representing all possible states of text
#[derive(Clone, Debug)]
pub enum Text {
    Pending(PendingText),
    Loaded(LoadedText),
    Engine(EngineText),
    Failed(FailedText),
}

impl Text {
    pub fn id(&self) -> TextId {
        match self {
            Self::Pending(pending_text) => pending_text.id(),
            Self::Loaded(loaded_text) => loaded_text.id(),
            Self::Engine(engine_text) => engine_text.id(),
            Self::Failed(failed_text) => failed_text.id(),
        }
    }

    pub fn path(&self) -> &AssetPath {
        match self {
            Self::Pending(pending_text) => pending_text.text_path(),
            Self::Loaded(loaded_text) => loaded_text.text_path(),
            Self::Engine(engine_text) => engine_text.text_path(),
            Self::Failed(failed_text) => failed_text.text_path(),
        }
    }

    pub const fn text_type(&self) -> TextType {
        match self {
            Self::Pending(_) => PendingText::text_type(),
            Self::Loaded(_) => LoadedText::text_type(),
            Self::Engine(_) => EngineText::text_type(),
            Self::Failed(_) => FailedText::text_type(),
        }
    }

    pub fn as_pending_text(&self) -> Option<&PendingText> {
        if let Self::Pending(pending_text) = self {
            Some(pending_text)
        } else {
            None
        }
    }

    pub fn as_engine_text(&self) -> Option<&EngineText> {
        if let Self::Engine(engine_text) = self {
            Some(engine_text)
        } else {
            None
        }
    }

    pub fn as_loaded_text(&self) -> Option<&LoadedText> {
        if let Self::Loaded(loaded_text) = self {
            Some(loaded_text)
        } else {
            None
        }
    }

    pub fn as_failed_text(&self) -> Option<&FailedText> {
        if let Self::Failed(failed_text) = self {
            Some(failed_text)
        } else {
            None
        }
    }
}

/// A [`Text`] that is being loaded by a platform
#[derive(Clone, Debug)]
pub struct PendingText {
    id: TextId,
    text_path: AssetPath,
    set_up_watcher: bool,
}

impl PendingText {
    pub fn new(id: TextId, text_path: &AssetPath, set_up_watcher: bool) -> Self {
        Self {
            id,
            text_path: text_path.clone(),
            set_up_watcher,
        }
    }

    pub fn id(&self) -> TextId {
        self.id
    }

    pub const fn text_type() -> TextType {
        TextType::Pending
    }

    pub fn text_path(&self) -> &AssetPath {
        &self.text_path
    }

    pub fn set_up_watcher(&self) -> bool {
        self.set_up_watcher
    }
}

impl From<PendingText> for Text {
    fn from(value: PendingText) -> Self {
        Self::Pending(value)
    }
}

impl From<&FfiPendingText> for PendingText {
    fn from(value: &FfiPendingText) -> Self {
        let text_path = PathBuf::from(
            unsafe { CStr::from_ptr(value.text_path) }
                .to_string_lossy()
                .as_ref(),
        );
        Self::new(value.id, &text_path.into(), value.set_up_watcher)
    }
}

impl From<FfiPendingText> for PendingText {
    fn from(value: FfiPendingText) -> Self {
        (&value).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatType {
    Json,
    Toml,
    Csv,
    Text,
    Unimplemented(String),
}

impl Display for FormatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string_representation = match self {
            FormatType::Json => "json",
            FormatType::Toml => "toml",
            FormatType::Csv => "csv",
            FormatType::Text => "text",
            FormatType::Unimplemented(description) => description,
        };
        write!(f, "{}", string_representation)
    }
}

impl AsRef<str> for FormatType {
    fn as_ref(&self) -> &str {
        match self {
            FormatType::Json => "json",
            FormatType::Toml => "toml",
            FormatType::Csv => "csv",
            FormatType::Text => "text",
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

/// An internal, engine only text. It's [`AssetPath`]'s are unique, seperate
/// from user [`LoadedText`]s. This is used for things like shader fragments,
/// and internal material definitions.
#[derive(Clone, Debug)]
pub struct EngineText {
    id: TextId,
    text_path: AssetPath,
    format: FormatType,
    raw_text: String,
}

impl EngineText {
    pub fn new(id: TextId, text_path: &AssetPath, format: FormatType, raw_text: &str) -> Self {
        Self {
            id,
            text_path: text_path.clone(),
            format,
            raw_text: raw_text.to_string(),
        }
    }

    pub fn id(&self) -> TextId {
        self.id
    }

    pub fn text_path(&self) -> &AssetPath {
        &self.text_path
    }

    pub const fn text_type() -> TextType {
        TextType::Engine
    }

    pub fn format(&self) -> &FormatType {
        &self.format
    }

    pub fn raw_text(&self) -> &str {
        &self.raw_text
    }
}

impl From<EngineText> for Text {
    fn from(value: EngineText) -> Self {
        Self::Engine(value)
    }
}

#[derive(Clone, Debug)]
pub struct LoadedText {
    id: TextId,
    text_path: AssetPath,
    pub(crate) version: TextHash,
    pub(crate) format_type: FormatType,
    raw_text: String,
    watcher_set_up: bool,
}

impl LoadedText {
    pub fn new(
        id: TextId,
        text_path: &AssetPath,
        version: &TextHash,
        format_type: FormatType,
        raw_text: &str,
        watcher_set_up: bool,
    ) -> Self {
        Self {
            id,
            text_path: text_path.clone(),
            version: *version,
            format_type,
            raw_text: raw_text.to_string(),
            watcher_set_up,
        }
    }

    pub fn id(&self) -> TextId {
        self.id
    }

    pub fn text_path(&self) -> &AssetPath {
        &self.text_path
    }

    pub const fn text_type() -> TextType {
        TextType::Loaded
    }

    pub fn version(&self) -> &TextHash {
        &self.version
    }

    pub fn format_type(&self) -> &FormatType {
        &self.format_type
    }

    pub fn raw_text(&self) -> &str {
        &self.raw_text
    }

    pub fn watcher_set_up(&self) -> bool {
        self.watcher_set_up
    }
}

impl From<LoadedText> for Text {
    fn from(value: LoadedText) -> Self {
        Self::Loaded(value)
    }
}

#[derive(Clone, Debug)]
pub struct FailedText {
    id: TextId,
    text_path: AssetPath,
    failure_reason: String,
}

impl FailedText {
    pub fn new(id: TextId, text_path: &AssetPath, failure_reason: &str) -> Self {
        Self {
            id,
            text_path: text_path.clone(),
            failure_reason: failure_reason.to_string(),
        }
    }

    pub fn id(&self) -> TextId {
        self.id
    }

    pub fn text_path(&self) -> &AssetPath {
        &self.text_path
    }

    pub const fn text_type() -> TextType {
        TextType::Failed
    }

    pub fn failure_reason(&self) -> &str {
        &self.failure_reason
    }
}

impl From<FailedText> for Text {
    fn from(value: FailedText) -> Self {
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
    include!(concat!(env!("OUT_DIR"), "/text_events_generated.rs"));
}
