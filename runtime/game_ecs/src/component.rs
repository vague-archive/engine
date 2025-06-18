use std::{
    ffi::{CStr, CString},
    mem::MaybeUninit,
    ops::Index,
};

use void_public::{ComponentId, Resource};

use crate::{CpuFrameData, FrameDataBufferRefMut};

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ComponentInfo {
    pub name: CString,
    pub size: usize,
    pub align: usize,
    pub gpu_compatible: bool,
    pub is_freely_mutable: bool,
    pub ecs_type_info: EcsTypeInfo,
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub enum EcsTypeInfo {
    AsyncCompletion(AsyncCompletionInfo),
    Callable(CallableInfo),
    Component(EntityComponentInfo),
    Resource(ResourceInfo),
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct EntityComponentInfo {
    /// The name of the module which declares this entity component.
    pub declaring_module_name: String,
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ResourceInfo {
    /// The index of the buffer in the `CpuFrameData` which holds the resource data.
    pub buffer_index: usize,
    /// The name of the module which declares this resource.
    pub declaring_module_name: String,
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct CallableInfo {
    pub is_sync: bool,
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct AsyncCompletionInfo {
    pub callable_id: ComponentId,
}

#[derive(Default, Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ComponentRegistry {
    components: Vec<ComponentInfo>,
}

impl ComponentRegistry {
    pub fn register(&mut self, info: ComponentInfo) -> ComponentId {
        self.components.push(info);

        // we are off-by-one when indexing into `components`, because component ids start at 1
        u16::try_from(self.components.len())
            .expect("cannot allocate any more ComponentIds")
            .try_into()
            .unwrap()
    }

    pub fn get(&self, component_id: &ComponentId) -> Option<&ComponentInfo> {
        let index: usize = (*component_id).get().into();
        // we are off-by-one when indexing into `components`, because component ids start at 1
        self.components.get(index - 1)
    }

    pub fn get_with_string_id(&self, string_id: &CStr) -> Option<(ComponentId, &ComponentInfo)> {
        self.components.iter().enumerate().find_map(|(i, c)| {
            if c.name.as_c_str() == string_id {
                // we are off-by-one when indexing into `components`, because component ids start at 1
                Some((((i + 1) as u16).try_into().unwrap(), c))
            } else {
                None
            }
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (ComponentId, &ComponentInfo)> {
        self.components
            .iter()
            .enumerate()
            // we are off-by-one when indexing into `components`, because component ids start at 1
            .map(|(i, info)| (((i + 1) as u16).try_into().unwrap(), info))
    }
}

impl Index<&ComponentId> for ComponentRegistry {
    type Output = ComponentInfo;

    #[track_caller]
    fn index(&self, component_id: &ComponentId) -> &Self::Output {
        self.get(component_id).unwrap()
    }
}

#[derive(Debug)]
pub struct ComponentBundle {
    pub source_components: Vec<ComponentId>,
    /// These components will be created if all source components are present
    pub bundled_components: Vec<ComponentDefault>,
}

#[derive(Debug)]
pub struct ComponentDefault {
    pub id: ComponentId,
    pub default_value: Box<[MaybeUninit<u8>]>,
}

/// Manually registers an ECS resource. Returns the `CpuFrameData` buffer index
/// which stores the resource.
///
/// Most resources will be automatically registered. This function is only for
/// special-cased resources which require manual registration, such as those in
/// `Gpu::register_resources`.
pub fn manually_register_resource<R: Resource>(
    resource: R,
    declaring_module_name: impl Into<String>,
    component_registry: &mut ComponentRegistry,
    cpu_data: &mut CpuFrameData,
) -> usize {
    // Allocate a new buffer for the resource.
    let buffer_index = cpu_data.new_buffer(size_of::<R>(), align_of::<R>());

    // Push the resource to the new buffer.
    cpu_data.get_buffer_mut(buffer_index).push(resource);

    // Register the resource with the component registry.
    let component_id = component_registry.register(ComponentInfo {
        name: R::string_id().to_owned(),
        size: size_of::<R>(),
        align: align_of::<R>(),
        gpu_compatible: false,
        is_freely_mutable: true,
        ecs_type_info: EcsTypeInfo::Resource(ResourceInfo {
            buffer_index,
            declaring_module_name: declaring_module_name.into(),
        }),
    });

    // Set the static resource ID.
    unsafe {
        R::set_id(component_id);
    }

    buffer_index
}
