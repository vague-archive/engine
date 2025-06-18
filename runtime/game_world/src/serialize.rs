use snapshot::{Deserialize, Deserializer, ReadUninit, Result, Serialize, Serializer, WriteUninit};

use super::*;

impl Serialize for World {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        self.entities.serialize(serializer)?;
        self.free_list_start_index.serialize(serializer)

        // `entity_label_map` can be reconstructed on deserialize
    }
}

impl Deserialize for World {
    unsafe fn deserialize<R>(_: &mut Deserializer<R>) -> Result<Self>
    where
        R: ReadUninit,
    {
        panic!("use deserialize_in_place()!")
    }

    unsafe fn deserialize_in_place<R>(&mut self, deserializer: &mut Deserializer<R>) -> Result<()>
    where
        R: ReadUninit,
    {
        unsafe {
            self.entities.deserialize_in_place(deserializer)?;
            self.free_list_start_index
                .deserialize_in_place(deserializer)?;
        }

        // Reconstruct entity label map.
        self.entity_label_map.clear();
        self.entity_label_map.extend(
            self.entities
                .iter()
                .enumerate()
                // Filter active entities.
                .filter_map(|(i, entry)| match &entry.entry_type {
                    EntityEntryType::Entry(entity_data) => Some((
                        entity_data,
                        EntityId::new(i.try_into().unwrap(), entry.lifecycle.0),
                    )),
                    EntityEntryType::Free(_) => None,
                })
                // Filter entities with labels.
                .filter_map(|(entity_data, entity_id)| {
                    entity_data
                        .label
                        .as_ref()
                        .map(|label| (label.clone(), entity_id))
                }),
        );

        Ok(())
    }
}

impl Serialize for EntityData {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        self.archetype_key.serialize(serializer)?;
        self.archetype_index.serialize(serializer)?;
        serializer.serialize_pod(&self.parent_id)?;
        serializer.serialize_pod_vec(&self.child_ids)?;
        self.label.serialize(serializer)
    }
}

impl Deserialize for EntityData {
    unsafe fn deserialize<R>(deserializer: &mut Deserializer<R>) -> Result<Self>
    where
        R: ReadUninit,
    {
        unsafe {
            Ok(Self {
                archetype_key: ArchetypeKey::deserialize(deserializer)?,
                archetype_index: usize::deserialize(deserializer)?,
                parent_id: deserializer.deserialize_pod()?,
                child_ids: deserializer.deserialize_pod_vec()?,
                label: Option::deserialize(deserializer)?,
            })
        }
    }

    unsafe fn deserialize_in_place<R>(&mut self, deserializer: &mut Deserializer<R>) -> Result<()>
    where
        R: ReadUninit,
    {
        unsafe {
            self.archetype_key.deserialize_in_place(deserializer)?;
            self.archetype_index.deserialize_in_place(deserializer)?;
            self.parent_id = deserializer.deserialize_pod()?;
            deserializer.deserialize_pod_vec_in_place(&mut self.child_ids)?;
            self.label.deserialize_in_place(deserializer)
        }
    }
}
