use std::{collections::HashMap, fmt::Display, ops::Deref};

use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use materials::Material;
use uniforms::{UniformValue, sort_uniforms_by_name_and_type};
use void_public::{
    AssetPath, EventWriter,
    event::graphics::NewText,
    material::{FilterMode, MaterialId, ShaderTemplateId},
    text::TextId,
};

use super::text_asset_manager::PendingText;
use crate::ecs_module::{MaterialManager, MaterialManagerTextTypes, TextAssetManager};

pub mod fixed_size_vec;
pub mod material_parameters_extension;
pub mod materials;
pub mod textures;
pub mod toml;
pub mod uniforms;

pub const UNIFORM_SHADER_INSERTION_POINT: &str = "%uniforms";
pub const TEXTURE_SHADER_INSERTION_POINT: &str = "%textures";
pub const WORLD_OFFSET_INSERTION_POINT: &str = "%get_world_offset";
pub const FRAGMENT_COLOR_INSERTION_POINT: &str = "%get_fragment_color";

/// A point defined in a shader template, ie default.fsh, that will have a value inserted into it
/// For example, %uniforms is the `ShaderInsertionPoint`, and will have a uniform struct inserted in its place
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ShaderInsertionPoint(String);

impl ShaderInsertionPoint {
    pub fn uniform() -> Self {
        Self(UNIFORM_SHADER_INSERTION_POINT.to_string())
    }

    pub fn texture() -> Self {
        Self(TEXTURE_SHADER_INSERTION_POINT.to_string())
    }

    pub fn world_offset() -> Self {
        Self(WORLD_OFFSET_INSERTION_POINT.to_string())
    }

    pub fn fragment_color() -> Self {
        Self(FRAGMENT_COLOR_INSERTION_POINT.to_string())
    }

    pub const fn valid_insertion_points() -> [&'static str; 4] {
        [
            UNIFORM_SHADER_INSERTION_POINT,
            TEXTURE_SHADER_INSERTION_POINT,
            WORLD_OFFSET_INSERTION_POINT,
            FRAGMENT_COLOR_INSERTION_POINT,
        ]
    }

    pub fn is_string_valid_insertion_point(test_string: &str) -> bool {
        Self::valid_insertion_points().contains(&test_string)
    }

    /// # Errors
    ///
    /// - Will error if attempting to use an invalid string, a string not in the list provided by [`Self::valid_insertion_points`]
    pub fn try_from_str<S: AsRef<str>>(value: S) -> Result<Self> {
        let value = value.as_ref();
        if !ShaderInsertionPoint::is_string_valid_insertion_point(value) {
            bail!(
                "ShaderInsertionPoint::try_from_str() Unexpected ShaderInsertionPoint {value} encountered, insertion point must be one of {}",
                Self::valid_insertion_points().join(",")
            );
        }
        Ok(Self(value.to_string()))
    }
}

impl Deref for ShaderInsertionPoint {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for ShaderInsertionPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents the data of a possible shader snippet to be injected into our
/// shader templates, such as default.fsh. This is used at a [`ShaderInjectionPoint`]
/// to create a string variant for a given shader template that will create a unique [`Material`].
/// For example, %uniforms would be the [`ShaderInjectionPoint`], and we would take the data
/// from a [`ShaderSnippet::Uniforms`] and create a struct in the wgsl code
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderSnippet {
    FunctionBody(String),
    Uniforms(IndexMap<String, UniformValue>),
    Textures(IndexMap<String, FilterMode>),
}

/// The [`ShaderTemplate`] is fairly self explanatory, but it is worth noting that it is more generic
/// than it has to be. This is intentional, as a user with source code access and deep graphics experience
/// could use the [`ShaderTemplate`] with their own custom built [`MaterialManager`] to manage shaders themselves.
#[derive(Clone, Debug)]
pub struct ShaderTemplate {
    id: ShaderTemplateId,
    name: String,
    shader_text: String,
    shader_insertion_points: Vec<ShaderInsertionPoint>,
}

impl ShaderTemplate {
    pub fn id(&self) -> ShaderTemplateId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl MaterialManager {
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    /// Reads a [`ShaderTemplate`] .fsh from a given [`AssetPath`].
    ///
    /// # Errors
    ///
    /// * This returns any errors from [`TextAssetManager::load_text`].
    pub fn load_shader_template_from_path<'a>(
        &mut self,
        name: &str,
        path: &AssetPath,
        new_text_event_writer: &EventWriter<NewText<'_>>,
        text_asset_manager: &'a mut TextAssetManager,
    ) -> Result<&'a PendingText> {
        let pending_shader_template = match text_asset_manager.load_text(
            path,
            false,
            new_text_event_writer,
        ) {
            Ok(shader_template) => shader_template,
            Err(err) => bail!(
                "MaterialManager::reader_shader_template_from_path Error starting shader template text load: {path:?}: {err}"
            ),
        };
        self.pending_texts.insert(
            pending_shader_template.id(),
            MaterialManagerTextTypes::ShaderFragment(name.to_string()),
        );
        Ok(pending_shader_template)
    }

