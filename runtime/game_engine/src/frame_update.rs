use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    ffi::CString,
    mem::{MaybeUninit, size_of},
    num::NonZero,
    slice::from_raw_parts,
};

use event::{
    AddComponents, CommandRef, EventManager, SpawnComponentData,
    events_generated::{RemoveComponents, SetEntityLabel},
};
use game_asset::{
    ecs_module::GpuInterface, resource_managers::texture_asset_manager::PendingTexture,
};
use game_ecs::{
    ArchetypeKey, ArchetypeStorage, ArchetypeStorageMap, AsyncCompletionInfo, Callables,
    ComponentBundle, ComponentInfo, ComponentRegistry, CpuFrameData, EcsSystemExecuteResources,
    EcsTypeInfo, EntityComponentInfo, FrameDataBufferBorrowRef, FrameDataBufferBorrowRefMut,
    FrameDataBufferRefMut, ResourceInfo, SystemGraph, bundle_required_components,
    cpu_frame_data::CpuDataBufferRefMut,
};
use game_entity::EntityId;
use game_input_manager::InputManager;
use game_scene::SceneEntityComponents;
use game_world::{EntityData, World};
use gpu_common::Gpu;
use platform::{EcsModule, Platform};
use void_public::{
    Component, ComponentData, ComponentId, ComponentRef, ComponentType, ENGINE_VERSION, EcsType,
    FrameConstants, LocalToWorld, Mat4, Quat, Transform, api_version_compatible, api_version_major,
    api_version_minor, api_version_patch,
    graphics::{TextureId, TextureRender},
};

use crate::{cpu_system::CpuSystem, transforms_update::update_world_transforms};

#[cfg(feature = "state_snapshots")]
mod serialize;

pub struct FrameUpdate<P: Platform, G: Gpu> {
    pub archetypes: ArchetypeStorageMap,
    pub system_graph: SystemGraph<P, G>,
    gpu_component_groupings: Vec<Vec<ComponentId>>,
    gpu_component_bundles: Vec<ComponentBundle>,
    /// (component id, buffer index)
    gpu_single_buffer_components: Vec<(ComponentId, Option<usize>)>,
    pub world: World,
    frame_timer: FrameTimer,
}

impl<P: Platform, G: Gpu> Default for FrameUpdate<P, G> {
    fn default() -> Self {
        Self {
            archetypes: Default::default(),
            system_graph: Default::default(),
            gpu_component_groupings: Default::default(),
            gpu_component_bundles: Default::default(),
            gpu_single_buffer_components: Default::default(),
            world: Default::default(),
            frame_timer: FrameTimer::default(),
        }
    }
}

impl<P: Platform, G: Gpu> FrameUpdate<P, G> {
    pub fn register_gpu(&mut self, gpu: &mut G) {
        self.gpu_component_groupings = gpu.component_groupings();
        self.gpu_component_bundles = gpu.component_bundles();
        self.gpu_single_buffer_components = gpu
            .single_buffer_components()
            .into_iter()
            .map(|component_id| (component_id, None))
            .collect();
    }

