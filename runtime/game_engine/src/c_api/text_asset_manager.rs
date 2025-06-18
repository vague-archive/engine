use std::{
    ffi::{CStr, CString, c_char, c_void},
    mem::MaybeUninit,
    path::PathBuf,
    slice::from_raw_parts,
};

use game_asset::{
    ecs_module::TextAssetManager,
    resource_managers::text_asset_manager::{
        EngineText, FailedText, LoadedText, PendingText, Text,
    },
};
use void_public::{
    EventWriter,
    event::graphics::NewText,
    text::{
        CreatePendingTextStatus, GetTextByIdStatus, GetTextByPathStatus, GetTextTypeByIdStatus,
        GetTextTypeByPathStatus, LoadTextByPendingTextStatus, LoadTextStatus, TextHash, TextId,
        TextType,
    },
};

/// We must convert the engine side [`Text`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiPendingText(void_public::text::PendingText);

impl FfiPendingText {
    /// # Safety
    ///
    /// `text_path` must be able to be dereferenced.
    pub unsafe fn new<C: Into<CString>>(id: TextId, text_path: C, set_up_watcher: bool) -> Self {
        Self(void_public::text::PendingText {
            id,
            text_path: text_path.into().into_raw(),
            set_up_watcher,
        })
    }
}

impl Drop for FfiPendingText {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.text_path.cast_mut()) };
    }
}

impl From<&FfiPendingText> for PendingText {
    fn from(value: &FfiPendingText) -> Self {
        let text_path = PathBuf::from(
            unsafe { CStr::from_ptr(value.0.text_path) }
                .to_string_lossy()
                .as_ref(),
        );
        Self::new(value.0.id, &text_path.into(), value.0.set_up_watcher)
    }
}

impl From<&PendingText> for FfiPendingText {
    fn from(value: &PendingText) -> Self {
        Self(void_public::text::PendingText {
            id: value.id(),
            text_path: value.text_path().as_c_string().into_raw(),
            set_up_watcher: value.set_up_watcher(),
        })
    }
}

/// We must convert the engine side [`Text`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiEngineText(void_public::text::EngineText);

impl Drop for FfiEngineText {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.text_path.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.format.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.raw_text.cast_mut()) };
    }
}

impl From<&EngineText> for FfiEngineText {
    fn from(value: &EngineText) -> Self {
        Self(void_public::text::EngineText {
            id: value.id(),
            text_path: value.text_path().as_c_string().into_raw(),
            format: value
                .format()
                .try_into()
                .unwrap_or_else(|_| CString::new("Error with format types").unwrap())
                .into_raw(),
            raw_text: CString::new(value.raw_text())
                .unwrap_or_else(|_| CString::new("Error with raw text").unwrap())
                .into_raw(),
        })
    }
}

/// We must convert the engine side [`Text`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiLoadedText(void_public::text::LoadedText);

impl Drop for FfiLoadedText {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.text_path.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.format.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.raw_text.cast_mut()) };
    }
}

impl From<&LoadedText> for FfiLoadedText {
    fn from(value: &LoadedText) -> Self {
        Self(void_public::text::LoadedText {
            id: value.id(),
            text_path: value.text_path().as_c_string().into_raw(),
            format: value
                .format_type()
                .try_into()
                .unwrap_or_else(|_| CString::new("Error with format type").unwrap())
                .into_raw(),
            version: *value.version(),
            raw_text: CString::new(value.raw_text())
                .unwrap_or_else(|_| CString::new("Error with raw text").unwrap())
                .into_raw(),
            watcher_set_up: value.watcher_set_up(),
        })
    }
}

/// We must convert the engine side [`Text`] types to their publically facing, C
/// structs in `void_public`. Ensuring we do not run into leaking memory or
/// deallocating memory with the incorrect allocator is tricky. `into_raw` on
/// Rust's [`CString`] API *MUST* be freed with the corresponding Rust API
/// `from_raw`. Attempting to free this memory in C with `free` is undefined
/// behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiFailedText(void_public::text::FailedText);

