use std::{
    ffi::{CStr, c_char, c_int, c_void},
    slice,
};

use game_ecs::GpuFrameData;
use gpu_common::Gpu;
use platform::Platform;
use void_public::{ComponentId, ComponentRef, EntityId, callable::AsyncCompletionValue};

use crate::module_api;

/// Spawns an entity with the given set of `components`.
///
/// Returns the `EntityId` of the spawned entity, if it was successfully spawned.
/// An entity may fail to spawn if any of the component data is found to be
/// invalid.
///
/// # Safety
///
/// A valid slice of `&[ComponentRef]` must be able to be constructed from
/// `components` and `components_len`.
///
/// The contents of `components` objects are type-erased, and cannot be verified
/// by the engine. Callers must ensure they contain valid component data.
pub unsafe extern "C" fn spawn<P: Platform, G: GpuFrameData>(
    components: *const ComponentRef<'_>,
    components_len: usize,
) -> Option<EntityId> {
    unsafe {
        match module_api::spawn::<P, G>(slice::from_raw_parts(components, components_len)) {
            Ok(entity_id) => Some(entity_id.into()),
            Err(err) => {
                log::warn!("{err}");
                None
            }
        }
    }
}

/// Despawns an entity.
pub extern "C" fn despawn<P: Platform, G: GpuFrameData>(entity_id: EntityId) {
    module_api::despawn::<P, G>(entity_id.into());
}

/// Returns the label associated with this entity, if it exists.
///
/// If no label is associated with this entity, `null` is returned.
pub extern "C" fn entity_label<P: Platform, G: GpuFrameData>(entity_id: EntityId) -> *const c_char {
    module_api::entity_label::<P, G>(entity_id.into())
}

/// Adds the given set of `components` to an existing entity.
///
/// # Safety
///
/// A valid slice of `&[ComponentRef]` must be able to be constructed from
/// `components` and `components_len`.
///
/// The contents of `components` objects are type-erased, and cannot be verified
/// by the engine. Callers must ensure they contain valid component data.
pub unsafe extern "C" fn add_components<P: Platform, G: GpuFrameData>(
    entity_id: EntityId,
    components: *const ComponentRef<'_>,
    components_len: usize,
) {
    let res = unsafe {
        module_api::add_components::<P, G>(
            entity_id.into(),
            slice::from_raw_parts(components, components_len),
        )
    };

    if let Err(err) = res {
        log::warn!("{err}");
    }
}

/// Removes the given set of `component_ids` from an existing entity.
///
/// All component ids should be `Some`. This function takes an array of
/// `Option<ComponentId>` for safety, so that the ids can be checked for zeroes.
///
/// # Safety
///
/// A valid slice of `&[Option<ComponentId>]` must be able to be constructed
/// from `component_ids` and `component_ids_len`.
pub unsafe extern "C" fn remove_components<P: Platform, G: GpuFrameData>(
    entity_id: EntityId,
    component_ids: *const Option<ComponentId>,
    component_ids_len: usize,
) {
    // Don't trust the raw input, check for zero-valued component ids.
    if let Some(index) = unsafe {
        slice::from_raw_parts(component_ids, component_ids_len)
            .iter()
            .position(|component_id| component_id.is_none())
    } {
        log::warn!(
            "remove_components: invalid ComponentId at index '{index}', \
            has it been set via set_component_id()?"
        );
        return;
    }

    // It is now safe to assume valid component ids.
    let component_ids = unsafe { slice::from_raw_parts(component_ids.cast(), component_ids_len) };

    module_api::remove_components::<P, G>(entity_id.into(), component_ids);
}

/// Sets or clears the label associated with an entity.
///
/// If `null` is passed for `label`, the entity's label is cleared.
///
/// # Safety
///
/// If `label` is non-null, it must point to a valid C string.
pub unsafe extern "C" fn set_entity_label<P: Platform, G: GpuFrameData>(
    entity_id: EntityId,
    label: *const c_char,
) {
    let label = if label.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(label) })
    };

    module_api::set_entity_label::<P, G>(entity_id.into(), label);
}

/// Sets the parent of an entity.
///
/// If `None` (represented by `0`) is passed for `parent_id`, the entity's
/// parent is set to the world root, i.e. cleared.
pub extern "C" fn set_parent<P: Platform, G: GpuFrameData>(
    entity_id: EntityId,
    parent_id: Option<EntityId>,
    keep_world_space_transform: bool,
) {
    module_api::set_parent::<P, G>(
        entity_id.into(),
        parent_id.map(EntityId::into),
        keep_world_space_transform,
    );
}

