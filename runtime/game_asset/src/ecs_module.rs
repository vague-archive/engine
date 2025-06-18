use std::collections::HashMap;

use game_module_macro::{ResourceWithoutSerialize, system};
use void_public::{
    AssetId, AssetPath, ComponentId, EcsType, EventReader, EventWriter, Resource,
    event::graphics::{
        MaterialIdFromTextId, NewPipeline, NewText, NewTexture, PipelineFailed, PipelineLoaded,
        TextFailed, TextLoaded, TextReloaded, TextureFailed, TextureLoaded,
    },
    material::{MaterialId, ShaderTemplateId},
    text::TextId,
};

use crate::{
    particles::ParticleEffectDescriptor,
    resource_managers::{
        material_manager::{ShaderTemplate, materials::Material},
        pipeline_asset_manager::PipelineAssetManager,
        text_asset_manager::Text,
        texture_asset_manager::TextureAssetManager,
    },
};

/// The [`GpuInterface`] is the determinsitic, platform independent means of a
/// developer interacting with the GPU for Fiasco. Each Platform implementation
/// is expected to have a `PlatformExtension` that responds to
/// [`GpuInterface`]'s messages.
///
/// This struct declaration is in a different file from its implementation, so
/// that it can be picked up by `build_tools` parsing and be automatically
/// included in the module.
#[derive(Debug, Default, ResourceWithoutSerialize)]
#[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
pub struct GpuInterface {
    /// This is an object to manage [`Material`]s inside Fiasco.
    pub material_manager: MaterialManager,
    /// This is an object to help with reading textures from the file system.
    pub texture_asset_manager: TextureAssetManager,
    pub pipeline_asset_manager: PipelineAssetManager,
    /// Legacy particle handling from the previous `AssetManager`, will be fully
    /// integrated later
    pub particle_effect_descriptors: HashMap<AssetId, ParticleEffectDescriptor>,
}

/// The [`TextAssetManager`] is the deterministic means of a developer interacting
/// with reading text for Fiasco. Each platform implementation is expected to
/// have a `PlatformExtension` that responds to [`TextAssetManager`]'s messages.
///
/// This struct declaration is in a different file from its implementation, so
/// that it can be picked up by `build_tools` parsing and be automatically
/// included in the module.
#[derive(Debug, ResourceWithoutSerialize)]
#[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
pub struct TextAssetManager {
    /// This is used in situation where potentially large numbers of text can be
    /// queued up, for example when loading a scene
    pub(crate) batched_text: HashMap<TextId, Text>,
    /// Similiar to `user_asset_path_to_id`, holds `asset_path` to `id`
    /// relationship for batched text
    pub(crate) batched_asset_path_to_id: HashMap<AssetPath, TextId>,
    /// New [`Text`]'s require a [`TextId`], this stores the next available one
    pub(crate) next_text_id: TextId,
    pub(crate) text: HashMap<TextId, Text>,
    /// Storage of all the user's [`AssetPath`]'s mapped to the [`TextId`].
    /// [`AssetPath`]s must be unique
    pub(crate) user_asset_path_to_id: HashMap<AssetPath, TextId>,
    /// Storage of all the internal engine [`AssetPath`]'s mapped to the
    /// [`TextId`]. [`AssetPath`]s must be unique. These maps are seperate to
    /// avoid name collisions
    pub(crate) engine_asset_path_to_id: HashMap<AssetPath, TextId>,
}

/// Used for messages from the [`TextAssetManager`], so that we can tell what
/// type of text asset the [`MaterialManager`] is waiting to load.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MaterialManagerTextTypes {
    ShaderFragment(String),
    /// This contains the [`ShaderTemplateId`] and the [`Material`] name.
    MaterialDefinition(ShaderTemplateId, String),
}

