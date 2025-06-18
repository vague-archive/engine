use std::{
    error::Error,
    ffi::CStr,
    mem::MaybeUninit,
    ptr::{self, NonNull},
    slice,
};

use event::{ComponentData, EventManager};
use game_entity::{EntityId, ParentType};
use platform::Platform;
use void_public::{ComponentId, ComponentRef};

use crate::{
    ArchetypeKey, ArchetypeStorage, Callables, ComponentBundle, ComponentDefault,
    ComponentRegistry, CpuFrameData, GpuFrameData,
};

#[cfg(feature = "state_snapshots")]
mod serialize;

/// Everything a game may access in the engine during frame processing is
/// contained here. Code running between each frame cannot use it because it is
/// created on each frame and destroyed on end of that same frame.
pub struct EcsSystemExecuteResources<'a, P: Platform, G: GpuFrameData> {
    pub cpu_data: &'a CpuFrameData,
    pub gpu_data: &'a G,
    pub event_manager: &'a EventManager<P>,
    pub input_buffer: &'a [u8],
    pub world_delegate: &'a dyn WorldDelegate,
    pub component_bundles: &'a [ComponentBundle],
    pub component_registry: &'a ComponentRegistry,
    pub callables: &'a Callables,
}

/// This is set to point to `EcsSystemExecuteResources` at the start of frame
/// processing. It is set to `ptr::null()` at the end of frame processing.
///
/// # Safety
///
/// Using a raw pointer (rather than a reference) to store a reference to a
/// stack-allocated `EcsSystemExecuteResources` that is accessible globally,
/// skipping Rust's usual lifetime checks (which are instead handled with
/// runtime checks, by unwrapping the reference made from the pointer).
///
/// It's a null pointer () to allow for generics (since generic-parameterized
/// statics are not supported).
static mut SYSTEM_EXECUTE_RESOURCES: *const () = ptr::null();

