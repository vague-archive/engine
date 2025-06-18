use anyhow::{Result, anyhow, bail};
use glam::Vec4;
use indexmap::IndexMap;
use log::{error, warn};
use snapshot::{Deserialize, Serialize};
use void_public::{
    graphics::TextureId,
    material::{FilterMode, MaterialId, MaterialParameters, ShaderTemplateId, UNIFORM_LIMIT},
};

use super::{
    DEFAULT_POST_PROCESSING_SHADER_ID, DEFAULT_SHADER_ID, FRAGMENT_COLOR_INSERTION_POINT,
    ShaderInsertionPoint, ShaderSnippet, TEXTURE_SHADER_INSERTION_POINT,
    UNIFORM_SHADER_INSERTION_POINT, WORLD_OFFSET_INSERTION_POINT,
    fixed_size_vec::FixedSizeVec,
    textures::{MaterialTextures, TextureMaterialSpec},
    uniforms::{MaterialUniforms, Uniform, UniformType, UniformValue},
};
use crate::resource_managers::texture_asset_manager::TextureAssetManager;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, serde::Deserialize)]
pub enum MaterialType {
    Sprite,
    PostProcessing,
}

impl MaterialType {
    /// # Errors
    ///
    /// Returns an error variant if the [`ShaderTemplateId`] is not 0 or 1. Currently there are only
    /// two shader templates, the `default.fsh` relates to [`MaterialType::Standard`] and `post_processing_default.fsh`
    /// relates to [`MaterialType::PostProcessing`]
    pub fn try_from_shader_template_id(shader_template_id: ShaderTemplateId) -> Result<Self> {
        match shader_template_id {
            ShaderTemplateId(0) => Ok(MaterialType::Sprite),
            ShaderTemplateId(1) => Ok(MaterialType::PostProcessing),
            unknown_shader_template_id => bail!(
                "MaterialType::try_from_shader_template_id() shader_template_id {unknown_shader_template_id} is not known"
            ),
        }
    }

    pub fn into_shader_template_id(&self) -> ShaderTemplateId {
        match self {
            MaterialType::Sprite => DEFAULT_SHADER_ID,
            MaterialType::PostProcessing => DEFAULT_POST_PROCESSING_SHADER_ID,
        }
    }
}

struct ShaderSnippetParsingHelper {
    pub get_world_offset_body: Option<String>,
    pub get_fragment_color_body: Option<String>,
    pub uniform_types_with_defaults: Option<IndexMap<String, UniformValue>>,
    pub texture_descs: Option<IndexMap<String, FilterMode>>,
}

/// The data representation of an instance of a [`ShaderTemplate`]. Contains the strings for the fragment ([`Self::get_fragment_color_body`])
/// and vertex ([`Self::get_world_offset_body`]) function bodies. Also optionally contains the uniform names and their corresponding [`UniformType`]
/// as well as optionally holding texture names and their corresponding [`FilterMode`] (which will grow as texture defintions become more complex).
/// This information is used to generate a unique shader per pipeline, filling out the function bodies as well as uniform struct, uniform group bindings,
/// and texture group bindings. The information for a specific set of [`UniformValue`] values is a [`MaterialParam`], and the [`MaterialManager`] manages
/// creating the correct information for a graphics pipeline to pass in specific [`UniformValue`] values
#[derive(Clone, Debug)]
pub struct Material {
    shader_template_id: ShaderTemplateId,
    material_id: MaterialId,
    name: String,
    get_world_offset_body: String,
    get_fragment_color_body: String,
    uniform_types_with_defaults: Option<MaterialUniforms>,
    uniform_buffer_size: usize,
    texture_descs: Option<MaterialTextures>,
}