impl Drop for FfiFailedText {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.failure_reason.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.text_path.cast_mut()) };
    }
}

impl From<&FailedText> for FfiFailedText {
    fn from(value: &FailedText) -> Self {
        Self(void_public::text::FailedText {
            id: value.id(),
            text_path: value.text_path().as_c_string().into_raw(),
            failure_reason: CString::new(value.failure_reason())
                .unwrap_or_else(|_| CString::new("failure reason failed").unwrap())
                .into_raw(),
        })
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiPendingText`].
pub unsafe extern "C" fn text_asset_manager_free_pending_text(ptr: *mut FfiPendingText) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiEngineText`].
pub unsafe extern "C" fn text_asset_manager_free_engine_text(ptr: *mut FfiEngineText) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiLoadedText`].
pub unsafe extern "C" fn text_asset_manager_free_loaded_text(ptr: *mut FfiLoadedText) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiFailedText`].
pub unsafe extern "C" fn text_asset_manager_free_failed_text(ptr: *mut FfiFailedText) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The caller must ensure that [`TextAssetManager`] is a valid pointer.  This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// `text_asset_manager` should exist during this call.
pub unsafe extern "C" fn text_asset_manager_register_next_text_id(
    text_asset_manager: *mut TextAssetManager,
) -> TextId {
    unsafe { text_asset_manager.as_mut().unwrap().register_next_text_id() }
}

/// # Safety
///
/// The caller must ensure that data is a valid pointer and that len is correct
/// for the number of u8s in the array.
pub unsafe extern "C" fn text_asset_manager_generate_hash(data: *const u8, len: usize) -> TextHash {
    let data_slice = unsafe { from_raw_parts(data, len) };
    TextAssetManager::generate_hash(data_slice)
}

/// # Errors
///
/// * 1 indicates that the `output_pending_text` pointer is invalid.
///
/// # Safety
///
/// `output_pending_text` must be able to be dereferenced.
pub unsafe extern "C" fn text_asset_manager_create_pending_text(
    id: TextId,
    asset_path: *const c_char,
    set_up_watcher: bool,
    output_pending_text: *mut MaybeUninit<FfiPendingText>,
) -> CreatePendingTextStatus {
    let Some(output_pending_text) = (unsafe { output_pending_text.as_mut() }) else {
        return CreatePendingTextStatus::OutputPendingTextNull;
    };
    let text_path = unsafe { CStr::from_ptr(asset_path) };
    let pending_text = unsafe { FfiPendingText::new(id, text_path, set_up_watcher) };
    output_pending_text.write(pending_text);

    CreatePendingTextStatus::Success
}