    /// Reads a [`Material`] toml from a given [`AssetPath`].
    ///
    /// # Errors
    ///
    /// * This returns an error if the [`ShaderTemplateId`] is not found.
    /// * This returns an error if the `material_name` is already pending. [`Material`] names must be unique.
    /// * This returns any errors from [`TextAssetManager::load_text`].
    pub fn load_material_from_path<'a>(
        &mut self,
        shader_template_id: ShaderTemplateId,
        material_name: &str,
        path: &AssetPath,
        set_up_watcher: bool,
        event_writer: &EventWriter<NewText<'_>>,
        text_asset_manager: &'a mut TextAssetManager,
    ) -> Result<&'a PendingText> {
        if self
            .shader_templates
            .get(*shader_template_id as usize)
            .is_none()
        {
            bail!(
                "MaterialManger::read_material_from_path Could not attempt to read material {material_name} at {path:?} because ShaderTemplateId {shader_template_id} is not found"
            );
        }

        let material_name_is_pending = self.pending_texts.iter().any(|(_, pending_text)| {
            if let MaterialManagerTextTypes::MaterialDefinition(_, pending_material_name) =
                pending_text
            {
                pending_material_name == material_name
            } else {
                false
            }
        });

        if material_name_is_pending {
            bail!(
                "MaterialManager::read_material_from_path Material name {material_name} is already in the process of being read"
            );
        }

        let pending_material_definition = match text_asset_manager.load_text(
            path,
            set_up_watcher,
            event_writer,
        ) {
            Ok(material_definition) => material_definition,
            Err(err) => bail!(
                "MaterialManager::read_materail_from_path Error starting material {material_name} definition text load: {path:?}: {err}"
            ),
        };

        self.pending_texts.insert(
            pending_material_definition.id(),
            MaterialManagerTextTypes::MaterialDefinition(
                shader_template_id,
                material_name.to_string(),
            ),
        );
        Ok(pending_material_definition)
    }

    pub fn get_material(&self, material_id: MaterialId) -> Option<&Material> {
        self.materials
            .iter()
            .find(|material| material.material_id() == material_id)
    }

    /// # Errors
    ///
    /// - Can trigger errors defined in [`Self::register_material`]
    /// - Can encounter an error if the toml crate is unable to read the toml string
    /// - Can encounter an error defined in [`TomlMaterial::generate_shader_snippets`]
    pub fn register_material_from_string(
        &mut self,
        shader_template_id: ShaderTemplateId,
        material_name: &str,
        toml_string: &str,
    ) -> Result<MaterialId> {
        use ::toml::from_str;
        use toml::TomlMaterial;

        let toml_material: TomlMaterial = from_str(toml_string)?;
        let shader_snippets = toml_material.generate_shader_snippets()?;
        self.register_material(
            shader_template_id,
            material_name,
            shader_snippets.as_slice(),
        )
    }

    /// Note, The sort order matters, and details can be found in the [`Ord`] implementation of [`UniformType`]
    ///
    /// # Errors
    ///
    /// If the [`MaterialId`] does not exist in [`MaterialManager`], an error is returned
    pub fn uniform_names_and_default_values(
        &self,
        material_id: MaterialId,
    ) -> Result<Option<Vec<(&str, &UniformValue)>>> {
        if let Some(material) = self.materials.get(material_id.0 as usize) {
            Ok(material
                .uniform_types_with_defaults()
                .map(|material_uniform| sort_uniforms_by_name_and_type(material_uniform)))
        } else {
            bail!(
                "MaterialManager::uniforms_names_and_types() Could not find material with id: {material_id}"
            )
        }
    }

    /// # Errors
    ///
    /// - If the [`MaterialId`] does not exist in [`MaterialManager`], an error is returned
    /// - If the [`ShaderTemplateId`] does not exist in [`MaterialManager`], an error is returned
    pub fn generate_shader_text(&self, material_id: MaterialId) -> Result<String> {
        let material = self.materials.get(material_id.0 as usize).ok_or(anyhow!(
            "MaterialManager::generate_shader_text() Could not find material {material_id}"
        ))?;
        let shader_template = self
            .shader_templates
            .get(material.shader_template_id().0 as usize)
            .ok_or(anyhow!(
                "MaterialManager::generate_shader_text() Could not find shader template {}",
                material.shader_template_id()
            ))?;
        material.generate_shader_text(
            &shader_template.shader_insertion_points,
            shader_template.shader_text.as_str(),
        )
    }

    pub fn get_material_id_from_text_id(&self, text_id: TextId) -> Option<&MaterialId> {
        self.text_id_to_material_id_map.get(&text_id)
    }
}