impl Material {
    /// # Errors
    ///
    /// - Will surface any errors from [`Self::parse_shader_snippets`]
    /// - If [`Self::parse_shader_snippets`] does not find `get_fragment_color_body` or `get_world_offset_body`, then will return an error since an empty function body will create invalid WGSL
    pub(crate) fn new(
        shader_template_id: ShaderTemplateId,
        material_id: MaterialId,
        name: &str,
        shader_snippets: &[(ShaderInsertionPoint, ShaderSnippet)],
    ) -> Result<Self> {
        let shader_snippet_parsing_helper = Self::parse_shader_snippets(shader_snippets)?;
        let material_uniforms = shader_snippet_parsing_helper
            .uniform_types_with_defaults
            .map(|map| {
                let mut material_uniforms = MaterialUniforms::new(material_id, map);
                material_uniforms.sort();
                material_uniforms
            });

        let material_textures = shader_snippet_parsing_helper.texture_descs.map(|map| {
            let transformed_map = map.iter().map(|(name, filter_mode)| {
                (
                    TextureMaterialSpec::new(name, filter_mode),
                    TextureAssetManager::missing_texture_id(),
                )
            });
            let mut material_textures =
                MaterialTextures::new_from_iter(material_id, transformed_map);
            material_textures.sort();
            material_textures
        });

        let uniform_buffer_size = Self::calculate_uniform_buffer_size(
            &MaterialType::try_from_shader_template_id(shader_template_id).unwrap(),
            material_uniforms.as_ref(),
        )?;
        Ok(Self {
            shader_template_id,
            material_id,
            name: name.to_string(),
            get_fragment_color_body: shader_snippet_parsing_helper
                .get_fragment_color_body
                .ok_or(anyhow!(
                    "Material::new() shader snippets must include a get_fragment_color_body string"
                ))?,
            get_world_offset_body: shader_snippet_parsing_helper.get_world_offset_body.ok_or(
                anyhow!(
                    "Material::new() shader snippets must include a get_world_offset_body string"
                ),
            )?,
            uniform_types_with_defaults: material_uniforms,
            uniform_buffer_size,
            texture_descs: material_textures,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub fn world_offset_body(&self) -> &str {
        &self.get_world_offset_body
    }

    pub fn fragment_color_body(&self) -> &str {
        &self.get_fragment_color_body
    }

    /// # Panics
    ///
    /// Will panic if an unexpected [`ShaderTemplateId`] is encountered in [`MaterialType::try_from_shader_template_id`]
    pub fn material_type(&self) -> MaterialType {
        MaterialType::try_from_shader_template_id(self.shader_template_id).unwrap()
    }

    pub fn shader_template_id(&self) -> ShaderTemplateId {
        self.shader_template_id
    }

    pub fn texture_descs(&self) -> Option<&MaterialTextures> {
        self.texture_descs.as_ref()
    }

    pub fn uniform_types_with_defaults(&self) -> Option<&MaterialUniforms> {
        self.uniform_types_with_defaults.as_ref()
    }

    /// Calculates the leftover space not used up by the uniforms the user provides. For example, if the user
    /// has a uniforms that are one of each type f32, vec4f and array<vec4f, 3>, we will need to pad out
    /// those number of f32s, plus our reserved f32s, minus the [`UNIFORM_LIMIT`]. So in that case, and if
    /// our uniform limit was 256, we would subtract 1 (f32) + 4(vec4f) + 12 (array<vec4f, 3>) + 24 (`reserved_uniform_size`)
    /// to get 215
    ///
    /// # Errors
    ///
    /// Returns an error if the calculated uniform buffer size is greater than or equal to the [`UNIFORM_LIMIT`]
    fn calculate_uniform_buffer_size(
        material_type: &MaterialType,
        uniforms_iterator: Option<&MaterialUniforms>,
    ) -> Result<usize> {
        let reserved_uniform_size = match material_type {
            MaterialType::Sprite => MaterialUniforms::calculate_reserved_uniform_buffer_size(),
            MaterialType::PostProcessing => 0,
        };
        let uniforms_offset_total_size = match uniforms_iterator {
            Some(uniforms_iterator) => uniforms_iterator
                .uniforms_map
                .iter()
                .fold(reserved_uniform_size, |accumulator, (_, uniform)| {
                    accumulator + uniform.generate_offset() as usize
                }),
            None => reserved_uniform_size,
        };

        if uniforms_offset_total_size >= UNIFORM_LIMIT {
            bail!(
                "The uniform offset size is {uniforms_offset_total_size}, which is great than {UNIFORM_LIMIT}. Please reduce the size of your uniforms"
            );
        }

        Ok(UNIFORM_LIMIT - uniforms_offset_total_size)
    }

    /// Attempts to associate [`ShaderInsertionPoint`]s, such as %uniforms, %textures, with a [`ShaderSnippet`]
    ///
    /// # Warnings
    ///
    /// - Will send an error log if two of the same [`ShaderInsertionPoint`] are found, and will use whichever is first
    ///
    /// # Errors
    ///
    /// - Will error if the [`ShaderInsertionPoint`] is associated with the wrong [`ShaderSnippet`] variant. The expectations is:
    ///   - `%uniform_types` is associated with [`ShaderSnippet::Uniforms`]
    ///   - `%texture_descs` is associated with [`ShaderSnippet::Textures`]
    ///   - `%get_world_offset` and `%get_fragment_color` are associated with [`ShaderSnippet::FunctionBody`]
    fn parse_shader_snippets<S>(shader_snippets: S) -> Result<ShaderSnippetParsingHelper>
    where
        S: AsRef<[(ShaderInsertionPoint, ShaderSnippet)]>,
    {
        let mut get_fragment_color_body = None;
        let mut get_world_offset_body = None;
        let mut texture_descs = None;
        let mut uniform_types_with_defaults = None;
        for (shader_insertion_point, shader_snippet) in shader_snippets.as_ref() {
            match shader_insertion_point.0.as_str() {
                insertion_point if insertion_point == UNIFORM_SHADER_INSERTION_POINT => {
                    if uniform_types_with_defaults.is_some() {
                        error!(
                            "Material::parse_shader_snippets() There are more than 1 shader snippets marked as uniforms. We are using the first uniforms encountered, but this is likely an error"
                        );
                        continue;
                    }
                    if let ShaderSnippet::Uniforms(snippet_uniforms) = shader_snippet {
                        uniform_types_with_defaults = Some(snippet_uniforms.clone());
                    } else {
                        bail!(
                            "Material::parse_shader_snippets() Uniform shader insertion point must use the Uniform ShaderSnippet variant"
                        )
                    }
                }
                insertion_point if insertion_point == FRAGMENT_COLOR_INSERTION_POINT => {
                    if get_fragment_color_body.is_some() {
                        error!(
                            "Material::parse_shader_snippets() There are more than 1 shader snippets marked as get_fragment_color. We are using the first one encountered, but this is likely an error"
                        );
                        continue;
                    }
                    if let ShaderSnippet::FunctionBody(function_body) = shader_snippet {
                        get_fragment_color_body = Some(function_body.clone());
                    } else {
                        bail!(
                            "Material::parse_shader_snippets() get_fragment_color shader insertion point must use the FunctionBody ShaderSnippet variant"
                        )
                    }
                }
                insertion_point if insertion_point == WORLD_OFFSET_INSERTION_POINT => {
                    if get_world_offset_body.is_some() {
                        error!(
                            "Material::parse_shader_snippets() There are more than 1 shader snippets marked as get_world_offset. We are using the first one encountered, but this is likely an error"
                        );
                        continue;
                    }
                    if let ShaderSnippet::FunctionBody(function_body) = shader_snippet {
                        get_world_offset_body = Some(function_body.clone());
                    } else {
                        bail!(
                            "Material::parse_shader_snippets() get_world_offset_body shader insertion point must use the FunctionBody ShaderSnippet variant"
                        )
                    }
                }
                insertion_point if insertion_point == TEXTURE_SHADER_INSERTION_POINT => {
                    if texture_descs.is_some() {
                        error!(
                            "Material::parse_shader_snippets() There are more than 1 shader snippets marked as textures. We are using the first textures encountered, but this is likely an error"
                        );
                        continue;
                    }
                    if let ShaderSnippet::Textures(snippet_textures) = shader_snippet {
                        texture_descs = Some(snippet_textures.clone());
                    } else {
                        bail!(
                            "Material::parse_shader_snippets() Texture shader insertion point must use the Texture ShaderSnippet variant"
                        )
                    }
                }
                unknown_insertion_point => bail!(
                    "Material::parse_shader_snippets() In Material creation, found an Unknown insertion point {unknown_insertion_point} found, must be one of {}",
                    ShaderInsertionPoint::valid_insertion_points().join(",")
                ),
            }
        }

        Ok(ShaderSnippetParsingHelper {
            get_world_offset_body,
            get_fragment_color_body,
            uniform_types_with_defaults,
            texture_descs,
        })
    }

    /// Can update either or both `name` and the various shader data touch points (ie, uniforms, textures, fragment and vertex shader data) with
    /// a slice of ([`ShaderInsertionPoint`], [`ShaderSnippet`])
    ///
    /// # Warnings
    ///
    /// - Will warn if both `name` and `shader_snippets` are [`Option::None`]
    ///
    /// # Errors
    ///
    /// - Will bubble errors up from [`Self::parse_shader_snippets`]
    #[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
    pub(crate) fn update_material<S>(
        &mut self,
        name: Option<&str>,
        shader_snippets: Option<S>,
    ) -> Result<()>
    where
        S: AsRef<[(ShaderInsertionPoint, ShaderSnippet)]>,
    {
        if name.is_none() && shader_snippets.is_none() {
            warn!(
                "Material::update_material() Attempted to update material {} with only none values",
                self.material_id
            );
            return Ok(());
        }

        if let Some(name) = name {
            self.name = name.to_string();
        }

        if let Some(shader_snippets) = shader_snippets {
            let parsing_helper = Self::parse_shader_snippets(shader_snippets)?;
            if let Some(get_world_offset_body) = parsing_helper.get_world_offset_body {
                self.get_world_offset_body = get_world_offset_body;
            }
            if let Some(get_fragment_color_body) = parsing_helper.get_fragment_color_body {
                self.get_fragment_color_body = get_fragment_color_body;
            }
            if let Some(uniform_types_with_defaults) = parsing_helper.uniform_types_with_defaults {
                let mut material_uniform =
                    MaterialUniforms::new(self.material_id, uniform_types_with_defaults);
                material_uniform.sort();
                self.uniform_types_with_defaults = Some(material_uniform);
            }
            if let Some(texture_descs) = parsing_helper.texture_descs {
                let mut material_textures = MaterialTextures::new_from_iter(
                    self.material_id,
                    texture_descs.iter().map(|(name, filter_mode)| {
                        (
                            TextureMaterialSpec::new(name, filter_mode),
                            TextureAssetManager::missing_texture_id(),
                        )
                    }),
                );
                material_textures.sort();
                self.texture_descs = Some(material_textures);
            }
        }

        Ok(())
    }

    /// Extracts the current values of the [`MaterialUniforms`] from a `uniform_buffer`.
    ///
    /// # Errors
    ///
    /// * Returns an error if there are no uniforms on material
    /// * Returns an error if the `uniform_buffer` is too small
    pub fn get_current_uniforms(&self, uniform_buffer: &[f32]) -> Result<MaterialUniforms> {
        let mut remaining_uniform_buffer = uniform_buffer;
        let Some(uniform_map) = &self.uniform_types_with_defaults else {
            bail!(
                "Material::get_uniform_buffer() Attempted to get uniform buffer for material {}, which no uniforms",
                &self.material_id()
            );
        };

        let expected_buffer_size = UniformType::doubleword_size(
            uniform_map
                .uniforms_map
                .iter()
                .map(|(_, uniform)| uniform.uniform_type()),
        );

        if uniform_buffer.len() > expected_buffer_size && uniform_buffer.len() != UNIFORM_LIMIT {
            bail!(
                "Material::get_uniform_buffer() uniform_buffer is of length {} but should be {expected_buffer_size}",
                uniform_buffer.len()
            );
        }

        let material_uniforms = uniform_map
            .uniforms_map
            .iter()
            .map(|(name, value)| {
                let value: UniformValue = match value.uniform_type() {
                    UniformType::Array(array_size) => {
                        let (values, remaining) = remaining_uniform_buffer.split_at(4 * array_size);
                        remaining_uniform_buffer = remaining;
                        let vec4_array = (0..array_size)
                            .map(|index| {
                                let starting_index = index * 4;
                                Vec4::from_slice(&values[starting_index..(starting_index + 4)])
                            })
                            .collect::<Vec<Vec4>>();
                        FixedSizeVec::new(&vec4_array).into()
                    }
                    UniformType::F32 => {
                        let (value, remaining) = remaining_uniform_buffer.split_first().unwrap();
                        remaining_uniform_buffer = remaining;
                        (*value).into()
                    }
                    UniformType::Vec4 => {
                        let (values, remaining) = remaining_uniform_buffer.split_at(4);
                        remaining_uniform_buffer = remaining;
                        Vec4::from_slice(values).into()
                    }
                };
                (name.clone(), value)
            })
            .collect::<IndexMap<String, UniformValue>>();
        // println!("{material_uniforms:?}");
        Ok(MaterialUniforms::new(self.material_id, material_uniforms))
    }

    /// Extracts the current values of the [`MaterialTextures`] from a `texture_buffer`.
    ///
    /// # Errors
    ///
    /// * Returns an error if there are no textures on material
    pub fn get_current_textures(&self, texture_buffer: &[TextureId]) -> Result<MaterialTextures> {
        match &self.texture_descs {
            Some(texture_descs) => {
                let updated_map = texture_descs
                    .iter()
                    .enumerate()
                    .map(|(index, (material_spec, default_texture_id))| {
                        let texture_id = match texture_buffer.get(index) {
                            Some(texture_id) => *texture_id,
                            None => *default_texture_id,
                        };
                        (material_spec.clone(), texture_id)
                    })
                    .collect::<IndexMap<TextureMaterialSpec, TextureId>>();
                Ok(MaterialTextures::new(self.material_id, updated_map))
            }
            None => bail!(
                "Material::get_current_textures failed because material {} has no MaterialTextures",
                self.material_id
            ),
        }
    }

    /// Takes the data in a [`Material`], marries it with a [`ShaderTemplate`] and creates wgsl shader text. Not intenteded to
    /// be called directly, but instead through [`MaterialManager`]. The sort order matters, and details can be found in the [`Ord`] implementation of [`UniformType`]
    ///
    /// # Errors
    ///
    /// - Will return an error if an unexpected [`ShaderInsertionPoint`] is found
    pub(crate) fn generate_shader_text(
        &self,
        shader_insertion_points: &[ShaderInsertionPoint],
        shader_template_text: &str,
    ) -> Result<String> {
        let mut shader_text = shader_template_text.to_string();

        for shader_insertion_point in shader_insertion_points {
            let string_to_be_inserted = match shader_insertion_point.0.as_str() {
                insertion_point if insertion_point == WORLD_OFFSET_INSERTION_POINT => {
                    self.get_world_offset_body.clone()
                }
                insertion_point if insertion_point == FRAGMENT_COLOR_INSERTION_POINT => {
                    self.get_fragment_color_body.clone()
                }
                insertion_point if insertion_point == UNIFORM_SHADER_INSERTION_POINT => {
                    let uniforms = if let Some(uniforms_types_with_defaults) =
                        &self.uniform_types_with_defaults
                    {
                        uniforms_types_with_defaults.clone()
                    } else {
                        MaterialUniforms::empty(self.material_id)
                    };
                    uniforms.generate_dynamic_uniforms_shader_string(
                        &self.material_type(),
                        self.uniform_buffer_size,
                    )
                }
                insertion_point if insertion_point == TEXTURE_SHADER_INSERTION_POINT => {
                    if let Some(texture_descs) = &self.texture_descs {
                        if !texture_descs.is_empty() {
                            let mut textures_string = "".to_string();
                            for (i, (texture_spec, _)) in texture_descs.iter().enumerate() {
                                let texture_name = texture_spec.name();
                                let texture_binding = 2 * i;
                                let sampler_binding = texture_binding + 1;
                                textures_string.push_str(format!("@group(2) @binding({texture_binding}) var {texture_name} : texture_2d<f32>;\n").as_str());
                                textures_string.push_str(format!("@group(2) @binding({sampler_binding}) var sampler_{texture_name} : sampler;\n\n").as_str());
                            }
                            textures_string
                        } else {
                            "".to_string()
                        }
                    } else {
                        "".to_string()
                    }
                }
                unknown_insertion_point => {
                    bail!(
                        "Material::generate_shader_text() In shader text generation, found an unknown insertion point {unknown_insertion_point} found, must be one of {}",
                        ShaderInsertionPoint::valid_insertion_points().join(",")
                    )
                }
            };
            shader_text =
                shader_text.replace(shader_insertion_point.0.as_str(), &string_to_be_inserted);
        }
        Ok(shader_text)
    }

    pub fn generate_default_material_uniforms(&self) -> Option<&MaterialUniforms> {
        self.uniform_types_with_defaults.as_ref()
    }

    pub fn generate_default_material_parameters(&self) -> MaterialParameters {
        let Some(default_material_uniforms) = self.generate_default_material_uniforms() else {
            return MaterialParameters::new(self.material_id);
        };

        let mut buffer_data = [0.0; UNIFORM_LIMIT];
        let mut buffer_data_slice = buffer_data.as_mut_slice();

        for (_, uniform) in default_material_uniforms.uniforms_map.iter() {
            let (length, f32_slice) = match uniform {
                UniformValue::Array(uniform_var) => (
                    uniform_var.current_value().generate_offset() as usize,
                    uniform_var.current_value().as_f32_buffer(),
                ),
                UniformValue::F32(uniform_var) => (
                    uniform_var.current_value().generate_offset() as usize,
                    uniform_var.current_value().as_f32_buffer(),
                ),
                UniformValue::Vec4(uniform_var) => (
                    uniform_var.current_value().generate_offset() as usize,
                    uniform_var.current_value().as_f32_buffer(),
                ),
            };

            buffer_data_slice[..length].copy_from_slice(&f32_slice);
            buffer_data_slice = &mut buffer_data_slice[length..];
        }

        MaterialParameters::new_with_buffer(self.material_id, buffer_data)
    }

    pub fn generate_default_material_textures(&self) -> Option<&MaterialTextures> {
        self.texture_descs.as_ref()
    }

    /// Ensures that the map of uniform names to [`UniformValue`] in [`MaterialUniforms`] is what is expected in the
    /// [`Material`]'s map of uniform names to [`UniformType`]. The sort order matters, and details can be found in the [`Ord`] implementation of [`UniformType`]
    ///
    /// # Errors
    ///
    /// We are using [`Result::Err`] here to return the cleaned up [`MaterialUniforms`] if any incorrect [`UniformValue`] are found.
    /// If no mistakes are found, the user can continue to use the [`MaterialUniforms`] that were passed in
    pub fn validate_material_uniforms(
        &self,
        material_uniforms: &MaterialUniforms,
    ) -> Result<(), MaterialUniforms> {
        let Some(default_uniforms) = &self.uniform_types_with_defaults else {
            return Err(MaterialUniforms::empty(self.material_id));
        };

        let mut corrected_uniforms = None;

        // correct uniform names

        let uniform_names_match = material_uniforms.uniforms_map.len()
            == default_uniforms.uniforms_map.len()
            && material_uniforms
                .uniforms_map
                .keys()
                .all(|name| default_uniforms.uniforms_map.contains_key(name));

        if !uniform_names_match {
            let corrected_uniforms = corrected_uniforms.insert(material_uniforms.clone());

            // remove names which don't exist in the defaults
            corrected_uniforms
                .uniforms_map
                .retain(|name, _| default_uniforms.uniforms_map.contains_key(name));

            // add missing names which exist in the defaults
            for (uniform_name, uniform_value) in default_uniforms.uniforms_map.iter() {
                if !material_uniforms.uniforms_map.contains_key(uniform_name) {
                    corrected_uniforms
                        .uniforms_map
                        .insert(uniform_name.clone(), uniform_value.clone());
                }
            }
        }

        // correct uniform values

        let invalid_uniforms: Vec<_> = corrected_uniforms
            .as_ref()
            .unwrap_or(material_uniforms)
            .uniforms_map
            .iter()
            .filter_map(|(uniform_name, uniform_value)| {
                let expected_uniform_value = &default_uniforms.uniforms_map[uniform_name];
                let expected_uniform_type = expected_uniform_value.uniform_type();

                if !uniform_value.matches_uniform_type(&expected_uniform_type) {
                    warn!(
                        "uniform {uniform_name} was input with type {} when {expected_uniform_type} was expected",
                        uniform_value.uniform_type()
                    );
                    Some((uniform_name.clone(), expected_uniform_value.clone()))
                } else {
                    None
                }
            })
            .collect();

        if !invalid_uniforms.is_empty() {
            // replace invalid uniform values with the expected values
            corrected_uniforms
                .get_or_insert_with(|| material_uniforms.clone())
                .uniforms_map
                .extend(invalid_uniforms);
        }

        match corrected_uniforms {
            Some(mut uniforms) => {
                uniforms.sort();
                Err(uniforms)
            }
            None => Ok(()),
        }
    }
}