    pub fn register_module(
        &mut self,
        ecs_module: &mut dyn EcsModule,
        event_manager: &mut EventManager<P>,
        cpu_data: &mut CpuFrameData,
        component_registry: &mut ComponentRegistry,
        is_gpu: bool,
    ) {
        let module_name = ecs_module.module_name();
        if !api_version_compatible(ecs_module.void_target_version()) {
            log::error!(
                "{:?} not loaded: module was built for Void version {}.{}.{} - expected: {}.{}.{}",
                module_name,
                api_version_major(ecs_module.void_target_version()),
                api_version_minor(ecs_module.void_target_version()),
                api_version_patch(ecs_module.void_target_version()),
                api_version_major(ENGINE_VERSION),
                api_version_minor(ENGINE_VERSION),
                api_version_patch(ENGINE_VERSION),
            );
            return;
        }

        // Run init functions.
        if let Err(err) = ecs_module.init() {
            log::error!("{module_name:?} not loaded: init() returned {err:?}");
            return;
        }

        // Register components.
        let mut i = 0;
        while let Some(string_id) = ecs_module.component_string_id(i) {
            let size = ecs_module.component_size(string_id.as_ref());
            let align = ecs_module.component_align(string_id.as_ref());
            let component_type = ecs_module.component_type(string_id.as_ref());

            let ecs_type_info = match component_type {
                ComponentType::AsyncCompletion => {
                    let callable_string_id =
                        ecs_module.component_async_completion_callable(&string_id);

                    let Some((callable_id, _)) =
                        component_registry.get_with_string_id(&callable_string_id)
                    else {
                        log::error!(
                            "{:?} not loaded: {:?} requests callable {:?}, but this callable is not registered",
                            module_name,
                            string_id,
                            callable_string_id,
                        );
                        return;
                    };

                    EcsTypeInfo::AsyncCompletion(AsyncCompletionInfo { callable_id })
                }
                ComponentType::Component => EcsTypeInfo::Component(EntityComponentInfo {
                    declaring_module_name: module_name.to_string(),
                }),
                ComponentType::Resource => {
                    let buffer_index = cpu_data.new_buffer(size, align);
                    let mut buffer = cpu_data.get_buffer_mut(buffer_index);
                    let data = buffer.grow();

                    let res = ecs_module.resource_init(string_id.as_ref(), data);

                    if let Err(err) = res {
                        panic!("failed to initialize resource {string_id:?}, error = {err}");
                    };

                    EcsTypeInfo::Resource(ResourceInfo {
                        buffer_index,
                        declaring_module_name: module_name.to_string(),
                    })
                }
            };

            component_registry.register(ComponentInfo {
                name: string_id.into_owned(),
                size,
                align,
                gpu_compatible: false,
                is_freely_mutable: true,
                ecs_type_info,
            });

            i += 1;
        }

        // Register systems.
        for system_index in 0..ecs_module.systems_len() {
            let system = Box::new(CpuSystem::<P, G>::new(
                system_index,
                component_registry,
                event_manager,
                ecs_module,
            ));

            let is_once = ecs_module.system_is_once(system_index);

            if is_gpu {
                self.system_graph.add_gpu_system(system, is_once);
            } else {
                self.system_graph.add_cpu_system(system, is_once);
            }
        }

        // Set component ids.
        let mut cid = ComponentId::new(1).unwrap();
        while let Some(component_info) = component_registry.get(&cid) {
            ecs_module.set_component_id(&component_info.name, cid);
            cid = cid.checked_add(1).unwrap();
        }
    }

    pub fn load_scene(
        &mut self,
        scene_file: &str,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.apply_prev_frame_changes(cpu_data, gpu_data);

        let all_scene_entities = game_scene::parse_scene(scene_file, |name, text| {
            Self::deserialize_component_json(name, text, component_registry, modules)
        })?;

        // map the JSON "id" field to the engine-generated `EntityId`
        let mut json_id_map: HashMap<&str, EntityId> = HashMap::new();

        let mut process_batched_textures = false;

        let texture_render_str_id = TextureRender::string_id();

        for (component_ref, asset_path) in all_scene_entities.iter().map(|scene_entity| {
            (
                scene_entity.components.iter().find_map(|component| {
                    component_registry
                        .get(&component.component_id().unwrap())
                        .filter(|comp_info| comp_info.name.as_c_str() == texture_render_str_id)
                        .map(|_| ComponentRef::from(component))
                }),
                scene_entity.texture_asset_path.as_ref(),
            )
        }) {
            if let Some(texture_asset_path) = asset_path {
                // reinterpret as TextureRender component
                let texture_render = unsafe {
                    component_ref
                        .unwrap()
                        .component_val
                        .cast_mut()
                        .cast::<TextureRender>()
                        .as_mut()
                        .unwrap()
                };

                // assign the texture id, based on the asset_path from json
                texture_render.texture_id = Self::load_texture_from_path(
                    texture_asset_path.as_str(),
                    cpu_data,
                    component_registry,
                );
                process_batched_textures = true;
            }
        }

        if process_batched_textures {
            cpu_data.get_resource_mut(component_registry, |gpu_interface: &mut GpuInterface| {
                gpu_interface
                    .texture_asset_manager
                    .trigger_batched_textures();
            });
        }

        for scene_entity in &all_scene_entities {
            // type-erase the component data
            let mut components: BTreeMap<ComponentId, Box<[MaybeUninit<u8>]>> = scene_entity
                .components
                .iter()
                .map(|component| {
                    let component_ref: ComponentRef<'_> = component.into();
                    let component_id = component_ref.component_id.unwrap();
                    let component_data = unsafe {
                        from_raw_parts(
                            component_ref.component_val.cast(),
                            component_ref.component_size,
                        )
                    };
                    (component_id, component_data.into())
                })
                .collect();

            // look up any required bundled components
            let required_components = bundle_required_components(
                &components.keys().copied(),
                &self.gpu_component_bundles,
            );

            for component in required_components {
                components.insert(component.id, component.default_value.clone());
            }

            let components = &SceneEntityComponents {
                component_ids: components.keys().copied().collect(),
                components: &components,
            };

            let archetype_key = ArchetypeKey {
                component_ids: components.sorted_component_ids().into(),
            };

            self.allocate_archetype_storage_if_needed(
                &archetype_key,
                cpu_data,
                gpu_data,
                component_registry,
            );

            let storage = &self.archetypes[&archetype_key];
            let buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);
            let entity_index = buffer.len();

            // spawn the entity
            let entity_id = self
                .world
                .spawn(EntityData::new(archetype_key, entity_index));

            write_spawn_component_data(
                entity_id,
                entity_index,
                components,
                storage,
                cpu_data,
                gpu_data,
                component_registry,
            );

            if let Some(label) = scene_entity.label.as_ref() {
                self.world
                    .set_entity_label(entity_id, CString::new(label.as_bytes()).unwrap());
            }

            if let Some(scene_id) = &scene_entity.scene_id {
                json_id_map.insert(scene_id, entity_id);
            } else {
                log::warn!(
                    "Detected entity without a scene id. Other entities will not be able to reference it"
                );
            }
        }

