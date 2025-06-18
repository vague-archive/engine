use std::{
    borrow::Borrow,
    ffi::{CStr, CString},
    hash::Hash,
    ops::Deref,
};

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use void_public::{
    graphics::TextureId,
    material::{FilterMode, MaterialId},
};

/// This specifies details about a texture a user can pass into a [`Material`]
#[derive(Clone, Debug, Eq)]
pub struct TextureMaterialSpec {
    name: String,
    sampler_filter_mode: FilterMode,
}

impl AsRef<TextureMaterialSpec> for TextureMaterialSpec {
    fn as_ref(&self) -> &TextureMaterialSpec {
        self
    }
}

impl TextureMaterialSpec {
    pub fn new(name: &str, sampler_filter_mode: &FilterMode) -> Self {
        Self {
            name: name.to_string(),
            sampler_filter_mode: *sampler_filter_mode,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sampler_filter_mode(&self) -> &FilterMode {
        &self.sampler_filter_mode
    }

    pub fn into_public_texture_desc(
        &self,
        texture_id: TextureId,
    ) -> void_public::material::TextureDesc {
        void_public::material::TextureDesc {
            name: CString::new(self.name())
                .unwrap_or_else(|_| c"texture_desc_malfromed".to_owned())
                .into_raw(),
            sampler_filter_mode: *self.sampler_filter_mode(),
            texture_id,
        }
    }
}

impl Hash for TextureMaterialSpec {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl Borrow<str> for TextureMaterialSpec {
    fn borrow(&self) -> &str {
        &self.name
    }
}

impl PartialEq for TextureMaterialSpec {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl PartialOrd for TextureMaterialSpec {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TextureMaterialSpec {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl<S: AsRef<str>> PartialEq<S> for TextureMaterialSpec {
    fn eq(&self, other: &S) -> bool {
        self.name == other.as_ref()
    }
}

/// These represents one specific mapping of [`TextureMaterialSpec`]s with specified values to a [`TextureId`].
/// For example, if we have a `noise` texture this would allow a user to specify different noise textures for their
/// specific use case while still using the same [`Material`]
#[derive(Debug, Clone)]
pub struct MaterialTextures {
    material_id: MaterialId,
    pub(crate) textures_map: IndexMap<TextureMaterialSpec, TextureId>,
}

impl Deref for MaterialTextures {
    type Target = IndexMap<TextureMaterialSpec, TextureId>;

    fn deref(&self) -> &Self::Target {
        &self.textures_map
    }
}

impl MaterialTextures {
    pub fn new(
        material_id: MaterialId,
        textures_map: IndexMap<TextureMaterialSpec, TextureId>,
    ) -> Self {
        Self {
            material_id,
            textures_map,
        }
    }

    pub fn new_from_iter<M, T, I>(material_id: MaterialId, iter: I) -> Self
    where
        M: AsRef<TextureMaterialSpec>,
        T: AsRef<TextureId>,
        I: IntoIterator<Item = (M, T)>,
    {
        let textures_map = iter
            .into_iter()
            .map(|(spec, texture_id)| (spec.as_ref().clone(), *texture_id.as_ref()))
            .collect::<IndexMap<_, _>>();

        Self::new(material_id, textures_map)
    }

    pub fn from_public_texture_descs<T: AsRef<void_public::material::TextureDesc>>(
        material_id: MaterialId,
        texture_descs: &[T],
    ) -> Self {
        Self::new_from_iter(
            material_id,
            texture_descs.iter().map(|texture_desc| {
                let texture_desc = texture_desc.as_ref();
                let name = unsafe { CStr::from_ptr(texture_desc.name) }.to_string_lossy();
                let texture_material_spec =
                    TextureMaterialSpec::new(name.as_ref(), &texture_desc.sampler_filter_mode);
                (texture_material_spec, texture_desc.texture_id)
            }),
        )
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub fn update(&mut self, name: &str, texture_id: TextureId) -> Result<()> {
        let key = self
            .get_key_value(name)
            .ok_or(anyhow!("Could not find key with name {name}"))?;
        self.textures_map.insert(key.0.clone(), texture_id);
        Ok(())
    }

    pub fn sort_into_vec(&self) -> Vec<(&TextureMaterialSpec, &TextureId)> {
        let mut output: Vec<(&TextureMaterialSpec, &TextureId)> = self.iter().collect();
        output.sort_by(|a, b| a.0.cmp(b.0));
        output
    }

    pub fn output_texture_ids(&self) -> Vec<TextureId> {
        self.sort_into_vec()
            .iter()
            .map(|(_, texture_id)| **texture_id)
            .collect()
    }

    pub fn sort(&mut self) {
        self.textures_map = self
            .sort_into_vec()
            .into_iter()
            .map(|(a, b)| (a.clone(), *b))
            .collect::<IndexMap<_, _>>();
    }
}
