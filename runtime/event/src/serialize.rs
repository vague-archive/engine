use bytemuck::cast_slice;
use platform::Platform;
use snapshot::{
    Deserialize, Deserializer, ReadUninit, Result, Serialize, SerializeMut, Serializer,
    WriteUninit, cast_uninit_slice_mut,
};

use super::*;

impl<P: Platform> SerializeMut for PlatformEventData<P> {
    fn serialize_mut<W>(&mut self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        // Number of event storages.
        self.len().serialize(serializer)?;

        // Event storages.
        for (ident, storage) in &mut self.0 {
            ident.serialize(serializer)?;
            storage.serialize_mut(serializer)?;
        }

        Ok(())
    }
}

impl<P: Platform> Deserialize for PlatformEventData<P> {
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
        // If this becomes a performance issue, we can reuse existing buffers
        // here as an optimization.
        self.0.clear();

        let event_types_len = unsafe { usize::deserialize(deserializer) }?;

        for _ in 0..event_types_len {
            let event_type_ident = unsafe { CString::deserialize(deserializer) }?;

            // Allocate per-thread buffers for this event writer.
            let per_thread_buffers: Vec<_> = (0..P::Executor::available_parallelism().get())
                .map(|_| {
                    UnsafeCell::new(ThreadBuffer {
                        data: Vec::new(),
                        event_count: 0,
                    })
                })
                .collect();

            let mut storage = EventWriterStorage::<P> {
                inner: EventWriterStorageInner { per_thread_buffers }.into(),
                marker: PhantomData,
            };

            unsafe { storage.deserialize_in_place(deserializer) }?;

            self.0.insert(event_type_ident, storage);
        }

        Ok(())
    }
}

impl<P: Platform> SerializeMut for ModuleEventData<P> {
    fn serialize_mut<W>(&mut self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        // Number of event types.
        self.len().serialize(serializer)?;

        for (ident, storages) in &mut self.0 {
            // Event type identifier.
            ident.serialize(serializer)?;

            // Number of event writers for this event type.
            storages.len().serialize(serializer)?;

            // Per-writer storages.
            for (writer_name, storage) in storages {
                writer_name.serialize(serializer)?;
                storage.serialize_mut(serializer)?;
            }
        }

        Ok(())
    }
}

impl<P: Platform> Deserialize for ModuleEventData<P> {
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
        // If this becomes a performance issue, we can reuse existing buffers
        // here as an optimization.
        self.0.clear();

        let event_types_len = unsafe { usize::deserialize(deserializer) }?;

        for _ in 0..event_types_len {
            let event_type_ident = unsafe { CString::deserialize(deserializer) }?;
            let event_writers_len = unsafe { usize::deserialize(deserializer) }?;

            // Individual storages for all event writers of this event type.
            let mut event_writer_storages = HashMap::new();

            for _ in 0..event_writers_len {
                let event_writer_ident = unsafe { String::deserialize(deserializer) }?;

                // Allocate per-thread buffers for this event writer.
                let per_thread_buffers: Vec<_> = (0..P::Executor::available_parallelism().get())
                    .map(|_| {
                        UnsafeCell::new(ThreadBuffer {
                            data: Vec::new(),
                            event_count: 0,
                        })
                    })
                    .collect();

                let mut storage = EventWriterStorage::<P> {
                    inner: EventWriterStorageInner { per_thread_buffers }.into(),
                    marker: PhantomData,
                };

                unsafe { storage.deserialize_in_place(deserializer) }?;

                event_writer_storages.insert(event_writer_ident, storage);
            }

            self.0.insert(event_type_ident, event_writer_storages);
        }

        Ok(())
    }
}

impl<P: Platform> SerializeMut for EventWriterStorage<P> {
    fn serialize_mut<W>(&mut self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        let storage = self.inner.get_mut();

        let event_count = storage
            .per_thread_buffers
            .iter_mut()
            .map(|buffer| buffer.get_mut().event_count)
            .sum::<usize>();

        let total_data_len = storage
            .per_thread_buffers
            .iter_mut()
            .map(|buffer| buffer.get_mut().data.len())
            .sum::<usize>();

        // Event count.
        event_count.serialize(serializer)?;

        // Combined data Vec len across all per-thread buffers (they are
        // serialized together as one data blob).
        //
        // Note the nuance here: we are serializing the `Vec<u64>` length, not
        // the byte length. This makes the deserialization logic simpler.
        total_data_len.serialize(serializer)?;

        // Event data.
        for buffer in &mut storage.per_thread_buffers {
            let data = buffer.get_mut().data.as_slice();
            // Convert data from [u64] to [u8].
            serializer.serialize_bytes(cast_slice(data))?;
        }

        Ok(())
    }
}

impl<P: Platform> Deserialize for EventWriterStorage<P> {
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
        let storage = self.inner.get_mut();

        for buffer in &mut storage.per_thread_buffers {
            buffer.get_mut().clear();
        }

        // Keep it simple, write everything to the first per-thread buffer.
        let buffer = storage.per_thread_buffers[0].get_mut();

        let event_count = unsafe { usize::deserialize(deserializer) }?;
        let data_len = unsafe { usize::deserialize(deserializer) }?;

        buffer.event_count = event_count;
        buffer.data.reserve(data_len);

        let spare_capacity = &mut buffer.data.spare_capacity_mut()[..data_len];
        let spare_capacity = cast_uninit_slice_mut(spare_capacity);
        deserializer.deserialize_into_uninit_bytes(spare_capacity)?;

        unsafe {
            buffer.data.set_len(data_len);
        }

        Ok(())
    }
}

impl SerializeMut for CommandData {
    fn serialize_mut<W>(&mut self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        let total_data_len = self
            .iter_mut()
            .map(|buffer| buffer.get_mut().len())
            .sum::<usize>();

        // Combined data len across all per-thread buffers (they are serialized
        // together as one data blob).
        total_data_len.serialize(serializer)?;

        // Event data.
        for buffer in self.as_mut_slice() {
            let data = buffer.get_mut().as_slice();
            serializer.serialize_uninit_bytes(data)?;
        }

        Ok(())
    }
}

impl Deserialize for CommandData {
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
        for buffer in self.as_mut_slice() {
            buffer.get_mut().clear();
        }

        // Keep it simple, write everything to the first per-thread buffer.
        let buffer = self[0].get_mut();

        let len = unsafe { usize::deserialize(deserializer) }?;

        buffer.resize(len, MaybeUninit::uninit());
        deserializer.deserialize_into_uninit_bytes(buffer)
    }
}
