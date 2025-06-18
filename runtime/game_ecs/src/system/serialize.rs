use snapshot::{Deserialize, Deserializer, ReadUninit, Result, Serialize, Serializer, WriteUninit};

use super::*;

impl<P: Platform, G: GpuFrameData> Serialize for SystemGraph<P, G> {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        let mut serialize = |systems: &[SystemInfo<P, G>]| {
            systems.len().serialize(serializer)?;

            for system in systems {
                system.system.name().serialize(serializer)?;
                system.enabled.serialize(serializer)?;
            }

            Ok(())
        };

        serialize(&self.cpu_systems)?;
        serialize(&self.gpu_systems)
    }
}

impl<P: Platform, G: GpuFrameData> Deserialize for SystemGraph<P, G> {
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
        // The loaded systems may have changed, so only apply deserialized state
        // if we find a matching current system. Note that we still need to
        // deserialize all serialized systems, even if we don't apply their
        // state, so that we don't corrupt the deserialization read pointer.

        let mut deserialize = |systems: &mut [SystemInfo<P, G>]| {
            let len = unsafe { usize::deserialize(deserializer) }?;

            for _ in 0..len {
                let name = unsafe { String::deserialize(deserializer) }?;
                let enabled = unsafe { bool::deserialize(deserializer) }?;

                if let Some(system) = systems
                    .iter_mut()
                    .find(|system| system.system.name() == name)
                {
                    system.enabled = enabled;
                }
            }

            Ok(())
        };

        deserialize(&mut self.cpu_systems)?;
        deserialize(&mut self.gpu_systems)
    }
}
