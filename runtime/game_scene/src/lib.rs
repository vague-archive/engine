use std::{collections::BTreeMap, error::Error, mem::MaybeUninit};

use event::SpawnComponentData;
use json::JsonValue;
use void_public::{ComponentData, ComponentId, EcsType, graphics::TextureRender};

const VERSION: &str = "0.0.2";

pub struct SceneEntityInfo {
    /// A per-entity `String` that uniquely identifies an entity in the scene file. This allows
    /// for other scene entities to reference this entity in the scene file.
    pub scene_id: Option<String>,

    /// An optional `String` that identifies the `scene_id` of this entity's parent if present. A
    /// value of `None` means no parent
    pub parent_scene_id: Option<String>,

    /// The saved label for this entity. This field is optional.
    pub label: Option<String>,

    /// Component data for this entity
    pub components: Vec<ComponentData>,

    /// An optional `String` that indicates the `asset_path` for the `TextureRender` component.
    pub texture_asset_path: Option<String>,
}

/// Parses a scene from a json file. It returns a `Vec<SceneEntityInfo>` containing parsed entity data
pub fn parse_scene<F>(
    scene_file: &str,
    parse_ecs_component_func: F,
) -> Result<Vec<SceneEntityInfo>, Box<dyn Error + Send + Sync>>
where
    F: Fn(&str, &str) -> Result<ComponentData, Box<dyn Error + Send + Sync>>,
{
    let json = json::parse(scene_file)?;

    let Some(version) = json["version"].as_str() else {
        return Err("scene does not contain 'version' field".into());
    };

    if version != VERSION {
        return Err(format!("unexpected scene version: '{version}', expected: '{VERSION}'").into());
    }

    let JsonValue::Array(entities) = &json["entities"] else {
        return Err("scene does not contain 'entities' array".into());
    };

    let texture_render_str_id = TextureRender::string_id().to_str().unwrap();

    let all_scene_entities = entities
        .iter()
        .map(|entity| {
            let mut texture_asset_path = None;
            let components = entity["components"]
                .entries()
                .map(|(name, val)| {
                    if name == texture_render_str_id {
                        // capture the `asset_path` of the `TextureRender` so it can be looked up later
                        texture_asset_path = Some(
                            val["asset_path"]
                                .as_str()
                                .ok_or("texture_render components require `asset_path`")?
                                .into(),
                        );
                    }

                    parse_ecs_component_func(name, &val.dump())
                })
                .filter(|comp_type| {
                    if let Err(e) = comp_type {
                        log::error!("{e}");
                        false
                    } else {
                        true
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            // capture the scene_file specific entity identifiers
            let scene_id = entity["id"].as_str().map(|id| id.to_string());
            let parent_id = entity["parent_id"]
                .as_str()
                .map(|parent_str| parent_str.to_string());
            let label = entity["label"]
                .as_str()
                .map(|label_str| label_str.to_string());

            Ok(SceneEntityInfo {
                scene_id,
                components,
                label,
                texture_asset_path,
                parent_scene_id: parent_id,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error + Send + Sync>>>()?;

    Ok(all_scene_entities)
}

pub struct SceneEntityComponents<'a> {
    pub component_ids: Vec<ComponentId>,
    pub components: &'a BTreeMap<ComponentId, Box<[MaybeUninit<u8>]>>,
}

impl SpawnComponentData for SceneEntityComponents<'_> {
    fn sorted_component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }

    fn component_data(&self, component_id: ComponentId) -> Option<&[MaybeUninit<u8>]> {
        self.components
            .get(&component_id)
            .map(|boxed_data| boxed_data.as_ref())
    }
}
