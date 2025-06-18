use std::{
    collections::HashMap,
    error::Error,
    ffi::{CStr, CString},
    mem::{ManuallyDrop, MaybeUninit},
    pin::pin,
    ptr::null_mut,
};

pub use cpu_system::module_api;
pub use event;
use event::{EventManager, PlatformEventDelegate, platform_event_iter};
use frame_update::FrameUpdate;
use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::texture_asset_manager::{FormatType, LoadedTexture, TextureAssetManager},
};
pub use game_ecs;
use game_ecs::{
    CallableInfo, Callables, ComponentInfo, ComponentRegistry, CpuFrameData, EcsTypeInfo,
    FrameDataBufferBorrowRefMut,
};
use game_entity::EntityId;
pub use game_input_manager;
use game_input_manager::InputManager;
use gpu_common::Gpu;
use gpu_web::ecs_module::CameraRenderResource;
pub use platform;
use platform::{EcsModule, Executor, Platform, PlatformLibrary};
pub use void_public;
use void_public::{
    Aspect, ComponentId, FrameConfig, api_version_compatible, api_version_major, api_version_minor,
    api_version_patch, callable::TaskId, event::input::WindowResized, graphics::TextureId,
    input::InputState,
};
pub use void_public_module;

pub mod c_api;
mod cpu_system;
mod frame_update;
pub mod include_module_macro;
mod transforms_update;

pub struct GameEngine<P: Platform, G: Gpu> {
    executor: P::Executor,
    event_manager: EventManager<P>,
    input_manager: InputManager,
    cpu_data: CpuFrameData,
    component_registry: ComponentRegistry,
    frame_update: FrameUpdate<P, G>,
    ecs_modules: HashMap<String, Box<dyn EcsModule>>,
    platform_libraries: Vec<Box<dyn PlatformLibrary>>,
    callables: Callables,
    gpu: ManuallyDrop<G>,
}

impl<P: Platform, G: Gpu> GameEngine<P, G> {
    pub fn new(executor: P::Executor, width: u32, height: u32, gpu: G) -> Self {
        let mut engine = Self {
            executor,
            event_manager: EventManager::default(),
            input_manager: Default::default(),
            cpu_data: Default::default(),
            component_registry: Default::default(),
            frame_update: Default::default(),
            ecs_modules: Default::default(),
            platform_libraries: Default::default(),
            callables: Default::default(),
            gpu: ManuallyDrop::new(gpu),
        };

        // Register statically-linked ECS modules which don't depend on `GpuWeb`.
        unsafe {
            include_module!(
                engine,
                void_public_module::ffi,
                VoidPublicModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );

            include_module!(
                engine,
                game_asset::ecs_module::ffi,
                GameAssetModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );
            include_module!(
                engine,
                ipc::systems::ffi,
                IpcHostModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );
            include_module!(
                engine,
                physics::systems::ffi,
                PhysicsModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );
        }

        // Initialize `Aspect` resource.
        engine
            .cpu_data
            .get_resource_mut(&engine.component_registry, |aspect: &mut Aspect| {
                aspect.width = width as f32;
                aspect.height = height as f32;
            });

        // GPU requires special ECS type registration.
        engine
            .gpu
            .register_components(&mut engine.component_registry);
        engine
            .gpu
            .register_resources(&mut engine.cpu_data, &mut engine.component_registry);
        engine.register_ecs_module(engine.gpu.ecs_module::<P>());

        // `FrameUpdate` GPU registration must happen after GPU ECS type
        // registration.
        engine.frame_update.register_gpu(&mut engine.gpu);

        // Register statically-linked ECS modules which depend on `GpuWeb`.
        unsafe {
            include_module!(
                engine,
                animation::ffi,
                AnimationModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );

            include_module!(
                engine,
                editor::ffi,
                EditorAgentModule,
                c_api::get_module_api_proc_addr_c::<P, G>
            );
        }

        engine
    }
}

impl<P: Platform, G: Gpu> Drop for GameEngine<P, G> {
    fn drop(&mut self) {
        let gpu = unsafe { ManuallyDrop::take(&mut self.gpu) };
        gpu.destroy(&mut self.cpu_data);
    }
}

impl<P: Platform, G: Gpu> GameEngine<P, G> {
    pub fn gpu(&mut self) -> &mut G {
        &mut self.gpu
    }