/// # Errors
///
/// * 1 means that we could not acquire [`TextAssetManager`] from the pointer.
/// * 2 means that `text_type` is an invalid pointer.
/// * 3 means the given [`TextId`] was not found
///
/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextAssetManager`] should be alive during this call.
pub unsafe extern "C" fn text_asset_manager_get_text_type_by_id(
    text_asset_manager: *const TextAssetManager,
    text_id: TextId,
    text_type: *mut MaybeUninit<TextType>,
) -> GetTextTypeByIdStatus {
    let Some(text_asset_manager) = (unsafe { text_asset_manager.as_ref() }) else {
        return GetTextTypeByIdStatus::TextAssetManagerNull;
    };
    let Some(text_type) = (unsafe { text_type.as_mut() }) else {
        return GetTextTypeByIdStatus::TextTypeNull;
    };
    if let Some(text) = text_asset_manager.get_text_by_id(text_id) {
        text_type.write(text.text_type());
        GetTextTypeByIdStatus::Success
    } else {
        GetTextTypeByIdStatus::TextIdNotFound
    }
}

macro_rules! text_asset_manager_text_type_by_id_functions {
    ($(($function_name:ident, $text_type:ident, $ffi_text_type:ident)),*) => {
        $(
            /// # Errors
            ///
            /// - 1 indicates that the `text_asset_manager` pointer is invalid.
            /// - 2 indicates that the `output_text` pointer is invalid.
            /// - 3, indicates that the input `text_id` wasn't found.
            /// - 4, indicates the `TextType` is incorrect, consider using
            ///   `text_asset_manager_get_text_type_by_id` to get the correct
            ///   one.
            ///
            /// # Safety
            ///
            /// `text_asset_manager` must point to a valid [`TextAssetManager`].
            /// This pointer is equivalent to Rust's immutable reference, so no
            /// "mutable" references to [`TextAssetManager`] should be alive
            /// during this call.
            pub unsafe extern "C" fn $function_name(text_asset_manager: *const TextAssetManager, text_id: TextId, output_text: *mut MaybeUninit<$ffi_text_type>) -> GetTextByIdStatus {
                let Some(text_asset_manager) = (unsafe { text_asset_manager.as_ref() }) else {
                    return GetTextByIdStatus::TextAssetManagerNull;
                };
                let Some(output_text) = (unsafe { output_text.as_mut() }) else {
                    return GetTextByIdStatus::OutputTextNull;
                };

                let text = text_asset_manager.get_text_by_id(text_id);
                let Some(text) = text else {
                    log::warn!("Text {text_id} not found");
                    return GetTextByIdStatus::TextIdNotFound;
                };
                match text {
                    Text::$text_type(text) => {
                        output_text.write(text.into());
                        GetTextByIdStatus::Success
                    },
                    _ => {
                        log::error!("Text {text_id} not the correct TextType");
                        GetTextByIdStatus::TextTypeIncorrect
                    }
                }
            }

        )*
    }
}

text_asset_manager_text_type_by_id_functions!(
    (
        text_asset_manager_get_pending_text_by_id,
        Pending,
        FfiPendingText
    ),
    (
        text_asset_manager_get_engine_text_by_id,
        Engine,
        FfiEngineText
    ),
    (
        text_asset_manager_get_loaded_text_by_id,
        Loaded,
        FfiLoadedText
    ),
    (
        text_asset_manager_get_failed_text_by_id,
        Failed,
        FfiFailedText
    )
);

/// # Errors
///
/// * 1 `text_path` is an invalid pointer.
/// * 2 Could not find `text_asset_manager`.
/// * 3 `text_type` is an invalid pointer.
/// * 4 Could not find [`Text`] at `text_path`
///
/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextAssetManager`] should be alive during this call.
pub unsafe extern "C" fn text_asset_manager_get_text_type_by_path(
    text_asset_manager: *const TextAssetManager,
    text_path: *const c_char,
    text_type: *mut MaybeUninit<TextType>,
) -> GetTextTypeByPathStatus {
    if text_path.is_null() {
        return GetTextTypeByPathStatus::TextPathNull;
    }
    let text_path = PathBuf::from(
        unsafe { CStr::from_ptr(text_path) }
            .to_string_lossy()
            .as_ref(),
    );
    let Some(text_asset_manager) = (unsafe { text_asset_manager.as_ref() }) else {
        return GetTextTypeByPathStatus::TextAssetManagerNull;
    };
    let Some(text_type) = (unsafe { text_type.as_mut() }) else {
        return GetTextTypeByPathStatus::TextTypeNull;
    };

    let asset_path = text_path.into();

    if let Some(text) = text_asset_manager.get_text_by_path(&asset_path) {
        text_type.write(text.text_type());
        GetTextTypeByPathStatus::Success
    } else {
        GetTextTypeByPathStatus::TextPathNotFound
    }
}