        // reconstruct the entity <-> entity relationships
        for scene_entity in &all_scene_entities {
            if let Some(parent_entity_id) = scene_entity
                .parent_scene_id
                .as_ref()
                .and_then(|id| json_id_map.get(id.as_str()))
            {
                if let Some(entity_id) = scene_entity
                    .scene_id
                    .as_ref()
                    .and_then(|id| json_id_map.get(id.as_str()))
                {
                    let entity = self.world.get_mut(*entity_id).unwrap();
                    entity.parent_id = Some(*parent_entity_id);

                    let parent_entity = self.world.get_mut(*parent_entity_id).unwrap();
                    parent_entity.child_ids.push(*entity_id);
                }
            }
        }

        Ok(())
    }

    fn deserialize_component_json(
        component_name: &str,
        text: &str,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
    ) -> Result<ComponentData, Box<dyn Error + Send + Sync>> {
        let c_str = CString::new(component_name)?;

        let result = component_registry
            .iter()
            .find(|component| component.1.name == c_str);

        if let Some((component_id, component_info)) = result {
            if let EcsTypeInfo::Component(entity_component_info) = &component_info.ecs_type_info {
                // create a buffer big enough for the data
                let mut dest_vec = Vec::<MaybeUninit<u8>>::with_capacity(component_info.size);

                // deserialize via the owning module
                if let Err(error) = modules[entity_component_info.declaring_module_name.as_str()]
                    .as_ref()
                    .component_deserialize_json(
                        component_info.name.as_c_str(),
                        dest_vec.as_mut_slice(),
                        text,
                    )
                {
                    return Err(format!(
                        "Component {} could not be deserialized: {:?}",
                        component_name, error
                    )
                    .into());
                }

                unsafe {
                    // since we're writing directly to dest_vec's buffer, set the size manually
                    dest_vec.set_len(component_info.size);
                };

                Ok(ComponentData::new(component_id, dest_vec))
            } else {
                Err(format!("Trying to deserialize non-component {}", component_name).into())
            }
        } else {
            Err(format!("Component {} not found", component_name).into())
        }
    }

    fn load_texture_from_path(
        asset_path: &str,
        cpu_data: &mut CpuFrameData,
        component_registry: &ComponentRegistry,
    ) -> TextureId {
        cpu_data.get_resource_mut(component_registry, |gpu_interface: &mut GpuInterface| {
            if let Some(texture) = gpu_interface
                .texture_asset_manager
                .get_texture_by_path(&asset_path.into())
            {
                texture.id()
            } else {
                let texture_id = gpu_interface
                    .texture_asset_manager
                    .register_next_texture_id();
                let pending_texture = PendingTexture::new(texture_id, &asset_path.into(), false);
                gpu_interface
                    .texture_asset_manager
                    .add_to_batched_textures(pending_texture);
                texture_id
            }
        })
    }

    /// Update systems which may update asynchronously, in parallel with frame rendering
    // We need more arguments for this function
    #[allow(clippy::too_many_arguments)]
    pub async fn update_async(
        &mut self,
        event_manager: &mut EventManager<P>,
        input_manager: &InputManager,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
        callables: &Callables,
        delta_time: f32,
    ) {
        self.frame_timer.update_frame(delta_time);

        self.apply_prev_frame_changes(cpu_data, gpu_data);

        self.update_frame_constants(cpu_data, component_registry, delta_time);

        // process cpu systems
        {
            let resources = &EcsSystemExecuteResources {
                cpu_data,
                gpu_data,
                event_manager,
                input_buffer: input_manager.binary_buffer(),
                world_delegate: &self.world.sync_delegate(),
                component_bundles: &self.gpu_component_bundles,
                component_registry,
                callables,
            };

            self.system_graph.execute_cpu(resources).await;
        }

        // Apply changes before running GPU systems, so that we don't change the number
        // of objects by spawning/despawning after the GPU has already issued draw calls
        self.apply_current_frame_changes(
            event_manager,
            cpu_data,
            gpu_data,
            component_registry,
            modules,
        );

        // process gpu systems

        let resources = &EcsSystemExecuteResources {
            cpu_data,
            gpu_data,
            event_manager,
            input_buffer: input_manager.binary_buffer(),
            world_delegate: &self.world.sync_delegate(),
            component_bundles: &self.gpu_component_bundles,
            component_registry,
            callables,
        };

        self.system_graph.execute_gpu(resources).await;
    }

    fn update_frame_constants(
        &self,
        cpu_data: &mut CpuFrameData,
        component_registry: &ComponentRegistry,
        delta_time: f32,
    ) {
        cpu_data.get_resource_mut(component_registry, |constants: &mut FrameConstants| {
            let prev_tick_count = constants.tick_count;

            *constants = FrameConstants {
                delta_time,
                frame_rate: self.frame_timer.get_average_fps(),
                tick_count: prev_tick_count + 1,
            };
        });
    }

    #[allow(clippy::unused_self)]
    fn apply_prev_frame_changes(&mut self, _cpu_data: &mut CpuFrameData, _gpu_data: &mut G) {
        assert!(
            !G::MULTI_BUFFERED,
            "multi-buffered support temporarily removed, pending readdition of `entity_changes`"
        );
    }

    fn apply_current_frame_changes(
        &mut self,
        event_manager: &mut EventManager<P>,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
    ) {
        event_manager.drain_commands(|command| match command {
            CommandRef::AddComponents(command) => {
                self.handle_add_components(&command, cpu_data, gpu_data, component_registry);
            }
            CommandRef::Despawn(command) => {
                let entity_id = NonZero::new(command.entity_id()).unwrap().into();
                self.handle_despawn(entity_id, cpu_data, gpu_data);
            }
            CommandRef::LoadScene(command) => {
                let scene_str = command.scene_json().unwrap();
                if let Err(e) =
                    self.load_scene(scene_str, cpu_data, gpu_data, component_registry, modules)
                {
                    log::warn!("Unable to load scene JSON: {:?}", e);
                }
            }
            CommandRef::RemoveComponents(command) => {
                self.handle_remove_components(&command, cpu_data, gpu_data, component_registry);
            }
            CommandRef::SetEntityLabel(command) => {
                self.handle_set_entity_label(&command);
            }
            CommandRef::SetParent(command) => {
                let entity_id = NonZero::new(command.entity_id()).unwrap().into();
                let parent_id = NonZero::new(command.parent_id()).map(|id| id.into());
                self.handle_set_parent(
                    entity_id,
                    parent_id,
                    command.keep_world_space_transform(),
                    cpu_data,
                );
            }
            CommandRef::SetSystemEnabled(command) => {
                self.system_graph
                    .set_system_enabled(command.system_name().unwrap(), command.enabled());
            }
            CommandRef::Spawn(command) => {
                self.handle_spawn(&command, cpu_data, gpu_data, component_registry);
            }
        });

        update_world_transforms(&self.world, &self.archetypes, cpu_data);
    }

    fn handle_spawn(
        &mut self,
        command: &AddComponents<'_>,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
    ) {
        let archetype_key = ArchetypeKey {
            component_ids: command.sorted_component_ids().into(),
        };

        self.allocate_archetype_storage_if_needed(
            &archetype_key,
            cpu_data,
            gpu_data,
            component_registry,
        );

        let storage = &self.archetypes[&archetype_key];
        let buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);
        let entity_index = buffer.len();

        let despawned = !self.world.spawn_preallocated(
            *command.entity_id,
            EntityData::new(archetype_key, entity_index),
        );

        if despawned {
            return;
        }

        log::info!("Spawning entity {}", command.entity_id.id);

        write_spawn_component_data(
            *command.entity_id,
            entity_index,
            command,
            storage,
            cpu_data,
            gpu_data,
            component_registry,
        );
    }

    fn handle_despawn(
        &mut self,
        entity_id: EntityId,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
    ) {
        let Some(mut entity_data) = self.world.despawn(entity_id) else {
            return;
        };

        let storage = self.archetypes.get(&entity_data.archetype_key).unwrap();

        log::info!("Despawning entity {}", entity_id.id);

        // cpu

        let mut buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);
        buffer.swap_remove(entity_data.archetype_index);

        // reassign world entity index for swapped entity data
        if let Some(entity_id) = unsafe {
            buffer.get_mut_with_offset_as::<EntityId>(
                entity_data.archetype_index,
                storage.cpu.entity_id_offset.unwrap(),
            )
        } {
            self.world[*entity_id].archetype_index = entity_data.archetype_index;
        }

        // gpu

        for storage_gpu in &storage.gpu {
            let mut buffer =
                gpu_data.get_buffer_mut(cpu_data, storage_gpu.buffer_index, storage_gpu.partition);
            buffer.swap_remove(entity_data.archetype_index);
        }

        for child_id in entity_data.child_ids.drain(..) {
            self.handle_despawn(child_id, cpu_data, gpu_data);
        }
    }

    fn handle_add_components(
        &mut self,
        command: &AddComponents<'_>,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
    ) {
        log::info!("Adding components to entity {}", command.entity_id.id);

        let Some(entity_data) = self.world.get(*command.entity_id) else {
            log::warn!(
                "... entity id {} with lifecycle {} does not exist",
                command.entity_id.id,
                command.entity_id.lifecycle
            );
            return;
        };

        // Add new component ids to the existing archetype key.
        let mut archetype_key = entity_data.archetype_key.clone();
        archetype_key
            .component_ids
            .extend(command.sorted_component_ids());
        archetype_key.component_ids.sort_unstable();
        archetype_key.component_ids.dedup();

        if archetype_key == entity_data.archetype_key {
            log::warn!("... archetypes do not differ, likely no new components were added");
            return;
        }

        self.allocate_archetype_storage_if_needed(
            &archetype_key,
            cpu_data,
            gpu_data,
            component_registry,
        );

        let entity_data = &mut self.world[*command.entity_id];

        let prev_storage = &self.archetypes[&entity_data.archetype_key];
        let storage = &self.archetypes[&archetype_key];
        let (mut prev_buffer, mut buffer) =
            cpu_data.get_buffer_pair_mut((prev_storage.cpu.buffer_index, storage.cpu.buffer_index));

        let prev_entry_ptr = prev_buffer.get_ptr(entity_data.archetype_index);

        // Copy data from old archetype to new archetype.
        write_cpu_component_data(
            *command.entity_id,
            storage,
            component_registry,
            buffer.grow(),
            |component_id| {
                // Prefer checking previous storage first, so that we don't
                // overwrite existing component data.
                if let Some(component_info) = prev_storage
                    .cpu
                    .components
                    .iter()
                    .find(|c| c.component_id == component_id)
                {
                    unsafe {
                        let ptr = prev_entry_ptr.add(component_info.offset);
                        let size = component_registry[&component_id].size;

                        from_raw_parts(ptr, size)
                    }
                } else {
                    command.component_data(component_id).unwrap()
                }
            },
        );

        assert!(storage.gpu.is_empty(), "gpu data currently unsupported");

        // Remove old archetype buffer entry.
        prev_buffer.swap_remove(entity_data.archetype_index);

        // Update entity data stored on the `World`.
        entity_data.archetype_key = archetype_key;
        entity_data.archetype_index = buffer.len() - 1;

        initialize_local_to_world_if_needed(cpu_data, entity_data.archetype_index, storage);
    }

    fn handle_remove_components(
        &mut self,
        command: &RemoveComponents<'_>,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
    ) {
        let entity_id: EntityId = NonZero::new(command.entity_id()).unwrap().into();

        log::info!("Removing components from entity {}", entity_id.id);

        let Some(entity_data) = self.world.get(entity_id) else {
            log::warn!(
                "... entity id {} with lifecycle {} does not exist",
                entity_id.id,
                entity_id.lifecycle
            );
            return;
        };

        // Remove component ids from the existing archetype key.
        let mut archetype_key = entity_data.archetype_key.clone();
        archetype_key.component_ids.retain(|cid| {
            command
                .component_ids()
                .unwrap()
                .iter()
                .all(|removed_id| cid.get() != removed_id)
        });

        if archetype_key == entity_data.archetype_key {
            log::warn!("... archetypes do not differ, likely no components were removed");
            return;
        }

        self.allocate_archetype_storage_if_needed(
            &archetype_key,
            cpu_data,
            gpu_data,
            component_registry,
        );

        let entity_data = &mut self.world[entity_id];

        let prev_storage = &self.archetypes[&entity_data.archetype_key];
        let storage = &self.archetypes[&archetype_key];
        let (mut prev_buffer, mut buffer) =
            cpu_data.get_buffer_pair_mut((prev_storage.cpu.buffer_index, storage.cpu.buffer_index));

        let prev_entry_ptr = prev_buffer.get_ptr(entity_data.archetype_index);

        // Copy data from old archetype to new archetype.
        write_cpu_component_data(
            entity_id,
            storage,
            component_registry,
            buffer.grow(),
            |component_id| {
                let prev_offset = prev_storage
                    .cpu
                    .components
                    .iter()
                    .find(|c| c.component_id == component_id)
                    .unwrap()
                    .offset;

                unsafe {
                    let ptr = prev_entry_ptr.add(prev_offset);
                    let size = component_registry[&component_id].size;

                    from_raw_parts(ptr, size)
                }
            },
        );

        assert!(storage.gpu.is_empty(), "gpu data currently unsupported");

        // Remove old archetype buffer entry.
        prev_buffer.swap_remove(entity_data.archetype_index);

        // Update entity data stored on the `World`.
        entity_data.archetype_key = archetype_key;
        entity_data.archetype_index = buffer.len() - 1;
    }

    /// Allocates a new storage for the archetype, if none exists.
    fn allocate_archetype_storage_if_needed(
        &mut self,
        archetype_key: &ArchetypeKey,
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
        component_registry: &ComponentRegistry,
    ) {
        if self.archetypes.contains_key(archetype_key) {
            return;
        }

        let storage = ArchetypeStorage::new(
            &archetype_key.component_ids,
            component_registry,
            &self.gpu_component_groupings,
            &mut self.gpu_single_buffer_components,
            cpu_data,
            gpu_data,
        );

        // add archetype as input to relevant systems
        self.system_graph
            .add_archetype_input(archetype_key, &storage);

        self.archetypes.insert(archetype_key.clone(), storage);
    }

    fn handle_set_entity_label(&mut self, command: &SetEntityLabel<'_>) {
        let entity_id = NonZero::new(command.entity_id()).unwrap().into();

        if let Some(label) = command.label() {
            let label = CString::new(label).unwrap();
            self.world.set_entity_label(entity_id, label);
        } else {
            self.world.remove_entity_label(entity_id);
        }
    }

    fn handle_set_parent(
        &mut self,
        entity_id: EntityId,
        parent_id: Option<EntityId>,
        keep_world_transform: bool,
        cpu_data: &mut CpuFrameData,
    ) {
        let Some(entity_data) = self.world.get(entity_id) else {
            // unable to find the entity to update
            return;
        };

        if entity_data.parent_id == parent_id {
            // parentage isn't actually changing, ignore.
            return;
        }

        if let Some(parent_id) = parent_id {
            if self.world.get(parent_id).is_none() {
                // unable to find the parent entity
                log::warn!("Unable to set parent. Parent Id {parent_id:?} not found");
                return;
            }
        }

        log::info!("Setting parent of {entity_id:?} to {parent_id:?}...");

        // remove the entity from its current parent's child list (if applicable)
        if let Some(prev_parent_id) = entity_data.parent_id {
            if let Some(parent_data) = self.world.get_mut(prev_parent_id) {
                parent_data.child_ids.retain(|id| *id != entity_id);
            }
        }

        // add to new parent's child list (if not root)
        if let Some(parent_id) = parent_id {
            if let Some(parent_data) = self.world.get_mut(parent_id) {
                if parent_data.child_ids.contains(&entity_id) {
                    log::warn!("Child cannot be added more than once");
                    return;
                }
                parent_data.child_ids.push(entity_id);
            }
        }

        // update the parent field
        self.world.get_mut(entity_id).unwrap().parent_id = parent_id;

        // if the re-parent is in-place, we need to update the Transform component based on the new parentage
        if keep_world_transform {
            // calculate world->new_parent matrix
            let mut mat_parent_world_to_local = Mat4::IDENTITY;
            if let Some(new_parent_id) = parent_id {
                let (parent_entity_data, parent_archetype_storage, parent_data_buffer) =
                    self.get_buffer_for_entity(new_parent_id, cpu_data);
                if let Some(data_parent_local_to_world) = unsafe {
                    get_data_from_buffer::<LocalToWorld>(
                        &parent_data_buffer,
                        parent_entity_data.archetype_index,
                        parent_archetype_storage,
                    )
                } {
                    mat_parent_world_to_local = data_parent_local_to_world.inverse();
                }
            }

            // get entity->world matrix
            let mut entity_buffer = self.get_buffer_for_entity(entity_id, cpu_data);

            let data_entity_local_to_world = unsafe {
                get_data_from_buffer::<LocalToWorld>(
                    &entity_buffer.2,
                    entity_buffer.0.archetype_index,
                    entity_buffer.1,
                )
            }
            .unwrap();

            // calculate entity->new_parent matrix
            let new_local_matrix = mat_parent_world_to_local.mul_mat4(data_entity_local_to_world);
            let (scale, rotation, translation) = new_local_matrix.to_scale_rotation_translation();

            // write back to the Transform component
            let transform_to_update = unsafe {
                get_data_from_buffer_mut::<Transform>(
                    &mut entity_buffer.2,
                    entity_buffer.0.archetype_index,
                    entity_buffer.1,
                )
            }
            .unwrap();
            let axis_angle = rotation.to_axis_angle();
            transform_to_update.position = translation.into();
            transform_to_update.scale = scale.truncate().into();
            transform_to_update.rotation = axis_angle.0.z.signum() * axis_angle.1;
        }
    }

    /// Returns a tuple containing the buffer for an entity along with the structs to lookup
    /// component specific data
    /// (`entity_data`, `archetype_storage`, `AtomicRef<Buffer>`)
    fn get_buffer_for_entity<'a>(
        &self,
        entity_id: EntityId,
        cpu_data: &'a mut CpuFrameData,
    ) -> (&EntityData, &ArchetypeStorage, CpuDataBufferRefMut<'a>) {
        let entity_data = self.world.get(entity_id).unwrap();
        let storage_map_parent = self.archetypes.get(&entity_data.archetype_key).unwrap();

        (
            entity_data,
            storage_map_parent,
            cpu_data.get_buffer_mut(storage_map_parent.cpu.buffer_index),
        )
    }
}