    pub fn register_platform_library(&mut self, mut lib: Box<dyn PlatformLibrary>) {
        if !api_version_compatible(lib.void_target_version()) {
            log::error!(
                "{:?} not loaded: module was built for Void version {}.{}.{}",
                lib.name(),
                api_version_major(lib.void_target_version()),
                api_version_minor(lib.void_target_version()),
                api_version_patch(lib.void_target_version()),
            );
            return;
        }

        let platform_lib_name = lib.name();
        let platform_lib_name = platform_lib_name.to_string_lossy();

        // Register functions.
        for i in 0..lib.function_count() {
            // Add namespacing to the component name.
            let name = format!(
                "{platform_lib_name}::{}",
                lib.function_name(i).to_string_lossy()
            );
            let name = CString::new(name).unwrap();

            let is_sync = lib.function_is_sync(i);
            let function = lib.function(i);

            let function_id = self.component_registry.register(ComponentInfo {
                name: name.clone(),
                size: size_of_val(&function),
                align: align_of_val(&function),
                gpu_compatible: false,
                is_freely_mutable: false,
                ecs_type_info: EcsTypeInfo::Callable(CallableInfo { is_sync }),
            });

            // Set newly-registered function component id for existing modules.
            for module in self.ecs_modules.values_mut() {
                module.set_component_id(&name, function_id);
            }

            self.callables
                .add_platform_function(function_id, function, is_sync);
        }

        let res = lib.init();
        assert_eq!(res, 0, "{:?} init() returned with code {res}", lib.name());

        // save library
        self.platform_libraries.push(lib);
    }

    pub fn register_ecs_module(&mut self, mut ecs_module: Box<dyn EcsModule>) {
        log::info!("Registering module {:?}...", ecs_module.module_name());
        self.frame_update.register_module(
            ecs_module.as_mut(),
            &mut self.event_manager,
            &mut self.cpu_data,
            &mut self.component_registry,
            false,
        );

        self.ecs_modules
            .insert(ecs_module.module_name().to_string(), ecs_module);
    }

    /// Get a list of all the loaded ECS Modules by module name.
    pub fn esc_module_names(&self) -> impl Iterator<Item = &str> {
        self.ecs_modules.keys().map(String::as_ref)
    }

