use std::{
    env::current_exe,
    error::Error,
    fmt::Display,
    fs::read_to_string,
    num::NonZero,
    path::{Path, PathBuf},
};

use game_asset::resource_managers::text_asset_manager::events::HandleTextEventParameters;
use platform_ecs::PlatformTextReadBuilder;
use platform_library_macro::platform;
use platform_public::{Engine, ParameterData};
use void_public::{AssetPath, text::TextId};

use crate::platform_ecs::{PlatformTextFailedBuilder, PlatformTextFormatType};

pub mod ecs_module;

fn send_error_case(error_for_message: &ErrorForMessage) {
    unsafe {
        Engine::send_platform_event_builder("PlatformTextFailed", |builder| {
            let asset_path = builder.create_string(error_for_message.asset_path_as_str());
            let error_reason = builder.create_string(error_for_message.error_reason());
            let mut text_failed_builder = PlatformTextFailedBuilder::new(builder);
            text_failed_builder.add_id((*error_for_message.text_id()).into());
            text_failed_builder.add_asset_path(asset_path);
            text_failed_builder.add_error_reason(error_reason);
            text_failed_builder.finish()
        });
    }
}

#[derive(Debug)]
pub(crate) struct ErrorForMessage {
    text_id: TextId,
    asset_path: String,
    error_reason: String,
}

impl ErrorForMessage {
    pub fn new(text_id: TextId, asset_path: &str, error_reason: &str) -> Self {
        Self {
            text_id,
            asset_path: asset_path.to_string(),
            error_reason: error_reason.to_string(),
        }
    }

    pub fn text_id(&self) -> TextId {
        self.text_id
    }

    pub fn asset_path_as_str(&self) -> &str {
        &self.asset_path
    }

    pub fn error_reason(&self) -> &str {
        &self.error_reason
    }
}

impl Display for ErrorForMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error with text {} at {} for reason: {}",
            self.text_id, self.asset_path, self.error_reason
        )
    }
}

impl Error for ErrorForMessage {}
unsafe impl Send for ErrorForMessage {}
unsafe impl Sync for ErrorForMessage {}

const ROOT_ASSET_PATH: &str = "assets";

pub(crate) fn get_path(
    text_id: TextId,
    relative_path: &AssetPath,
) -> Result<PathBuf, ErrorForMessage> {
    let current_path = match current_exe() {
        Ok(path) => path,
        Err(error) => {
            return Err(ErrorForMessage::new(
                text_id,
                &relative_path.to_string_lossy(),
                &format!("Could not read application's current directory: {error}"),
            ));
        }
    };

    let mut asset_path = if let Some(path) = current_path.parent() {
        path.to_path_buf()
    } else {
        return Err(ErrorForMessage::new(
            text_id,
            &relative_path.to_string_lossy(),
            "Could not read the parent of application's current directory",
        ));
    };

    asset_path.push(ROOT_ASSET_PATH);
    asset_path.push(&**relative_path);

    if !asset_path.exists() {
        return Err(ErrorForMessage::new(
            text_id,
            &relative_path.to_string_lossy(),
            &format!("Could not find asset directory or asset path {asset_path:?}"),
        ));
    }

    Ok(asset_path)
}

pub(crate) fn get_extension(
    text_id: TextId,
    relative_path: &AssetPath,
    asset_path: &Path,
) -> Result<PlatformTextFormatType, ErrorForMessage> {
    let Some(extension) = asset_path.extension().and_then(|ext| ext.to_str()) else {
        return Err(ErrorForMessage::new(
            text_id,
            &relative_path.to_string_lossy(),
            &format!("Asset at path {asset_path:?} does not have an extension"),
        ));
    };

    match extension.to_lowercase().as_str() {
        "toml" => Ok(PlatformTextFormatType::Toml),
        "json" => Ok(PlatformTextFormatType::Json),
        "csv" => Ok(PlatformTextFormatType::Csv),
        "txt" => Ok(PlatformTextFormatType::Text),
        unknown_extension => Err(ErrorForMessage::new(
            text_id,
            &relative_path.to_string_lossy(),
            &format!(
                "Only allowed extensions are toml, json, csv and text, found {unknown_extension} at {asset_path:?}"
            ),
        )),
    }
}

