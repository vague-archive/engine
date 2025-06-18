use std::time::{Duration, SystemTime};

use game_asset::{
    ecs_module::TextAssetManager,
    resource_managers::text_asset_manager::events::HandleTextEventParametersBuilder,
};
use game_module_macro::{Component, system};
use serde_big_array::BigArray;
use void_public::{
    AssetPath, Component, ComponentId, EcsType, Engine, EventReader, EventWriter,
    event::graphics::{
        NewText, TextFailed, TextFailedBuilder, TextLoaded, TextLoadedBuilder,
        TextMessageFormatType,
    },
    graphics::TextRender,
    text::TextId,
};

use crate::platform_ecs::{
    HandleTextEventsFn, PlatformTextFailed, PlatformTextFormatType, PlatformTextRead,
};

const PATH_LENGTH: usize = 256;

#[derive(Debug, Component, serde::Deserialize, serde::Serialize)]
struct FileLastUpdated {
    text_id: TextId,
    #[serde(with = "BigArray")]
    path: [u8; PATH_LENGTH],
    last_modified: f64,
    path_valid: bool,
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
impl FileLastUpdated {
    pub fn new(text_id: TextId, path: &AssetPath, last_modified: &SystemTime) -> Self {
        let path_as_str = path.to_string_lossy();
        let path_valid = path_as_str.len() <= PATH_LENGTH;
        let path = TextRender::str_to_u8array(&path_as_str);
        Self {
            text_id,
            path,
            last_modified: last_modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            path_valid,
        }
    }

    pub fn text_id(&self) -> TextId {
        self.text_id
    }

    pub fn path_as_str(&self) -> Option<&str> {
        if !self.path_valid {
            return None;
        }

        TextRender::u8array_to_str(&self.path).ok()
    }

    pub fn last_modified(&self) -> Duration {
        Duration::from_secs_f64(self.last_modified)
    }

