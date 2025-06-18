use std::{
    alloc::{Layout, alloc, dealloc},
    borrow::Cow,
    env,
    ffi::{CStr, CString, c_void},
    mem::MaybeUninit,
    num::NonZero,
    ptr::{self, null_mut},
};

use deno_core::v8;
use deno_runtime::deno_core::op2;
use game_asset::{
    ecs_module::{GpuInterface, TextAssetManager},
    resource_managers::texture_asset_manager::TextureAssetManager,
};
use game_engine::{
    c_api::{
        self,
        text_asset_manager::{self, FfiEngineText, FfiFailedText, FfiLoadedText, FfiPendingText},
        texture_asset_manager,
    },
    game_ecs::{EcsSystemExecuteResources, system_execute_resources},
    module_api,
    void_public::{
        ComponentId, ComponentRef,
        graphics::{TextureId, TextureType},
        text::{TextId, TextType},
    },
};
use game_entity::EntityId;
use gpu_web::GpuWeb;

use crate::Platform;

deno_core::extension!(
  fiasco,
  ops = [
    // Memory Related
    op_fiasco_debug_addr,
    op_fiasco_null_ptr,
    op_fiasco_read_bool,
    op_fiasco_read_u8,
    op_fiasco_read_i8,
    op_fiasco_read_u16,
    op_fiasco_read_i16,
    op_fiasco_read_u32,
    op_fiasco_read_i32,
    op_fiasco_read_u64,
    op_fiasco_read_i64,
    op_fiasco_read_f32,
    op_fiasco_read_f64,
    op_fiasco_read_ptr,
    op_fiasco_write_bool,
    op_fiasco_write_u8,
    op_fiasco_write_i8,
    op_fiasco_write_u16,
    op_fiasco_write_i16,
    op_fiasco_write_u32,
    op_fiasco_write_i32,
    op_fiasco_write_u64,
    op_fiasco_write_i64,
    op_fiasco_write_f32,
    op_fiasco_write_f64,
    op_fiasco_write_ptr,
    op_fiasco_alloc,
    op_fiasco_dealloc,
    op_fiasco_register_engine,
    op_fiasco_get_engine,

    // Engine API's
    op_fiasco_load_scene,
    op_fiasco_spawn,
    op_fiasco_despawn,
    op_fiasco_add_components,
    op_fiasco_remove_components,
    op_fiasco_get_entity_label,
    op_fiasco_set_entity_label,
    op_fiasco_query_len,
    op_fiasco_query_get,
    op_fiasco_query_get_entity,
    op_fiasco_query_get_label,
    op_fiasco_set_system_enabled,
    op_fiasco_event_count,
    op_fiasco_event_get,
    op_fiasco_event_send,
    op_fiasco_set_parent,
    op_fiasco_clear_parent,
    op_fiasco_get_parent,
    op_fiasco_input_buffer_ptr,
    op_fiasco_input_buffer_len,

    // Texture Asset Manager
    op_texture_asset_manager_white_texture_id,
    op_texture_asset_manager_missing_texture_id,
    op_texture_asset_manager_register_next_texture_id,
    op_texture_asset_manager_create_pending_texture,
    op_texture_asset_manager_free_pending_texture,
    op_texture_asset_manager_free_engine_texture,
    op_texture_asset_manager_free_loaded_texture,
    op_texture_asset_manager_free_failed_texture,
    op_texture_asset_manager_get_texture_type_by_id,
    op_texture_asset_manager_get_pending_texture_by_id,
    op_texture_asset_manager_get_engine_texture_by_id,
    op_texture_asset_manager_get_loaded_texture_by_id,
    op_texture_asset_manager_get_failed_texture_by_id,
    op_texture_asset_manager_get_texture_type_by_path,
    op_texture_asset_manager_get_pending_texture_by_path,
    op_texture_asset_manager_get_engine_texture_by_path,
    op_texture_asset_manager_get_loaded_texture_by_path,
    op_texture_asset_manager_get_failed_texture_by_path,
    op_texture_asset_manager_are_ids_loaded,
    op_texture_asset_manager_is_id_loaded,
    op_texture_asset_manager_load_texture,
    op_texture_asset_manager_load_texture_by_pending_texture,
    op_gpu_interface_get_texture_asset_manager_mut,

    // Text Asset Manager
    op_text_asset_manager_register_next_text_id,
    op_text_asset_manager_create_pending_text,
    op_text_asset_manager_free_pending_text,
    op_text_asset_manager_free_engine_text,
    op_text_asset_manager_free_loaded_text,
    op_text_asset_manager_free_failed_text,
    op_text_asset_manager_get_text_type_by_id,
    op_text_asset_manager_get_pending_text_by_id,
    op_text_asset_manager_get_engine_text_by_id,
    op_text_asset_manager_get_loaded_text_by_id,
    op_text_asset_manager_get_failed_text_by_id,
    op_text_asset_manager_get_text_type_by_path,
    op_text_asset_manager_get_pending_text_by_path,
    op_text_asset_manager_get_engine_text_by_path,
    op_text_asset_manager_get_loaded_text_by_path,
    op_text_asset_manager_get_failed_text_by_path,
    op_text_asset_manager_are_ids_loaded,
    op_text_asset_manager_is_id_loaded,
    op_text_asset_manager_load_text,
    op_text_asset_manager_load_text_by_pending_text
  ],
  esm_entry_point = "ext:fiasco/extensions.ts",
  esm = [dir "js/src", "extensions.ts"],
  state = |state| {
    state.put(SharedState {
        engine: None,
        change_in_external_memory: 0,
    });
  }
);