pub fn system_execute_resources<T, F, P: Platform, G: GpuFrameData>(f: F) -> T
where
    F: FnOnce(&EcsSystemExecuteResources<'_, P, G>) -> T,
{
    let resources = unsafe {
        SYSTEM_EXECUTE_RESOURCES
            .cast::<EcsSystemExecuteResources<'_, P, G>>()
            .as_ref()
            .unwrap()
    };

    f(resources)
}

pub trait EcsSystem<P: Platform, G: GpuFrameData>: Send {
    fn name(&self) -> &str;

    fn add_archetype_input(&mut self, archetype: &ArchetypeKey, storage: &ArchetypeStorage);

    fn clear_archetype_inputs(&mut self);

    fn execute(&mut self) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Prompts the system to clear all of its `EventWriter` buffers.
    ///
    /// This is necessary for systems which have just been disabled. Because
    /// systems are responsible for writing and clearing these buffers, we have
    /// the systems handle this explicit clearing as well.
    ///
    /// This is only called if `execute()` will not be called this frame. Thus,
    /// `execute()` should still clear the event writer buffers on every call.
    fn clear_event_writer_buffers(&mut self);
}

pub trait WorldDelegate: Sync {
    fn allocate_entity_id(&self) -> EntityId;

    /// Returns the `EntityId` associated with this label.
    fn label_entity(&self, label: &CStr) -> Option<EntityId>;

    /// Returns the label associated with this `EntityId`.
    fn entity_label(&self, entity_id: EntityId) -> Option<&CStr>;

    fn get_parent_type(&self, entity_id: EntityId) -> Option<ParentType>;
}

/// Checks for valid component ids, bundles required components, and provides
/// the closure used in add-component-related command functions.
pub fn add_components_helper<'a, F, U>(
    components: &[ComponentRef<'a>],
    component_bundles: &'a [ComponentBundle],
    f: F,
) -> Result<U, Box<dyn Error + Send + Sync>>
where
    F: Fn(usize, &dyn Fn(usize) -> ComponentData<'a>) -> U,
{
    // check for valid component ids
    if let Some(index) = components
        .iter()
        .position(|component| component.component_id.is_none())
    {
        return Err(format!(
            "invalid ComponentId at index '{index}', has it been set via set_component_id()?"
        )
        .into());
    }

    // Add any necessary bundled components. Do it here (vs during reading) for free multithreading
    let component_ids = components.iter().map(|c| c.component_id.unwrap());
    let bundled_components = bundle_required_components(&component_ids, component_bundles);

    let user_data = f(components.len() + bundled_components.len(), &|i| {
        if let Some(component) = components.get(i) {
            let component_data = unsafe {
                let ptr = NonNull::new(component.component_val.cast::<MaybeUninit<u8>>() as *mut _)
                    .unwrap_or(NonNull::dangling())
                    .as_ptr();

                slice::from_raw_parts(ptr, component.component_size)
            };

            ComponentData {
                component_id: component.component_id.unwrap(),
                component_data,
            }
        } else {
            let component = &bundled_components[i - components.len()];

            ComponentData {
                component_id: component.id,
                component_data: &component.default_value,
            }
        }
    });

    Ok(user_data)
}

pub fn bundle_required_components<'a, I>(
    components: &I,
    component_bundles: &'a [ComponentBundle],
) -> Vec<&'a ComponentDefault>
where
    I: IntoIterator<Item = ComponentId> + Clone,
{
    let mut bundled_components = component_bundles
        .iter()
        .filter(|bundle| {
            bundle
                .source_components
                .iter()
                .all(|&source_id| components.clone().into_iter().any(|cid| cid == source_id))
        })
        .flat_map(|bundle| &bundle.bundled_components)
        .collect::<Vec<_>>();

    bundled_components.sort_by_key(|component| component.id);
    bundled_components.dedup_by_key(|component| component.id);

    // remove components already present in spawn command
    bundled_components.retain(|component| {
        components
            .clone()
            .into_iter()
            .all(|cid| cid != component.id)
    });

    bundled_components
}

pub struct SystemGraph<P: Platform, G: GpuFrameData> {
    // separate CPU and GPU systems for now, to simulate a graph barrier
    cpu_systems: Vec<SystemInfo<P, G>>,
    gpu_systems: Vec<SystemInfo<P, G>>,
}

pub struct SystemInfo<P: Platform, G: GpuFrameData> {
    system: Box<dyn EcsSystem<P, G>>,
    enabled: bool,
    is_once: bool,
}

impl<P: Platform, G: GpuFrameData> Default for SystemGraph<P, G> {
    fn default() -> Self {
        Self {
            cpu_systems: Default::default(),
            gpu_systems: Default::default(),
        }
    }
}

impl<P: Platform, G: GpuFrameData> SystemGraph<P, G> {
    pub fn add_cpu_system(&mut self, system: Box<dyn EcsSystem<P, G>>, is_once: bool) {
        self.cpu_systems.push(SystemInfo {
            system,
            enabled: true,
            is_once,
        });
    }

    pub fn add_gpu_system(&mut self, system: Box<dyn EcsSystem<P, G>>, is_once: bool) {
        self.gpu_systems.push(SystemInfo {
            system,
            enabled: true,
            is_once,
        });
    }

    pub fn system_names(&self) -> impl Iterator<Item = &str> {
        self.cpu_systems
            .iter()
            .chain(&self.gpu_systems)
            .map(|system_info| system_info.system.name())
    }

    pub fn system_enabled(&self, system_name: &str) -> Option<bool> {
        if let Some(system_info) = self
            .cpu_systems
            .iter()
            .find(|system_info| system_info.system.name() == system_name)
        {
            Some(system_info.enabled)
        } else {
            self.gpu_systems
                .iter()
                .find(|system_info| system_info.system.name() == system_name)
                .map(|system_info| system_info.enabled)
        }
    }

    pub fn set_system_enabled(&mut self, system_name: &str, enabled: bool) {
        if let Some(system_info) = self
            .cpu_systems
            .iter_mut()
            .find(|system_info| system_info.system.name() == system_name)
        {
            system_info.enabled = enabled;
        } else if let Some(system_info) = self
            .gpu_systems
            .iter_mut()
            .find(|system_info| system_info.system.name() == system_name)
        {
            system_info.enabled = enabled;
        } else {
            log::warn!("SystemGraph::set_system_enabled(): {system_name:?} not found");
        }
    }

    /// Executes all CPU systems.
    pub async fn execute_cpu(&mut self, resources: &EcsSystemExecuteResources<'_, P, G>) {
        unsafe {
            SYSTEM_EXECUTE_RESOURCES =
                (resources as *const EcsSystemExecuteResources<'_, P, G>).cast();
        }

        for system_info in &mut self.cpu_systems {
            if system_info.enabled {
                if let Err(err) = system_info.system.execute() {
                    log::error!(
                        "system `{}` returned error: {err:?}",
                        system_info.system.name()
                    );
                    system_info.enabled = false;
                }

                if system_info.is_once {
                    system_info.enabled = false;
                }
            } else {
                system_info.system.clear_event_writer_buffers();
            }
        }

        unsafe {
            SYSTEM_EXECUTE_RESOURCES = ptr::null();
        }
    }

    /// Executes all GPU systems.
    pub async fn execute_gpu(&mut self, resources: &EcsSystemExecuteResources<'_, P, G>) {
        unsafe {
            SYSTEM_EXECUTE_RESOURCES =
                (resources as *const EcsSystemExecuteResources<'_, P, G>).cast();
        }

        for system_info in &mut self.gpu_systems {
            if system_info.enabled {
                if let Err(err) = system_info.system.execute() {
                    log::error!(
                        "system `{}` returned error: {err:?}",
                        system_info.system.name()
                    );
                    system_info.enabled = false;
                }

                if system_info.is_once {
                    system_info.enabled = false;
                }
            } else {
                system_info.system.clear_event_writer_buffers();
            }
        }

        unsafe {
            SYSTEM_EXECUTE_RESOURCES = ptr::null();
        }
    }

    pub fn add_archetype_input(
        &mut self,
        archetype_key: &ArchetypeKey,
        storage: &ArchetypeStorage,
    ) {
        for system_info in &mut self.cpu_systems {
            system_info
                .system
                .add_archetype_input(archetype_key, storage);
        }

        for system_info in &mut self.gpu_systems {
            system_info
                .system
                .add_archetype_input(archetype_key, storage);
        }
    }

    pub fn clear_archetype_inputs(&mut self) {
        for system_info in &mut self.cpu_systems {
            system_info.system.clear_archetype_inputs();
        }

        for system_info in &mut self.gpu_systems {
            system_info.system.clear_archetype_inputs();
        }
    }
}
