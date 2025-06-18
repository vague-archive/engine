use std::{env::current_exe, fs::read, io::Cursor};

use game_asset::resource_managers::texture_asset_manager::events::HandleTextureEventParameters;
use image::{GenericImageView, ImageFormat, ImageReader};
use platform_ecs::PlatformTextureReadBuilder;
use platform_library_macro::platform;
use platform_public::{Engine, ParameterData};

use crate::platform_ecs::{PlatformFormatType, PlatformTextureFailedBuilder};

fn send_error_case<'a>(
    parameters: &ParameterData<'a, HandleTextureEventParameters<'a>>,
    error_reason: &str,
) {
    unsafe {
        Engine::send_platform_event_builder("PlatformTextureFailed", |builder| {
            let asset_path =
                builder.create_string(parameters.asset_path().unwrap_or("@@invalid_path"));
            let error_reason = builder.create_string(error_reason);
            let mut texture_failed_builder = PlatformTextureFailedBuilder::new(builder);
            texture_failed_builder.add_id(parameters.id());
            texture_failed_builder.add_asset_path(asset_path);
            texture_failed_builder.add_error_reason(error_reason);
            texture_failed_builder.finish()
        });
    }
}

#[platform]
pub fn handle_texture_events<'a>(parameters: ParameterData<'a, HandleTextureEventParameters<'a>>) {
    let current_path = match current_exe() {
        Ok(path) => path,
        Err(error) => {
            send_error_case(
                &parameters,
                &format!("Could not read application's current directory: {error}"),
            );
            return;
        }
    };

    let mut asset_path = if let Some(path) = current_path.parent() {
        path.to_path_buf()
    } else {
        send_error_case(
            &parameters,
            "Could not read the parent of application's current directory",
        );
        return;
    };

    asset_path.push("assets");
    asset_path.push(parameters.asset_path().unwrap_or("@@nonsense_path@@"));

    if !asset_path.exists() {
        send_error_case(
            &parameters,
            &format!("Could not find asset directory or asset path {asset_path:?}"),
        );
        return;
    }

    let Some(extension) = asset_path.extension().and_then(|ext| ext.to_str()) else {
        send_error_case(
            &parameters,
            &format!("Asset at path {asset_path:?} does not have an extension"),
        );
        return;
    };

    let (extension, image_format) = match extension.to_lowercase().as_str() {
        "png" => (PlatformFormatType::Png, ImageFormat::Png),
        "jpeg" | "jpg" => (PlatformFormatType::Jpeg, ImageFormat::Jpeg),
        unknown_extension => {
            send_error_case(
                &parameters,
                &format!(
                    "Only allowed extensions are png and jpg, found {unknown_extension} at {asset_path:?}"
                ),
            );
            return;
        }
    };

    let image_bytes: Vec<u8> = match read(&asset_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            send_error_case(
                &parameters,
                &format!("Could not read asset at path {asset_path:?}: {error}"),
            );
            return;
        }
    };

    let mut image_reader = ImageReader::new(Cursor::new(image_bytes.as_slice()));

    image_reader.set_format(image_format);
    let image = match image_reader.decode() {
        Ok(image) => image,
        Err(error) => {
            send_error_case(
                &parameters,
                &format!("Could not parse jpeg image at {asset_path:?}: {error}"),
            );
            return;
        }
    };

    let (width, height) = image.dimensions();

    unsafe {
        Engine::send_platform_event_builder("PlatformTextureRead", |builder| {
            let asset_path = builder.create_string(parameters.asset_path().unwrap());
            let data = builder.create_vector(&image_bytes);
            let mut texture_loaded_builder = PlatformTextureReadBuilder::new(builder);
            texture_loaded_builder.add_id(parameters.id());
            texture_loaded_builder.add_asset_path(asset_path);
            texture_loaded_builder.add_width(width);
            texture_loaded_builder.add_height(height);
            texture_loaded_builder.add_insert_in_atlas(parameters.insert_in_atlas());
            texture_loaded_builder.add_format(extension);
            texture_loaded_builder.add_data(data);
            texture_loaded_builder.finish()
        });
    };
}

#[allow(
    clippy::derivable_impls,
    clippy::extra_unused_lifetimes,
    clippy::needless_lifetimes,
    clippy::match_like_matches_macro,
    clippy::missing_safety_doc,
    clippy::ptr_as_ptr,
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

    use super::*;

    #[derive(Debug, Default)]
    pub struct GpuEcsPlatformLibrary;

    pub struct GpuEcsPlatformFn(unsafe extern "C" fn(TaskId, *const c_void, usize));

    impl PlatformLibraryFn for GpuEcsPlatformFn {
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

    impl PlatformLibrary for GpuEcsPlatformLibrary {
        fn name(&self) -> std::borrow::Cow<'_, std::ffi::OsStr> {
            OsStr::new("gpu_web_platform").into()
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
            Box::new(GpuEcsPlatformFn(function_ptr(function_index)))
        }
    }
}
