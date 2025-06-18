use std::{
    cell::UnsafeCell,
    cmp::Ordering,
    error::Error,
    ffi::{CString, c_void},
    marker::PhantomData,
    mem::transmute,
    ops::Range,
    process::abort,
    ptr, slice,
};

use atomic_refcell::{AtomicRef, AtomicRefMut};
use event::{EventManager, EventWriterStorageMut, EventWriterStorageRef};
use game_ecs::{
    ArchetypeKey, ArchetypeStorage, ComponentRegistry, CpuDataBuffer, CpuFrameData, EcsSystem,
    EcsSystemExecuteResources, EcsTypeInfo, FrameDataBufferBorrowRef, FrameDataBufferBorrowRefMut,
    GpuFrameData, PartitionIndex, system_execute_resources,
};
use game_entity::{EntityId, ParentType};
use gpu_common::Gpu;
use platform::{EcsModule, EcsSystemFn, Executor, Platform};
use void_public::{
    ArgType, ComponentId, ComponentRef, ForEachResult, callable::AsyncCompletionValue,
    system::system_name_generator,
};

pub struct CpuSystem<P: Platform, G: Gpu> {
    /// The system name, namespaced by the module name. For example: `MyModule::my_system`.
    name: String,
    queries: Vec<Query<G>>,
    cpu_resources_ref: Vec<CpuBufferInput<Option<AtomicRef<'static, CpuDataBuffer>>>>,
    cpu_resources_mut: Vec<CpuBufferInput<Option<AtomicRefMut<'static, CpuDataBuffer>>>>,
    gpu_resources_ref: Vec<GpuBufferInputRef<G>>,
    gpu_resources_mut: Vec<GpuBufferInputMut<G>>,
    event_read_buffers: Vec<EventBufferReaderInfo>,
    event_write_buffers: Vec<EventBufferMut<P>>,
    update: Box<dyn EcsSystemFn>,
    update_data: UpdateDataSingle,
    marker: PhantomData<P>,
}

struct Query<G: Gpu> {
    components: Vec<SystemComponent>,
    archetypes: Vec<QueryArchetype<G>>,
    update_data_index: usize,
    update_data: UpdateData,
}

#[derive(Debug)]
struct QueryArchetype<G: Gpu> {
    cpu_buffer_input: CpuBufferInput<CpuBufferInputBuffer>,
    gpu_buffer_inputs_ref: Vec<GpuBufferInputRef<G>>,
    gpu_buffer_inputs_mut: Vec<GpuBufferInputMut<G>>,
}

type EventReaderHandle = *const Vec<EventWriterStorageRef<'static>>;

type EventWriterHandle<P> = *const EventWriterStorageMut<'static, P>;

