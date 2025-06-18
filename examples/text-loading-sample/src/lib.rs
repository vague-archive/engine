use std::collections::HashMap;

use game_asset::{ecs_module::TextAssetManager, resource_managers::text_asset_manager::FormatType};
use game_module_macro::{Resource, system, system_once};
use serde::{Deserialize, Serialize};
use void_public::{
    Aspect, ComponentId, EcsType, Engine, EntityId, EventReader, EventWriter, Query, Resource,
    Transform, Vec2, bundle,
    colors::palette,
    event::graphics::{NewText, TextAlignment, TextReloaded},
    graphics::TextRender,
    linalg,
    text::TextId,
};

#[system_once]
// EventWriter/Reader must be passed by value for codegen.
#[allow(clippy::needless_pass_by_value)]
fn startup_system(
    aspect: &Aspect,
    text_asset_manager: &mut TextAssetManager,
    assets_being_loaded: &mut AssetsBeingLoaded,
    new_text_event_writer: EventWriter<NewText<'_>>,
) {
    let default_text = TextRender::str_to_u8array("Lorem Ipsum");

    let quarter_width = aspect.width / 4.;
    let quarter_height = aspect.height / 4.;

    let text_and_position_data = &[
        ("hello_world.txt", (quarter_width, quarter_height)),
        ("hello_world.toml", (-quarter_width, quarter_height)),
        ("hello_world.json", (quarter_width, -quarter_height)),
        ("hello_world.csv", (-quarter_width, -quarter_height)),
    ];
    let text_render = TextRender {
        text: default_text,
        visible: true,
        font_size: 64.,
        alignment: TextAlignment::Center.into(),
        ..Default::default()
    };
    let color = palette::PINK;

    for (text_path, position) in text_and_position_data {
        let pending_text = text_asset_manager
            .load_text(&text_path.into(), true, &new_text_event_writer)
            .unwrap();
        let transform = Transform {
            scale: linalg::Vec2::new(Vec2::splat(1.)),
            position: linalg::Vec3::from_xyz(position.0, position.1, 0.),
            ..Default::default()
        };
        let entity_id = Engine::spawn(bundle!(&text_render, &transform, &color));

        assets_being_loaded.set_ids(&[(entity_id, pending_text.id())]);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TextFromFile {
    text: String,
}

fn get_text_by_extension(raw_text: &str, extension: &FormatType) -> String {
    match extension {
        FormatType::Json => match serde_json::from_str::<TextFromFile>(raw_text) {
            Ok(json) => json.text,
            Err(error) => {
                log::warn!("Error deserializing json string: {error}");
                "BadJson".to_string()
            }
        },
        FormatType::Toml => match toml::from_str::<TextFromFile>(raw_text) {
            Ok(toml) => toml.text,
            Err(error) => {
                log::warn!("Error deserializing toml string: {error}");
                "BadToml".to_string()
            }
        },
        FormatType::Csv => {
            let mut reader = csv::ReaderBuilder::new().from_reader(raw_text.as_bytes());
            if let Some(result) = reader.records().next() {
                match result {
                    Ok(string_record) => string_record[0].to_string(),
                    Err(error) => {
                        log::warn!("Error reading csv: {error}");
                        "BadCsv".to_string()
                    }
                }
            } else {
                log::warn!("Error reading csv string, likely empty");
                "EmptyCsv".to_string()
            }
        }
        FormatType::Text => raw_text.to_string(),
        FormatType::Unimplemented(unknown_format) => format!("unknown format {unknown_format}"),
    }
}

#[system]
fn check_text_loaded_system(
    text_asset_manager: &TextAssetManager,
    assets_being_loaded: &mut AssetsBeingLoaded,
    mut text_to_transform: Query<(&EntityId, &mut TextRender)>,
) {
    if !assets_being_loaded.is_loaded()
        && text_asset_manager.are_all_ids_loaded(text_to_transform.iter().map(
            |text_to_transform_component| {
                let (entity_id, _) = text_to_transform_component.unpack();
                assets_being_loaded.text_ids().get(*entity_id).unwrap()
            },
        ))
    {
        text_to_transform.for_each(|(entity_id, text_render)| {
            let text_id = assets_being_loaded.text_ids().get(entity_id).unwrap();
            let loaded_texture = text_asset_manager
                .get_text_by_id(*text_id)
                .unwrap()
                .as_loaded_text()
                .unwrap();
            let text =
                get_text_by_extension(loaded_texture.raw_text(), loaded_texture.format_type());
            text_render.text = TextRender::str_to_u8array(&text);
        });

        assets_being_loaded.set_loaded();
    }
}

#[system]
// EventWriter/Reader must be passed by value for codegen.
#[allow(clippy::needless_pass_by_value)]
fn read_text_reloaded_events(
    text_asset_manager: &TextAssetManager,
    assets_being_loaded: &AssetsBeingLoaded,
    mut text_to_transform: Query<(&EntityId, &mut TextRender)>,
    text_reloaded_event_reader: EventReader<TextReloaded<'_>>,
) {
    for text_reloaded_event in &text_reloaded_event_reader {
        let Some(loaded_entity_id) = assets_being_loaded
            .reverse_text_ids_map()
            .get(&text_reloaded_event.id().try_into().unwrap())
        else {
            continue;
        };
        for mut query_component in text_to_transform.iter_mut() {
            let (entity_id, text_render) = query_component.unpack();
            if **entity_id == *loaded_entity_id {
                let loaded_text = text_asset_manager
                    .get_text_by_id(text_reloaded_event.id().try_into().unwrap())
                    .unwrap()
                    .as_loaded_text()
                    .unwrap();
                let text = get_text_by_extension(loaded_text.raw_text(), loaded_text.format_type());
                text_render.text = TextRender::str_to_u8array(&text);
                break;
            }
        }
    }
}

#[derive(Debug, Default, Resource)]
struct AssetsBeingLoaded {
    text_ids: HashMap<EntityId, TextId>,
    reverse_text_ids_map: HashMap<TextId, EntityId>,
    loaded: bool,
}

impl AssetsBeingLoaded {
    pub fn set_ids(&mut self, text_ids: &[(EntityId, TextId)]) {
        for (entity_id, text_id) in text_ids {
            self.text_ids.insert(*entity_id, *text_id);
            self.reverse_text_ids_map.insert(*text_id, *entity_id);
        }
    }

    pub fn text_ids(&self) -> &HashMap<EntityId, TextId> {
        &self.text_ids
    }

    pub fn reverse_text_ids_map(&self) -> &HashMap<TextId, EntityId> {
        &self.reverse_text_ids_map
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn set_loaded(&mut self) {
        self.loaded = true;
    }
}

// This includes auto-generated C FFI code (saves you from writing it manually).
include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