#[cfg(feature = "internal_features")]
impl MaterialManager {
    /// # Errors
    ///
    /// - This can trigger an error if shader template name has already been registered.
    /// - This can trigger an error defined in
    ///   [`ShaderInsertionPoint::try_from_str`].
    /// - This can trigger an error if a [`ShaderInsertionPoint`] has been
    ///   duplicated (ie, two %uniforms or two %textures).
    pub fn register_shader_template(&mut self, name: &str, shader_text: &str) -> Result<()> {
        let is_shader_template_name_reserved = self
            .shader_templates
            .iter()
            .any(|shader_template| shader_template.name() == name);

        if is_shader_template_name_reserved {
            bail!("MaterialManager::register_shader_template The name {name} is already in used.");
        }

        let shader_insertion_points = shader_text.split_whitespace().try_fold(vec![], |mut accumulator: Vec<ShaderInsertionPoint>, current_word| {
            if current_word.starts_with("%") {
                let shader_insertion_point = ShaderInsertionPoint::try_from_str(current_word)?;
                if accumulator.contains(&shader_insertion_point) {
                    return Err(anyhow!("MaterialManager::register_shader_template() Shader Template insertion point {current_word} already exists, insertion points must be unique"));
                }
                accumulator.push(shader_insertion_point);
            }
            Ok(accumulator)
        })?;
        let id = ShaderTemplateId(self.shader_templates.len() as u32);
        let shader_template = ShaderTemplate {
            id,
            name: name.to_string(),
            shader_text: shader_text.to_string(),
            shader_insertion_points: shader_insertion_points.clone(),
        };
        self.shader_templates.push(shader_template);
        Ok(())
    }

    /// # Errors
    ///
    /// - This can generate an error if the [`ShaderTemplateId`] is invalid, ie no
    ///   [`ShaderTemplate`] has been registered with that id.
    /// - This can generate an error if the `name` has already been used.
    /// - This can generate an error defined in [`Material::new`].
    pub fn register_material(
        &mut self,
        shader_template_id: ShaderTemplateId,
        name: &str,
        shader_snippets: &[(ShaderInsertionPoint, ShaderSnippet)],
    ) -> Result<MaterialId> {
        if self
            .shader_templates
            .get(shader_template_id.0 as usize)
            .is_none()
        {
            bail!(
                "MaterialManager::register_material() Shader Template {shader_template_id} not found"
            );
        };

        if self.material_names_to_id.contains_key(name) {
            bail!("MaterialManager::register_material() material name {name} already registered");
        }

        let material = Material::new(
            shader_template_id,
            (self.materials.len() as u32).into(),
            name,
            shader_snippets,
        )?;
        let material_id = MaterialId(self.materials.len() as u32);
        self.materials.push(material);
        Ok(material_id)
    }

    /// Update material will update name and/or the Material's shader snippets if the values passed in are not [`Option::None`]
    ///
    /// # Errors
    ///
    /// - If the [`MaterialId`] does not exist in [`MaterialManager`], an error is returned
    /// - Will surface errors from [`Material::update_material`]
    pub fn update_material<S>(
        &mut self,
        material_id: MaterialId,
        name: Option<&str>,
        shader_snippets_map: Option<S>,
    ) -> Result<()>
    where
        S: AsRef<[(ShaderInsertionPoint, ShaderSnippet)]>,
    {
        if name.is_none() && shader_snippets_map.is_none() {
            log::warn!(
                "MaterialManager::update_material() Attempted to update material {material_id} with all none values"
            );
            return Ok(());
        }
        if let Some(name) = name {
            if let Some(existing_material_id) = self.material_names_to_id.get(name) {
                bail!(
                    "MaterialManager::update_material() Attempted to rename material {material_id} to {name}, but material {existing_material_id} already uses this name."
                );
            }
        }
        let Some(material) = self.materials.get_mut(material_id.0 as usize) else {
            bail!("MaterialManager::update_material() No material found with id {material_id}")
        };
        material.update_material(name, shader_snippets_map)?;
        Ok(())
    }