impl<P: Platform, G: Gpu> CpuSystem<P, G> {
    pub fn new(
        system_index: usize,
        component_registry: &ComponentRegistry,
        event_manager: &mut EventManager<P>,
        module: &dyn EcsModule,
    ) -> Self {
        let name = system_name_generator(
            &module.module_name(),
            &module.system_name(system_index).as_ref().to_string_lossy(),
        );

        let update_data =
            UpdateDataSingle(vec![ptr::null_mut(); module.system_args_len(system_index)]);

        let mut system = Self {
            name,
            queries: Vec::new(),
            cpu_resources_ref: Vec::new(),
            cpu_resources_mut: Vec::new(),
            gpu_resources_ref: Vec::new(),
            gpu_resources_mut: Vec::new(),
            event_read_buffers: Vec::new(),
            event_write_buffers: Vec::new(),
            update: module.system_fn(system_index),
            update_data,
            marker: PhantomData,
        };

        // check for incompatible inputs

        let mut read_events = Vec::new();
        let mut write_events = Vec::new();
        for arg_index in 0..module.system_args_len(system_index) {
            let arg_type = module.system_arg_type(system_index, arg_index);

            match arg_type {
                ArgType::EventReader => {
                    read_events.push(module.system_arg_event(system_index, arg_index));
                }
                ArgType::EventWriter => {
                    write_events.push(module.system_arg_event(system_index, arg_index));
                }
                _ => {}
            }
        }

        for write_event in &write_events {
            if read_events.contains(write_event) {
                log::error!(
                    "{write_event:?}: cannot read and write the same event within a system"
                );
                panic!();
            }
        }

        // initialize system

        for arg_index in 0..module.system_args_len(system_index) {
            let arg_type = module.system_arg_type(system_index, arg_index);

            match arg_type {
                ArgType::DataAccessMut | ArgType::DataAccessRef => {
                    // resource access

                    let string_id = module.system_arg_component(system_index, arg_index);
                    let string_id = string_id.as_ref();

                    let (_, component_info) = component_registry
                        .get_with_string_id(string_id)
                        .unwrap_or_else(|| {
                            panic!("could not find component for string {string_id:?}")
                        });

                    let EcsTypeInfo::Resource(resource_info) = &component_info.ecs_type_info else {
                        panic!(
                            "invalid system parameter '{string_id:?}'. must be a resource or query"
                        );
                    };

                    if matches!(arg_type, ArgType::DataAccessRef) {
                        // resource ref
                        if component_info.gpu_compatible {
                            system.gpu_resources_ref.push(GpuBufferInputRef {
                                buffer_index: resource_info.buffer_index,
                                partition: 0,
                                component_input_info: Vec::from([ComponentInputInfo {
                                    input_buffer_offset: 0,
                                    update_data_index: arg_index,
                                }]),
                                buffer: None,
                            });
                        } else {
                            system.cpu_resources_ref.push(CpuBufferInput {
                                buffer_index: resource_info.buffer_index,
                                entity_id_buffer_offset: 0,
                                component_input_info: Vec::from([ComponentInputInfo {
                                    input_buffer_offset: 0,
                                    update_data_index: arg_index,
                                }]),
                                buffer: None,
                            });
                        }
                    } else {
                        // resource mut
                        if component_info.gpu_compatible {
                            system.gpu_resources_mut.push(GpuBufferInputMut {
                                buffer_index: resource_info.buffer_index,
                                partition: 0,
                                component_input_info: Vec::from([ComponentInputInfo {
                                    input_buffer_offset: 0,
                                    update_data_index: arg_index,
                                }]),
                                stride: 0,
                                buffer: None,
                                buffer_prev: None,
                            });
                        } else {
                            system.cpu_resources_mut.push(CpuBufferInput {
                                buffer_index: resource_info.buffer_index,
                                entity_id_buffer_offset: 0,
                                component_input_info: Vec::from([ComponentInputInfo {
                                    input_buffer_offset: 0,
                                    update_data_index: arg_index,
                                }]),
                                buffer: None,
                            });
                        }
                    }
                }
                ArgType::EventReader => {
                    let event_type = module.system_arg_event(system_index, arg_index);

                    system.event_read_buffers.push(EventBufferReaderInfo {
                        update_data_index: arg_index,
                        event_type: event_type.into_owned(),
                        borrows: Vec::new(),
                    });
                }
                ArgType::EventWriter => {
                    let event_ident = module.system_arg_event(system_index, arg_index);
                    event_manager.register_module_event_writer(event_ident.as_ref(), &system.name);

                    system.event_write_buffers.push(EventBufferMut {
                        event_ident: event_ident.into_owned(),
                        update_data_index: arg_index,
                        borrow: None,
                    });
                }
                ArgType::Query => {
                    let mut query_components = Vec::new();

                    let query_arg_len = module.system_query_args_len(system_index, arg_index);

                    for query_arg_index in 0..query_arg_len {
                        let arg_type =
                            module.system_query_arg_type(system_index, arg_index, query_arg_index);

                        if !matches!(arg_type, ArgType::DataAccessRef | ArgType::DataAccessMut) {
                            log::error!("queries may only access components");
                            abort();
                        };

                        let string_id = module.system_query_arg_component(
                            system_index,
                            arg_index,
                            query_arg_index,
                        );
                        let string_id = string_id.as_ref();

                        let Some((component_id, component_info)) =
                            component_registry.get_with_string_id(string_id)
                        else {
                            panic!("component {string_id:?} is not registered");
                        };

                        if matches!(component_info.ecs_type_info, EcsTypeInfo::Resource(_)) {
                            log::error!("queries may not access resources");
                            abort();
                        };

                        if !component_info.is_freely_mutable
                            && matches!(arg_type, ArgType::DataAccessMut)
                        {
                            log::error!(
                                "mutable read-only components not yet supported: {string_id:?}"
                            );
                            abort();
                        }

                        query_components.push(SystemComponent {
                            id: component_id,
                            update_data_index: query_arg_index,
                            mutable: matches!(arg_type, ArgType::DataAccessMut),
                        });
                    }

                    query_components.sort_unstable();

                    let update_data = UpdateData(
                        (0..P::Executor::available_parallelism().get())
                            .map(|_| UnsafeCell::new(vec![ptr::null(); query_arg_len]))
                            .collect(),
                    );

                    system.queries.push(Query {
                        components: query_components,
                        archetypes: Default::default(),
                        update_data_index: arg_index,
                        update_data,
                    });
                }
                ArgType::Completion => {
                    // for now we do nothing (completion API functions take the completion id)
                }
            }
        }

        system
    }
}

impl<P: Platform, G: Gpu> EcsSystem<P, G> for CpuSystem<P, G> {
    fn name(&self) -> &str {
        &self.name
    }

    fn add_archetype_input(&mut self, archetype_key: &ArchetypeKey, storage: &ArchetypeStorage) {
        for query in &mut self.queries {
            query.add_archetype_input(archetype_key, storage);
        }
    }

    fn clear_archetype_inputs(&mut self) {
        for query in &mut self.queries {
            query.clear_archetype_inputs();
        }
    }

    fn execute(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.lock_buffers();

        self.assign_resource_ptrs();

        self.assign_event_ptrs();

        // Assign query pointers.

        for (i, update_data_index) in self.queries.iter().map(|q| q.update_data_index).enumerate() {
            unsafe {
                let ptr = self.queries.as_ptr().add(i).cast();
                self.update_data.0[update_data_index] = ptr;
            }
        }

        // Execute system.

        let res = unsafe { self.update.call(self.update_data.0.as_mut_ptr()) };

        self.unlock_buffers();

        res
    }

