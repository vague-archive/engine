use std::{
    ffi::{CStr, CString, c_char, c_void},
    mem::MaybeUninit,
    path::PathBuf,
    ptr::null_mut,
    slice::from_raw_parts,
};

use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::texture_asset_manager::{
        EngineTexture, FailedTexture, LoadedTexture, PendingTexture, Texture, TextureAssetManager,
    },
};
use void_public::{
    EventWriter,
    event::graphics::NewTexture,
    graphics::{
        CreatePendingTexture, GetTextureByIdStatus, GetTextureByPathStatus,
        GetTextureTypeByIdStatus, GetTextureTypeByPathStatus, LoadTextureByPendingTextureStatus,
        LoadTextureStatus, TextureHash, TextureId, TextureType,
    },
};

#[repr(transparent)]
/// We must convert the engine side [`Texture`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiPendingTexture(void_public::graphics::PendingTexture);

impl FfiPendingTexture {
    /// # Safety
    ///
    /// `texture_path` must be able to be dereferenced.
    pub unsafe fn new<C: Into<CString>>(
        id: TextureId,
        texture_path: C,
        insert_in_atlas: bool,
    ) -> Self {
        Self(void_public::graphics::PendingTexture {
            id,
            texture_path: texture_path.into().into_raw(),
            insert_in_atlas,
        })
    }
}

impl Drop for FfiPendingTexture {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.texture_path.cast_mut()) };
    }
}

impl From<&FfiPendingTexture> for PendingTexture {
    fn from(value: &FfiPendingTexture) -> Self {
        let texture_path = PathBuf::from(
            unsafe { CStr::from_ptr(value.0.texture_path) }
                .to_string_lossy()
                .as_ref(),
        );
        Self::new(value.0.id, &texture_path.into(), value.0.insert_in_atlas)
    }
}

impl From<&PendingTexture> for FfiPendingTexture {
    fn from(value: &PendingTexture) -> Self {
        Self(void_public::graphics::PendingTexture {
            id: value.id(),
            texture_path: value.texture_path().as_c_string().into_raw(),
            insert_in_atlas: value.insert_in_atlas(),
        })
    }
}

#[repr(transparent)]
/// We must convert the engine side [`Texture`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiEngineTexture(void_public::graphics::EngineTexture);

impl Drop for FfiEngineTexture {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.texture_path.cast_mut()) };
    }
}

impl From<&EngineTexture> for FfiEngineTexture {
    fn from(value: &EngineTexture) -> Self {
        Self(void_public::graphics::EngineTexture {
            id: value.id(),
            width: value.width() as u32,
            height: value.height() as u32,
            texture_path: value.texture_path().as_c_string().into_raw(),
            in_atlas: value.in_atlas(),
        })
    }
}

#[repr(transparent)]
/// We must convert the engine side [`Texture`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiLoadedTexture(void_public::graphics::LoadedTexture);

impl Drop for FfiLoadedTexture {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.texture_path.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.format_type.cast_mut()) };
    }
}

impl From<&LoadedTexture> for FfiLoadedTexture {
    fn from(value: &LoadedTexture) -> Self {
        Self(void_public::graphics::LoadedTexture {
            id: value.id(),
            version: *value.version(),
            width: value.width() as u32,
            height: value.height() as u32,
            texture_path: value.texture_path().as_c_string().into_raw(),
            format_type: value
                .format_type()
                .try_into()
                .unwrap_or_else(|_| CString::new("Could not read format type").unwrap())
                .into_raw(),
            in_atlas: value.in_atlas(),
        })
    }
}

#[repr(transparent)]
/// We must convert the engine side [`Texture`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiFailedTexture(void_public::graphics::FailedTexture);

impl Drop for FfiFailedTexture {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.texture_path.cast_mut()) };
    }
}

impl From<&FailedTexture> for FfiFailedTexture {
    fn from(value: &FailedTexture) -> Self {
        Self(void_public::graphics::FailedTexture {
            id: value.id(),
            texture_path: value.texture_path().as_c_string().into_raw(),
            failure_reason: CString::new(value.failure_reason())
                .unwrap_or_else(|_| CString::new("Error reading failure reason").unwrap())
                .into_raw(),
        })
    }
}

