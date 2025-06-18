use std::{
    collections::HashMap,
    ffi::CStr,
    io::{StdoutLock, Write, stdout},
    mem::replace,
    num::Wrapping,
    ops::{Index, IndexMut},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use game_ecs::{ArchetypeKey, WorldDelegate};
use game_entity::{EntityId, ParentType};

#[cfg(feature = "state_snapshots")]
mod serialize;

#[derive(Debug)]
pub struct EntityData {
    /// The archetype key defining the components associated with this entity
    pub archetype_key: ArchetypeKey,

    /// The index of the entity in an archetype
    pub archetype_index: usize,

    /// The entity id of the parent of this entity. `None` if there is no parent.
    pub parent_id: Option<EntityId>,

    /// The entity ids of all children of this entity.
    pub child_ids: Vec<EntityId>,

    /// An optional string label associated with this entity.
    pub label: Option<Arc<CStr>>,
}

impl EntityData {
    pub fn new(archetype_key: ArchetypeKey, archetype_index: usize) -> Self {
        Self {
            archetype_key,
            archetype_index,
            parent_id: None,
            child_ids: Vec::new(),
            label: None,
        }
    }
}

#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
struct EntityEntry {
    entry_type: EntityEntryType,
    lifecycle: Wrapping<u32>,
}

#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
enum EntityEntryType {
    Entry(EntityData),
    /// Next free index. One-past-end is the last free entry in the list.
    Free(usize),
}

const DUMMY_ENTRY: EntityEntry = EntityEntry {
    entry_type: EntityEntryType::Free(0),
    lifecycle: Wrapping(0),
};

pub struct World {
    /// `entities` forms a free-list of entities. Each entry is either occupied, thus
    /// containing entity information, or free, storing the next free index.
    entities: Vec<EntityEntry>,

    /// Stores the start index of the free index, or one-past-end of `entities` if
    /// all entries are occupied.
    free_list_start_index: usize,

    /// Stores a mapping of entity labels to an `EntityId`.
    entity_label_map: HashMap<Arc<CStr>, EntityId>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            entities: Vec::from([DUMMY_ENTRY]),
            free_list_start_index: 1,
            entity_label_map: HashMap::new(),
        }
    }
}

impl World {
    /// Creates a delegate which allows multithreaded Sync access to World.
    ///
    /// IMPORTANT: All `EntityId` values allocated via the `SyncWorldDelegate` are assumed to
    /// be later spawned via `spawn_preallocated()`. Failure to do so will corrupt the world.
    pub fn sync_delegate(&mut self) -> SyncWorldDelegate<'_> {
        let next_index = AtomicUsize::new(self.free_list_start_index);