    fn clear_event_writer_buffers(&mut self) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            for input in &self.event_write_buffers {
                let storage = resources
                    .event_manager
                    .module_event_storage(&input.event_ident, &self.name)
                    .expect("error getting module event writer");

                storage.borrow_mut().clear();
            }
        });
    }
}

impl<P: Platform, G: Gpu> CpuSystem<P, G> {
    fn lock_buffers(&mut self) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            let cpu_data = resources.cpu_data;
            let gpu_data = resources.gpu_data;

            for query in &mut self.queries {
                for buffer in &mut query.archetypes {
                    buffer.lock_buffers(cpu_data, gpu_data);
                }
            }

            for input in &mut self.cpu_resources_ref {
                // SAFETY: we drop the buffer at the end of the execute function
                let buffer = unsafe {
                    transmute::<AtomicRef<'_, CpuDataBuffer>, AtomicRef<'static, CpuDataBuffer>>(
                        cpu_data.borrow_buffer(input.buffer_index),
                    )
                };
                input.buffer = Some(buffer);
            }

            for input in &mut self.cpu_resources_mut {
                // SAFETY: we drop the buffer at the end of the execute function
                let buffer = unsafe {
                    transmute::<AtomicRefMut<'_, CpuDataBuffer>, AtomicRefMut<'static, CpuDataBuffer>>(
                        cpu_data.borrow_buffer_mut(input.buffer_index),
                    )
                };
                input.buffer = Some(buffer);
            }

            for input in &mut self.gpu_resources_ref {
                let buffer = gpu_data.borrow_buffer(input.buffer_index, input.partition);
                input.buffer = Some(buffer);
            }

            for input in &mut self.gpu_resources_mut {
                let buffer = gpu_data.borrow_buffer_mut(input.buffer_index, input.partition);
                input.buffer = Some(buffer);
            }

            for input in &mut self.event_read_buffers {
                resources
                    .event_manager
                    .event_storage(&input.event_type, |storage| unsafe {
                        // SAFETY: we drop the borrow at the end of the execute function
                        let borrow = transmute::<
                            EventWriterStorageRef<'_>,
                            EventWriterStorageRef<'static>,
                        >(storage.borrow());

                        input.borrows.push(borrow);
                    });
            }

            for input in &mut self.event_write_buffers {
                let storage = resources
                    .event_manager
                    .module_event_storage(&input.event_ident, &self.name)
                    .expect("error getting module event writer");

                let borrow = storage.borrow_mut();
                // SAFETY: we drop the borrow at the end of the execute function
                let mut borrow = unsafe {
                    transmute::<EventWriterStorageMut<'_, P>, EventWriterStorageMut<'static, P>>(
                        borrow,
                    )
                };

                // clear old events from the buffer
                borrow.clear();

                input.borrow = Some(borrow);
            }
        });
    }

    fn unlock_buffers(&mut self) {
        for query in &mut self.queries {
            for buffer in &mut query.archetypes {
                buffer.unlock_buffers();
            }
        }

        for input in &mut self.cpu_resources_ref {
            input.buffer = None;
        }

        for input in &mut self.cpu_resources_mut {
            input.buffer = None;
        }

        for input in &mut self.gpu_resources_ref {
            input.buffer = None;
        }

        for input in &mut self.gpu_resources_mut {
            input.buffer = None;
        }

        for input in &mut self.event_read_buffers {
            input.borrows.clear();
        }

        for input in &mut self.event_write_buffers {
            input.borrow = None;
        }
    }

    fn assign_resource_ptrs(&mut self) {
        for input in &self.cpu_resources_ref {
            let base_ptr = input.buffer.as_ref().unwrap().get_ptr(0);

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                self.update_data.0[component.update_data_index] = ptr.cast();
            }
        }

        for input in &mut self.cpu_resources_mut {
            let base_ptr = input.buffer.as_mut().unwrap().get_mut_ptr(0);

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                self.update_data.0[component.update_data_index] = ptr.cast();
            }
        }

        for input in &self.gpu_resources_ref {
            let base_ptr = input.buffer.as_ref().unwrap().get_ptr(0);

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                self.update_data.0[component.update_data_index] = ptr.cast();
            }
        }

        for input in &mut self.gpu_resources_mut {
            let base_ptr = input.buffer.as_mut().unwrap().get_mut_ptr(0);

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                self.update_data.0[component.update_data_index] = ptr.cast();
            }
        }
    }

    fn assign_event_ptrs(&mut self) {
        for input in &self.event_read_buffers {
            let ptr = &input.borrows as EventReaderHandle;
            self.update_data.0[input.update_data_index] = ptr.cast();
        }

        for input in &self.event_write_buffers {
            let ptr = input.borrow.as_ref().unwrap() as EventWriterHandle<P>;
            self.update_data.0[input.update_data_index] = ptr.cast();
        }
    }
}