    pub fn update_last_modified(&mut self, system_time: &SystemTime) {
        self.last_modified = system_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
    }
}

#[system]
#[allow(clippy::needless_pass_by_value)] // Need to pass this by value for codegen.
fn send_text_events_to_platform(new_texture_event_reader: EventReader<NewText<'_>>) {
    for event in &new_texture_event_reader {
        Engine::call_with_builder::<HandleTextEventsFn>(|builder| {
            let asset_path = builder.create_string(event.asset_path().unwrap_or("@@invalid_path"));
            let mut handle_text_params_builder = HandleTextEventParametersBuilder::new(builder);
            handle_text_params_builder.add_id(event.id());
            handle_text_params_builder.add_asset_path(asset_path);
            handle_text_params_builder.add_set_up_watcher(event.set_up_watcher());
            handle_text_params_builder.finish()
        });
    }
}

fn write_text_failed(
    text_id: u32,
    text_path: &str,
    reason: &str,
    event_writer: &mut EventWriter<TextFailed<'_>>,
) {
    event_writer.write_builder(|builder| {
        let text_path = builder.create_string(text_path);
        let reason = builder.create_string(reason);
        let mut texture_failed_builder = TextFailedBuilder::new(builder);
        texture_failed_builder.add_id(text_id);
        texture_failed_builder.add_asset_path(text_path);
        texture_failed_builder.add_reason(reason);
        texture_failed_builder.finish()
    });
}

const INVALID_PATH: &str = "@@invalid_path@@";

#[system]
#[allow(clippy::needless_pass_by_value)] // Need to pass this by value for codegen.
fn send_platform_events_to_text_asset_manager(
    text_loaded_reader: EventReader<PlatformTextRead<'_>>,
    text_failed_reader: EventReader<PlatformTextFailed<'_>>,
    text_loaded_writer: EventWriter<TextLoaded<'_>>,
    mut text_failed_writer: EventWriter<TextFailed<'_>>,
) {
    for event in &text_loaded_reader {
        let text_path = event.asset_path().unwrap_or(INVALID_PATH);
        let format = match event.format() {
            PlatformTextFormatType::Toml => TextMessageFormatType::Toml,
            PlatformTextFormatType::Csv => TextMessageFormatType::Csv,
            PlatformTextFormatType::Json => TextMessageFormatType::Json,
            PlatformTextFormatType::Text => TextMessageFormatType::Text,
            unaccounted_for_type => {
                let reason = format!(
                    "Platform type {unaccounted_for_type:?} currently unhandled, cannot process text {} at path {text_path}",
                    event.id()
                );
                write_text_failed(event.id(), text_path, &reason, &mut text_failed_writer);
                continue;
            }
        };
        let Some(raw_text) = event.raw_text() else {
            let reason = format!(
                "Could not read data to load into GPU for {} with path {text_path}",
                event.id()
            );
            write_text_failed(event.id(), text_path, &reason, &mut text_failed_writer);
            continue;
        };
        let version = TextAssetManager::generate_hash(raw_text.as_bytes());

        let watcher_set_up = if event.watcher_set_up() {
            #[cfg(not(debug_assertions))]
            {
                false
            }
            #[cfg(debug_assertions)]
            {
                use std::{fs::metadata, num::NonZero};

                use void_public::bundle;

                if let Some(full_disk_path) = event.full_disk_path() {
                    match metadata(full_disk_path) {
                        Ok(metadata) => match metadata.modified() {
                            Ok(modified_system_time) => {
                                Engine::spawn(bundle!(&FileLastUpdated::new(
                                    TextId(unsafe { NonZero::new_unchecked(event.id()) }),
                                    &text_path.into(),
                                    &modified_system_time
                                )));
                                true
                            }
                            Err(error) => {
                                log::warn!(
                                    "Could not find modified time for setting up watcher for text with id {}: {error:?}",
                                    event.id()
                                );
                                false
                            }
                        },
                        Err(error) => {
                            log::warn!(
                                "Could not find metadata to set up watcher for text with id {}: {error:?}",
                                event.id()
                            );
                            false
                        }
                    }
                } else {
                    log::error!(
                        "full_disk_path was not sent in event for text {} at {text_path}",
                        event.id()
                    );
                    false
                }
            }
        } else {
            false
        };
        text_loaded_writer.write_builder(|builder| {
            let asset_path = builder.create_string(text_path);
            let raw_text = builder.create_string(raw_text);
            let version = builder.create_vector(&*version);
            let mut text_loaded_builder = TextLoadedBuilder::new(builder);
            text_loaded_builder.add_id(event.id());
            text_loaded_builder.add_asset_path(asset_path);
            text_loaded_builder.add_format(format);
            text_loaded_builder.add_version(version);
            text_loaded_builder.add_raw_text(raw_text);
            text_loaded_builder.add_watcher_set_up(watcher_set_up);
            text_loaded_builder.finish()
        });
    }

    for event in &text_failed_reader {
        let text_path = event.asset_path().unwrap_or(INVALID_PATH);
        let reason = event
            .error_reason()
            .unwrap_or("Error reason lost in message");
        write_text_failed(event.id(), text_path, reason, &mut text_failed_writer);
    }
}

#[cfg(debug_assertions)]
const TICKS_TO_WAIT: u64 = 200;

// TODO: This currently breaks our rules for non deterministic modules.
// <https://github.com/vaguevoid/engine/pull/355>
#[cfg(debug_assertions)]
#[cfg_attr(debug_assertions, system)]
#[allow(clippy::needless_pass_by_value, dead_code)]
fn scan_files_for_changes(
    frame_constants: &void_public::FrameConstants,
    mut files_last_updated: void_public::Query<&mut FileLastUpdated>,
    text_loaded_writer: EventWriter<TextLoaded<'_>>,
) {
    use std::{fs::metadata, time::UNIX_EPOCH};

    use crate::{get_extension, get_path, get_raw_text};

    if frame_constants.tick_count % TICKS_TO_WAIT != 0 {
        return;
    }

    if files_last_updated.is_empty() {
        return;
    }

    files_last_updated.for_each(|file_to_update| {
        let Some(relative_path_as_str) = file_to_update.path_as_str() else {
            log::warn!(
                "Malformed asset_path for scanning text with id {}",
                file_to_update.text_id()
            );
            return;
        };
        let relative_path = relative_path_as_str.into();
        let text_id = file_to_update.text_id();
        let asset_path = match get_path(text_id, &relative_path) {
            Ok(asset_path) => asset_path,
            Err(err) => {
                log::error!("{err}");
                return;
            }
        };

        let Ok(metadata) = metadata(&asset_path) else {
            log::info!(
                "Could not access metadata for text at {asset_path:?} with id {}",
                file_to_update.text_id()
            );
            return;
        };

        let Ok(modified_at) = metadata.modified() else {
            log::info!(
                "Could not access modified_at time for text at {asset_path:?} with id {}",
                file_to_update.text_id()
            );
            return;
        };

        let Ok(duration_since) = modified_at.duration_since(UNIX_EPOCH) else {
            log::info!(
                "Could not determine duration_since epoch for text at {asset_path:?} with id {}",
                file_to_update.text_id()
            );
            return;
        };

        if duration_since.as_secs() > file_to_update.last_modified().as_secs() {
            let text_format = match get_extension(text_id, &relative_path, &asset_path) {
                Ok(text_format) => match text_format {
                    PlatformTextFormatType::Toml => TextMessageFormatType::Toml,
                    PlatformTextFormatType::Csv => TextMessageFormatType::Csv,
                    PlatformTextFormatType::Json => TextMessageFormatType::Json,
                    PlatformTextFormatType::Text => TextMessageFormatType::Text,
                    unexpected_type => {
                        log::error!("Unhandled PlatformtextFormatType {unexpected_type:?}");
                        return;
                    }
                },
                Err(error) => {
                    log::error!("{error}");
                    return;
                }
            };

            let raw_text = match get_raw_text(text_id, &relative_path, &asset_path) {
                Ok(raw_text) => raw_text,
                Err(error) => {
                    log::error!("{error}");
                    return;
                }
            };

            let version = TextAssetManager::generate_hash(raw_text.as_bytes());

            text_loaded_writer.write_builder(|builder| {
                let asset_path = builder.create_string(relative_path_as_str);
                let raw_text = builder.create_string(&raw_text);
                let version = builder.create_vector(&*version);
                let mut text_loaded_builder = TextLoadedBuilder::new(builder);
                text_loaded_builder.add_id((*text_id).into());
                text_loaded_builder.add_asset_path(asset_path);
                text_loaded_builder.add_format(text_format);
                text_loaded_builder.add_version(version);
                text_loaded_builder.add_raw_text(raw_text);
                text_loaded_builder.add_watcher_set_up(true);
                text_loaded_builder.finish()
            });
            file_to_update.update_last_modified(&modified_at);
        }
    });
}

// =========== Codegen Below ===========

#[allow(unused, clippy::all)]
pub mod ffi {
    use super::*;

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
}