    pub fn update_material_from_string(
        &mut self,
        material_id: MaterialId,
        name: Option<&str>,
        material_toml_str: Option<&str>,
    ) -> Result<()> {
        if name.is_none() && material_toml_str.is_none() {
            log::warn!(
                "MaterialManager::update_material_from_string() Attempted to update material {material_id} with all none values"
            );
            return Ok(());
        }

        let shader_snippets_map = material_toml_str
            .map(|material_toml_str| {
                let toml_material: toml::TomlMaterial = ::toml::from_str(material_toml_str)?;
                let shader_snippets = toml_material.generate_shader_snippets()?;
                Ok::<_, anyhow::Error>(shader_snippets)
            })
            .transpose()?;
        self.update_material(material_id, name, shader_snippets_map.as_ref())
    }

    pub fn set_material_id_from_text_id(&mut self, text_id: TextId, material_id: MaterialId) {
        self.text_id_to_material_id_map.insert(text_id, material_id);
    }
}

pub const DEFAULT_SHADER_ID: ShaderTemplateId = ShaderTemplateId(0);
pub const DEFAULT_SHADER_TEXT: &str = include_str!("../../../shaders/shader_templates/default.fsh");
pub const DEFAULT_POST_PROCESSING_SHADER_ID: ShaderTemplateId = ShaderTemplateId(1);
pub const DEFAULT_POST_PROCESSING_SHADER_TEXT: &str =
    include_str!("../../../shaders/shader_templates/post_processing_default.fsh");

impl Default for MaterialManager {
    fn default() -> Self {
        #[cfg_attr(not(feature = "internal_features"), allow(unused_mut))]
        let mut default_manager = Self {
            shader_templates: vec![],
            materials: vec![],
            material_names_to_id: HashMap::new(),
            pending_texts: HashMap::new(),
            reloading_materials: HashMap::new(),
            text_id_to_material_id_map: HashMap::new(),
        };

        #[cfg(feature = "internal_features")]
        {
            default_manager
                .register_shader_template("default_shader_template", DEFAULT_SHADER_TEXT)
                .expect("Default shader template should parse correctly");
            default_manager
                .register_shader_template(
                    "default_post_processing_shader",
                    DEFAULT_POST_PROCESSING_SHADER_TEXT,
                )
                .expect("Default post processing shader template should parse correctly");
            let default_sprite_toml = include_str!(
                "../../../../game_asset/shaders/toml_shaders/standard/default_sprite.toml"
            );
            default_manager
                .register_material_from_string(
                    DEFAULT_SHADER_ID,
                    "default_sprite",
                    default_sprite_toml,
                )
                .unwrap();
            let pass_thru_toml = include_str!(
                "../../../../game_asset/shaders/toml_shaders/post_process/pass_thru.toml"
            );
            default_manager
                .register_material_from_string(
                    DEFAULT_POST_PROCESSING_SHADER_ID,
                    "pass_thru",
                    pass_thru_toml,
                )
                .unwrap();
            let missing_toml =
                include_str!("../../../../game_asset/shaders/toml_shaders/standard/missing.toml");
            default_manager
                .register_material_from_string(DEFAULT_SHADER_ID, "missing", missing_toml)
                .unwrap();
        }
        default_manager
    }
}

#[cfg(test)]
mod test {
    use glam::Vec4;
    use pretty_assertions::assert_eq;
    use void_public::{
        graphics::TextureId,
        material::{DefaultMaterials, FilterMode, MaterialParameters},
    };

    use super::MaterialManager;
    use crate::resource_managers::{
        material_manager::{
            DEFAULT_POST_PROCESSING_SHADER_ID, DEFAULT_POST_PROCESSING_SHADER_TEXT,
            DEFAULT_SHADER_ID, DEFAULT_SHADER_TEXT,
            fixed_size_vec::FixedSizeVec,
            material_parameters_extension::MaterialParametersExt,
            textures::TextureMaterialSpec,
            uniforms::{MaterialUniforms, UniformValue, UniformVar},
        },
        texture_asset_manager::TextureAssetManager,
    };

    #[test]
    fn shader_template_defaults_correctly_set() {
        let material_manager = MaterialManager::default();
        assert_eq!(
            &material_manager
                .shader_templates
                .get(DEFAULT_SHADER_ID.0 as usize)
                .unwrap()
                .shader_text,
            DEFAULT_SHADER_TEXT
        );
        assert_eq!(
            &material_manager
                .shader_templates
                .get(DEFAULT_POST_PROCESSING_SHADER_ID.0 as usize)
                .unwrap()
                .shader_text,
            DEFAULT_POST_PROCESSING_SHADER_TEXT
        );
        assert_eq!(
            material_manager
                .get_material(DefaultMaterials::PassThru.material_id())
                .unwrap()
                .material_id(),
            DefaultMaterials::PassThru.material_id()
        );
    }