impl<G: Gpu> Query<G> {
    fn add_archetype_input(&mut self, archetype_key: &ArchetypeKey, storage: &ArchetypeStorage) {
        use void_public::{EcsType, EntityId};

        // check if this archetype fits this system
        if self.components.is_empty()
            || !self
                .components
                .iter()
                .filter(|component| component.id != EntityId::id())
                .all(|component| archetype_key.contains(&component.id))
        {
            return;
        }

        // CPU buffer

        let cpu_buffer_input = {
            let entity_id_buffer_offset = storage
                .cpu
                .components
                .iter()
                .find(|component_offset_info| component_offset_info.component_id == EntityId::id())
                .unwrap()
                .offset;

            let mut mutable = false;

            let component_input_info: Vec<_> = self
                .components
                .iter()
                // only include components contained in the CPU buffer
                .filter(|component| {
                    storage.cpu.components.iter().any(|component_offset_info| {
                        component.id == component_offset_info.component_id
                    })
                })
                .map(|component| {
                    mutable |= component.mutable;

                    let input_buffer_offset = storage
                        .cpu
                        .components
                        .iter()
                        .find(|component_offset_info| {
                            component.id == component_offset_info.component_id
                        })
                        .map(|coi| coi.offset)
                        .unwrap();

                    ComponentInputInfo {
                        input_buffer_offset,
                        update_data_index: component.update_data_index,
                    }
                })
                .collect();

            let buffer = if mutable {
                CpuBufferInputBuffer::Mut(None)
            } else {
                CpuBufferInputBuffer::Ref(None)
            };

            CpuBufferInput {
                buffer_index: storage.cpu.buffer_index,
                entity_id_buffer_offset,
                component_input_info,
                buffer,
            }
        };

        // GPU buffers

        let mut gpu_buffer_inputs_ref = Vec::new();
        let mut gpu_buffer_inputs_mut = Vec::new();

        let required_storages_gpu = storage.gpu.iter().filter(|storage_gpu| {
            storage_gpu.components.iter().any(|component_offset_info| {
                self.components
                    .iter()
                    .any(|component| component.id == component_offset_info.component_id)
            })
        });

        for storage_gpu in required_storages_gpu {
            let mut mutable = false;

            let component_input_info = self
                .components
                .iter()
                // only include components contained in this GPU buffer
                .filter(|component| {
                    storage_gpu.components.iter().any(|component_offset_info| {
                        component.id == component_offset_info.component_id
                    })
                })
                .map(|component| {
                    mutable |= component.mutable;

                    let input_buffer_offset = storage_gpu
                        .components
                        .iter()
                        .find(|component_offset_info| {
                            component.id == component_offset_info.component_id
                        })
                        .map(|coi| coi.offset)
                        .unwrap();

                    ComponentInputInfo {
                        input_buffer_offset,
                        update_data_index: component.update_data_index,
                    }
                })
                .collect();

            if mutable {
                gpu_buffer_inputs_mut.push(GpuBufferInputMut {
                    buffer_index: storage_gpu.buffer_index,
                    partition: storage_gpu.partition,
                    component_input_info,
                    stride: storage_gpu.stride,
                    buffer: None,
                    buffer_prev: None,
                });
            } else {
                gpu_buffer_inputs_ref.push(GpuBufferInputRef {
                    buffer_index: storage_gpu.buffer_index,
                    partition: storage_gpu.partition,
                    component_input_info,
                    buffer: None,
                });
            }
        }

        self.archetypes.push(QueryArchetype {
            cpu_buffer_input,
            gpu_buffer_inputs_ref,
            gpu_buffer_inputs_mut,
        });
    }

    fn clear_archetype_inputs(&mut self) {
        self.archetypes.clear();
    }
}

impl<G: Gpu> QueryArchetype<G> {
    fn entity_count(&self) -> usize {
        match &self.cpu_buffer_input.buffer {
            CpuBufferInputBuffer::Ref(buffer) => buffer.as_ref().unwrap().len(),
            CpuBufferInputBuffer::Mut(buffer) => buffer.as_ref().unwrap().len(),
        }
    }

    /// Returns `true` on success, or `false` if `entity_index` is out-of-bounds.
    fn write_ptrs(&self, update_data: &mut [*const c_void], entity_index: usize) -> bool {
        let cpu_buffer = match &self.cpu_buffer_input.buffer {
            CpuBufferInputBuffer::Ref(buffer) => &**buffer.as_ref().unwrap(),
            CpuBufferInputBuffer::Mut(buffer) => &**buffer.as_ref().unwrap(),
        };

        if entity_index >= cpu_buffer.len() {
            return false;
        }

        let base_ptr = cpu_buffer.get_ptr(entity_index);

        for component in &self.cpu_buffer_input.component_input_info {
            let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
            update_data[component.update_data_index] = ptr.cast();
        }

        for input in &self.gpu_buffer_inputs_ref {
            let base_ptr = input.buffer.as_ref().unwrap().get_ptr(entity_index);

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                update_data[component.update_data_index] = ptr.cast();
            }
        }