pub extern "C" fn texture_asset_manager_white_texture_id() -> TextureId {
    TextureAssetManager::white_texture_id()
}

pub extern "C" fn texture_asset_manager_missing_texture_id() -> TextureId {
    TextureAssetManager::missing_texture_id()
}

/// # Safety
///
/// The caller must ensure that [`TextureAssetManager`] is a valid pointer.  This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// `texture_asset_manager` should exist during this call.
pub unsafe extern "C" fn texture_asset_manager_register_next_texture_id(
    texture_asset_manager: *mut TextureAssetManager,
) -> TextureId {
    unsafe {
        texture_asset_manager
            .as_mut()
            .unwrap()
            .register_next_texture_id()
    }
}

/// # Safety
///
/// The caller must ensure that data is a valid pointer and that len is correct
/// for the number of u8s in the array.
pub unsafe extern "C" fn texture_asset_manager_generate_hash(
    data: *const u8,
    len: usize,
) -> TextureHash {
    unsafe {
        let data_slice = from_raw_parts(data, len);
        TextureAssetManager::generate_hash(data_slice)
    }
}

/// # Safety
///
/// `output_pending_texture` must be able to be dereferenced
pub unsafe extern "C" fn texture_asset_manager_create_pending_texture(
    id: TextureId,
    asset_path: *const c_char,
    insert_in_atlas: bool,
    output_pending_texture: *mut MaybeUninit<FfiPendingTexture>,
) -> CreatePendingTexture {
    let Some(output_pending_texture) = (unsafe { output_pending_texture.as_mut() }) else {
        return CreatePendingTexture::OutputPendingTextureNull;
    };
    let pending_texture =
        unsafe { FfiPendingTexture::new(id, CStr::from_ptr(asset_path), insert_in_atlas) };

    output_pending_texture.write(pending_texture);
    CreatePendingTexture::Success
}