/// Enables or disables an ECS system. The name of the system is namespaced by
/// the module name, followed by the system name, i.e. `my_module::my_system`.
///
/// # Safety
///
/// `system_name` must point to a valid C string.
pub unsafe extern "C" fn set_system_enabled<P: Platform, G: GpuFrameData>(
    system_name: *const c_char,
    enabled: bool,
) {
    let system_name = unsafe { CStr::from_ptr(system_name).to_string_lossy() };

    module_api::set_system_enabled::<P, G>(&system_name, enabled);
}

/// Looks up an entity's parent, writing the parent to `out_parent_id`. If the
/// entity has no parent, `None` (represented by `0`) will be written.
///
/// Returns `true` if `out_parent_id` has been written, or `false` if the entity
/// lookup failed and `out_parent_id` was not written to.
///
///  # Safety
///
/// `out_parent_id` must point to a 64-bit `EntityId` value.
/// It must be valid for writes and must be properly aligned.
pub unsafe extern "C" fn get_parent<P: Platform, G: GpuFrameData>(
    entity_id: EntityId,
    out_parent_id: *mut Option<EntityId>,
) -> bool {
    match module_api::get_parent::<P, G>(entity_id.into()) {
        Ok(parent_id) => {
            unsafe { out_parent_id.write(parent_id.map(EntityId::from)) };
            true
        }
        Err(err) => {
            log::warn!("{err:?}");
            false
        }
    }
}

/// Returns the number of entities captured by a query.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input.
pub unsafe extern "C" fn query_len<G: Gpu>(query: *const c_void) -> usize {
    unsafe { module_api::query_len::<G>(query) }
}

/// Gets a set of components for a given index. An index of `0` will return the
/// first entity captured by a query, `1` the second, etc.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input.
///
/// `component_ptrs` must be a pointer to an array of pointers, sized to the
/// number of components in the query.
///
/// Returns non-zero on error.
pub unsafe extern "C" fn query_get<G: Gpu>(
    query: *const c_void,
    index: usize,
    component_ptrs: *mut *const c_void,
) -> i32 {
    match unsafe { module_api::query_get::<G>(query, index, component_ptrs) } {
        true => 0,
        false => 1,
    }
}

/// Gets a set of components for a given entity id captured by the query.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input.
///
/// `component_ptrs` must be a pointer to an array of pointers, sized to the
/// number of components in the query.
///
/// Returns non-zero on error.
pub unsafe extern "C" fn query_get_entity<G: Gpu>(
    query: *const c_void,
    entity_id: EntityId,
    component_ptrs: *mut *const c_void,
) -> i32 {
    match unsafe { module_api::query_get_entity::<G>(query, entity_id.into(), component_ptrs) } {
        true => 0,
        false => 1,
    }
}

/// Gets a set of components for a given entity id captured by the query.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input.
///
/// `label` must point to a valid C string.
///
/// `component_ptrs` must be a pointer to an array of pointers, sized to the
/// number of components in the query.
///
/// Returns non-zero on error.
pub unsafe extern "C" fn query_get_label<P: Platform, G: Gpu>(
    query: *const c_void,
    label: *const c_char,
    component_ptrs: *mut *const c_void,
) -> i32 {
    match unsafe {
        module_api::query_get_label::<P, G>(query, CStr::from_ptr(label), component_ptrs)
    } {
        true => 0,
        false => 1,
    }
}

/// This function takes a function pointer, `callback`, and calls it once per
/// entity captured by the query.
///
/// `callback` must take two parameters:
/// - A pointer to an array of component pointers.
/// - The `user_data` pointer passed to `query_for_each`.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input. It must not
/// be aliased or be in use by any other thread.
pub unsafe extern "C" fn query_for_each<G: Gpu>(
    query: *mut c_void,
    callback: unsafe extern "C" fn(*mut *const c_void, *mut c_void) -> c_int,
    user_data: *mut c_void,
) {
    unsafe {
        module_api::query_for_each::<G, _, _>(query, user_data, |component_ptrs, user_data| {
            callback(component_ptrs.as_mut_ptr(), *user_data)
                .try_into()
                .expect("invalid query_for_each return code")
        });
    };
}

#[derive(Clone, Copy)]
struct QueryUserData(*const c_void);

unsafe impl Sync for QueryUserData {}

