use std::{
    collections::{
        HashMap,
        hash_map::{Entry, Iter},
    },
    ops::{Index, IndexMut},
};

use void_public::{ComponentId, EcsType, EntityId};

use crate::{ComponentRegistry, CpuFrameData, GpuFrameData, PartitionIndex};

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ArchetypeKey {
    pub component_ids: Vec<ComponentId>,
}

impl ArchetypeKey {
    pub fn contains(&self, component_id: &ComponentId) -> bool {
        self.component_ids.contains(component_id)
    }
}

#[derive(Default, Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ArchetypeStorageMap {
    entries: HashMap<ArchetypeKey, ArchetypeStorage>,
}

impl ArchetypeStorageMap {
    pub fn insert(
        &mut self,
        key: ArchetypeKey,
        storage: ArchetypeStorage,
    ) -> Option<ArchetypeStorage> {
        debug_assert!(key.component_ids.windows(2).all(|a| a[0] < a[1]));
        self.entries.insert(key, storage)
    }

    pub fn contains_key(&self, key: &ArchetypeKey) -> bool {
        debug_assert!(key.component_ids.windows(2).all(|a| a[0] < a[1]));
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &ArchetypeKey) -> Option<&ArchetypeStorage> {
        debug_assert!(key.component_ids.windows(2).all(|a| a[0] < a[1]));
        self.entries.get(key)
    }

    pub fn get_mut(&mut self, key: &ArchetypeKey) -> Option<&mut ArchetypeStorage> {
        debug_assert!(key.component_ids.windows(2).all(|a| a[0] < a[1]));
        self.entries.get_mut(key)
    }

    pub fn entry(&mut self, key: ArchetypeKey) -> Entry<'_, ArchetypeKey, ArchetypeStorage> {
        debug_assert!(key.component_ids.windows(2).all(|a| a[0] < a[1]));
        self.entries.entry(key)
    }

    pub fn values(&self) -> impl Iterator<Item = &ArchetypeStorage> {
        self.entries.values()
    }
}

impl<'a> IntoIterator for &'a ArchetypeStorageMap {
    type Item = (&'a ArchetypeKey, &'a ArchetypeStorage);
    type IntoIter = Iter<'a, ArchetypeKey, ArchetypeStorage>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl Index<&ArchetypeKey> for ArchetypeStorageMap {
    type Output = ArchetypeStorage;

    fn index(&self, index: &ArchetypeKey) -> &Self::Output {
        self.get(index).unwrap()
    }
}