        SyncWorldDelegate {
            world: self,
            next_index,
        }
    }

    pub fn spawn(&mut self, entity_data: EntityData) -> EntityId {
        if let Some(free_entry) = self.entities.get_mut(self.free_list_start_index) {
            // recycle free entry

            let EntityEntryType::Free(next_free_index) = free_entry.entry_type else {
                panic!("entity free list corrupted");
            };

            let entity_id = EntityId::new(
                self.free_list_start_index
                    .try_into()
                    .expect("cannot allocate any more entity ids"),
                free_entry.lifecycle.0,
            );

            self.free_list_start_index = next_free_index;

            entity_id
        } else {
            // no free entries, push a new one

            let lifecycle = Wrapping(0);

            let entity_id = EntityId::new(
                self.free_list_start_index
                    .try_into()
                    .expect("cannot allocate any more entity ids"),
                lifecycle.0,
            );

            self.entities.push(EntityEntry {
                entry_type: EntityEntryType::Entry(entity_data),
                lifecycle,
            });

            self.free_list_start_index += 1;

            entity_id
        }
    }

    /// Spawns an entity based on a preallocated `EntityId`. Returns `true` if successful -- i.e.
    /// the entity has not already been despawned this same frame.
    pub fn spawn_preallocated(&mut self, entity_id: EntityId, entity_data: EntityData) -> bool {
        let entry = self
            .entities
            .get_mut(entity_id.as_index())
            .expect("preallocated entity does not exist");

        if entry.lifecycle.0 == entity_id.lifecycle {
            entry.entry_type = EntityEntryType::Entry(entity_data);
            true
        } else {
            // entity was spawned and despawned in the same frame
            false
        }
    }

    /// Attempts to despawn an entity, returning the `EntityData` if `spawn_preallocated` has been
    /// called for this entity. The entity must have been spawned or the function will panic.
    #[track_caller]
    pub fn despawn(&mut self, entity_id: EntityId) -> Option<EntityData> {
        let entry = self
            .entities
            .get_mut(entity_id.as_index())
            .expect("entity does not exist");

        // check if entity was already despawned (handles double despawns)
        if entry.lifecycle.0 != entity_id.lifecycle {
            return None;
        }

        entry.lifecycle += 1;

        if let EntityEntryType::Free(_) = &entry.entry_type {
            // entity was spawned and despawned in the same frame, append entry to free list
            entry.entry_type = EntityEntryType::Free(self.free_list_start_index);
            self.free_list_start_index = entity_id.as_index();

            None
        } else {
            // normal despawn, return entry data and append entry to free list
            let EntityEntryType::Entry(mut entity_data) = replace(
                &mut entry.entry_type,
                EntityEntryType::Free(self.free_list_start_index),
            ) else {
                unreachable!();
            };

            self.free_list_start_index = entity_id.as_index();

            // Make sure to clear the label mapping, if this entity had a label.
            if let Some(label) = entity_data.label.take() {
                self.entity_label_map.remove(&label);
            }

            Some(entity_data)
        }
    }

    /// Returns the `EntityId` associated with the given `label`. Returns `None` if no entity
    /// exists with the `label`.
    pub fn label_entity(&self, label: &CStr) -> Option<EntityId> {
        self.entity_label_map.get(label).copied()
    }

    /// Returns the label associated with the given `entity_id`. Returns `None` if the entity
    /// doesn't exist or has no label.
    pub fn entity_label(&self, entity_id: EntityId) -> Option<&CStr> {
        self.get(entity_id)
            .and_then(|entity_data| entity_data.label.as_ref())
            .map(|label| label.as_ref())
    }

    pub fn set_entity_label<T>(&mut self, entity_id: EntityId, label: T)
    where
        T: Into<Arc<CStr>>,
    {
        let label = label.into();

        // Copied from `get_mut()` to avoid double mutable borrow on `self`.
        let Some(entity_data) = self
            .entities
            .get_mut(entity_id.as_index())
            .and_then(|entry| match &mut entry.entry_type {
                EntityEntryType::Entry(val) => Some(val),
                EntityEntryType::Free(_) => None,
            })
        else {
            log::warn!(
                "setting label {:?} for entity {:?}, \
                but entity does not exist",
                label,
                entity_id,
            );

            return;
        };

        let prev = self.entity_label_map.insert(label.clone(), entity_id);

        if let Some(prev) = prev {
            log::warn!(
                "setting label {label:?} for entity {entity_id:?}, \
                which removes existing identical label from entity {prev:?}",
            );
        }

        entity_data.label = Some(label);
    }

    pub fn remove_entity_label(&mut self, entity_id: EntityId) {
        let Some(entity_data) = self.get_mut(entity_id) else {
            log::warn!("clearing label for entity {entity_id:?}, but entity does not exist",);

            return;
        };

        let Some(label) = entity_data.label.take() else {
            log::warn!("clearing label for entity {entity_id:?}, but entity has no label",);

            return;
        };

        self.entity_label_map.remove(&label);
    }

    pub fn get(&self, index: EntityId) -> Option<&EntityData> {
        self.entities
            .get(index.as_index())
            .and_then(|entry| match &entry.entry_type {
                EntityEntryType::Entry(val) => Some(val),
                EntityEntryType::Free(_) => None,
            })
    }

    pub fn get_mut(&mut self, index: EntityId) -> Option<&mut EntityData> {
        self.entities
            .get_mut(index.as_index())
            .and_then(|entry| match &mut entry.entry_type {
                EntityEntryType::Entry(val) => Some(val),
                EntityEntryType::Free(_) => None,
            })
    }

    /// Returns an iterator over all the entities in the world.
    pub fn entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.entities
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| match entry.entry_type {
                EntityEntryType::Entry(_) => Some((i, entry.lifecycle.0)),
                EntityEntryType::Free(_) => None,
            })
            .map(|(i, lifecycle)| EntityId::new(i.try_into().unwrap(), lifecycle))
    }

    /// Prints a Depth-First hierarchy of all the entities to standard out
    pub fn print_entity_hierarchy(&self) {
        let mut lock = stdout().lock();
        writeln!(lock, "Entity Hierarchy:").unwrap();
        for entity_id in self.entities() {
            if let Some(entity_data) = self.get(entity_id) {
                if entity_data.parent_id.is_none() {
                    // if it's a root level entity
                    self.write_entity_children(&entity_id, entity_data, 0, &mut lock);
                }
            }
        }
        stdout().flush().unwrap();
    }

    fn write_entity_children(
        &self,
        entity_id: &EntityId,
        cpu_data: &EntityData,
        depth: usize,
        lock: &mut StdoutLock<'_>,
    ) {
        for _ in 0..depth {
            write!(lock, "  ").unwrap();
        }

        writeln!(lock, "{}", entity_id.id).unwrap();

        for child in cpu_data.child_ids.as_slice() {
            if let Some(child_data) = self.get(*child) {
                self.write_entity_children(child, child_data, depth + 1, lock);
            }
        }
    }
}