macro_rules! text_asset_manager_text_type_by_path_functions {
    ($(($function_name:ident, $text_type:ident, $ffi_text_type:ident)),*) => {
        $(
            /// # Errors
            ///
            /// - 1, indicates that the `text_path` pointer is null.
            /// - 2, indicates that the `text_asset_manager` pointer is invalid.
            /// - 3, indicates that the `output_text` pointer is invalid.
            /// - 4, indicates that the input `text_id` wasn't found.
            /// - 5, indicates the `TextType` is incorrect, consider using
            ///   `text_asset_manager_get_text_type_by_id` to get the correct
            ///   one.
            ///
            /// # Safety
            ///
            /// `text_asset_manager` must point to a valid [`TextAssetManager`].
            /// This pointer is equivalent to Rust's immutable reference, so no
            /// "mutable" references to [`TextAssetManager`] should be alive
            /// during this call.
            pub unsafe extern "C" fn $function_name(text_asset_manager: *const TextAssetManager, text_path: *const c_char, output_text: *mut MaybeUninit<$ffi_text_type>) -> GetTextByPathStatus {
                if text_path.is_null() {
                    log::warn!("text_path {text_path:?} is null");
                    return GetTextByPathStatus::TextPathNull;
                }
                let text_path = PathBuf::from(unsafe { CStr::from_ptr(text_path) }.to_string_lossy().as_ref());
                let Some(text_asset_manager) = (unsafe { text_asset_manager.as_ref() }) else {
                    return GetTextByPathStatus::TextAssetManagerNull;
                };
                let Some(output_text) = (unsafe { output_text.as_mut() }) else {
                    return GetTextByPathStatus::OutputTextNull;
                };

                let asset_path = text_path.into();
                let text = text_asset_manager.get_text_by_path(&asset_path);
                let Some(text) = text else {
                    log::warn!("Text with path {asset_path:?} not found");
                    return GetTextByPathStatus::TextIdNotFound;
                };
                match text {
                    Text::$text_type(text) => {
                        output_text.write(text.into());
                        return GetTextByPathStatus::Success;
                    },
                    _ => {
                        log::error!("Text at path {asset_path:?} not the correct TextType");
                        return GetTextByPathStatus::TextTypeIncorrect;
                    },
                }
            }

        )*
    }
}

text_asset_manager_text_type_by_path_functions!(
    (
        text_asset_manager_get_pending_text_by_path,
        Pending,
        FfiPendingText
    ),
    (
        text_asset_manager_get_engine_text_by_path,
        Engine,
        FfiEngineText
    ),
    (
        text_asset_manager_get_loaded_text_by_path,
        Loaded,
        FfiLoadedText
    ),
    (
        text_asset_manager_get_failed_text_by_path,
        Failed,
        FfiFailedText
    )
);