pub struct SharedState {
    /// The global `engine` object. Created in the main module
    /// and then registered here. This is the accessed once per
    /// JS ECS Module when it first initializes and a reference
    /// is stored on the JS side on the global scope.
    pub engine: Option<v8::Global<v8::Object>>,

    /// Tracks the change in the amount of externally-allocated memory, i.e. the
    /// memory allocated or deallocated in the `alloc`/`dealloc` ops.
    pub change_in_external_memory: isize,
}

#[op2(stack_trace)]
pub fn op_fiasco_register_engine(
    #[state] state: &mut SharedState,
    #[global] engine: v8::Global<v8::Object>,
) {
    state.engine = Some(engine);
}

#[op2(stack_trace)]
#[global]
pub fn op_fiasco_get_engine(#[state] state: &SharedState) -> v8::Global<v8::Object> {
    state.engine.clone().unwrap()
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_bool(ptr: *mut c_void, #[number] offset: isize) -> bool {
    unsafe { ptr::read_unaligned::<bool>(ptr.offset(offset).cast::<bool>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_u8(ptr: *mut c_void, #[number] offset: isize) -> u32 {
    unsafe { ptr::read_unaligned::<u8>(ptr.offset(offset).cast::<u8>()) as u32 }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_i8(ptr: *mut c_void, #[number] offset: isize) -> i32 {
    unsafe { ptr::read_unaligned::<i8>(ptr.offset(offset).cast::<i8>()) as i32 }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_u16(ptr: *mut c_void, #[number] offset: isize) -> u32 {
    unsafe { ptr::read_unaligned::<u16>(ptr.offset(offset).cast::<u16>()) as u32 }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_i16(ptr: *mut c_void, #[number] offset: isize) -> i32 {
    unsafe { ptr::read_unaligned::<i16>(ptr.offset(offset).cast::<i16>()) as i32 }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_u32(ptr: *mut c_void, #[number] offset: isize) -> u32 {
    unsafe { ptr::read_unaligned::<u32>(ptr.offset(offset).cast::<u32>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_i32(ptr: *mut c_void, #[number] offset: isize) -> i32 {
    unsafe { ptr::read_unaligned::<i32>(ptr.offset(offset).cast::<i32>()) }
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_read_u64(ptr: *mut c_void, #[bigint] offset: isize) -> u64 {
    unsafe { ptr::read_unaligned::<u64>(ptr.offset(offset).cast::<u64>()) }
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_read_i64(ptr: *mut c_void, #[bigint] offset: isize) -> i64 {
    unsafe { ptr::read_unaligned::<i64>(ptr.offset(offset).cast::<i64>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_f32(ptr: *mut c_void, #[number] offset: isize) -> f32 {
    unsafe { ptr::read_unaligned::<f32>(ptr.offset(offset).cast::<f32>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_f64(ptr: *mut c_void, #[number] offset: isize) -> f64 {
    unsafe { ptr::read_unaligned::<f64>(ptr.offset(offset).cast::<f64>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_read_ptr(ptr: *mut c_void, #[number] offset: isize) -> *mut c_void {
    unsafe { ptr::read_unaligned::<*mut c_void>(ptr.offset(offset).cast::<*mut c_void>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_bool(ptr: *mut c_void, #[number] offset: isize, val: bool) {
    unsafe { ptr::write_unaligned::<bool>(ptr.offset(offset).cast::<bool>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_u8(ptr: *mut c_void, #[number] offset: isize, val: u32) {
    unsafe { ptr::write_unaligned::<u8>(ptr.offset(offset).cast::<u8>(), val as u8) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_i8(ptr: *mut c_void, #[number] offset: isize, val: i32) {
    unsafe { ptr::write_unaligned::<i8>(ptr.offset(offset).cast::<i8>(), val as i8) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_u16(ptr: *mut c_void, #[number] offset: isize, val: u32) {
    unsafe { ptr::write_unaligned::<u16>(ptr.offset(offset).cast::<u16>(), val as u16) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_i16(ptr: *mut c_void, #[number] offset: isize, val: i32) {
    unsafe { ptr::write_unaligned::<i16>(ptr.offset(offset).cast::<i16>(), val as i16) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_u32(ptr: *mut c_void, #[number] offset: isize, val: u32) {
    unsafe { ptr::write_unaligned::<u32>(ptr.offset(offset).cast::<u32>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_i32(ptr: *mut c_void, #[number] offset: isize, val: i32) {
    unsafe { ptr::write_unaligned::<i32>(ptr.offset(offset).cast::<i32>(), val) }
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_write_u64(ptr: *mut c_void, #[bigint] offset: isize, #[bigint] val: u64) {
    unsafe { ptr::write_unaligned::<u64>(ptr.offset(offset).cast::<u64>(), val) }
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_write_i64(ptr: *mut c_void, #[bigint] offset: isize, #[bigint] val: i64) {
    unsafe { ptr::write_unaligned::<i64>(ptr.offset(offset).cast::<i64>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_f32(ptr: *mut c_void, #[number] offset: isize, val: f32) {
    unsafe { ptr::write_unaligned::<f32>(ptr.offset(offset).cast::<f32>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_f64(ptr: *mut c_void, #[number] offset: isize, val: f64) {
    unsafe { ptr::write_unaligned::<f64>(ptr.offset(offset).cast::<f64>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_write_ptr(ptr: *mut c_void, #[number] offset: isize, val: *mut c_void) {
    unsafe { ptr::write_unaligned::<*mut c_void>(ptr.offset(offset).cast::<*mut c_void>(), val) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_load_scene(#[string] scene_json: &str) {
    let scene_str = CString::new(scene_json).unwrap();
    module_api::load_scene::<Platform, GpuWeb>(&scene_str);
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_spawn(components: *const c_void, #[number] components_len: usize) -> u64 {
    let components = components.cast::<ComponentRef<'_>>();

    unsafe {
        c_api::engine_core::spawn::<Platform, GpuWeb>(components, components_len)
            .map_or(0, |entity_id| {
                NonZero::from(EntityId::from(entity_id)).get()
            })
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_despawn(#[bigint] entity_id: u64) {
    let Ok(entity_id) = entity_id.try_into() else {
        log::warn!("passed EntityId of zero");
        return;
    };

    module_api::despawn::<Platform, GpuWeb>(entity_id);
}

/// All of the following conditions must be met:
///
/// * `align` must not be zero,
///
/// * `align` must be a power of two,
///
/// * `size`, when rounded up to the nearest multiple of `align`,
///    must not overflow `isize` (i.e., the rounded value must be
///    less than or equal to `isize::MAX`).
#[op2(fast, stack_trace)]
pub fn op_fiasco_alloc(
    #[state] state: &mut SharedState,
    #[number] size: usize,
    #[number] align: usize,
) -> *mut c_void {
    assert_ne!(size, 0);

    state.change_in_external_memory += size as isize;

    let layout = Layout::from_size_align(size, align).unwrap();
    unsafe { alloc(layout).cast() }
}

#[op2(fast, stack_trace)]
#[bigint]
pub fn op_fiasco_debug_addr(ptr: *const c_void) -> usize {
    ptr.addr()
}

/// # Safety
///
/// The `size` and `align` for this pointer **must** match the exact `size` and
/// `align` which were passed to the `op_ffi_alloc` function.
#[op2(fast, stack_trace)]
pub fn op_fiasco_dealloc(
    #[state] state: &mut SharedState,
    ptr: *mut c_void,
    #[number] size: usize,
    #[number] align: usize,
) {
    state.change_in_external_memory -= size as isize;

    let layout = Layout::from_size_align(size, align).unwrap();

    unsafe {
        dealloc(ptr.cast(), layout);
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_add_components(
    #[bigint] entity_id: u64,
    components: *const c_void,
    #[number] size: usize,
) {
    let components = components.cast::<ComponentRef<'_>>();

    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return;
    };

    unsafe {
        c_api::engine_core::add_components::<Platform, GpuWeb>(entity_id.into(), components, size);
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_remove_components(#[bigint] entity_id: u64, #[arraybuffer] component_ids: &[u8]) {
    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return;
    };

    unsafe {
        c_api::engine_core::remove_components::<Platform, GpuWeb>(
            entity_id.into(),
            component_ids.as_ptr().cast::<Option<ComponentId>>(),
            component_ids.len() / size_of::<Option<ComponentId>>(),
        );
    }
}

#[op2(stack_trace)]
#[string]
pub fn op_fiasco_get_entity_label(#[bigint] entity_id: u64) -> String {
    let entity_id = EntityId::try_from(entity_id).unwrap();
    let ptr = module_api::entity_label::<Platform, GpuWeb>(entity_id);

    if ptr.is_null() {
        return String::new();
    }

    let c_str = unsafe { CStr::from_ptr(ptr) };
    c_str.to_str().map(String::from).unwrap()
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_set_entity_label(#[bigint] entity_id: u64, #[string] label: Cow<'_, str>) {
    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return;
    };

    let c_label_result = match label {
        Cow::Borrowed(s) => CString::new(s),
        Cow::Owned(s) => CString::new(s),
    };

    match c_label_result {
        Ok(c_label) => {
            module_api::set_entity_label::<Platform, GpuWeb>(entity_id, Some(c_label.as_c_str()));
        }
        Err(nul_error) => eprintln!("Error: label contained a null byte: {}", nul_error),
    }
}

#[op2(fast, stack_trace)]
#[number]
pub fn op_fiasco_query_len(query: *const c_void) -> usize {
    unsafe { module_api::query_len::<GpuWeb>(query) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_query_get(
    query: *const c_void,
    #[number] index: usize,
    component_ptrs: *mut c_void,
) -> bool {
    unsafe { module_api::query_get::<GpuWeb>(query, index, component_ptrs.cast::<*const c_void>()) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_query_get_entity(
    query: *const c_void,
    #[bigint] entity_id: u64,
    component_ptrs: *mut c_void,
) -> bool {
    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return false;
    };

    unsafe {
        module_api::query_get_entity::<GpuWeb>(
            query,
            entity_id,
            component_ptrs.cast::<*const c_void>(),
        )
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_query_get_label(
    query: *const c_void,
    #[string] label: Cow<'_, str>,
    component_ptrs: *mut c_void,
) -> bool {
    let c_label_result = match label {
        Cow::Borrowed(s) => CString::new(s),
        Cow::Owned(s) => CString::new(s),
    };

    match c_label_result {
        Ok(c_label) => unsafe {
            module_api::query_get_label::<Platform, GpuWeb>(
                query,
                c_label.as_c_str(),
                component_ptrs.cast::<*const c_void>(),
            )
        },
        Err(nul_error) => {
            eprintln!("Error: label contained a null byte: {}", nul_error);
            false
        }
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_set_system_enabled(#[string] system_name: &str, enabled: bool) {
    module_api::set_system_enabled::<Platform, GpuWeb>(system_name, enabled);
}

#[op2(fast, stack_trace)]
#[number]
pub fn op_fiasco_event_count(reader: *const c_void) -> usize {
    unsafe { module_api::event_count(reader) }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_event_get(reader: *const c_void, #[number] index: usize) -> *const c_void {
    unsafe { module_api::event_get(reader, index).cast::<c_void>() }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_event_send(writer: *const c_void, #[arraybuffer] data: &[u8]) {
    unsafe { module_api::event_send::<Platform>(writer, data) };
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_set_parent(
    #[bigint] entity_id: u64,
    #[bigint] parent_id: u64,
    keep_world_space_transform: bool,
) {
    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return;
    };

    let Ok(parent_id) = EntityId::try_from(parent_id) else {
        log::warn!("passed EntityId of zero (parent_id)");
        return;
    };

    module_api::set_parent::<Platform, GpuWeb>(
        entity_id,
        Some(parent_id),
        keep_world_space_transform,
    );
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_clear_parent(#[bigint] entity_id: u64, keep_world_space_transform: bool) {
    let Ok(entity_id) = EntityId::try_from(entity_id) else {
        log::warn!("passed EntityId of zero");
        return;
    };

    module_api::set_parent::<Platform, GpuWeb>(entity_id, None, keep_world_space_transform);
}

#[op2(stack_trace)]
pub fn op_fiasco_get_parent<'scope>(
    scope: &mut v8::HandleScope<'scope>,
    #[bigint] entity_id: u64,
) -> v8::Local<'scope, v8::Value> {
    let entity_id = EntityId::try_from(entity_id).unwrap();

    match module_api::get_parent::<Platform, GpuWeb>(entity_id) {
        Ok(Some(id)) => {
            let value = NonZero::from(id).get();
            v8::BigInt::new_from_u64(scope, value).into()
        }
        Ok(None) => v8::String::new(scope, "no_parent").unwrap().into(),
        Err(_) => v8::String::new(scope, "invalid_id").unwrap().into(),
    }
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_input_buffer_ptr() -> *const c_void {
    system_execute_resources(
        |resources: &EcsSystemExecuteResources<'_, Platform, GpuWeb>| {
            resources.input_buffer.as_ptr()
        },
    )
    .cast::<c_void>()
}

#[op2(fast, stack_trace)]
#[number]
pub fn op_fiasco_input_buffer_len() -> usize {
    system_execute_resources(
        |resources: &EcsSystemExecuteResources<'_, Platform, GpuWeb>| resources.input_buffer.len(),
    )
}

#[op2(fast, stack_trace)]
pub fn op_fiasco_null_ptr() -> *mut c_void {
    null_mut()
}

// Texture Asset Manager

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_white_texture_id() -> u32 {
    texture_asset_manager::texture_asset_manager_white_texture_id().0
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_missing_texture_id() -> u32 {
    texture_asset_manager::texture_asset_manager_missing_texture_id().0
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_register_next_texture_id(
    texture_asset_manager: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_register_next_texture_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
        )
    }
    .0
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_create_pending_texture(
    id: u32,
    #[string] asset_path: &str,
    insert_in_atlas: bool,
    output_pending_texture: *mut c_void,
) -> u32 {
    let asset_path = CString::new(asset_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_create_pending_texture(
            id.into(),
            asset_path.as_ptr(),
            insert_in_atlas,
            output_pending_texture.cast::<MaybeUninit<texture_asset_manager::FfiPendingTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_free_pending_texture(pending_texture: *mut c_void) {
    unsafe {
        texture_asset_manager::texture_asset_manager_free_pending_texture(
            pending_texture.cast::<texture_asset_manager::FfiPendingTexture>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_free_engine_texture(engine_texture: *mut c_void) {
    unsafe {
        texture_asset_manager::texture_asset_manager_free_engine_texture(
            engine_texture.cast::<texture_asset_manager::FfiEngineTexture>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_free_loaded_texture(loaded_texture: *mut c_void) {
    unsafe {
        texture_asset_manager::texture_asset_manager_free_loaded_texture(
            loaded_texture.cast::<texture_asset_manager::FfiLoadedTexture>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_free_failed_texture(failed_texture: *mut c_void) {
    unsafe {
        texture_asset_manager::texture_asset_manager_free_failed_texture(
            failed_texture.cast::<texture_asset_manager::FfiFailedTexture>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_texture_type_by_id(
    texture_asset_manager: *const c_void,
    texture_id: u32,
    texture_type: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_get_texture_type_by_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_id.into(),
            texture_type.cast::<MaybeUninit<TextureType>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_pending_texture_by_id(
    texture_asset_manager: *const c_void,
    texture_id: u32,
    output_texture: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_get_pending_texture_by_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_id.into(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiPendingTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_engine_texture_by_id(
    texture_asset_manager: *const c_void,
    texture_id: u32,
    output_texture: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_get_engine_texture_by_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_id.into(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiEngineTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_loaded_texture_by_id(
    texture_asset_manager: *const c_void,
    texture_id: u32,
    output_texture: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_get_loaded_texture_by_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_id.into(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiLoadedTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_failed_texture_by_id(
    texture_asset_manager: *const c_void,
    texture_id: u32,
    output_texture: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_get_failed_texture_by_id(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_id.into(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiFailedTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_texture_type_by_path(
    texture_asset_manager: *const c_void,
    #[string] texture_path: &str,
    texture_type: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_get_texture_type_by_path(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_path.as_ptr(),
            texture_type.cast::<MaybeUninit<TextureType>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_pending_texture_by_path(
    texture_asset_manager: *const c_void,
    #[string] texture_path: &str,
    output_texture: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_get_pending_texture_by_path(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_path.as_ptr(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiPendingTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_engine_texture_by_path(
    texture_asset_manager: *const c_void,
    #[string] texture_path: &str,
    output_texture: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_get_engine_texture_by_path(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_path.as_ptr(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiEngineTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_loaded_texture_by_path(
    texture_asset_manager: *const c_void,
    #[string] texture_path: &str,
    output_texture: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_get_loaded_texture_by_path(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_path.as_ptr(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiLoadedTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_get_failed_texture_by_path(
    texture_asset_manager: *const c_void,
    #[string] texture_path: &str,
    output_texture: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_get_failed_texture_by_path(
            texture_asset_manager.cast::<TextureAssetManager>(),
            texture_path.as_ptr(),
            output_texture.cast::<MaybeUninit<texture_asset_manager::FfiFailedTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_are_ids_loaded(
    texture_asset_manager: *const c_void,
    #[buffer] ids: &[u32],
) -> bool {
    assert_eq!(size_of::<u32>(), size_of::<TextureId>());
    unsafe {
        texture_asset_manager::texture_asset_manager_are_ids_loaded(
            texture_asset_manager.cast::<TextureAssetManager>(),
            ids.as_ptr().cast::<TextureId>(),
            ids.len(),
        )
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_is_id_loaded(
    texture_asset_manager: *const c_void,
    id: u32,
) -> bool {
    unsafe {
        texture_asset_manager::texture_asset_manager_is_id_loaded(
            texture_asset_manager.cast::<TextureAssetManager>(),
            id.into(),
        )
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_load_texture(
    texture_asset_manager: *mut c_void,
    new_texture_event_writer_handle: *const c_void,
    #[string] texture_path: &str,
    insert_in_atlas: bool,
    output_pending_texture: *mut c_void,
) -> u32 {
    let texture_path = CString::new(texture_path).unwrap();
    unsafe {
        texture_asset_manager::texture_asset_manager_load_texture(
            texture_asset_manager.cast::<TextureAssetManager>(),
            new_texture_event_writer_handle,
            texture_path.as_ptr(),
            insert_in_atlas,
            output_pending_texture.cast::<MaybeUninit<texture_asset_manager::FfiPendingTexture>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_texture_asset_manager_load_texture_by_pending_texture(
    texture_asset_manager: *mut c_void,
    new_texture_event_writer_handle: *const c_void,
    output_pending_texture: *mut c_void,
) -> u32 {
    unsafe {
        texture_asset_manager::texture_asset_manager_load_texture_by_pending_texture(
            texture_asset_manager.cast::<TextureAssetManager>(),
            new_texture_event_writer_handle,
            output_pending_texture.cast::<texture_asset_manager::FfiPendingTexture>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_gpu_interface_get_texture_asset_manager_mut(gpu_interface: *mut c_void) -> *const c_void {
    unsafe {
        texture_asset_manager::gpu_interface_get_texture_asset_manager_mut(
            gpu_interface.cast::<GpuInterface>(),
        )
        .cast::<c_void>()
    }
}

// Text Asset Manager

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_register_next_text_id(text_asset_manager: *mut c_void) -> u32 {
    unsafe {
        text_asset_manager::text_asset_manager_register_next_text_id(
            text_asset_manager.cast::<TextAssetManager>(),
        )
    }
    .0
    .get()
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_create_pending_text(
    text_id: u32,
    #[string] asset_path: &str,
    set_up_watcher: bool,
    out_pending_text: *mut c_void,
) -> u32 {
    let asset_path = CString::new(asset_path).unwrap();
    unsafe {
        text_asset_manager::text_asset_manager_create_pending_text(
            TextId::try_from(text_id).unwrap(),
            asset_path.as_ptr(),
            set_up_watcher,
            out_pending_text.cast::<MaybeUninit<FfiPendingText>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_free_pending_text(pending_text: *mut c_void) {
    unsafe {
        text_asset_manager::text_asset_manager_free_pending_text(
            pending_text.cast::<FfiPendingText>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_free_engine_text(engine_text: *mut c_void) {
    unsafe {
        text_asset_manager::text_asset_manager_free_engine_text(
            engine_text.cast::<FfiEngineText>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_free_loaded_text(loaded_text: *mut c_void) {
    unsafe {
        text_asset_manager::text_asset_manager_free_loaded_text(
            loaded_text.cast::<FfiLoadedText>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_free_failed_text(failed_text: *mut c_void) {
    unsafe {
        text_asset_manager::text_asset_manager_free_failed_text(
            failed_text.cast::<FfiFailedText>(),
        );
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_get_text_type_by_id(
    text_asset_manager: *const c_void,
    text_id: u32,
    text_type: *mut c_void,
) -> u32 {
    unsafe {
        text_asset_manager::text_asset_manager_get_text_type_by_id(
            text_asset_manager.cast::<TextAssetManager>(),
            TextId::try_from(text_id).unwrap(),
            text_type.cast::<MaybeUninit<TextType>>(),
        ) as u32
    }
}

macro_rules! op_text_asset_manager_get_text_by_id {
    ($(($func_name:ident, $text_asset_manager_func_name:ident, $ffi_text_type:ident)),*) => {
        $(
            #[op2(fast, stack_trace)]
            pub fn $func_name(text_asset_manager: *const c_void, text_id: u32, output_text: *mut c_void) -> u32 {
                unsafe {
                    text_asset_manager::$text_asset_manager_func_name(
                        text_asset_manager.cast::<TextAssetManager>(),
                        TextId::try_from(text_id).unwrap(),
                        output_text.cast::<MaybeUninit<$ffi_text_type>>(),
                    ) as u32
                }
            }
        )*
    }
}

op_text_asset_manager_get_text_by_id!(
    (
        op_text_asset_manager_get_pending_text_by_id,
        text_asset_manager_get_pending_text_by_id,
        FfiPendingText
    ),
    (
        op_text_asset_manager_get_engine_text_by_id,
        text_asset_manager_get_engine_text_by_id,
        FfiEngineText
    ),
    (
        op_text_asset_manager_get_loaded_text_by_id,
        text_asset_manager_get_loaded_text_by_id,
        FfiLoadedText
    ),
    (
        op_text_asset_manager_get_failed_text_by_id,
        text_asset_manager_get_failed_text_by_id,
        FfiFailedText
    )
);

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_get_text_type_by_path(
    text_asset_manager: *const c_void,
    #[string] text_path: &str,
    text_type: *mut c_void,
) -> u32 {
    let text_path = CString::new(text_path).unwrap();
    unsafe {
        text_asset_manager::text_asset_manager_get_text_type_by_path(
            text_asset_manager.cast::<TextAssetManager>(),
            text_path.as_ptr(),
            text_type.cast::<MaybeUninit<TextType>>(),
        ) as u32
    }
}

macro_rules! op_text_asset_manager_get_text_by_path {
    ($(($func_name:ident, $text_asset_manager_func_name:ident, $ffi_text_type:ident)),*) => {
        $(
            #[op2(fast, stack_trace)]
            pub fn $func_name(text_asset_manager: *const c_void, #[string] text_path: &str, output_text: *mut c_void) -> u32 {
                let text_path = CString::new(text_path).unwrap();
                unsafe {
                    text_asset_manager::$text_asset_manager_func_name(
                        text_asset_manager.cast::<TextAssetManager>(),
                        text_path.as_ptr(),
                        output_text.cast::<MaybeUninit<$ffi_text_type>>(),
                    ) as u32
                }
            }
        )*
    }
}

op_text_asset_manager_get_text_by_path!(
    (
        op_text_asset_manager_get_pending_text_by_path,
        text_asset_manager_get_pending_text_by_path,
        FfiPendingText
    ),
    (
        op_text_asset_manager_get_engine_text_by_path,
        text_asset_manager_get_engine_text_by_path,
        FfiEngineText
    ),
    (
        op_text_asset_manager_get_loaded_text_by_path,
        text_asset_manager_get_loaded_text_by_path,
        FfiLoadedText
    ),
    (
        op_text_asset_manager_get_failed_text_by_path,
        text_asset_manager_get_failed_text_by_path,
        FfiFailedText
    )
);

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_are_ids_loaded(
    text_asset_manager: *const c_void,
    #[buffer] ids: &[u32],
) -> bool {
    assert_eq!(size_of::<u32>(), size_of::<TextId>());
    assert!(ids.iter().all(|id| TextId::try_from(*id).is_ok()));
    unsafe {
        text_asset_manager::text_asset_manager_are_ids_loaded(
            text_asset_manager.cast::<TextAssetManager>(),
            ids.as_ptr().cast::<TextId>(),
            ids.len(),
        )
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_is_id_loaded(text_asset_manager: *const c_void, id: u32) -> bool {
    unsafe {
        text_asset_manager::text_asset_manager_is_id_loaded(
            text_asset_manager.cast::<TextAssetManager>(),
            TextId::try_from(id).unwrap(),
        )
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_load_text(
    text_asset_manager: *mut c_void,
    new_text_event_writer_handle: *const c_void,
    #[string] text_path: &str,
    set_up_watcher: bool,
    output_pending_text: *mut c_void,
) -> u32 {
    let text_path = CString::new(text_path).unwrap();
    unsafe {
        text_asset_manager::text_asset_manager_load_text(
            text_asset_manager.cast::<TextAssetManager>(),
            new_text_event_writer_handle,
            text_path.as_ptr(),
            set_up_watcher,
            output_pending_text.cast::<MaybeUninit<FfiPendingText>>(),
        ) as u32
    }
}

#[op2(fast, stack_trace)]
pub fn op_text_asset_manager_load_text_by_pending_text(
    text_asset_manager: *mut c_void,
    new_text_event_writer_handle: *const c_void,
    output_pending_text: *mut c_void,
) -> u32 {
    unsafe {
        text_asset_manager::text_asset_manager_load_text_by_pending_text(
            text_asset_manager.cast::<TextAssetManager>(),
            new_text_event_writer_handle,
            output_pending_text.cast::<FfiPendingText>(),
        ) as u32
    }
}