impl IndexMut<&ArchetypeKey> for ArchetypeStorageMap {
    fn index_mut(&mut self, index: &ArchetypeKey) -> &mut Self::Output {
        self.get_mut(index).unwrap()
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ArchetypeStorage {
    pub cpu: ArchetypeStorageInfo,
    pub gpu: Vec<ArchetypeStorageInfo>,
}

impl ArchetypeStorage {
    pub fn new<G: GpuFrameData>(
        component_ids: &[ComponentId],
        component_registry: &ComponentRegistry,
        gpu_component_groupings: &[Vec<ComponentId>],
        gpu_single_buffer_components: &mut [(ComponentId, Option<usize>)],
        cpu_data: &mut CpuFrameData,
        gpu_data: &mut G,
    ) -> Self {
        let cpu = {
            let mut storage_info = ArchetypeStorageInfo::default();

            let mut components: Vec<_> = component_ids
                .iter()
                .chain(&[EntityId::id()])
                .map(|id| (*id, component_registry.get(id).unwrap()))
                .filter(|(_, info)| !info.gpu_compatible)
                .collect();

            // iterate in descending alignment, to tightly-pack component data
            components.sort_unstable_by_key(|(_, info)| info.align);

            let mut offset = 0;
            for (id, info) in components.iter().rev() {
                if EntityId::id() == *id {
                    storage_info.entity_id_offset = Some(offset);
                }

                storage_info.components.push(ComponentOffsetInfo {
                    component_id: *id,
                    offset,
                });
                offset += info.size;
            }

            let align = components.last().map_or(1, |(_, info)| info.align);
            let align_offset = (offset as *const u8).align_offset(align);

            storage_info.align = align;
            storage_info.stride = offset + align_offset;

            // allocate cpu buffer
            storage_info.buffer_index =
                cpu_data.new_buffer(storage_info.stride, storage_info.align);
            storage_info.partition = 0;

            storage_info
        };

        let gpu = gpu_component_groupings
            .iter()
            .filter(|grouping| {
                // only iterate groupings containing a component from this archetype
                component_ids
                    .iter()
                    .any(|cid| grouping.iter().any(|g_cid| g_cid == cid))
            })
            .map(|grouping| {
                // only iterate grouped components included in this archetype
                grouping
                    .iter()
                    .filter(|g_cid| component_ids.iter().any(|cid| *g_cid == cid))
            })
            .map(|grouped_components| {
                let mut storage_info = ArchetypeStorageInfo::default();

                let mut offset = 0;

                storage_info.components = grouped_components
                    .map(|g_cid| {
                        let info = component_registry.get(g_cid).unwrap();

                        offset += (offset as *const u8).align_offset(info.align);

                        let res = ComponentOffsetInfo {
                            component_id: *g_cid,
                            offset,
                        };

                        offset += info.size;
                        storage_info.align = storage_info.align.max(info.align);

                        res
                    })
                    .collect();

                let align_offset = (offset as *const u8).align_offset(storage_info.align);
                storage_info.stride = offset + align_offset;

                // allocate or assign gpu buffer

                if let Some((_, buffer_index)) = gpu_single_buffer_components
                    .iter_mut()
                    .find(|(cid, _)| *cid == storage_info.components[0].component_id)
                {
                    if let Some(buffer_index) = *buffer_index {
                        // we've already allocated a buffer for this component
                        storage_info.buffer_index = buffer_index;
                        storage_info.partition =
                            gpu_data.allocate_buffer_partition(storage_info.buffer_index);
                    } else {
                        // this is the first time we've seen this component used
                        storage_info.buffer_index =
                            gpu_data.new_buffer(cpu_data, storage_info.stride);
                        storage_info.partition = 0;

                        *buffer_index = Some(storage_info.buffer_index);
                    }
                } else {
                    storage_info.buffer_index = gpu_data.new_buffer(cpu_data, storage_info.stride);
                    storage_info.partition = 0;
                }

                storage_info
            })
            .collect();

        Self { cpu, gpu }
    }

    /// Return the offset in bytes of the data for the given component id for this archetype storage.
    /// Returns `None` if the given `component_id` isn't found.
    pub fn get_component_byte_offset(&self, component_id: &ComponentId) -> Option<usize> {
        // check the cpu storage first
        let offset = self
            .cpu
            .components
            .iter()
            .find(|c| c.component_id == *component_id)
            .map(|c| c.offset);

        if offset.is_some() {
            return offset;
        }

        // check the gpu storage
        self.gpu
            .iter()
            .flat_map(|g| g.components.iter())
            .find(|c| c.component_id == *component_id)
            .map(|c| c.offset);

        None
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ComponentOffsetInfo {
    /// The id of the component
    pub component_id: ComponentId,

    /// The offset in bytes from the front of the entity's component data buffer
    pub offset: usize,
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct ArchetypeStorageInfo {
    /// List of component id and data buffer offset (in bytes) pairs
    pub components: Vec<ComponentOffsetInfo>,
    pub align: usize,
    pub stride: usize,
    pub buffer_index: usize,
    pub partition: PartitionIndex,
    /// Quick lookup for entity ID byte offset in archetype, if it exists
    pub entity_id_offset: Option<usize>,
}

impl ArchetypeStorageInfo {
    pub fn contains_component(&self, component_id: &ComponentId) -> bool {
        self.components
            .iter()
            .any(|component_offset_info| component_offset_info.component_id == *component_id)
    }
}