/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextAssetManager`] should be alive during this call. `ids`
/// must point to a valid array of [`TextId`] and the `id_len` must be correct.
pub unsafe extern "C" fn text_asset_manager_are_ids_loaded(
    text_asset_manager: *const TextAssetManager,
    ids: *const TextId,
    id_len: usize,
) -> bool {
    unsafe {
        let ids = from_raw_parts(ids, id_len);
        text_asset_manager.as_ref().unwrap().are_all_ids_loaded(ids)
    }
}

/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`TextAssetManager`] should be alive during this call.
pub unsafe extern "C" fn text_asset_manager_is_id_loaded(
    text_asset_manager: *const TextAssetManager,
    id: TextId,
) -> bool {
    unsafe {
        text_asset_manager
            .as_ref()
            .unwrap()
            .are_all_ids_loaded(&[id])
    }
}

/// A return value of `0` indicates that the process of loading the text has
/// started correctly. A return value of `1` indicates an error, and the error
/// should be printed to stderr unless the log level is ignoring errors. Note,
/// this does not mean the text has loaded correctly, as that error could occur
/// later in the process.
///
/// # Errors
///
/// * 1 indicates that `text_asset_manager` is an invalid pointer.
/// * 2 indicates that the `output_pending_text` is an invalid pointer.
/// * 3 indicates there was an error loading the text in [`TextAssetManager`].
///
/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// [`TextAssetManager`] should be alive during this call.
/// `new_text_event_writer` should point to a valid [`EventWriter<NewText>`]
/// handle. This pointer is equivalent to Rust's immutable reference, so no
/// mutable references to [`EventWriter<NewText>`] should be alive during this
/// call. `text_path` should be a valid pointer to a C string.
pub unsafe extern "C" fn text_asset_manager_load_text(
    text_asset_manager: *mut TextAssetManager,
    new_text_event_writer_handle: *const c_void,
    text_path: *const c_char,
    set_up_watcher: bool,
    output_pending_text: *mut MaybeUninit<FfiPendingText>,
) -> LoadTextStatus {
    let new_text_event_writer =
        unsafe { EventWriter::<NewText<'_>>::new(new_text_event_writer_handle) };
    let text_path = PathBuf::from(
        unsafe { CStr::from_ptr(text_path) }
            .to_string_lossy()
            .as_ref(),
    );
    let Some(text_asset_manager) = (unsafe { text_asset_manager.as_mut() }) else {
        return LoadTextStatus::TextAssetManagerNull;
    };
    let Some(output_pending_text) = (unsafe { output_pending_text.as_mut() }) else {
        return LoadTextStatus::OutputPendingTextNull;
    };

    let asset_path = text_path.into();

    match text_asset_manager.load_text(&asset_path, set_up_watcher, &new_text_event_writer) {
        Ok(pending_text) => {
            let ffi_pending_text = pending_text.into();
            output_pending_text.write(ffi_pending_text);
            LoadTextStatus::Success
        }
        Err(err) => {
            log::error!("Error loading text from C API: {err}");
            LoadTextStatus::LoadTextError
        }
    }
}

/// A return value of `0` indicates that the process of loading the text has
/// started correctly. A return value of `1` indicates an error, and the error
/// should be printed to stderr unless the log level is ignoring errors. Note,
/// this does not mean the text has loaded correctly, as that error could occur
/// later in the process.
///
/// # Errors
///
/// * 1 indicates that the `pending_text` pointer is invalid.
/// * 2 indicates that the `text_asset_manager` pointer is invalid.
/// * 3 indicates that there was an error loading the text.
///
/// # Safety
///
/// `text_asset_manager` must point to a valid [`TextAssetManager`]. This
/// pointer is equivalent to Rust's mutable reference, so no other references to
/// [`TextAssetManager`] should be alive during this call.
/// `new_text_event_writer` should point to a valid [`EventWriter<NewText>`]
/// handle. This pointer is equivalent to Rust's immutable reference, so no
/// mutable references to [`EventWriter<NewText>`] should be alive during this
/// call.
pub unsafe extern "C" fn text_asset_manager_load_text_by_pending_text(
    text_asset_manager: *mut TextAssetManager,
    new_text_event_writer_handle: *const c_void,
    pending_text: *const FfiPendingText,
) -> LoadTextByPendingTextStatus {
    let new_text_event_writer =
        unsafe { EventWriter::<NewText<'_>>::new(new_text_event_writer_handle) };
    let Some(pending_text) = (unsafe { pending_text.as_ref() }) else {
        return LoadTextByPendingTextStatus::PendingTextNull;
    };
    let Some(text_asset_manager) = (unsafe { text_asset_manager.as_mut() }) else {
        return LoadTextByPendingTextStatus::TextAssetManagerNull;
    };

    let pending_text = pending_text.into();
    match text_asset_manager.load_text_by_pending_text(&pending_text, &new_text_event_writer) {
        Ok(_) => LoadTextByPendingTextStatus::Success,
        Err(err) => {
            log::error!("Error loading text by pending text from C API: {err}");
            LoadTextByPendingTextStatus::LoadTextError
        }
    }
}