/// Writes component data from a new spawn event into archetype storage buffers.
fn write_spawn_component_data<G: Gpu, T: SpawnComponentData>(
    entity_id: EntityId,
    entity_index: usize,
    components: &T,
    storage: &ArchetypeStorage,
    cpu_data: &mut CpuFrameData,
    gpu_data: &mut G,
    component_registry: &ComponentRegistry,
) {
    let mut buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);
    let component_entry_bytes = buffer.grow();

    assert_eq!(
        component_entry_bytes
            .as_ptr()
            .align_offset(storage.cpu.align),
        0
    );

    // write cpu component data

    write_cpu_component_data(
        entity_id,
        storage,
        component_registry,
        component_entry_bytes,
        |component_id| components.component_data(component_id).unwrap(),
    );

    // initialize/sync component data if required

    initialize_local_to_world_if_needed(cpu_data, entity_index, storage);

    // write gpu component data

    for storage_gpu in &storage.gpu {
        let mut buffer =
            gpu_data.get_buffer_mut(cpu_data, storage_gpu.buffer_index, storage_gpu.partition);
        let buffer = buffer.grow();

        for component_offset_info in &storage_gpu.components {
            let data = components
                .component_data(component_offset_info.component_id)
                .unwrap();
            buffer[component_offset_info.offset..component_offset_info.offset + data.len()]
                .copy_from_slice(data);
        }
    }
}

