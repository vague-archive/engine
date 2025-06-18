use std::{
    ffi::{CString, c_void},
    mem::MaybeUninit,
    ptr::null_mut,
    slice::from_raw_parts,
};

use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::pipeline_asset_manager::{
        EnginePipeline, FailedPipeline, LoadedPipeline, PendingPipeline, Pipeline,
        PipelineAssetManager,
    },
};
use void_public::{
    EventWriter,
    event::graphics::NewPipeline,
    material::MaterialId,
    pipeline::{
        GetPipelineByIdStatus, GetPipelineTypeByIdStatus, LoadPipelineByPendingPipelineStatus,
        LoadPipelineStatus, PipelineId, PipelineType,
    },
};

/// We must convert the engine side [`Pipeline`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiPendingPipeline(void_public::pipeline::PendingPipeline);

impl FfiPendingPipeline {
    pub fn new(id: PipelineId, material_id: MaterialId) -> Self {
        Self(void_public::pipeline::PendingPipeline { id, material_id })
    }
}

impl From<&FfiPendingPipeline> for PendingPipeline {
    fn from(value: &FfiPendingPipeline) -> Self {
        Self::new(value.0.id, value.0.material_id)
    }
}

impl From<&PendingPipeline> for FfiPendingPipeline {
    fn from(value: &PendingPipeline) -> Self {
        Self(void_public::pipeline::PendingPipeline {
            id: value.id(),
            material_id: value.material_id(),
        })
    }
}

/// We must convert the engine side [`Pipeline`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiEnginePipeline(void_public::pipeline::EnginePipeline);

impl From<&EnginePipeline> for FfiEnginePipeline {
    fn from(value: &EnginePipeline) -> Self {
        Self(void_public::pipeline::EnginePipeline {
            id: value.id(),
            material_id: value.material_id(),
        })
    }
}

/// We must convert the engine side [`Pipeline`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiLoadedPipeline(void_public::pipeline::LoadedPipeline);

impl From<&LoadedPipeline> for FfiLoadedPipeline {
    fn from(value: &LoadedPipeline) -> Self {
        Self(void_public::pipeline::LoadedPipeline {
            id: value.id(),
            material_id: value.material_id(),
        })
    }
}

/// We must convert the engine side [`Pipeline`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiFailedPipeline(void_public::pipeline::FailedPipeline);

impl Drop for FfiFailedPipeline {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.failure_reason.cast_mut()) };
    }
}