/// # Safety
///
/// The `ptr` must be of type [`FfiPendingTexture`].
pub unsafe extern "C" fn texture_asset_manager_free_pending_texture(ptr: *mut FfiPendingTexture) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiEngineTexture`].
pub unsafe extern "C" fn texture_asset_manager_free_engine_texture(ptr: *mut FfiEngineTexture) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiLoadedTexture`].
pub unsafe extern "C" fn texture_asset_manager_free_loaded_texture(ptr: *mut FfiLoadedTexture) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiFailedTexture`].
pub unsafe extern "C" fn texture_asset_manager_free_failed_texture(ptr: *mut FfiFailedTexture) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Errors
///
/// * 1 means that the `text_asset_manager` pointer is invalid.
/// * 2 means that the `texture_type` pointer is invalid.
/// * 3 means that we could not acquire [`TextureAssetManager`] from the pointer.
///
/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This pointer is
/// equivalent to Rust's immutable reference, so no "mutable" references to
/// [`TextureAssetManager`] should be alive during this call.
pub unsafe extern "C" fn texture_asset_manager_get_texture_type_by_id(
    texture_asset_manager: *const TextureAssetManager,
    texture_id: TextureId,
    texture_type: *mut MaybeUninit<TextureType>,
) -> GetTextureTypeByIdStatus {
    let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_ref() }) else {
        return GetTextureTypeByIdStatus::TextureAssetManagerNull;
    };
    let Some(texture_type) = (unsafe { texture_type.as_mut() }) else {
        return GetTextureTypeByIdStatus::TextureTypeNull;
    };

    if let Some(texture) = texture_asset_manager.get_texture_by_id(texture_id) {
        texture_type.write(texture.texture_type());
        GetTextureTypeByIdStatus::Success
    } else {
        GetTextureTypeByIdStatus::TextureIdNotFound
    }
}

macro_rules! texture_asset_manager_texture_type_by_id_functions {
    ($(($function_name:ident, $texture_type:ident, $ffi_texture_type:ident)),*) => {
        $(
            /// # Errors
            ///
            /// - 1 indicates `texture_asset_manager` pointer is invalid.
            /// - 2 indicates `output_texture` pointer is invalid.
            /// - 3 indicates that the input `texture_id` wasn't found.
            /// - 4 indicates the `TextureType` is incorrect, consider using
            ///   `texture_asset_manager_get_texture_type_by_id` to get the
            ///   correct one.
            ///
            /// # Safety
            ///
            /// `texture_asset_manager` must point to a valid
            /// [`TextureAssetManager`]. This pointer is equivalent to Rust's
            /// immutable reference, so no "mutable" references to
            /// [`TextureAssetManager`] should be alive during this call.
            pub unsafe extern "C" fn $function_name(texture_asset_manager: *const TextureAssetManager, texture_id: TextureId, output_texture: *mut MaybeUninit<$ffi_texture_type>) -> GetTextureByIdStatus {
                let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_ref() }) else {
                    return GetTextureByIdStatus::TextureAssetManagerNull;
                };
                let Some(output_texture) = (unsafe { output_texture.as_mut() }) else {
                    return GetTextureByIdStatus::OutputTextureNull;
                };

                let texture = texture_asset_manager.get_texture_by_id(texture_id);
                let Some(texture) = texture else {
                    log::warn!("Texture {texture_id} not found");
                    return GetTextureByIdStatus::TextureIdNotFound;
                };
                match texture {
                    Texture::$texture_type(texture) => {
                        output_texture.write(texture.into());
                        GetTextureByIdStatus::Success
                    },
                    _ => {
                        log::error!("Texture {texture_id} not the correct TextureType");
                        GetTextureByIdStatus::TextureTypeIncorrect
                    }
                }
            }

        )*
    }
}

texture_asset_manager_texture_type_by_id_functions!(
    (
        texture_asset_manager_get_pending_texture_by_id,
        Pending,
        FfiPendingTexture
    ),
    (
        texture_asset_manager_get_engine_texture_by_id,
        Engine,
        FfiEngineTexture
    ),
    (
        texture_asset_manager_get_loaded_texture_by_id,
        Loaded,
        FfiLoadedTexture
    ),
    (
        texture_asset_manager_get_failed_texture_by_id,
        Failed,
        FfiFailedTexture
    )
);

/// # Errors
///
/// * 1 `texture_path` is invalid.
/// * 2 indicates we could not find `texture_asset_manager`.
/// * 3 indicates that `texture_type` is an invalid pointer.
/// * 4 indicates we could not find [`Texture`] at `texture_path`.
///
/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This pointer is
/// equivalent to Rust's immutable reference, so no "mutable" references to
/// [`TextureAssetManager`] should be alive during this call.
pub unsafe extern "C" fn texture_asset_manager_get_texture_type_by_path(
    texture_asset_manager: *const TextureAssetManager,
    texture_path: *const c_char,
    texture_type: *mut MaybeUninit<TextureType>,
) -> GetTextureTypeByPathStatus {
    if texture_path.is_null() {
        return GetTextureTypeByPathStatus::TexturePathNull;
    }
    let texture_path = PathBuf::from(
        unsafe { CStr::from_ptr(texture_path) }
            .to_string_lossy()
            .as_ref(),
    );
    let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_ref() }) else {
        return GetTextureTypeByPathStatus::TextureAssetManagerNull;
    };
    let Some(texture_type) = (unsafe { texture_type.as_mut() }) else {
        return GetTextureTypeByPathStatus::TextureTypeNull;
    };

    let asset_path = texture_path.into();
    if let Some(texture) = texture_asset_manager.get_texture_by_path(&asset_path) {
        texture_type.write(texture.texture_type());
        GetTextureTypeByPathStatus::Success
    } else {
        GetTextureTypeByPathStatus::TexturePathNotFound
    }
}

macro_rules! texture_asset_manager_texture_type_by_path_functions {
    ($(($function_name:ident, $texture_type:ident, $ffi_texture_type:ident)),*) => {
        $(
            /// # Errors
            ///
            /// - 1 indicates that the `texture_path` pointer is null.
            /// - 2 indicates that the `texture_asset_manager` pointer is invalid.
            /// - 3 indicates that the `output_texture` pointer is invalid.
            /// - 4 indicates that the input `texture_id` wasn't found.
            /// - 5 indicates the `TextureType` is incorrect, consider using
            ///   `texture_asset_manager_get_texture_type_by_id` to get the
            ///   correct one.
            ///
            /// # Safety
            ///
            /// `texture_asset_manager` must point to a valid
            /// [`TextureAssetManager`]. This pointer is equivalent to Rust's
            /// immutable reference, so no "mutable" references to
            /// [`TextureAssetManager`] should be alive during this call.
            pub unsafe extern "C" fn $function_name(texture_asset_manager: *const TextureAssetManager, texture_path: *const c_char, output_texture: *mut MaybeUninit<$ffi_texture_type>) -> GetTextureByPathStatus {
                if texture_path.is_null() {
                    log::warn!("texture_path {texture_path:?} is null");
                    return GetTextureByPathStatus::TexturePathNull;
                }
                let texture_path = PathBuf::from(unsafe { CStr::from_ptr(texture_path) }.to_string_lossy().as_ref());
                let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_ref() }) else {
                    return GetTextureByPathStatus::TextureAssetManagerNull;
                };
                let Some(output_texture) = (unsafe { output_texture.as_mut() }) else {
                    return GetTextureByPathStatus::OutputTextureNull;
                };

                let asset_path = texture_path.into();
                let texture = texture_asset_manager.get_texture_by_path(&asset_path);
                let Some(texture) = texture else {
                    log::warn!("Texture with path {asset_path:?} not found");
                    return GetTextureByPathStatus::TextureIdNotFound;
                };
                match texture {
                    Texture::$texture_type(texture) => {
                        output_texture.write(texture.into());
                        return GetTextureByPathStatus::Success;
                    },
                    _ => {
                        log::error!("Texture at path {asset_path:?} not the correct TextureType");
                        return GetTextureByPathStatus::TextureTypeIncorrect;
                    },
                }
            }

        )*
    }
}

texture_asset_manager_texture_type_by_path_functions!(
    (
        texture_asset_manager_get_pending_texture_by_path,
        Pending,
        FfiPendingTexture
    ),
    (
        texture_asset_manager_get_engine_texture_by_path,
        Engine,
        FfiEngineTexture
    ),
    (
        texture_asset_manager_get_loaded_texture_by_path,
        Loaded,
        FfiLoadedTexture
    ),
    (
        texture_asset_manager_get_failed_texture_by_path,
        Failed,
        FfiFailedTexture
    )
);

/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextureAssetManager`] should be alive during this call.
/// `ids` must point to a valid array of [`TextureId`] and the `id_len` must be
/// correct.
pub unsafe extern "C" fn texture_asset_manager_are_ids_loaded(
    texture_asset_manager: *const TextureAssetManager,
    ids: *const TextureId,
    id_len: usize,
) -> bool {
    unsafe {
        let ids = from_raw_parts(ids, id_len);
        texture_asset_manager
            .as_ref()
            .unwrap()
            .are_all_ids_loaded(ids)
    }
}