    #[test]
    fn get_material_instances_outputs_correct_f32s() {
        let mut material_manager = MaterialManager::default();
        let wiggle_toml_string = include_str!("../../../shaders/toml_shaders/standard/wiggle.toml");
        let wiggle_material_id = material_manager
            .register_material_from_string(DEFAULT_SHADER_ID, "wiggle", wiggle_toml_string)
            .unwrap();

        let color_source_array = UniformVar::new(
            Some(FixedSizeVec::<Vec4>::new(&[
                Vec4::new(1.0, 0.8, 0.4, 1.0),
                Vec4::new(1.0, 1.0, 0.8, 1.0),
                Vec4::new(0.5, 0.5, 0.5, 1.0),
            ])),
            FixedSizeVec::<Vec4>::new(&[Vec4::new(0.0, 0.0, 0.0, 0.0); 3]),
        );
        let color_destination_array = UniformVar::new(
            Some(FixedSizeVec::<Vec4>::new(&[
                Vec4::new(0.0, 0.2, 0.6, 1.0),
                Vec4::new(0.0, 0.0, 0.2, 1.0),
                Vec4::new(0.5, 0.5, 0.5, 1.0),
            ])),
            FixedSizeVec::<Vec4>::new(&[Vec4::new(0.0, 0.0, 0.0, 0.0); 3]),
        );
        let wiggle_material = material_manager.get_material(wiggle_material_id).unwrap();
        let mut material_uniforms = wiggle_material
            .generate_default_material_uniforms()
            .unwrap()
            .clone();
        let valid_material_uniforms = material_uniforms
            .update("wiggle_time", 3.3.into())
            .unwrap()
            .update("color_param_1", Vec4::new(1.0, 0.8, 0.6, 1.0).into())
            .unwrap()
            .update("sun_dir", Vec4::new(0.5, 0.5, 0.5, 0.5).into())
            .unwrap()
            .update("color_src", UniformValue::Array(color_source_array))
            .unwrap()
            .update("color_dst", UniformValue::Array(color_destination_array))
            .unwrap();
        let invalid_material_uniforms = MaterialUniforms::new_from_iter::<_, &UniformValue, _>(
            wiggle_material_id,
            [
                ("sun_dir", &3.3.into()),
                ("wiggle_time", &Vec4::new(1.0, 1.0, 1.0, 1.0).into()),
                ("color_srcs", &4.4.into()), // deliberately misspelled
                                             // color_dst deliberately missing
            ],
        );

        let mut material_parameters = MaterialParameters::new(wiggle_material_id);
        wiggle_material
            .validate_material_uniforms(valid_material_uniforms)
            .unwrap();
        material_parameters
            .update_from_material_uniforms(valid_material_uniforms)
            .unwrap();
        let mut f32_buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        f32_buffer.reverse();

        #[rustfmt::skip]
        assert_eq!(f32_buffer, vec![
            // valid_material_uniform
            1.0, 0.8, 0.6, 1.0, // color_param_1
            0.5, 0.5, 0.5, 0.5, // sun_dir
            0.0, 0.2, 0.6, 1.0, // color_dst
            0.0, 0.0, 0.2, 1.0,
            0.5, 0.5, 0.5, 1.0,
            1.0, 0.8, 0.4, 1.0, // color_src
            1.0, 1.0, 0.8, 1.0,
            0.5, 0.5, 0.5, 1.0,
            3.3, // wiggle_time
        ]);

        let mut material_parameters = MaterialParameters::new(wiggle_material_id);
        let corrected_material_uniforms = wiggle_material
            .validate_material_uniforms(&invalid_material_uniforms)
            .unwrap_err();
        material_parameters
            .update_from_material_uniforms(&corrected_material_uniforms)
            .unwrap();
        let mut f32_buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        f32_buffer.reverse();

        #[rustfmt::skip]
        assert_eq!(f32_buffer, vec![
            // invalid_material_uniforms
            1.0, 1.0, 0.5, 1.0, // color_param_1 (defaulting to definition defaults)
            0.0, 0.0, 0.0, 0.0, // sun_dir (defaulting to 0 from wrong type)
            0.0, 0.0, 0.0, 0.0, // color_dst (defaulting to 0 from incorrect spelling)
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            1.0, 0.8, 0.6, 1.0, // color_src (defaulting to defintions defaults)
            0.5, 0.7, 0.9, 1.0,
            0.1, 0.2, 0.3, 1.0,
            1.2, // wiggle_time (defaulting to 0 from wrong type)
        ]);
    }