pub(crate) fn get_raw_text(
    text_id: TextId,
    relative_path: &AssetPath,
    asset_path: &Path,
) -> Result<String, ErrorForMessage> {
    read_to_string(asset_path).map_err(|error| {
        ErrorForMessage::new(
            text_id,
            &relative_path.to_string_lossy(),
            &format!("Could not read asset at path {asset_path:?}: {error}"),
        )
    })
}

#[platform]
pub fn handle_text_events<'a>(parameters: ParameterData<'a, HandleTextEventParameters<'a>>) {
    let text_id = TextId(unsafe { NonZero::new_unchecked(parameters.id()) });
    let relative_path = parameters
        .asset_path()
        .unwrap_or("@@nonsense_path@@")
        .into();
    let full_on_disk_path = match get_path(text_id, &relative_path) {
        Ok(full_on_disk_path) => full_on_disk_path,
        Err(error) => {
            send_error_case(&error);
            return;
        }
    };

    let text_format = match get_extension(text_id, &relative_path, &full_on_disk_path) {
        Ok(text_format) => text_format,
        Err(error) => {
            send_error_case(&error);
            return;
        }
    };

    let raw_text = match get_raw_text(text_id, &relative_path, &full_on_disk_path) {
        Ok(raw_text) => raw_text,
        Err(error) => {
            send_error_case(&error);
            return;
        }
    };

    unsafe {
        Engine::send_platform_event_builder("PlatformTextRead", |builder| {
            let asset_path = builder.create_string(parameters.asset_path().unwrap());
            let full_on_disk_path = builder.create_string(&full_on_disk_path.to_string_lossy());
            let raw_text = builder.create_string(&raw_text);
            let mut text_loaded_builder = PlatformTextReadBuilder::new(builder);
            text_loaded_builder.add_id(parameters.id());
            text_loaded_builder.add_asset_path(asset_path);
            text_loaded_builder.add_full_disk_path(full_on_disk_path);
            text_loaded_builder.add_watcher_set_up(parameters.set_up_watcher());
            text_loaded_builder.add_format(text_format);
            text_loaded_builder.add_raw_text(raw_text);
            text_loaded_builder.finish()
        });
    };
}

#[allow(
    clippy::derivable_impls,
    clippy::extra_unused_lifetimes,
    clippy::needless_lifetimes,
    clippy::match_like_matches_macro,
    clippy::missing_safety_doc,
    clippy::size_of_in_element_count,
    elided_lifetimes_in_paths,
    explicit_outlives_requirements,
    unused_extern_crates,
    unused_imports,
    unused_variables,
    unsafe_op_in_unsafe_fn
)]
pub mod platform_ecs {
    include!(concat!(env!("OUT_DIR"), "/ffi_platform.rs"));
    include!(concat!(env!("OUT_DIR"), "/platform_generated.rs"));

    use std::{
        ffi::{CStr, OsStr, c_void},
        mem::MaybeUninit,
    };

    use platform::{PlatformLibrary, PlatformLibraryFn};
    use platform_public::TaskId;

    pub use super::*;

    #[derive(Debug, Default)]
    pub struct TextNativePlatformLibrary;

    pub struct TextNativePlatformFn(unsafe extern "C" fn(TaskId, *const c_void, usize));

    impl PlatformLibraryFn for TextNativePlatformFn {
        fn call(&self, task_id: TaskId, parameter_data: &[MaybeUninit<u8>]) {
            unsafe {
                (self.0)(
                    task_id,
                    parameter_data.as_ptr().cast(),
                    parameter_data.len(),
                );
            };
        }
    }

    impl PlatformLibrary for TextNativePlatformLibrary {
        fn name(&self) -> std::borrow::Cow<'_, std::ffi::OsStr> {
            OsStr::new("text_native_platform").into()
        }

        fn void_target_version(&self) -> u32 {
            void_target_version()
        }

        fn init(&mut self) -> u32 {
            init()
        }

        fn function_count(&self) -> usize {
            function_count()
        }

        fn function_name(&self, function_index: usize) -> std::borrow::Cow<'_, std::ffi::CStr> {
            unsafe { CStr::from_ptr(function_name(function_index)).into() }
        }

        fn function_is_sync(&self, function_index: usize) -> bool {
            function_is_sync(function_index)
        }

        fn function(&self, function_index: usize) -> Box<dyn PlatformLibraryFn> {
            Box::new(TextNativePlatformFn(function_ptr(function_index)))
        }
    }
}
