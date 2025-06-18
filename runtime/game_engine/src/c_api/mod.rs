//! Engine API bindings for modules using the C ABI
//!
//! While C API functions/retrieving function pointers are technically a
//! platform concern and could be implemented at the platform level, virtually
//! all platforms will use the native C API. Putting C API implementation
//! details here allows us to reuse code across various platforms.

use std::{
    ffi::{CStr, c_char, c_void},
    mem::ManuallyDrop,
    ptr,
};

use engine_core::{
    add_components, call, call_async, completion_count, completion_get, despawn, entity_label,
    event_count, event_get, event_send, get_parent, load_scene, query_for_each, query_get,
    query_get_entity, query_get_label, query_len, query_par_for_each, remove_components,
    set_entity_label, set_parent, set_system_enabled, spawn,
};
use gpu_common::Gpu;
use material_manager::*;
use pipeline_asset_manager::*;
use platform::Platform;
use text_asset_manager::*;
use texture_asset_manager::*;
use void_public::FfiVec;

pub mod engine_core;
pub mod material_manager;
pub mod pipeline_asset_manager;
pub mod text_asset_manager;
pub mod texture_asset_manager;

/// Get a function pointer for the given `proc_name`.
///
/// Available procedures may vary by engine version. This function is the most
/// reliable place to check available procedures. These procedures wrap Rust ABI
/// functions found in `game_engine::module_api`.
///
/// Note that we use string labels for lookup, rather than an enum, because enum
/// values are easy to misalign during development and between engine versions.
/// This also aligns with the standard way to fetch function pointers from dlls.
///
/// Returns a function pointer if the procedure exists.
/// Returns a null pointer if the procedure does not exist.
pub fn get_module_api_proc_addr<P: Platform, G: Gpu>(proc_name: &CStr) -> *const c_void {
    if proc_name == c"add_components" {
        add_components::<P, G> as *const c_void
    } else if proc_name == c"call" {
        call::<P, G> as *const c_void
    } else if proc_name == c"call_async" {
        call_async::<P, G> as *const c_void
    } else if proc_name == c"completion_count" {
        completion_count::<P, G> as *const c_void
    } else if proc_name == c"completion_get" {
        completion_get::<P, G> as *const c_void
    } else if proc_name == c"despawn" {
        despawn::<P, G> as *const c_void
    } else if proc_name == c"entity_label" {
        entity_label::<P, G> as *const c_void
    } else if proc_name == c"event_count" {
        event_count as *const c_void
    } else if proc_name == c"event_get" {
        event_get as *const c_void
    } else if proc_name == c"event_send" {
        event_send::<P> as *const c_void
    } else if proc_name == c"get_parent" {
        get_parent::<P, G> as *const c_void
    } else if proc_name == c"load_scene" {
        load_scene::<P, G> as *const c_void
    } else if proc_name == c"query_for_each" {
        query_for_each::<G> as *const c_void
    } else if proc_name == c"query_get" {
        query_get::<G> as *const c_void
    } else if proc_name == c"query_get_entity" {
        query_get_entity::<G> as *const c_void
    } else if proc_name == c"query_get_label" {
        query_get_label::<P, G> as *const c_void
    } else if proc_name == c"query_len" {
        query_len::<G> as *const c_void
    } else if proc_name == c"query_par_for_each" {
        query_par_for_each::<P, G> as *const c_void
    } else if proc_name == c"remove_components" {
        remove_components::<P, G> as *const c_void
    } else if proc_name == c"set_entity_label" {
        set_entity_label::<P, G> as *const c_void
    } else if proc_name == c"set_parent" {
        set_parent::<P, G> as *const c_void
    } else if proc_name == c"set_system_enabled" {
        set_system_enabled::<P, G> as *const c_void
    } else if proc_name == c"spawn" {
        spawn::<P, G> as *const c_void
    } else if proc_name == c"texture_asset_manager_white_texture_id" {
        texture_asset_manager_white_texture_id as *const c_void
    } else if proc_name == c"texture_asset_manager_missing_texture_id" {
        texture_asset_manager_missing_texture_id as *const c_void
    } else if proc_name == c"texture_asset_manager_register_next_texture_id" {
        texture_asset_manager_register_next_texture_id as *const c_void
    } else if proc_name == c"texture_asset_manager_generate_hash" {
        texture_asset_manager_generate_hash as *const c_void
    } else if proc_name == c"texture_asset_manager_create_pending_texture" {
        texture_asset_manager_create_pending_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_free_pending_texture" {
        texture_asset_manager_free_pending_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_free_engine_texture" {
        texture_asset_manager_free_engine_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_free_loaded_texture" {
        texture_asset_manager_free_loaded_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_free_failed_texture" {
        texture_asset_manager_free_failed_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_get_texture_type_by_id" {
        texture_asset_manager_get_texture_type_by_id as *const c_void
    } else if proc_name == c"texture_asset_manager_get_pending_texture_by_id" {
        texture_asset_manager_get_pending_texture_by_id as *const c_void
    } else if proc_name == c"texture_asset_manager_get_engine_texture_by_id" {
        texture_asset_manager_get_engine_texture_by_id as *const c_void
    } else if proc_name == c"texture_asset_manager_get_loaded_texture_by_id" {
        texture_asset_manager_get_loaded_texture_by_id as *const c_void
    } else if proc_name == c"texture_asset_manager_get_failed_texture_by_id" {
        texture_asset_manager_get_failed_texture_by_id as *const c_void
    } else if proc_name == c"texture_asset_manager_get_texture_type_by_path" {
        texture_asset_manager_get_texture_type_by_path as *const c_void
    } else if proc_name == c"texture_asset_manager_get_pending_texture_by_path" {
        texture_asset_manager_get_pending_texture_by_path as *const c_void
    } else if proc_name == c"texture_asset_manager_get_engine_texture_by_path" {
        texture_asset_manager_get_engine_texture_by_path as *const c_void
    } else if proc_name == c"texture_asset_manager_get_loaded_texture_by_path" {
        texture_asset_manager_get_loaded_texture_by_path as *const c_void
    } else if proc_name == c"texture_asset_manager_get_failed_texture_by_path" {
        texture_asset_manager_get_failed_texture_by_path as *const c_void
    } else if proc_name == c"texture_asset_manager_are_ids_loaded" {
        texture_asset_manager_are_ids_loaded as *const c_void
    } else if proc_name == c"texture_asset_manager_is_id_loaded" {
        texture_asset_manager_is_id_loaded as *const c_void
    } else if proc_name == c"texture_asset_manager_load_texture" {
        texture_asset_manager_load_texture as *const c_void
    } else if proc_name == c"texture_asset_manager_load_texture_by_pending_texture" {
        texture_asset_manager_load_texture_by_pending_texture as *const c_void
    } else if proc_name == c"text_asset_manager_register_next_text_id" {
        text_asset_manager_register_next_text_id as *const c_void
    } else if proc_name == c"text_asset_manager_generate_hash" {
        text_asset_manager_generate_hash as *const c_void
    } else if proc_name == c"text_asset_manager_create_pending_text" {
        text_asset_manager_create_pending_text as *const c_void
    } else if proc_name == c"text_asset_manager_free_pending_text" {
        text_asset_manager_free_pending_text as *const c_void
    } else if proc_name == c"text_asset_manager_free_engine_text" {
        text_asset_manager_free_engine_text as *const c_void
    } else if proc_name == c"text_asset_manager_free_loaded_text" {
        text_asset_manager_free_loaded_text as *const c_void
    } else if proc_name == c"text_asset_manager_free_failed_text" {
        text_asset_manager_free_failed_text as *const c_void
    } else if proc_name == c"text_asset_manager_get_text_type_by_id" {
        text_asset_manager_get_text_type_by_id as *const c_void
    } else if proc_name == c"text_asset_manager_get_pending_text_by_id" {
        text_asset_manager_get_pending_text_by_id as *const c_void
    } else if proc_name == c"text_asset_manager_get_engine_text_by_id" {
        text_asset_manager_get_engine_text_by_id as *const c_void
    } else if proc_name == c"text_asset_manager_get_loaded_text_by_id" {
        text_asset_manager_get_loaded_text_by_id as *const c_void
    } else if proc_name == c"text_asset_manager_get_failed_text_by_id" {
        text_asset_manager_get_failed_text_by_id as *const c_void
    } else if proc_name == c"text_asset_manager_get_text_type_by_path" {
        text_asset_manager_get_text_type_by_path as *const c_void
    } else if proc_name == c"text_asset_manager_get_pending_text_by_path" {
        text_asset_manager_get_pending_text_by_path as *const c_void
    } else if proc_name == c"text_asset_manager_get_engine_text_by_path" {
        text_asset_manager_get_engine_text_by_path as *const c_void
    } else if proc_name == c"text_asset_manager_get_loaded_text_by_path" {
        text_asset_manager_get_loaded_text_by_path as *const c_void
    } else if proc_name == c"text_asset_manager_get_failed_text_by_path" {
        text_asset_manager_get_failed_text_by_path as *const c_void
    } else if proc_name == c"text_asset_manager_are_ids_loaded" {
        text_asset_manager_are_ids_loaded as *const c_void
    } else if proc_name == c"text_asset_manager_is_id_loaded" {
        text_asset_manager_is_id_loaded as *const c_void
    } else if proc_name == c"text_asset_manager_load_text" {
        text_asset_manager_load_text as *const c_void
    } else if proc_name == c"text_asset_manager_load_text_by_pending_text" {
        text_asset_manager_load_text_by_pending_text as *const c_void
    } else if proc_name == c"pipeline_asset_manager_free_pending_pipeline" {
        pipeline_asset_manager_free_pending_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_free_engine_pipeline" {
        pipeline_asset_manager_free_engine_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_free_loaded_pipeline" {
        pipeline_asset_manager_free_loaded_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_free_failed_pipeline" {
        pipeline_asset_manager_free_failed_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_register_next_pipeline_id" {
        pipeline_asset_manager_register_next_pipeline_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_create_pending_pipeline" {
        pipeline_asset_manager_create_pending_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_get_pipeline_type_by_id" {
        pipeline_asset_manager_get_pipeline_type_by_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_get_pending_pipeline_by_id" {
        pipeline_asset_manager_get_pending_pipeline_by_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_get_engine_pipeline_by_id" {
        pipeline_asset_manager_get_engine_pipeline_by_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_get_loaded_pipeline_by_id" {
        pipeline_asset_manager_get_loaded_pipeline_by_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_get_failed_pipeline_by_id" {
        pipeline_asset_manager_get_failed_pipeline_by_id as *const c_void
    } else if proc_name == c"pipeline_asset_manager_are_ids_loaded" {
        pipeline_asset_manager_are_ids_loaded as *const c_void
    } else if proc_name == c"pipeline_asset_manager_is_id_loaded" {
        pipeline_asset_manager_is_id_loaded as *const c_void
    } else if proc_name == c"pipeline_asset_manager_load_pipeline" {
        pipeline_asset_manager_load_pipeline as *const c_void
    } else if proc_name == c"pipeline_asset_manager_load_pipeline_by_pending_pipeline" {
        pipeline_asset_manager_load_pipeline_by_pending_pipeline as *const c_void
    } else if proc_name == c"material_manager_materials_len" {
        material_manager_materials_len as *const c_void
    } else if proc_name == c"material_manager_materials" {
        material_manager_materials as *const c_void
    } else if proc_name == c"material_manager_free_material" {
        material_manager_free_material as *const c_void
    } else if proc_name == c"material_manager_free_uniform_value" {
        material_manager_free_uniform_value as *const c_void
    } else if proc_name == c"material_manager_free_texture_desc" {
        material_manager_free_texture_desc as *const c_void
    } else if proc_name == c"free_rust_generated_c_str" {
        free_rust_generated_c_str as *const c_void
    } else if proc_name == c"material_manager_load_shader_template_from_path" {
        material_manager_load_shader_template_from_path as *const c_void
    } else if proc_name == c"material_manager_load_material_from_path" {
        material_manager_load_material_from_path as *const c_void
    } else if proc_name == c"material_manager_register_material_from_string" {
        material_manager_register_material_from_string as *const c_void
    } else if proc_name == c"material_manager_uniform_names_and_default_values_len" {
        material_manager_uniform_names_and_default_values_len as *const c_void
    } else if proc_name == c"material_manager_uniform_names_and_default_values" {
        material_manager_uniform_names_and_default_values as *const c_void
    } else if proc_name == c"material_manager_generate_shader_text" {
        material_manager_generate_shader_text as *const c_void
    } else if proc_name == c"material_manager_get_id_from_text_id" {
        material_manager_get_id_from_text_id as *const c_void
    } else if proc_name == c"material_manager_update_material_from_string" {
        material_manager_update_material_from_string as *const c_void
    } else if proc_name == c"material_params_as_uniform_values_len" {
        material_params_as_uniform_values_len as *const c_void
    } else if proc_name == c"material_params_as_uniform_values" {
        material_params_as_uniform_values as *const c_void
    } else if proc_name == c"material_params_update_from_uniform_values" {
        material_params_update_from_uniform_values as *const c_void
    } else if proc_name == c"material_params_as_texture_descs_len" {
        material_params_as_texture_descs_len as *const c_void
    } else if proc_name == c"material_params_as_texture_descs" {
        material_params_as_texture_descs as *const c_void
    } else if proc_name == c"material_params_update_from_texture_descs" {
        material_params_update_from_texture_descs as *const c_void
    } else if proc_name == c"gpu_interface_get_texture_asset_manager_mut" {
        gpu_interface_get_texture_asset_manager_mut as *const c_void
    } else if proc_name == c"gpu_interface_get_pipeline_asset_manager_mut" {
        gpu_interface_get_pipeline_asset_manager_mut as *const c_void
    } else if proc_name == c"gpu_interface_get_material_manager_mut" {
        gpu_interface_get_material_manager_mut as *const c_void
    } else {
        log::warn!("native module attempted to load invalid procedure: {proc_name:?}");
        ptr::null()
    }
}

/// C Interface for getting a function pointer from a given `proc_name`
///
/// # Safety
///
/// All FFI calls are unsafe. `proc_name` and the return function pointer are
/// both raw pointers and bring all the unsafe baggage of raw pointers
pub unsafe extern "C" fn get_module_api_proc_addr_c<P: Platform, G: Gpu>(
    proc_name: *const c_char,
) -> *const c_void {
    let proc_name = unsafe { CStr::from_ptr(proc_name) };
    get_module_api_proc_addr::<P, G>(proc_name)
}

pub(crate) fn ffi_vec_from_vec<T>(rust_vec: Vec<T>) -> FfiVec<T> {
    let mut value = ManuallyDrop::new(rust_vec);
    let ptr = value.as_mut_ptr();
    let len = value.len();
    let capacity = value.capacity();

    FfiVec { ptr, len, capacity }
}