    #[test]
    fn expected_shader_text_created() {
        let mut shader_manager = MaterialManager::default();
        let wiggle_toml_string = include_str!("../../../shaders/toml_shaders/standard/wiggle.toml");
        let wiggle_material_id = shader_manager
            .register_material_from_string(DEFAULT_SHADER_ID, "wiggle", wiggle_toml_string)
            .unwrap();
        // replace CRNL -> NL on both strings to handle unix/windows line endings
        assert_eq!(shader_manager.generate_shader_text(wiggle_material_id).unwrap().as_str().replace("\r\n", "\n"), "struct GlobalUniforms {
    view_proj_matrix: mat4x4f,
}
@group(0) @binding(0) var<uniform> global_uniforms: GlobalUniforms;

struct SceneInstance {
  local_to_world: mat4x4f,
  color: vec4f,
  uv_scale_offset: vec4f,
  color_param_1 : vec4f,
  sun_dir : vec4f,
  color_dst : array<vec4f, 3>,
  color_src : array<vec4f, 3>,
  wiggle_time : f32,
  f32_0_padding: f32,
  f32_1_padding: f32,
  f32_2_padding: f32,
  padding: array<vec4f, 49>
};

// Constant data for draw objects. ex: sprites, text, etc
@group(1) @binding(0) var<storage, read> scene_instances: array<SceneInstance>;

// Maps draw instances to their index into scene_instances
@group(1) @binding(1) var<storage, read> scene_indices: array<u32>;

struct VertexInput {
    @location(0) color: vec4f,
    @location(1) position: vec3f,
    @location(2) tex_coords: vec2f,
    @builtin(instance_index) instance_idx: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) tex_coords: vec2f,
    @location(1) color: vec4f,
    @location(2) instance_idx: u32,
}

fn get_world_offset(uv0: vec2f, instance_index: u32) -> vec2f {
    let scene_instance = scene_instances[instance_index];
    return vec2f(0., 0.);
/*if (uv0.y > 0) {
    return vec2f(32. * sin(scene_instance.wiggle_time), 0.0);
} else {
    return vec2f(32.0 * -sin(scene_instance.wiggle_time), 0.0);
} */
}

@vertex
fn vs_main(
    vertex: VertexInput,
) -> VertexOutput {
    let scene_idx = scene_indices[vertex.instance_idx];
    let scene_instance = scene_instances[scene_idx];

    var out: VertexOutput;
    out.instance_idx = scene_idx;

    out.tex_coords = vertex.tex_coords * scene_instance.uv_scale_offset.xy + scene_instance.uv_scale_offset.zw;
    var vertex_world_offset = get_world_offset(out.tex_coords, out.instance_idx);

    var mvp = global_uniforms.view_proj_matrix * scene_instance.local_to_world;
    out.clip_position = mvp * vec4f(vertex.position + vec3f(vertex_world_offset, 0.0), 1);

    out.color = scene_instance.color * vertex.color;
    return out;
}

@group(2) @binding(0) var color_tex : texture_2d<f32>;
@group(2) @binding(1) var sampler_color_tex : sampler;

@group(2) @binding(2) var extra_tex_1 : texture_2d<f32>;
@group(2) @binding(3) var sampler_extra_tex_1 : sampler;

@group(2) @binding(4) var extra_tex_2 : texture_2d<f32>;
@group(2) @binding(5) var sampler_extra_tex_2 : sampler;

@group(2) @binding(6) var normal_tex : texture_2d<f32>;
@group(2) @binding(7) var sampler_normal_tex : sampler;



fn get_fragment_color(uv0: vec2f, instance_index: u32, vertex_color: vec4f) -> vec4f {
    let scene_instance = scene_instances[instance_index];
    var texcolor=textureSample(color_tex, s_diffuse, uv0.xy);
var normalcolor=textureSample(normal_tex, s_diffuse, uv0.xy);
let normal = normalcolor.xyz * 2.0 - 1.0;
let sun_dir = vec3f(1.f, 0.0f, 1.f);
let light_val = saturate(dot(normal, scene_instance.sun_dir.xyz));
return vec4f(light_val * texcolor.x, light_val* texcolor.y, light_val* texcolor.z, 1.);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    var fragment_color = get_fragment_color(in.tex_coords.xy, in.instance_idx, in.color);
    return fragment_color;
}
".replace("\r\n", "\n"));