fn write_cpu_component_data<'a, F>(
    entity_id: EntityId,
    storage: &ArchetypeStorage,
    component_registry: &ComponentRegistry,
    mut buffer_data: &mut [MaybeUninit<u8>],
    mut component_data: F,
) where
    F: FnMut(ComponentId) -> &'a [MaybeUninit<u8>],
{
    let mut write_offset_check = 0;
    for component_offset_info in &storage.cpu.components {
        debug_assert_eq!(write_offset_check, component_offset_info.offset);

        if component_offset_info.component_id == void_public::EntityId::id() {
            // EntityId
            let size = size_of::<void_public::EntityId>();
            buffer_data[..size].copy_from_slice(
                &NonZero::<u64>::from(entity_id)
                    .get()
                    .to_ne_bytes()
                    .map(MaybeUninit::new),
            );
            buffer_data = &mut buffer_data[size..];
            write_offset_check += size;
        } else {
            let data = component_data(component_offset_info.component_id);
            let component_info = component_registry
                .get(&component_offset_info.component_id)
                .unwrap();

            if data.len() != component_info.size {
                log::error!(
                    "spawn: data for component {:?} has size {}, does not match expected size {}",
                    component_info.name,
                    data.len(),
                    component_info.size
                );
                panic!();
            }

            buffer_data[..data.len()].copy_from_slice(data);
            buffer_data = &mut buffer_data[data.len()..];
            write_offset_check += data.len();
        }
    }
}