    pub fn platform_event_delegate(&mut self) -> PlatformEventDelegate<'_, P> {
        self.event_manager.platform_event_delegate()
    }

    /// # Safety
    ///
    /// `return_value` must be valid data which matches the expected type associated with `TaskId`.
    pub unsafe fn complete_async_task(
        &mut self,
        task_id: TaskId,
        return_value: Box<[MaybeUninit<u8>]>,
    ) {
        unsafe { self.callables.complete_task(task_id, return_value) };
    }

    // The following few ECS functions were written to unblock Editor and give it simple
    // access to the ECS world. They will likely be removed in the future, in favor of
    // a `World` query, which gives unrestricted, exclusive access to the world.

    /// Returns all registered components + resources.
    pub fn component_ids(&self) -> impl Iterator<Item = ComponentId> + '_ {
        self.component_registry
            .iter()
            .map(|(component_id, _)| component_id)
    }

    /// Returns `ComponentInfo` for the given component or resource.
    pub fn component_info(&self, component_id: &ComponentId) -> Option<&ComponentInfo> {
        self.component_registry.get(component_id)
    }

    /// Returns all entities in the world.
    pub fn entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.frame_update.world.entities()
    }

    /// Returns all the components attached to an entity.
    pub fn entity_components(&self, entity_id: EntityId) -> Option<&[ComponentId]> {
        self.frame_update
            .world
            .get(entity_id)
            .map(|entity| entity.archetype_key.component_ids.as_slice())
    }

    /// Returns a pointer to the specified component data for the specified entity.
    /// If the entity + component pair does not exist, returns a null pointer.
    pub fn entity_component_data_mut(
        &mut self,
        entity_id: EntityId,
        component_id: ComponentId,
    ) -> *mut MaybeUninit<u8> {
        let Some(entity_data) = self.frame_update.world.get(entity_id) else {
            return null_mut();
        };

        let Some(storage) = self.frame_update.archetypes.get(&entity_data.archetype_key) else {
            return null_mut();
        };

        if let Some(found_component) = storage
            .cpu
            .components
            .iter()
            .find(|component_info| component_info.component_id == component_id)
        {
            return unsafe {
                self.cpu_data
                    .get_buffer_mut(storage.cpu.buffer_index)
                    .get_mut_ptr(entity_data.archetype_index)
                    .add(found_component.offset)
            };
        }

        if let Some((gpu_storage, found_component)) = storage.gpu.iter().find_map(|gpu_storage| {
            gpu_storage
                .components
                .iter()
                .find(|component_info| component_info.component_id == component_id)
                .map(|info| (gpu_storage, info))
        }) {
            return unsafe {
                self.gpu
                    .get_buffer_mut(
                        &mut self.cpu_data,
                        gpu_storage.buffer_index,
                        gpu_storage.partition,
                    )
                    .get_mut_ptr(entity_data.archetype_index)
                    .add(found_component.offset)
            };
        }

        null_mut()
    }

    /// Returns the `EntityId` associated with the given `label`. Returns `None` if no entity
    /// exists with the `label`.
    pub fn get_entity_from_label(&self, label: &CStr) -> Option<EntityId> {
        self.frame_update.world.label_entity(label)
    }

    /// Returns the label associated with the given `entity_id`. Returns `None` if the entity
    /// doesn't exist or has no label.
    pub fn get_label_from_entity(&self, entity_id: EntityId) -> Option<&CStr> {
        self.frame_update.world.entity_label(entity_id)
    }

    /// Associates the given `label` with the given `entity_id`.
    pub fn set_entity_label(&mut self, entity_id: EntityId, label: &CStr) {
        self.frame_update.world.set_entity_label(entity_id, label);
    }

    /// Returns a pointer to the specified resource data.
    /// If the resource does not exist, returns a null pointer.
    pub fn resource_mut(&mut self, component_id: ComponentId) -> *mut MaybeUninit<u8> {
        let Some(component_info) = self.component_registry.get(&component_id) else {
            return null_mut();
        };

        let buffer_index = match &component_info.ecs_type_info {
            EcsTypeInfo::Resource(resource_info) => resource_info.buffer_index,
            _ => {
                return null_mut();
            }
        };

        if !component_info.gpu_compatible {
            self.cpu_data.get_buffer_mut(buffer_index).get_mut_ptr(0)
        } else {
            self.gpu
                .get_buffer_mut(&mut self.cpu_data, buffer_index, 0)
                .get_mut_ptr(0)
        }
    }

    pub fn system_names(&self) -> impl Iterator<Item = &str> {
        self.frame_update.system_graph.system_names()
    }

    /// Returns `None` if the system was not found.
    pub fn system_enabled(&self, system_name: &str) -> Option<bool> {
        self.frame_update.system_graph.system_enabled(system_name)
    }

    pub fn set_system_enabled(&mut self, system_name: &str, enabled: bool) {
        self.frame_update
            .system_graph
            .set_system_enabled(system_name, enabled);
    }

    // End Editor-facing ECS functions

    pub fn register_preloaded_texture(
        &mut self,
        path: String,
        data: Vec<u8>,
        width_and_height: (u32, u32),
        use_atlas: bool,
    ) -> (TextureId, u32, u32) {
        let texture_id = self.cpu_data.get_resource_mut(
            &self.component_registry,
            |gpu_interface: &mut GpuInterface| {
                gpu_interface
                    .texture_asset_manager
                    .register_next_texture_id()
            },
        );

        let hash = TextureAssetManager::generate_hash(&data);

        let (width, height) = self.gpu.register_preloaded_texture(
            &mut self.cpu_data,
            &self.component_registry,
            texture_id,
            path.clone(),
            data,
            width_and_height,
            use_atlas,
        );

        let loaded_texture = LoadedTexture::new(
            texture_id,
            &path.into(),
            &hash,
            width as usize,
            height as usize,
            FormatType::Png,
            false,
        );

        self.cpu_data.get_resource_mut(
            &self.component_registry,
            |gpu_interface: &mut GpuInterface| {
                gpu_interface
                    .texture_asset_manager
                    .insert_loaded_texture(&loaded_texture)
                    .unwrap();
            },
        );

        (texture_id, width, height)
    }

    pub fn load_scene(&mut self, scene_file: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.frame_update.load_scene(
            scene_file,
            &mut self.cpu_data,
            &mut self.gpu,
            &self.component_registry,
            &self.ecs_modules,
        )
    }

    #[cfg(feature = "state_snapshots")]
    pub fn take_state_snapshot<W: snapshot::WriteUninit>(
        &mut self,
        writer: W,
    ) -> snapshot::Result<W> {
        use snapshot::SerializeMut;

        let mut serializer = snapshot::Serializer::new(writer);
        self.serialize_mut(&mut serializer)?;
        Ok(serializer.into_writer())
    }

    /// Deserializes a state snapshot.
    ///
    /// On failure, this function will panic rather than returning a result.
    /// Because deserialization occurs in-place, failing in the middle of
    /// deserialization may result in corrupt gamestate.
    #[cfg(feature = "state_snapshots")]
    pub fn restore_state_snapshot<R: snapshot::ReadUninit>(&mut self, reader: R) {
        use snapshot::Deserialize;

        let mut deserializer = snapshot::Deserializer::new(reader);

        if let Err(error) = unsafe { self.deserialize_in_place(&mut deserializer) } {
            panic!(
                "state snapshot deserialization failed, \
                aborting due to possible corrupt gamestate\n\
                {error}"
            );
        }

        // Reset system archetype inputs, so that they point to the correct
        // storage buffers.

        self.frame_update.system_graph.clear_archetype_inputs();

        for (archetype_key, storage) in &self.frame_update.archetypes {
            self.frame_update
                .system_graph
                .add_archetype_input(archetype_key, storage);
        }
    }

    /// Execute one frame covering `delta_time`.
    ///
    /// A running game is made from a series of still frame images.
    ///
    /// Use `delta_time` to determine how much of the game state should be
    /// advanced. E.g. a moving game object should move half the distance in a
    /// frame that is 0.004 seconds versus a frame that is 0.008 seconds (twice
    /// as much time).
    pub fn frame(&mut self, delta_time: f32) {
        let delta_time = self.clamp_delta_time(delta_time);

        self.check_window_resize();

        self.update_input_state();

        self.gpu.begin_frame(&mut self.cpu_data);

        self.update_and_record_frame(delta_time);

        self.gpu.submit_frame(&mut self.cpu_data);

        self.event_manager.platform_event_delegate().clear();

        self.callables.clear_call_queue_and_completions();
    }

    /// Clamp `delta_time` to a maximum, so that it doesn't unexpectedly explode
    fn clamp_delta_time(&mut self, delta_time: f32) -> f32 {
        let max_delta_time = self
            .cpu_data
            .get_resource(&self.component_registry, |frame_config: &FrameConfig| {
                frame_config.max_delta_time
            });

        if max_delta_time >= 0. {
            delta_time.min(max_delta_time)
        } else {
            log::warn!("max_delta_time is set to {max_delta_time}, ignoring");
            delta_time
        }
    }

    fn check_window_resize(&mut self) {
        let delegate = self.event_manager.platform_event_delegate();

        platform_event_iter!(delegate, WindowResized, |event| {
            self.gpu.window_resized(
                event.width(),
                event.height(),
                &self.component_registry,
                &mut self.cpu_data,
            );

            if event.update_aspect() {
                // Update `Aspect` resource.
                self.cpu_data
                    .get_resource_mut(&self.component_registry, |aspect: &mut Aspect| {
                        aspect.width = event.width() as f32;
                        aspect.height = event.height() as f32;
                    });

                // Flag the `CameraRenderResource` that it needs to resize its
                // camera render textures.
                self.cpu_data.get_resource_mut(
                    &self.component_registry,
                    |camera_render: &mut CameraRenderResource| {
                        camera_render.resize_camera_render_textures = true;
                    },
                );
            }
        });
    }

    fn update_input_state(&mut self) {
        self.cpu_data
            .get_resource_mut(&self.component_registry, |input_state: &mut InputState| {
                self.input_manager
                    .read_events(&self.event_manager.platform_event_delegate(), input_state);
            });
    }

    fn update_and_record_frame(&mut self, delta_time: f32) {
        let frame_task = self.frame_update.update_async(
            &mut self.event_manager,
            &self.input_manager,
            &mut self.cpu_data,
            &mut self.gpu,
            &self.component_registry,
            &self.ecs_modules,
            &self.callables,
            delta_time,
        );

        let frame_task = pin!(frame_task);

        self.executor.execute_blocking(frame_task);
    }
}