/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextureAssetManager`] should be alive during this call.
pub unsafe extern "C" fn texture_asset_manager_is_id_loaded(
    texture_asset_manager: *const TextureAssetManager,
    id: TextureId,
) -> bool {
    unsafe {
        texture_asset_manager
            .as_ref()
            .unwrap()
            .are_all_ids_loaded(&[id])
    }
}

/// A return value of `0` indicates that the process of loading the texture has
/// started correctly. A return value of `1` or higher indicates an error, and
/// the error should be printed to stderr unless the log level is ignoring
/// errors. Note, this does not mean the texture has loaded correctly, as that
/// error could occur later in the process.
///
/// # Errors
///
/// * 1 indicates that `texture_asset_manager` has an invalid pointer.
/// * 2 indicates that the `output_pending_texture` has an invalid pointer.
/// * 3 indicates there was an error loading the texture.
///
/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// [`TextureAssetManager`] should be alive during this call.
/// `new_texture_event_writer` should point to a valid
/// [`EventWriter<NewTexture>`]'s handle. This pointer is equivalent to Rust's
/// immutable reference, so no mutable references to [`EventWriter<NewTexture>`]
/// should be alive during this call. `texture_path` should be a valid pointer
/// to a C string.
pub unsafe extern "C" fn texture_asset_manager_load_texture(
    texture_asset_manager: *mut TextureAssetManager,
    new_texture_event_writer_handle: *const c_void,
    texture_path: *const c_char,
    insert_in_atlas: bool,
    output_pending_texture: *mut MaybeUninit<FfiPendingTexture>,
) -> LoadTextureStatus {
    let new_texture_event_writer =
        unsafe { EventWriter::<NewTexture<'_>>::new(new_texture_event_writer_handle) };
    let texture_path = PathBuf::from(
        unsafe { CStr::from_ptr(texture_path) }
            .to_string_lossy()
            .as_ref(),
    );
    let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_mut() }) else {
        return LoadTextureStatus::TextureAssetManagerNull;
    };
    let Some(output_pending_texture) = (unsafe { output_pending_texture.as_mut() }) else {
        return LoadTextureStatus::OutputPendingTextureNull;
    };

    let asset_path = texture_path.into();

    match texture_asset_manager.load_texture(
        &asset_path,
        insert_in_atlas,
        &new_texture_event_writer,
    ) {
        Ok(pending_texture) => {
            let ffi_pending_texture = pending_texture.into();
            output_pending_texture.write(ffi_pending_texture);
            LoadTextureStatus::Success
        }
        Err(err) => {
            log::error!("Error loading texture from C API: {err}");
            LoadTextureStatus::LoadTextureError
        }
    }
}

/// A return value of `0` indicates that the process of loading the texture has
/// started correctly. A return value of `1`or higher indicates an error, and
/// the error should be printed to stderr unless the log level is ignoring
/// errors. Note, this does not mean the texture has loaded correctly, as that
/// error could occur later in the process.
///
/// # Errors
///
/// * 1 indicates that `pending_texture` is an invalid pointer.
/// * 2 indicates that the `texture_asset_manager` is an invalid pointer.
/// * 3 indicates that the texture load failed.
///
/// # Safety
///
/// `texture_asset_manager` must point to a valid [`TextureAssetManager`]. This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// [`TextureAssetManager`] should be alive during this call.
/// `new_texture_event_writer` should point to a valid
/// [`EventWriter<NewTexture>`] handle. This pointer is equivalent to Rust's
/// immutable reference, so no mutable references to [`EventWriter<NewTexture>`]
/// should be alive during this call.
pub unsafe extern "C" fn texture_asset_manager_load_texture_by_pending_texture(
    texture_asset_manager: *mut TextureAssetManager,
    new_texture_event_writer_handle: *const c_void,
    pending_texture: *const FfiPendingTexture,
) -> LoadTextureByPendingTextureStatus {
    let new_texture_event_writer =
        unsafe { EventWriter::<NewTexture<'_>>::new(new_texture_event_writer_handle) };
    let Some(pending_texture) = (unsafe { pending_texture.as_ref() }) else {
        return LoadTextureByPendingTextureStatus::PendingTextureNull;
    };
    let Some(texture_asset_manager) = (unsafe { texture_asset_manager.as_mut() }) else {
        return LoadTextureByPendingTextureStatus::TextureAssetManagerNull;
    };

    let pending_texture = pending_texture.into();
    match texture_asset_manager
        .load_texture_by_pending_texture(&pending_texture, &new_texture_event_writer)
    {
        Ok(_) => LoadTextureByPendingTextureStatus::Success,
        Err(err) => {
            log::error!("Error loading texture by pending texture from C API: {err}");
            LoadTextureByPendingTextureStatus::LoadTextureError
        }
    }
}

/// # Safety
///
/// `gpu_interface` can be a null pointer, but then it will return a null
/// pointer. However, the lifetime of [`TextureAssetManager`] must be the same
/// or less than [`GpuInterface`].
pub unsafe extern "C" fn gpu_interface_get_texture_asset_manager_mut(
    gpu_interface: *mut GpuInterface,
) -> *mut TextureAssetManager {
    if gpu_interface.is_null() {
        return null_mut();
    }
    let gpu_interface = unsafe { gpu_interface.as_mut().unwrap() };
    &mut gpu_interface.texture_asset_manager as *mut TextureAssetManager
}