/// This function takes a function pointer, `callback`, and calls it once per
/// entity captured by the query. This version of `for_each` will iterate the
/// query across all available threads, providing potentially faster execution.
///
/// `callback` must take two parameters:
/// - A pointer to an array of component pointers.
/// - The `user_data` pointer passed to `query_for_each`.
///
/// # Safety
///
/// `query` must point to a valid query, provided as a system input. It must not
/// be aliased or be in use by any other thread.
///
/// `callback` must be able to be called across threads (i.e. `Sync`).
///
/// `user_data` must be able to be shared across threads (i.e. `Sync`).
pub unsafe extern "C" fn query_par_for_each<P: Platform, G: Gpu>(
    query: *mut c_void,
    callback: unsafe extern "C" fn(*mut *const c_void, *const c_void) -> c_int,
    user_data: *const c_void,
) {
    let user_data = QueryUserData(user_data);
    unsafe {
        module_api::query_par_for_each::<P, G, _, _>(
            query,
            user_data,
            |component_ptrs, user_data| {
                callback(component_ptrs.as_mut_ptr(), user_data.0)
                    .try_into()
                    .expect("invalid query_par_for_each return code")
            },
        );
    };
}

/// Gets the number of events in an event reader.
///
/// # Safety
///
/// `event_reader_handle` must point to a valid event reader, provided as a
/// system input.
pub unsafe extern "C" fn event_count(event_reader: *const c_void) -> usize {
    unsafe { module_api::event_count(event_reader) }
}

/// Gets the event data for a given event in an event reader.
///
/// The returned pointer points to a `u64` byte length of the event, followed
/// eight bytes later by the event data.
///
/// | Event Byte Length | Event Data           |
/// |-------------------|----------------------|
/// | 8 bytes           | variable byte length |
///
/// # Safety
///
/// `event_reader_handle` must point to a valid event reader, provided as a
/// system input.
pub unsafe extern "C" fn event_get(event_reader: *const c_void, index: usize) -> *const u64 {
    unsafe { module_api::event_get(event_reader, index) }
}

/// Sumbits an event to an event writer.
///
/// # Safety
///
/// `event_reader_handle` must point to a valid event writer, provided as a
/// system input.
///
/// A valid `&[u8]` slice must be able to be constructed from `data` and `len`,
/// and correspond to the event writer's type.
pub unsafe extern "C" fn event_send<P: Platform>(
    event_writer: *const c_void,
    data: *const u8,
    len: usize,
) {
    unsafe { module_api::event_send::<P>(event_writer, slice::from_raw_parts(data, len)) };
}

/// Dispatches a call to a function.
///
/// # Safety
///
/// A slice of `&[MaybeUninit<u8>]` be able to be constructed from
/// `parameter_data_ptr` and `parameter_data_len`. The data must be valid and
/// correspond to the callable's type.
pub unsafe extern "C" fn call<P: Platform, G: GpuFrameData>(
    function_id: ComponentId,
    parameter_data_ptr: *const c_void,
    parameter_data_size: usize,
) {
    unsafe {
        let parameter_data = slice::from_raw_parts(parameter_data_ptr.cast(), parameter_data_size);

        module_api::call::<P, G>(function_id, parameter_data);
    }
}

/// Dispatches a call to a function with an async return value.
///
/// # Safety
///
/// A slice of `&[MaybeUninit<u8>]` be able to be constructed from
/// `parameter_data_ptr` and `parameter_data_len`. The data must be valid and
/// correspond to the callable's type.
///
/// A slice of `&[MaybeUninit<u8>]` be able to be constructed from
/// `user_data_ptr` and `user_data_size`. The data must be valid and correspond
/// to the callable's type.
pub unsafe extern "C" fn call_async<P: Platform, G: GpuFrameData>(
    completion_id: ComponentId,
    parameter_data_ptr: *const c_void,
    parameter_data_size: usize,
    user_data_ptr: *const c_void,
    user_data_size: usize,
) {
    unsafe {
        let parameter_data = slice::from_raw_parts(parameter_data_ptr.cast(), parameter_data_size);
        let user_data = slice::from_raw_parts(user_data_ptr.cast(), user_data_size);

        module_api::call_async::<P, G>(completion_id, parameter_data, user_data);
    }
}

/// Gets the number of async completions for a given `completion_id`.
pub extern "C" fn completion_count<P: Platform, G: GpuFrameData>(
    completion_id: ComponentId,
) -> usize {
    module_api::completion_count::<P, G>(completion_id)
}

/// Gets the async completion for a given `completion_id` and `index`.
pub extern "C" fn completion_get<P: Platform, G: GpuFrameData>(
    completion_id: ComponentId,
    index: usize,
) -> AsyncCompletionValue {
    module_api::completion_get::<P, G>(completion_id, index)
}

/// Loads a string of JSON that represents a scene of entities into the engine
///
/// # Safety
///
/// The pointer `scene_json` must not be null and its memory must be
/// null-terminated.
pub unsafe extern "C" fn load_scene<P: Platform, G: Gpu>(scene_json: *const c_char) {
    module_api::load_scene::<P, G>(unsafe { CStr::from_ptr(scene_json) });
}