#[cfg(feature = "state_snapshots")]
impl<P: Platform, G: Gpu> snapshot::SerializeMut for GameEngine<P, G> {
    fn serialize_mut<W>(&mut self, serializer: &mut snapshot::Serializer<W>) -> snapshot::Result<()>
    where
        W: snapshot::WriteUninit,
    {
        use snapshot::Serialize;

        self.event_manager.serialize_mut(serializer)?;
        self.input_manager.serialize(serializer)?;
        self.component_registry.serialize(serializer)?;
        self.frame_update.serialize(serializer)?;
        self.callables.serialize_mut(serializer)?;
        self.cpu_data
            .serialize_mut(serializer, &self.component_registry, &self.ecs_modules)
    }
}

#[cfg(feature = "state_snapshots")]
impl<P: Platform, G: Gpu> snapshot::Deserialize for GameEngine<P, G> {
    unsafe fn deserialize<R>(_: &mut snapshot::Deserializer<R>) -> snapshot::Result<Self>
    where
        R: snapshot::ReadUninit,
    {
        panic!("use deserialize_in_place()!")
    }

    unsafe fn deserialize_in_place<R>(
        &mut self,
        deserializer: &mut snapshot::Deserializer<R>,
    ) -> snapshot::Result<()>
    where
        R: snapshot::ReadUninit,
    {
        unsafe {
            self.event_manager.deserialize_in_place(deserializer)?;
            self.input_manager.deserialize_in_place(deserializer)?;
            self.component_registry.deserialize_in_place(deserializer)?;
            self.frame_update.deserialize_in_place(deserializer)?;
            self.callables.deserialize_in_place(deserializer)?;
            // Important: deserialize `cpu_data` last, as it depends on others.
            self.cpu_data.deserialize_in_place(
                deserializer,
                &self.component_registry,
                &self.ecs_modules,
            )
        }
    }
}