        for input in &self.gpu_buffer_inputs_mut {
            let base_ptr = input.buffer.as_ref().unwrap().get_ptr(entity_index);

            if G::MULTI_BUFFERED {
                // copy prev frame data
                if let Some(buffer_prev) = &input.buffer_prev {
                    let prev_ptr = buffer_prev.get_ptr(entity_index);

                    unsafe {
                        ptr::copy_nonoverlapping(prev_ptr, base_ptr as *mut _, input.stride);
                    }
                }
            }

            for component in &input.component_input_info {
                let ptr = unsafe { base_ptr.add(component.input_buffer_offset) };
                update_data[component.update_data_index] = ptr.cast();
            }
        }

        true
    }

    /// Copies previous frame data to the current frame for the entire archetype. It may not occur
    /// naturally during frame processing, but we must copy previous frame changes in any case.
    fn copy_prev_frame_changes(&mut self, entity_range: Range<usize>) {
        if G::MULTI_BUFFERED {
            for input in &mut self.gpu_buffer_inputs_mut {
                if let Some(buffer_prev) = &input.buffer_prev {
                    for i in entity_range.clone() {
                        let base_ptr = input.buffer.as_ref().unwrap().get_ptr(i);
                        let prev_ptr = buffer_prev.get_ptr(i);

                        unsafe {
                            ptr::copy_nonoverlapping(prev_ptr, base_ptr as *mut _, input.stride);
                        }
                    }

                    input.buffer_prev = None;
                }
            }
        }
    }

    fn lock_buffers(&mut self, cpu_data: &CpuFrameData, gpu_data: &G) {
        match &mut self.cpu_buffer_input.buffer {
            CpuBufferInputBuffer::Ref(buffer) => {
                *buffer = unsafe {
                    transmute::<
                        Option<AtomicRef<'_, CpuDataBuffer>>,
                        Option<AtomicRef<'_, CpuDataBuffer>>,
                    >(Some(
                        cpu_data.borrow_buffer(self.cpu_buffer_input.buffer_index),
                    ))
                };
            }
            CpuBufferInputBuffer::Mut(buffer) => {
                *buffer = unsafe {
                    transmute::<
                        Option<AtomicRefMut<'_, CpuDataBuffer>>,
                        Option<AtomicRefMut<'_, CpuDataBuffer>>,
                    >(Some(
                        cpu_data.borrow_buffer_mut(self.cpu_buffer_input.buffer_index),
                    ))
                };
            }
        };

        for input in &mut self.gpu_buffer_inputs_ref {
            let buffer = gpu_data.borrow_buffer(input.buffer_index, input.partition);
            input.buffer = Some(buffer);
        }

        for input in &mut self.gpu_buffer_inputs_mut {
            let mut buffer = gpu_data.borrow_buffer_mut(input.buffer_index, input.partition);

            if G::MULTI_BUFFERED && !buffer.has_been_copied_this_frame() {
                buffer.mark_has_been_copied_this_frame();

                let buffer = gpu_data.borrow_buffer_prev(input.buffer_index, input.partition);
                input.buffer_prev = Some(buffer);
            }

            input.buffer = Some(buffer);
        }
    }

    fn unlock_buffers(&mut self) {
        let entity_count = self.entity_count();

        match &mut self.cpu_buffer_input.buffer {
            CpuBufferInputBuffer::Ref(buffer) => {
                *buffer = None;
            }
            CpuBufferInputBuffer::Mut(buffer) => {
                *buffer = None;
            }
        };

        for input in &mut self.gpu_buffer_inputs_ref {
            input.buffer = None;
        }

        // it's possible that user code did not trigger multi-buffered data copying
        self.copy_prev_frame_changes(0..entity_count);

        for input in &mut self.gpu_buffer_inputs_mut {
            input.buffer = None;
            input.buffer_prev = None;
        }
    }
}

struct UpdateDataSingle(Vec<*const c_void>);

unsafe impl Send for UpdateDataSingle {}
unsafe impl Sync for UpdateDataSingle {}

struct UpdateData(Vec<UnsafeCell<Vec<*const c_void>>>);

unsafe impl Send for UpdateData {}
unsafe impl Sync for UpdateData {}

impl UpdateData {
    unsafe fn borrow_mut(&self, index: usize) -> *mut Vec<*const c_void> {
        self.0[index].get()
    }
}

#[derive(Debug)]
struct SystemComponent {
    id: ComponentId,
    update_data_index: usize,
    mutable: bool,
}