/// Initialize the `LocalToWorld` component based on `Transform` data, if
/// needed.
fn initialize_local_to_world_if_needed(
    cpu_data: &mut CpuFrameData,
    entity_index: usize,
    storage: &ArchetypeStorage,
) {
    let mut buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);

    let Some(transform_state) =
        (unsafe { get_data_from_buffer::<Transform>(&buffer, entity_index, storage) })
    else {
        return;
    };

    // create the initial matrix
    let initial_matrix = Mat4::from_scale_rotation_translation(
        transform_state.scale.extend(1.),
        Quat::from_rotation_z(transform_state.rotation),
        *transform_state.position,
    );

    // write it back to the LocalToWorld component
    let local_to_world_state =
        unsafe { get_data_from_buffer_mut::<LocalToWorld>(&mut buffer, entity_index, storage) }
            .expect("LocalToWorld is expected if Transform is present");
    *local_to_world_state = initial_matrix.into();
}

unsafe fn get_data_from_buffer<'a, T>(
    data_buffer: &'a CpuDataBufferRefMut<'a>,
    entity_index: usize,
    archetype_storage: &ArchetypeStorage,
) -> Option<&'a T>
where
    T: Component,
{
    let data_offset = archetype_storage.get_component_byte_offset(&T::id())?;
    unsafe { data_buffer.get_with_offset_as::<T>(entity_index, data_offset) }
}