        let invert_y_toml_string =
            include_str!("../../../shaders/toml_shaders/post_process/invert_y.toml");
        let invert_y_material_id = shader_manager
            .register_material_from_string(
                DEFAULT_POST_PROCESSING_SHADER_ID,
                "post_process",
                invert_y_toml_string,
            )
            .unwrap();
        // replace CRNL -> NL on both strings to handle unix/windows line endings
        assert_eq!(shader_manager.generate_shader_text(invert_y_material_id).unwrap().as_str().replace("\r\n", "\n"), "struct GlobalUniforms {
    inv_screen_dimensions: vec4f,
    camera_transform:  vec4f,
}
@group(0) @binding(0) var<uniform> global_uniforms: GlobalUniforms;

struct SceneInstance {
  padding: array<vec4f, 64>
};

// Constant data for post processes
@group(1) @binding(0) var<storage, read> scene_instances: array<SceneInstance>;

// Maps draw instances to their index into scene_instances
@group(1) @binding(1) var<storage, read> scene_indices: array<u32>;

struct VertexInput {
    @location(0) color: vec4f,
    @location(1) position: vec3f,
    @location(2) tex_coords: vec2f,
    @builtin(instance_index) instance_idx: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) tex_coords: vec2f,
    @location(1) color: vec4f,
    @location(2) instance_idx: u32,
}

fn get_world_offset(uv0: vec2f, instance_index: u32) -> vec2f {
    let scene_instance = scene_instances[instance_index];
    return vec2f(0., 0.);

}

@vertex
fn vs_main(
    vertex: VertexInput
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex.tex_coords;
    out.clip_position = vec4f(vertex.position, 1);
    out.color = vec4f(1.0, 1.0, 1.0, 1.0);
    out.instance_idx = vertex.instance_idx;
    return out;
}

@group(2) @binding(0) var scene_color_texture : texture_2d<f32>;
@group(2) @binding(1) var sampler_scene_color_texture : sampler;