/// Manages [`Material`]s and their [`ShaderTemplates`]. These are the data
/// definitions of the shader templates, such as default.fsh, along with
/// [`Material`] definitions, which currently come from TOML files but could
/// come from any source with the [`Self::register_material`] method.
///
/// The two key outputs from the manager are [`Self::generate_shader_text`] and
/// [`Self::generate_material_instances_buffer`]. The first creates the final
/// text for a shader that can be used to generate a render pipeline. The second
/// is used along with [`MaterialUniforms`] to defined the uniforms that will be
/// attached to various shaders to define various instances of the Material and
/// their unique uniform combinations.
///
/// This struct declaration is in a different file from its implementation, so
/// that it can be picked up by `build_tools` parsing and be automatically
/// included in the module.
#[derive(Debug)]
#[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
pub struct MaterialManager {
    pub(crate) shader_templates: Vec<ShaderTemplate>,
    pub(crate) materials: Vec<Material>,
    pub(crate) material_names_to_id: HashMap<String, MaterialId>,
    pub(crate) pending_texts: HashMap<TextId, MaterialManagerTextTypes>,
    pub(crate) reloading_materials: HashMap<TextId, MaterialId>,
    pub(crate) text_id_to_material_id_map: HashMap<TextId, MaterialId>,
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn process_batched_textures(
    gpu_interface: &mut GpuInterface,
    new_texture_event_writer: EventWriter<NewTexture<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use void_public::event::graphics::NewTextureBuilder;

        use crate::resource_managers::texture_asset_manager::Texture;

        for (_, texture) in gpu_interface.texture_asset_manager.drain_batched_textures() {
            let Texture::Pending(pending_texture) = texture else {
                log::warn!(
                    "Found texture type {:?} found when processing batched textures, could not handle: {texture:?}",
                    texture.texture_type()
                );
                continue;
            };
            new_texture_event_writer.write_builder(|builder| {
                let asset_path = builder.create_string(
                    pending_texture
                        .texture_path()
                        .as_os_str()
                        .to_str()
                        .unwrap_or("@@initial_texture_asset_bad_path@@"),
                );
                let mut new_texture_builder = NewTextureBuilder::new(builder);
                new_texture_builder.add_id(*pending_texture.id());
                new_texture_builder.add_asset_path(asset_path);
                new_texture_builder.add_insert_in_atlas(pending_texture.insert_in_atlas());
                new_texture_builder.finish()
            });
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn process_batched_text(
    text_asset_manager: &mut TextAssetManager,
    new_text_event_writer: EventWriter<NewText<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use void_public::event::graphics::NewTextBuilder;

        for (_, text) in text_asset_manager.drain_batched_text() {
            let Text::Pending(pending_text) = text else {
                log::warn!(
                    "Found text type {:?} found when processing batched text, could not handle: {text:?}",
                    text.text_type()
                );
                continue;
            };
            new_text_event_writer.write_builder(|builder| {
                let asset_path = builder.create_string(
                    pending_text
                        .text_path()
                        .as_os_str()
                        .to_str()
                        .unwrap_or("@@initial_text_asset_bad_path@@"),
                );
                let mut new_texture_builder = NewTextBuilder::new(builder);
                new_texture_builder.add_id((*pending_text.id()).into());
                new_texture_builder.add_asset_path(asset_path);
                new_texture_builder.add_set_up_watcher(pending_text.set_up_watcher());
                new_texture_builder.finish()
            });
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn process_batched_pipelines(
    gpu_interface: &mut GpuInterface,
    new_text_event_writer: EventWriter<NewPipeline>,
) {
    #[cfg(feature = "internal_features")]
    {
        use crate::resource_managers::pipeline_asset_manager::Pipeline;

        for (_, pipeline) in gpu_interface
            .pipeline_asset_manager
            .drain_batched_pipelines()
        {
            let Pipeline::Pending(pending_pipeline) = pipeline else {
                log::warn!(
                    "Found pipeline type {:?} found when processing batched pipeline, could not handle: {pipeline:?}",
                    pipeline.pipeline_type()
                );
                continue;
            };
            new_text_event_writer.write(NewPipeline::new(
                pending_pipeline.id().get(),
                *pending_pipeline.material_id(),
            ));
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn handle_texture_events(
    gpu_interface: &mut GpuInterface,
    texture_loaded_events: EventReader<TextureLoaded<'_>>,
    texture_failed_events: EventReader<TextureFailed<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use void_public::{event::graphics::MessageFormatType, graphics::TextureHash};

        use crate::resource_managers::texture_asset_manager::{self, FailedTexture, LoadedTexture};

        for texture_loaded_event in &texture_loaded_events {
            let asset_path = texture_loaded_event
                .asset_path()
                .map(ToString::to_string)
                .unwrap_or(format!(
                    "@@path_malformed@@-id{}",
                    texture_loaded_event.id()
                ));
            let version = match texture_loaded_event.version() {
                Some(data) => data.bytes().iter().into(),
                None => TextureHash::default(),
            };
            let texture_format = match texture_loaded_event.format() {
                MessageFormatType::Png => texture_asset_manager::FormatType::Png,
                MessageFormatType::Jpeg => texture_asset_manager::FormatType::Jpeg,
                unimplemented_type => texture_asset_manager::FormatType::Unimplemented(format!(
                    "{unimplemented_type:?}"
                )),
            };
            let loaded_texture = LoadedTexture::new(
                texture_loaded_event.id().into(),
                &asset_path.into(),
                &version,
                texture_loaded_event.width() as usize,
                texture_loaded_event.height() as usize,
                texture_format,
                texture_loaded_event.in_atlas(),
            );

            gpu_interface
                .texture_asset_manager
                .replace_loaded_texture(&loaded_texture);
        }

        for texture_failed_event in &texture_failed_events {
            let texture_path = texture_failed_event
                .asset_path()
                .map(ToString::to_string)
                .unwrap_or(format!(
                    "@@path_malformed@@-id{}",
                    texture_failed_event.id()
                ));
            let failure_reason = texture_failed_event
                .asset_path()
                .unwrap_or("Failure reason lost in messaging");
            let failed_texture = FailedTexture::new(
                texture_failed_event.id().into(),
                &texture_path.into(),
                failure_reason,
            );
            log::error!("Texture load failed {failed_texture:?}");
            gpu_interface
                .texture_asset_manager
                .replace_failed_texture(&failed_texture);
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn handle_text_events(
    text_asset_manager: &mut TextAssetManager,
    text_loaded_events: EventReader<TextLoaded<'_>>,
    text_failed_events: EventReader<TextFailed<'_>>,
    text_reloaded_writer: EventWriter<TextReloaded<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use void_public::{
            event::graphics::{TextMessageFormatType, TextReloadedBuilder},
            text::TextHash,
        };

        use crate::resource_managers::text_asset_manager::{self, FailedText, LoadedText};

        for text_loaded_event in &text_loaded_events {
            let text_already_loaded = text_asset_manager
                .get_text_by_id(TextId(text_loaded_event.id().try_into().unwrap()))
                .map(|text| text.as_loaded_text())
                .is_some();
            let asset_path = text_loaded_event
                .asset_path()
                .map(ToString::to_string)
                .unwrap_or(format!("@@path_malformed@@-id{}", text_loaded_event.id()));
            let version = match text_loaded_event.version() {
                Some(data) => data.bytes().iter().into(),
                None => TextHash::default(),
            };
            let text_format = match text_loaded_event.format() {
                TextMessageFormatType::Toml => text_asset_manager::FormatType::Toml,
                TextMessageFormatType::Csv => text_asset_manager::FormatType::Csv,
                TextMessageFormatType::Json => text_asset_manager::FormatType::Json,
                TextMessageFormatType::Text => text_asset_manager::FormatType::Text,
                unimplemented_type => {
                    text_asset_manager::FormatType::Unimplemented(format!("{unimplemented_type:?}"))
                }
            };
            let Some(raw_text) = text_loaded_event.raw_text() else {
                let failed_text = FailedText::new(
                    TextId(text_loaded_event.id().try_into().unwrap()),
                    &asset_path.into(),
                    format!("Raw text lost for text {}", text_loaded_event.id()).as_str(),
                );
                text_asset_manager.replace_failed_text(&failed_text);
                continue;
            };
            let loaded_text = LoadedText::new(
                text_loaded_event.id().try_into().unwrap(),
                &asset_path.into(),
                &version,
                text_format,
                raw_text,
                text_loaded_event.watcher_set_up(),
            );

            text_asset_manager.replace_loaded_text(&loaded_text);

            if text_already_loaded {
                text_reloaded_writer.write_builder(|builder| {
                    let version = builder.create_vector(&*version);
                    let mut text_reloaded_builder = TextReloadedBuilder::new(builder);
                    text_reloaded_builder.add_id(text_loaded_event.id());
                    text_reloaded_builder.add_version(version);
                    text_reloaded_builder.finish()
                });
            }
        }

        for text_failed_event in &text_failed_events {
            let text_path = text_failed_event
                .asset_path()
                .map(ToString::to_string)
                .unwrap_or(format!("@@path_malformed@@-id{}", text_failed_event.id()));
            let failure_reason = text_failed_event
                .reason()
                .unwrap_or("Failure reason lost in messaging");
            let failed_text = FailedText::new(
                text_failed_event.id().try_into().unwrap(),
                &text_path.into(),
                failure_reason,
            );
            log::error!("Text load failed {failed_text:?}");
            text_asset_manager.replace_failed_text(&failed_text);
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn handle_pipeline_events(
    gpu_interface: &mut GpuInterface,
    pipeline_loaded_events: EventReader<PipelineLoaded>,
    pipeline_failed_events: EventReader<PipelineFailed<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use std::num::NonZero;

        use crate::resource_managers::pipeline_asset_manager::{FailedPipeline, LoadedPipeline};

        for pipeline_loaded_event in &pipeline_loaded_events {
            let loaded_pipeline = LoadedPipeline::new(
                unsafe { NonZero::new_unchecked(pipeline_loaded_event.id()) }.into(),
                pipeline_loaded_event.material_id().into(),
            );
            gpu_interface
                .pipeline_asset_manager
                .replace_loaded_pipeline(&loaded_pipeline);
        }

        for pipeline_failed_event in &pipeline_failed_events {
            let failed_pipeline = FailedPipeline::new(
                unsafe { NonZero::new_unchecked(pipeline_failed_event.id()) }.into(),
                pipeline_failed_event.material_id().into(),
                pipeline_failed_event.reason().unwrap_or("Reason not found"),
            );
            gpu_interface
                .pipeline_asset_manager
                .replace_failed_pipeline(&failed_pipeline);
        }
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)]
#[cfg_attr(not(feature = "internal_features"), allow(unused_variables))]
fn check_for_material_manager_text_events(
    gpu_interface: &mut GpuInterface,
    text_asset_manager: &TextAssetManager,
    new_pipeline_event_writer: EventWriter<NewPipeline>,
    material_id_from_text_id_event_writer: EventWriter<MaterialIdFromTextId>,
    text_loaded_events: EventReader<TextLoaded<'_>>,
    text_reloaded_events: EventReader<TextReloaded<'_>>,
) {
    #[cfg(feature = "internal_features")]
    {
        use std::num::NonZero;

        for text_loaded_event in &text_loaded_events {
            let text_id = TextId(NonZero::new(text_loaded_event.id()).unwrap());
            if let Some(pending_text) = gpu_interface
                .material_manager
                .pending_texts
                .remove(&text_id)
            {
                match pending_text {
                    MaterialManagerTextTypes::ShaderFragment(name) => {
                        if let Err(error) = gpu_interface.material_manager.register_shader_template(
                            &name,
                            text_loaded_event.raw_text().unwrap_or(""),
                        ) {
                            log::error!("Error registering shader template {name}: {error}");
                        }
                    }
                    MaterialManagerTextTypes::MaterialDefinition(shader_template_id, name) => {
                        let material_id = match gpu_interface
                            .material_manager
                            .register_material_from_string(
                                shader_template_id,
                                &name,
                                text_loaded_event.raw_text().unwrap_or(""),
                            ) {
                            Ok(material_id) => material_id,
                            Err(error) => {
                                log::error!("Error registering material {name}: {error}");
                                continue;
                            }
                        };
                        material_id_from_text_id_event_writer
                            .write(MaterialIdFromTextId::new(*material_id, text_id.get()));
                        gpu_interface
                            .material_manager
                            .set_material_id_from_text_id(text_id, material_id);
                        gpu_interface
                            .pipeline_asset_manager
                            .load_pipeline(material_id, &new_pipeline_event_writer);
                    }
                }
            }
        }

        for text_reloaded_event in &text_reloaded_events {
            let (text_id, reloadable_material_id) = {
                let Some((text_id, reloadable_material_id)) = gpu_interface
                    .material_manager
                    .reloading_materials
                    .get_key_value(&TextId(NonZero::new(text_reloaded_event.id()).unwrap()))
                else {
                    continue;
                };
                (*text_id, *reloadable_material_id)
            };
            let Some(Text::Loaded(loaded_text)) = text_asset_manager.get_text_by_id(text_id) else {
                log::error!(
                    "Reloaded text_id {text_id} either not found in TextAssetManager or not Loaded"
                );
                continue;
            };

            if let Err(err) = gpu_interface.material_manager.update_material_from_string(
                reloadable_material_id,
                None,
                Some(loaded_text.raw_text()),
            ) {
                log::error!(
                    "Could not reload material {reloadable_material_id} from reload event: {err}"
                );
                continue;
            }
        }
    }
}

#[allow(unused, clippy::all)]
pub mod ffi {
    use super::*;

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
}