unsafe fn get_data_from_buffer_mut<'a, T>(
    data_buffer: &'a mut CpuDataBufferRefMut<'a>,
    entity_index: usize,
    archetype_storage: &ArchetypeStorage,
) -> Option<&'a mut T>
where
    T: Component,
{
    let data_offset = archetype_storage.get_component_byte_offset(&T::id())?;
    unsafe { data_buffer.get_mut_with_offset_as::<T>(entity_index, data_offset) }
}

#[derive(Debug, Default)]
struct FrameTimer {
    accumulated_time: f32,
    num_frames: u32,
    average_frame_time: f32,
}

impl FrameTimer {
    pub fn update_frame(&mut self, delta_time: f32) {
        self.accumulated_time += delta_time;
        self.num_frames += 1;
        if self.accumulated_time >= 1.0 && self.num_frames >= 120 {
            self.average_frame_time = self.accumulated_time / self.num_frames as f32;
            self.accumulated_time = 0.;
            self.num_frames = 0;
        }
    }

    pub fn get_average_frame_time(&self) -> f32 {
        self.average_frame_time
    }

    pub fn get_average_fps(&self) -> f32 {
        if self.get_average_frame_time() == 0.0 {
            0.
        } else {
            1. / self.get_average_frame_time()
        }
    }
}
