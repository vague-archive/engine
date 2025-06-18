use std::{collections::HashMap, ffi::CStr};

use platform::EcsModule;
use snapshot::{
    Deserialize, Deserializer, Error, ReadUninit, Result, Serialize, Serializer, WriteUninit,
};

use super::*;
use crate::{ComponentRegistry, EcsTypeInfo, ResourceInfo};

impl CpuFrameData {
    /// Non-canonical implementation of `SerializeMut` to support custom
    /// serialization for certain ECS types.
    pub fn serialize_mut<W>(
        &mut self,
        serializer: &mut Serializer<W>,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
    ) -> Result<()>
    where
        W: WriteUninit,
    {
        self.buffers.len().serialize(serializer)?;

        for (i, buffer) in self.buffers.iter_mut().enumerate() {
            buffer
                .get_mut()
                .serialize(serializer, component_registry, modules, i)?;
        }

        Ok(())
    }

    /// Non-canonical implementation of `Deserialize` to support custom
    /// deserialization for certain ECS types.
    ///
    /// Copied from the `Deserialize` impl for `Vec<T>`.
    ///
    /// # Safety
    ///
    /// `deserializer` must be at the correct read position.
    pub unsafe fn deserialize_in_place<R>(
        &mut self,
        deserializer: &mut Deserializer<R>,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
    ) -> Result<()>
    where
        R: ReadUninit,
    {
        let len = unsafe { usize::deserialize(deserializer) }?;

        let existing_len = len.min(self.buffers.len());
        let additional_len = len.saturating_sub(self.buffers.len());

        // Initialize existing elements in-place.
        for (i, buffer) in self.buffers[..existing_len].iter_mut().enumerate() {
            unsafe {
                buffer
                    .get_mut()
                    .deserialize_in_place(deserializer, component_registry, modules, i)
            }?;
        }

        // Use `deserialize()` to push new elements.
        for i in existing_len..existing_len + additional_len {
            let buffer = unsafe {
                CpuDataBuffer::deserialize(deserializer, component_registry, modules, i)
            }?;
            self.buffers.push(buffer.into());
        }

        // Resize in case deserialized length is shorter than existing length.
        // This will always resize smaller, so we can just use `unreachable!()`.
        self.buffers.resize_with(len, || unreachable!());
        Ok(())
    }
}

impl CpuDataBuffer {
    /// Non-canonical implementation of `Serialize` to support custom
    /// serialization for certain ECS types.
    pub fn serialize<W>(
        &self,
        serializer: &mut Serializer<W>,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
        buffer_index: usize,
    ) -> Result<()>
    where
        W: WriteUninit,
    {
        // Check if we are serializing a Resource or just POD.
        if let Some((component_info, resource_info)) =
            component_registry.iter().find_map(|(_, component_info)| {
                match &component_info.ecs_type_info {
                    EcsTypeInfo::Resource(resource_info)
                        if resource_info.buffer_index == buffer_index =>
                    {
                        Some((component_info, resource_info))
                    }
                    _ => None,
                }
            })
        {
            assert_eq!(
                self.data.len(),
                component_info.size,
                "resource storage should only store a single entry"
            );

            // Accumulate serialize callbacks and then commit to serializer with
            // one go, as an extra safety measure. We don't want to give modules
            // free reign to write into the serialized buffer directly.
            let mut resource_data = Vec::new();

            // Call the declaring module's serialization routine.
            modules[&resource_info.declaring_module_name]
                .resource_serialize(&component_info.name, &self.data, &mut |buf| {
                    resource_data.extend_from_slice(buf);
                    Ok(buf.len())
                })
                .map_err(Error::Serialize)?;

            // Serialize the complete resource buffer.
            resource_data.len().serialize(serializer)?;
            serializer.serialize_uninit_bytes(&resource_data)
        } else {
            // POD, use canonical serialize impl.
            Serialize::serialize(self, serializer)
        }
    }

    /// Non-canonical implementation of `Deserialize` to support custom
    /// deserialization for certain ECS types.
    ///
    /// # Safety
    ///
    /// `deserializer` must be at the correct read position.
    pub unsafe fn deserialize<R>(
        deserializer: &mut Deserializer<R>,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
        buffer_index: usize,
    ) -> Result<Self>
    where
        R: ReadUninit,
    {
        // Check if we are deserializing a Resource or just POD.
        if let Some((component_info, resource_info)) =
            component_registry.iter().find_map(|(_, component_info)| {
                match &component_info.ecs_type_info {
                    EcsTypeInfo::Resource(resource_info)
                        if resource_info.buffer_index == buffer_index =>
                    {
                        Some((component_info, resource_info))
                    }
                    _ => None,
                }
            })
        {
            let mut buffer = CpuDataBuffer::new(component_info.size, component_info.align);
            let mut buffer_mut = CpuDataBufferRefMut(&mut buffer);
            let data = buffer_mut.grow();

            unsafe {
                deserialize_resource(
                    deserializer,
                    resource_info,
                    &component_info.name,
                    data,
                    modules,
                )
            }?;

            Ok(buffer)
        } else {
            // POD, use canonical deserialize impl.
            unsafe { Deserialize::deserialize(deserializer) }
        }
    }

    /// Non-canonical implementation of `Deserialize` to support custom
    /// deserialization for certain ECS types.
    ///
    /// # Safety
    ///
    /// `deserializer` must be at the correct read position.
    pub unsafe fn deserialize_in_place<R>(
        &mut self,
        deserializer: &mut Deserializer<R>,
        component_registry: &ComponentRegistry,
        modules: &HashMap<String, Box<dyn EcsModule>>,
        buffer_index: usize,
    ) -> Result<()>
    where
        R: ReadUninit,
    {
        // Check if we are deserializing a Resource or just POD.
        if let Some((component_info, resource_info)) =
            component_registry.iter().find_map(|(_, component_info)| {
                match &component_info.ecs_type_info {
                    EcsTypeInfo::Resource(resource_info)
                        if resource_info.buffer_index == buffer_index =>
                    {
                        Some((component_info, resource_info))
                    }
                    _ => None,
                }
            })
        {
            unsafe {
                deserialize_resource(
                    deserializer,
                    resource_info,
                    &component_info.name,
                    &mut self.data,
                    modules,
                )
            }
        } else {
            // POD, use canonical deserialize impl.
            unsafe { Deserialize::deserialize_in_place(self, deserializer) }
        }
    }
}

unsafe fn deserialize_resource<R>(
    deserializer: &mut Deserializer<R>,
    resource_info: &ResourceInfo,
    string_id: &CStr,
    resource_data: &mut [MaybeUninit<u8>],
    modules: &HashMap<String, Box<dyn EcsModule>>,
) -> Result<()>
where
    R: ReadUninit,
{
    // Keep track of how many bytes have been read. It should match how many
    // were written.
    let mut remaining_bytes = unsafe { usize::deserialize(deserializer) }?;

    // Call the declaring module's deserialization routine.
    modules[&resource_info.declaring_module_name]
        .resource_deserialize(string_id, resource_data, &mut |buf| {
            if buf.len() > remaining_bytes {
                return Err(format!("{string_id:?}: read more bytes than were written").into());
            }

            remaining_bytes -= buf.len();

            deserializer
                .deserialize_into_uninit_bytes(buf)
                .map(|_| buf.len()) // Return the number of bytes read on `Ok`.
                .map_err(Error::into)
        })
        .map_err(Error::Deserialize)?;

    if remaining_bytes == 0 {
        Ok(())
    } else {
        Err(Error::Deserialize(
            format!("resource {string_id:?}: read fewer bytes than were written").into(),
        ))
    }
}