impl From<&FailedPipeline> for FfiFailedPipeline {
    fn from(value: &FailedPipeline) -> Self {
        Self(void_public::pipeline::FailedPipeline {
            id: value.id(),
            material_id: value.material_id(),
            failure_reason: CString::new(value.failure_reason())
                .unwrap_or_else(|_| CString::new("failure reason failed").unwrap())
                .into_raw(),
        })
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiPendingPipeline`].
pub unsafe extern "C" fn pipeline_asset_manager_free_pending_pipeline(
    ptr: *mut FfiPendingPipeline,
) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiEnginePipeline`].
pub unsafe extern "C" fn pipeline_asset_manager_free_engine_pipeline(ptr: *mut FfiEnginePipeline) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiLoadedPipeline`].
pub unsafe extern "C" fn pipeline_asset_manager_free_loaded_pipeline(ptr: *mut FfiLoadedPipeline) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The `ptr` must be of type [`FfiFailedPipeline`].
pub unsafe extern "C" fn pipeline_asset_manager_free_failed_pipeline(ptr: *mut FfiFailedPipeline) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        ptr.read();
    }
}

/// # Safety
///
/// The caller must ensure that [`PipelineAssetManager`] is a valid pointer.
/// This pointer is equivalent to Rust's mutable reference, so no other
/// references to `pipeline_asset_manager` should exist during this call.
pub unsafe extern "C" fn pipeline_asset_manager_register_next_pipeline_id(
    pipeline_asset_manager: *mut PipelineAssetManager,
) -> PipelineId {
    unsafe {
        pipeline_asset_manager
            .as_mut()
            .unwrap()
            .register_next_pipeline_id()
    }
}

/// # Safety
///
/// `output_pending_pipeline` must be able to be dereferenced.
pub unsafe extern "C" fn pipeline_asset_manager_create_pending_pipeline(
    id: PipelineId,
    material_id: MaterialId,
    output_pending_pipeline: *mut MaybeUninit<FfiPendingPipeline>,
) -> u32 {
    unsafe {
        output_pending_pipeline
            .as_mut()
            .unwrap()
            .write(FfiPendingPipeline::new(id, material_id));
    };
    0
}

/// # Errors
///
/// * 1 - Means that we could not acquire [`PipelineAssetManager`] from the
///   pointer.
/// * 2 - Means we could not acquire [`PipelineType`] from the pointer
/// * 3- Means the given [`PipelineId`] was not found
///
/// # Safety
///
/// `pipeline_asset_manager` must point to a valid [`PipelineAssetManager`].
/// This pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`PipelineAssetManager`] should be alive during this call.
pub unsafe extern "C" fn pipeline_asset_manager_get_pipeline_type_by_id(
    pipeline_asset_manager: *const PipelineAssetManager,
    pipeline_id: PipelineId,
    pipeline_type: *mut MaybeUninit<PipelineType>,
) -> GetPipelineTypeByIdStatus {
    let Some(pipeline_asset_manager) = (unsafe { pipeline_asset_manager.as_ref() }) else {
        return GetPipelineTypeByIdStatus::PipelineAssetManagerNull;
    };
    let Some(pipeline_type) = (unsafe { pipeline_type.as_mut() }) else {
        return GetPipelineTypeByIdStatus::PipelineTypeNull;
    };

    if let Some(pipeline) = pipeline_asset_manager.get_pipeline_by_id(pipeline_id) {
        pipeline_type.write(pipeline.pipeline_type());
        GetPipelineTypeByIdStatus::Success
    } else {
        GetPipelineTypeByIdStatus::PipelineIdNotFound
    }
}

macro_rules! pipeline_asset_manager_pipeline_type_by_id_functions {
    ($(($function_name:ident, $pipeline_type:ident, $ffi_pipeline_type:ident)),*) => {
        $(
            /// # Errors
            ///
            /// - 1, indicates that the `pipeline_asset_manager` pointer is invalid.
            /// - 2, indicates that the `output_pipeline` pointer is invalid.
            /// - 3, indicates that the input `pipeline_id` wasn't found.
            /// - 4, indicates the `PipelineType` is incorrect, consider using
            ///   `pipeline_asset_manager_get_pipeline_type_by_id` to get the
            ///   correct one.
            ///
            /// # Safety
            ///
            /// `pipeline_asset_manager` must point to a valid
            /// [`PipelineAssetManager`]. This pointer is equivalent to Rust's
            /// immutable reference, so no "mutable" references to
            /// [`PipelineAssetManager`] should be alive during this call.
            pub unsafe extern "C" fn $function_name(pipeline_asset_manager: *const PipelineAssetManager, pipeline_id: PipelineId, output_pipeline: *mut MaybeUninit<$ffi_pipeline_type>) -> GetPipelineByIdStatus {
                let Some(pipeline_asset_manager) = (unsafe { pipeline_asset_manager.as_ref() }) else {
                    return GetPipelineByIdStatus::PipelineAssetManagerNull;
                };
                let Some(output_pipeline) = (unsafe { output_pipeline.as_mut()}) else {
                    return GetPipelineByIdStatus::OutputPipelineNull;
                };

                let pipeline = pipeline_asset_manager.get_pipeline_by_id(pipeline_id);
                let Some(pipeline) = pipeline else {
                    log::warn!("Pipeline {pipeline_id} not found");
                    return GetPipelineByIdStatus::PipelineIdNotFound;
                };
                match pipeline {
                    Pipeline::$pipeline_type(pipeline) => {
                        output_pipeline.write(pipeline.into());
                        GetPipelineByIdStatus::Success
                    },
                    _ => {
                        log::error!("Pipeline {pipeline_id} not the correct PipelineType");
                        GetPipelineByIdStatus::PipelineTypeIncorrect
                    }
                }
            }

        )*
    }
}

pipeline_asset_manager_pipeline_type_by_id_functions!(
    (
        pipeline_asset_manager_get_pending_pipeline_by_id,
        Pending,
        FfiPendingPipeline
    ),
    (
        pipeline_asset_manager_get_engine_pipeline_by_id,
        Engine,
        FfiEnginePipeline
    ),
    (
        pipeline_asset_manager_get_loaded_pipeline_by_id,
        Loaded,
        FfiLoadedPipeline
    ),
    (
        pipeline_asset_manager_get_failed_pipeline_by_id,
        Failed,
        FfiFailedPipeline
    )
);

/// # Safety
///
/// `pipeline_asset_manager` must point to a valid [`PipelineAssetManager`].
/// This pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`PipelineAssetManager`] should be alive during this call.
/// `ids` must point to a valid array of [`PipelineId`] and the `id_len` must be
/// correct.
pub unsafe extern "C" fn pipeline_asset_manager_are_ids_loaded(
    pipeline_asset_manager: *const PipelineAssetManager,
    ids: *const PipelineId,
    id_len: usize,
) -> bool {
    let pipeline_asset_manager = unsafe { pipeline_asset_manager.as_ref().unwrap() };
    let ids = unsafe { from_raw_parts(ids, id_len) };

    pipeline_asset_manager.are_all_ids_loaded(ids)
}

/// # Safety
///
/// `pipeline_asset_manager` must point to a valid [`PipelineAssetManager`].
/// This pointer is equivalent to Rust's immutable reference, so no "mutable"
/// references to [`PipelineAssetManager`] should be alive during this call.
pub unsafe extern "C" fn pipeline_asset_manager_is_id_loaded(
    pipeline_asset_manager: *const PipelineAssetManager,
    id: PipelineId,
) -> bool {
    let pipeline_asset_manager = unsafe { pipeline_asset_manager.as_ref().unwrap() };

    pipeline_asset_manager.are_all_ids_loaded(&[id])
}

/// A return value of `0` indicates that the process of loading the pipeline has
/// started correctly. A return value of `1`  or higher indicates an error, and
/// the error should be printed to stderr unless the log level is ignoring
/// errors. Note, this does not mean the pipeline has loaded correctly, as that
/// error could occur later in the process.
///
/// # Errors
///
/// * 1 indicates that the `pipeline_asset_manager` is an invalid pointer.
/// * 2 indicates that the `output_pending_pipeline` is an invalid pointer.
///
/// # Safety
///
/// `pipeline_asset_manager` must point to a valid [`PipelineAssetManager`].
/// This pointer is equivalent to Rust's mutable reference, so no other
/// references to [`PipelineAssetManager`] should be alive during this call.
/// `new_pipeline_event_writer` should point to a valid
/// [`EventWriter<NewPipeline>`] handle. This pointer is equivalent to Rust's
/// immutable reference, so no mutable references to
/// [`EventWriter<NewPipeline>`] should be alive during this call.
pub unsafe extern "C" fn pipeline_asset_manager_load_pipeline(
    pipeline_asset_manager: *mut PipelineAssetManager,
    new_pipeline_event_writer_handle: *const c_void,
    material_id: MaterialId,
    output_pending_pipeline: *mut MaybeUninit<FfiPendingPipeline>,
) -> LoadPipelineStatus {
    let new_pipeline_event_writer =
        unsafe { EventWriter::<NewPipeline>::new(new_pipeline_event_writer_handle) };
    let Some(pipeline_asset_manager) = (unsafe { pipeline_asset_manager.as_mut() }) else {
        return LoadPipelineStatus::PipelineAssetManagerNull;
    };
    let Some(output_pending_pipeline) = (unsafe { output_pending_pipeline.as_mut() }) else {
        return LoadPipelineStatus::OutputPendingPipelineNull;
    };

    let pending_pipeline =
        pipeline_asset_manager.load_pipeline(material_id, &new_pipeline_event_writer);
    let ffi_pending_pipeline = pending_pipeline.into();
    output_pending_pipeline.write(ffi_pending_pipeline);

    LoadPipelineStatus::Success
}

/// A return value of `0` indicates that the process of loading the pipeline has
/// started correctly. A return value of `1` or higher indicates an error, and
/// the error should be printed to stderr unless the log level is ignoring
/// errors. Note, this does not mean the pipeline has loaded correctly, as that
/// error could occur later in the process.
///
/// # Safety
///
/// `pipeline_asset_manager` must point to a valid [`PipelineAssetManager`].
/// This pointer is equivalent to Rust's mutable reference, so no other
/// references to [`PipelineAssetManager`] should be alive during this call.
/// `new_pipeline_event_writer` should point to a valid
/// [`EventWriter<NewPipeline>`] handle. This pointer is equivalent to Rust's
/// immutable reference, so no mutable references to
/// [`EventWriter<NewPipeline>`] should be alive during this call.
pub unsafe extern "C" fn pipeline_asset_manager_load_pipeline_by_pending_pipeline(
    pipeline_asset_manager: *mut PipelineAssetManager,
    new_pipeline_event_writer_handle: *const c_void,
    pending_pipeline: *const FfiPendingPipeline,
) -> LoadPipelineByPendingPipelineStatus {
    let pending_pipeline = unsafe { pending_pipeline.as_ref().unwrap() }.into();
    let new_pipeline_event_writer =
        unsafe { EventWriter::<NewPipeline>::new(new_pipeline_event_writer_handle) };
    let Some(pipeline_asset_manager) = (unsafe { pipeline_asset_manager.as_mut() }) else {
        return LoadPipelineByPendingPipelineStatus::Success;
    };

    pipeline_asset_manager
        .load_pipeline_by_pending_pipeline(&pending_pipeline, &new_pipeline_event_writer);

    LoadPipelineByPendingPipelineStatus::Success
}

/// # Safety
///
/// `gpu_interface` can be a null pointer, but then it will return a null
/// pointer. However, the lifetime of [`PipelineAssetManager`] must be the same
/// or less than [`GpuInterface`].
pub unsafe extern "C" fn gpu_interface_get_pipeline_asset_manager_mut(
    gpu_interface: *mut GpuInterface,
) -> *mut PipelineAssetManager {
    if gpu_interface.is_null() {
        return null_mut();
    }
    let gpu_interface = unsafe { gpu_interface.as_mut().unwrap() };
    &mut gpu_interface.pipeline_asset_manager as *mut PipelineAssetManager
}