impl PartialEq for SystemComponent {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for SystemComponent {}

impl PartialOrd for SystemComponent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SystemComponent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

#[derive(Debug)]
struct CpuBufferInput<T> {
    buffer_index: usize,
    entity_id_buffer_offset: usize,
    component_input_info: Vec<ComponentInputInfo>,
    buffer: T,
}

#[derive(Debug)]
enum CpuBufferInputBuffer {
    Ref(Option<AtomicRef<'static, CpuDataBuffer>>),
    Mut(Option<AtomicRefMut<'static, CpuDataBuffer>>),
}

#[derive(Debug)]
struct GpuBufferInputRef<G: GpuFrameData> {
    buffer_index: usize,
    partition: PartitionIndex,
    component_input_info: Vec<ComponentInputInfo>,
    buffer: Option<G::FrameDataBufferBorrowRef>,
}

#[derive(Debug)]
struct GpuBufferInputMut<G: GpuFrameData> {
    buffer_index: usize,
    partition: PartitionIndex,
    component_input_info: Vec<ComponentInputInfo>,
    stride: usize,
    buffer: Option<G::FrameDataBufferBorrowRefMut>,
    buffer_prev: Option<G::FrameDataBufferBorrowRef>,
}

#[derive(Debug)]
struct ComponentInputInfo {
    input_buffer_offset: usize,
    update_data_index: usize,
}

struct EventBufferReaderInfo {
    update_data_index: usize,
    event_type: CString,
    borrows: Vec<EventWriterStorageRef<'static>>,
}

struct EventBufferMut<P: Platform> {
    event_ident: CString,
    update_data_index: usize,
    borrow: Option<EventWriterStorageMut<'static, P>>,
}

/// This mod contains all of the functions exported by the engine which are
/// callable from an `EcsModule`. These functions must only be called from
/// within a system in an `EcsModule`. If they are called outside of a system in
/// an `EcsModule`, these functions will panic.
pub mod module_api {
    use std::{
        error::Error,
        ffi::{CStr, c_char},
        mem::MaybeUninit,
    };

    use game_ecs::add_components_helper;

    use super::*;

    pub fn load_scene<P: Platform, G: GpuFrameData>(scene_str: &CStr) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources.event_manager.command_load_scene(scene_str);
        });
    }