impl Index<EntityId> for World {
    type Output = EntityData;

    #[track_caller]
    fn index(&self, index: EntityId) -> &Self::Output {
        self.get(index).expect("entity does not exist")
    }
}

impl IndexMut<EntityId> for World {
    #[track_caller]
    fn index_mut(&mut self, index: EntityId) -> &mut Self::Output {
        self.get_mut(index).expect("entity does not exist")
    }
}

pub struct SyncWorldDelegate<'a> {
    world: &'a mut World,
    next_index: AtomicUsize,
}

impl Drop for SyncWorldDelegate<'_> {
    fn drop(&mut self) {
        let next_index = *self.next_index.get_mut();
        self.world.free_list_start_index = next_index;

        if next_index > self.world.entities.len() {
            self.world.entities.resize_with(next_index, || DUMMY_ENTRY);
        }
    }
}

impl WorldDelegate for SyncWorldDelegate<'_> {
    fn allocate_entity_id(&self) -> EntityId {
        let mut current = self.next_index.load(Ordering::Relaxed);

        loop {
            let (new, lifecycle) = if current >= self.world.entities.len() {
                (current + 1, 0)
            } else {
                let entry = &self.world.entities[current];

                let EntityEntryType::Free(entry_next_entity_index) = entry.entry_type else {
                    panic!("entity free list corrupted");
                };

                (entry_next_entity_index, entry.lifecycle.0)
            };

            match self.next_index.compare_exchange_weak(
                current,
                new,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return EntityId::new(
                        current
                            .try_into()
                            .expect("cannot allocate any more entity ids"),
                        lifecycle,
                    );
                }
                Err(previous) => {
                    current = previous;
                }
            }
        }
    }

    fn label_entity(&self, label: &CStr) -> Option<EntityId> {
        self.world.entity_label_map.get(label).copied()
    }

    fn entity_label(&self, entity_id: EntityId) -> Option<&CStr> {
        self.world
            .get(entity_id)
            .and_then(|entity_data| entity_data.label.as_ref())
            .map(|label| label.as_ref())
    }

    /// Returns an `Option<ParentType>` of the parent data for the given entity id.
    fn get_parent_type(&self, entity_id: EntityId) -> Option<ParentType> {
        if let Some(entity_data) = self.world.get(entity_id) {
            entity_data
                .parent_id
                .map(ParentType::Parent)
                .or_else(|| ParentType::Root.into())
        } else {
            None
        }
    }
}