fn get_fragment_color(uv0: vec2f, instance_idx: u32, vertex_color: vec4f) -> vec4f {
    let scene_instance = scene_instances[instance_idx];
    return textureSample(scene_color_texture, sampler_scene_color_texture, vec2f(uv0.x, 1.0 - uv0.y));

}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    var fragment_color = get_fragment_color(in.tex_coords.xy, in.instance_idx, in.color);

    return fragment_color;
}
".replace("\r\n", "\n"));
    }

    #[test]
    fn update_material_buffer() {
        let mut material_manager = MaterialManager::default();
        let wiggle_toml_string = include_str!("../../../shaders/toml_shaders/standard/wiggle.toml");
        let wiggle_material_id = material_manager
            .register_material_from_string(DEFAULT_SHADER_ID, "wiggle", wiggle_toml_string)
            .unwrap();
        let material = material_manager.get_material(wiggle_material_id).unwrap();
        let mut material_parameters = material.generate_default_material_parameters();
        let mut buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        buffer.reverse();
        #[rustfmt::skip]
        assert_eq!(vec![
            1.0, 1.0, 0.5, 1.0, // color_param_1
            0.0, 0.0, 0.0, 0.0, // sun_dir
            0.0, 0.0, 0.0, 0.0, // color_dst
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            1.0, 0.8, 0.6, 1.0, // color_src
            0.5, 0.7, 0.9, 1.0,
            0.1, 0.2, 0.3, 1.0,
            1.2,] // wiggle_time
        , buffer);

        material_parameters
            .update_uniform(
                &material_manager,
                &("color_param_1", &Vec4::new(0.25, 0.2, 0.15, 0.5).into()),
            )
            .unwrap();

        let mut buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        buffer.reverse();
        #[rustfmt::skip]
        assert_eq!(vec![
            0.25, 0.2, 0.15, 0.5, // color_param_1 UPDATED
            0.0, 0.0, 0.0, 0.0, // sun_dir
            0.0, 0.0, 0.0, 0.0, // color_dst
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            1.0, 0.8, 0.6, 1.0, // color_src
            0.5, 0.7, 0.9, 1.0,
            0.1, 0.2, 0.3, 1.0,
            1.2,] // wiggle_time
        , buffer);
        material_parameters
            .update_uniform(
                &material_manager,
                &(
                    "color_dst",
                    &FixedSizeVec::new(&[
                        Vec4::new(0.25, 0.26, 0.27, 0.7),
                        Vec4::new(0.3, 0.31, 0.32, 0.8),
                        Vec4::new(0.4, 0.41, 0.42, 0.9),
                    ])
                    .into(),
                ),
            )
            .unwrap();
        // material_uniforms
        //     .update(
        //         "color_dst",
        //         &FixedSizeVec::new(&[
        //             Vec4::new(0.25, 0.26, 0.27, 0.7),
        //             Vec4::new(0.3, 0.31, 0.32, 0.8),
        //             Vec4::new(0.4, 0.41, 0.42, 0.9),
        //         ])
        //         .into(),
        //     )
        //     .unwrap();
        // material_parameters
        //     .update_from_material_uniforms(&material_uniforms)
        //     .unwrap();
        let mut buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        buffer.reverse();
        #[rustfmt::skip]
        assert_eq!(
            vec![
                0.25, 0.2, 0.15, 0.5, // color_param_1
                0.0, 0.0, 0.0, 0.0, // sun_dir
                0.25, 0.26, 0.27, 0.7, // color_dst UPDATED
                0.3, 0.31, 0.32, 0.8, 
                0.4, 0.41, 0.42, 0.9, 
                1.0, 0.8, 0.6, 1.0, // color_src
                0.5, 0.7, 0.9, 1.0, 
                0.1, 0.2, 0.3, 1.0, 
                1.2, // wiggle_time
            ],
            buffer
        );
        // material_uniforms
        //     .update("wiggle_time", &2.4.into())
        //     .unwrap();
        // material_parameters
        //     .update_from_material_uniforms(&material_uniforms)
        //     .unwrap();
        material_parameters
            .update_uniform(&material_manager, &("wiggle_time", &2.4.into()))
            .unwrap();
        let mut buffer = material_parameters
            .data
            .into_iter()
            .rev()
            .skip_while(|x| *x == 0.)
            .collect::<Vec<_>>();
        buffer.reverse();
        #[rustfmt::skip]
        assert_eq!(
            vec![
                0.25, 0.2, 0.15, 0.5, // color_param_1
                0.0, 0.0, 0.0, 0.0, // sun_dir
                0.25, 0.26, 0.27, 0.7, // color_dst
                0.3, 0.31, 0.32, 0.8,
                0.4, 0.41, 0.42, 0.9,
                1.0, 0.8, 0.6, 1.0, // color_src
                0.5, 0.7, 0.9, 1.0,
                0.1, 0.2, 0.3, 1.0,
                2.4, // wiggle_time UPDATED
            ],
            buffer
        );
    }

    #[test]
    fn update_texture_buffer() {
        let mut material_manager = MaterialManager::default();
        let wiggle_toml_string = include_str!("../../../shaders/toml_shaders/standard/wiggle.toml");
        let wiggle_material_id = material_manager
            .register_material_from_string(DEFAULT_SHADER_ID, "wiggle", wiggle_toml_string)
            .unwrap();
        let material = material_manager.get_material(wiggle_material_id).unwrap();
        let mut material_parameters = material.generate_default_material_parameters();
        let name0_texture_specification =
            TextureMaterialSpec::new("color_tex", &FilterMode::Nearest);
        let name1_texture_specification =
            TextureMaterialSpec::new("extra_tex_1", &FilterMode::Nearest);
        let name2_texture_specification =
            TextureMaterialSpec::new("extra_tex_2", &FilterMode::Nearest);
        let name3_texture_specification =
            TextureMaterialSpec::new("normal_tex", &FilterMode::Nearest);
        let missing_texture_id = TextureAssetManager::missing_texture_id();
        assert_eq!(
            vec![
                (&name0_texture_specification, &missing_texture_id),
                (&name1_texture_specification, &missing_texture_id),
                (&name2_texture_specification, &missing_texture_id),
                (&name3_texture_specification, &missing_texture_id)
            ],
            material_parameters
                .as_material_textures(&material_manager)
                .unwrap()
                .sort_into_vec()
        );
        let texture_ids = material_parameters
            .as_material_textures(&material_manager)
            .unwrap()
            .output_texture_ids();
        assert_eq!(
            vec![
                missing_texture_id,
                missing_texture_id,
                missing_texture_id,
                missing_texture_id
            ],
            texture_ids
        );
        material_parameters
            .update_texture(&material_manager, &("extra_tex_1", &TextureId(2)))
            .unwrap();
        assert_eq!(
            vec![
                missing_texture_id,
                TextureId(2),
                missing_texture_id,
                missing_texture_id
            ],
            material_parameters
                .as_material_textures(&material_manager)
                .unwrap()
                .output_texture_ids()
        );
        material_parameters
            .update_texture(&material_manager, &("color_tex", &TextureId(3)))
            .unwrap();
        assert_eq!(
            vec![
                TextureId(3),
                TextureId(2),
                missing_texture_id,
                missing_texture_id,
            ],
            material_parameters
                .as_material_textures(&material_manager)
                .unwrap()
                .output_texture_ids()
        );
    }
}