    /// # Safety
    ///
    /// The contents of `ComponentRef` objects are type-erased, and must be valid.
    pub unsafe fn spawn<P: Platform, G: GpuFrameData>(
        components: &[ComponentRef<'_>],
    ) -> Result<EntityId, Box<dyn Error + Send + Sync>> {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            add_components_helper(
                components,
                resources.component_bundles,
                |components_len, closure| {
                    let entity_id = resources.world_delegate.allocate_entity_id();

                    resources
                        .event_manager
                        .command_spawn(entity_id, components_len, closure);

                    entity_id
                },
            )
            .map_err(|err| {
                let mut msg = err.to_string();
                msg.insert_str(0, "spawn(): ");
                msg.into()
            })
        })
    }

    pub fn despawn<P: Platform, G: GpuFrameData>(entity_id: EntityId) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources.event_manager.command_despawn(entity_id);
        });
    }

    /// # Safety
    ///
    /// The contents of `ComponentRef` objects are type-erased, and must be valid.
    pub unsafe fn add_components<P: Platform, G: GpuFrameData>(
        entity_id: EntityId,
        components: &[ComponentRef<'_>],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            add_components_helper(
                components,
                resources.component_bundles,
                |components_len, closure| {
                    resources.event_manager.command_add_components(
                        entity_id,
                        components_len,
                        closure,
                    );
                },
            )
            .map_err(|err| {
                let mut msg = err.to_string();
                msg.insert_str(0, "add_components(): ");
                msg.into()
            })
        })
    }

    pub fn remove_components<P: Platform, G: GpuFrameData>(
        entity_id: EntityId,
        component_ids: &[ComponentId],
    ) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources
                .event_manager
                .command_remove_components(entity_id, component_ids);
        });
    }

    pub fn entity_label<P: Platform, G: GpuFrameData>(entity_id: EntityId) -> *const c_char {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources
                .world_delegate
                .entity_label(entity_id)
                .map_or(ptr::null(), |label| label.as_ptr())
        })
    }

    pub fn set_entity_label<P: Platform, G: GpuFrameData>(
        entity_id: EntityId,
        label: Option<&CStr>,
    ) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources
                .event_manager
                .command_set_entity_label(entity_id, label);
        });
    }

    pub fn set_parent<P: Platform, G: GpuFrameData>(
        entity_id: EntityId,
        parent_id: Option<EntityId>,
        keep_world_space_transform: bool,
    ) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources.event_manager.command_set_parent(
                entity_id,
                parent_id,
                keep_world_space_transform,
            );
        });
    }

    pub fn get_parent<P: Platform, G: GpuFrameData>(
        entity_id: EntityId,
    ) -> Result<Option<EntityId>, Box<dyn Error + Send + Sync>> {
        system_execute_resources(
            |resources: &EcsSystemExecuteResources<'_, P, G>| match resources
                .world_delegate
                .get_parent_type(entity_id)
            {
                Some(parent_data) => match parent_data {
                    ParentType::Parent(parent_id) => Ok(Some(parent_id)),
                    ParentType::Root => Ok(None),
                },
                None => Err("get_parent: entity does not exist".into()),
            },
        )
    }

    /// # Safety
    ///
    /// `query` must point to a valid `Query`.
    pub unsafe fn query_len<G: Gpu>(query: *const c_void) -> usize {
        let query = unsafe { query.cast::<Query<G>>().as_ref().unwrap() };

        query
            .archetypes
            .iter()
            .map(|archetype| archetype.entity_count())
            .sum()
    }

    /// Returns `true` on success.
    ///
    ///  # Safety
    ///
    /// `query` must point to a valid `Query`.
    ///
    /// `component_ptrs` must be a pointer to an array of pointers, sized to the
    /// number of components in the query. `component_ptrs` should be assumed to
    /// be uninitialized if the function returns `false`.
    pub unsafe fn query_get<G: Gpu>(
        query: *const c_void,
        mut index: usize,
        component_ptrs: *mut *const c_void,
    ) -> bool {
        let query = unsafe { query.cast::<Query<G>>().as_ref().unwrap() };
        let component_ptrs =
            unsafe { slice::from_raw_parts_mut(component_ptrs, query.components.len()) };

        for archetype in &query.archetypes {
            if archetype.write_ptrs(component_ptrs, index) {
                return true;
            }

            if let Some(i) = index.checked_sub(archetype.entity_count()) {
                index = i;
            } else {
                break;
            }
        }

        false
    }

    /// Returns `true` on success.
    ///
    ///  # Safety
    ///
    /// `query` must point to a valid `Query`.
    ///
    /// `component_ptrs` must be a pointer to an array of pointers, sized to the
    /// number of components in the query. `component_ptrs` should be assumed to
    /// be uninitialized if the function returns `false`.
    pub unsafe fn query_get_entity<G: Gpu>(
        query: *const c_void,
        entity_id: EntityId,
        component_ptrs: *mut *const c_void,
    ) -> bool {
        let query = unsafe { query.cast::<Query<G>>().as_ref().unwrap() };
        let component_ptrs =
            unsafe { slice::from_raw_parts_mut(component_ptrs, query.components.len()) };

        for archetype in &query.archetypes {
            let cpu_buffer = match &archetype.cpu_buffer_input.buffer {
                CpuBufferInputBuffer::Ref(buffer) => &**buffer.as_ref().unwrap(),
                CpuBufferInputBuffer::Mut(buffer) => &**buffer.as_ref().unwrap(),
            };

            if let Some(entity_index) = (0..archetype.entity_count()).find(|i| {
                entity_id
                    == unsafe {
                        *cpu_buffer
                            .get_with_offset_as::<EntityId>(
                                *i,
                                archetype.cpu_buffer_input.entity_id_buffer_offset,
                            )
                            .unwrap()
                    }
            }) {
                let res = archetype.write_ptrs(component_ptrs, entity_index);
                assert!(res); // should be guaranteed by the index lookup
                return true;
            };
        }

        false
    }

    /// Returns `true` on success.
    ///
    ///  # Safety
    ///
    /// `query` must point to a valid `Query`.
    ///
    /// `component_ptrs` must be a pointer to an array of pointers, sized to the
    /// number of components in the query. `component_ptrs` should be assumed to
    /// be uninitialized if the function returns `false`.
    pub unsafe fn query_get_label<P: Platform, G: Gpu>(
        query: *const c_void,
        label: &CStr,
        component_ptrs: *mut *const c_void,
    ) -> bool {
        let query = unsafe { query.cast::<Query<G>>().as_ref().unwrap() };
        let component_ptrs =
            unsafe { slice::from_raw_parts_mut(component_ptrs, query.components.len()) };

        let Some(entity_id) =
            system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
                resources.world_delegate.label_entity(label)
            })
        else {
            return false;
        };

        for archetype in &query.archetypes {
            let cpu_buffer = match &archetype.cpu_buffer_input.buffer {
                CpuBufferInputBuffer::Ref(buffer) => &**buffer.as_ref().unwrap(),
                CpuBufferInputBuffer::Mut(buffer) => &**buffer.as_ref().unwrap(),
            };

            if let Some(entity_index) = (0..archetype.entity_count()).find(|i| {
                entity_id
                    == unsafe {
                        *cpu_buffer
                            .get_with_offset_as::<EntityId>(
                                *i,
                                archetype.cpu_buffer_input.entity_id_buffer_offset,
                            )
                            .unwrap()
                    }
            }) {
                let res = archetype.write_ptrs(component_ptrs, entity_index);
                assert!(res); // should be guaranteed by the index lookup
                return true;
            };
        }

        false
    }

    /// # Safety
    ///
    /// `query` must point to a valid `Query`. It must not be aliased or be in use by any other thread.
    pub unsafe fn query_for_each<G: Gpu, F, U>(query: *mut c_void, mut user_data: U, mut closure: F)
    where
        F: FnMut(&mut [*const c_void], &mut U) -> ForEachResult,
    {
        let query = unsafe { query.cast::<Query<G>>().as_mut().unwrap() };
        let update_data = unsafe { &mut *query.update_data.borrow_mut(0) };

        for archetype in &mut query.archetypes {
            for i in 0..archetype.entity_count() {
                archetype.write_ptrs(update_data, i);

                match closure(update_data, &mut user_data) {
                    ForEachResult::Continue => {
                        continue;
                    }
                    ForEachResult::Break => {
                        // copy the rest of the archetype
                        archetype.copy_prev_frame_changes(i + 1..archetype.entity_count());
                        return;
                    }
                    ForEachResult::Error => {
                        panic!("query for_each panic");
                    }
                }
            }

            for input in &mut archetype.gpu_buffer_inputs_mut {
                input.buffer_prev = None;
            }
        }
    }

    /// # Safety
    ///
    /// `query` must point to a valid `Query`. It must not be aliased or be in use by any other thread.
    pub unsafe fn query_par_for_each<P: Platform, G: Gpu, F, U>(
        query: *mut c_void,
        user_data: U,
        closure: F,
    ) where
        F: Fn(&mut [*const c_void], &U) -> ForEachResult + Sync,
        U: Sync,
    {
        let query = unsafe { query.cast::<Query<G>>().as_mut().unwrap() };

        for archetype in &mut query.archetypes {
            const BLOCK_SIZE: usize = 256;

            let parallelism = archetype.entity_count().next_multiple_of(BLOCK_SIZE) / BLOCK_SIZE;

            P::Executor::parallel_iter(parallelism, |i, thread_index| {
                let start_index = i * archetype.entity_count() / parallelism;
                let end_index = (i + 1) * archetype.entity_count() / parallelism;

                let update_data = unsafe { &mut *query.update_data.borrow_mut(thread_index) };

                for entity_index in start_index..end_index {
                    archetype.write_ptrs(update_data, entity_index);

                    let res = closure(update_data, &user_data);
                    assert_eq!(
                        res,
                        ForEachResult::Continue,
                        "query_par_for_each did not return `Continue`"
                    );
                }
            });

            for input in &mut archetype.gpu_buffer_inputs_mut {
                input.buffer_prev = None;
            }
        }
    }

    /// # Safety
    ///
    /// `event_reader_handle` must point to a valid `EventReaderHandle`.
    pub unsafe fn event_count(event_reader_handle: *const c_void) -> usize {
        let event_buffers = unsafe { (event_reader_handle as EventReaderHandle).as_ref().unwrap() };

        event_buffers.iter().map(|buffer| buffer.count()).sum()
    }

    /// # Safety
    ///
    /// `event_reader_handle` must point to a valid `EventReaderHandle`.
    pub unsafe fn event_get(event_reader_handle: *const c_void, index: usize) -> *const u64 {
        let event_buffers = unsafe { (event_reader_handle as EventReaderHandle).as_ref().unwrap() };

        let mut i = 0;

        for buffer in event_buffers {
            let count = buffer.count();

            if index - i < count {
                return buffer.read_event(index - i);
            }

            i += count;
        }

        ptr::null()
    }

    /// # Safety
    ///
    /// `event_writer_handle` must point to a valid `EventWriterHandle`.
    ///
    /// Event data in `data` must be valid and correspond to the event writer's type.
    pub unsafe fn event_send<P: Platform>(event_writer_handle: *const c_void, data: &[u8]) {
        unsafe {
            let event_writer = (event_writer_handle as EventWriterHandle<P>)
                .as_ref()
                .unwrap();

            event_writer.write_module_event(data);
        };
    }

    /// # Safety
    ///
    /// Event data in `parameter_data` must be valid and correspond to the callable's type.
    pub unsafe fn call<P: Platform, G: GpuFrameData>(
        function_id: ComponentId,
        parameter_data: &[MaybeUninit<u8>],
    ) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources.callables.call(function_id, parameter_data);
        });
    }

    /// # Safety
    ///
    /// Event data in `parameter_data` must be valid and correspond to the callable's type.
    ///
    /// Event data in `user_data` must be valid and correspond to the callable's type.
    pub unsafe fn call_async<P: Platform, G: GpuFrameData>(
        completion_id: ComponentId,
        parameter_data: &[MaybeUninit<u8>],
        user_data: &[MaybeUninit<u8>],
    ) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            let function_id = match &resources.component_registry[&completion_id].ecs_type_info {
                EcsTypeInfo::AsyncCompletion(info) => info.callable_id,
                _ => unreachable!(),
            };

            resources.callables.call_async(
                function_id,
                completion_id,
                parameter_data,
                user_data.into(),
            );
        });
    }

    pub fn completion_count<P: Platform, G: GpuFrameData>(completion_id: ComponentId) -> usize {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources.callables.completions(completion_id).len()
        })
    }

    pub fn completion_get<P: Platform, G: GpuFrameData>(
        completion_id: ComponentId,
        index: usize,
    ) -> AsyncCompletionValue {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources
                .callables
                .completions(completion_id)
                .get(index)
                .map_or(AsyncCompletionValue::null(), |task| AsyncCompletionValue {
                    return_value_ptr: task.return_value.as_ptr(),
                    return_value_size: task.return_value.len(),
                    user_data_ptr: task.user_data.as_ptr(),
                    user_data_size: task.user_data.len(),
                })
        })
    }

    pub fn set_system_enabled<P: Platform, G: GpuFrameData>(system_name: &str, enabled: bool) {
        system_execute_resources(|resources: &EcsSystemExecuteResources<'_, P, G>| {
            resources
                .event_manager
                .command_set_system_enabled(system_name, enabled);
        });
    }
}
