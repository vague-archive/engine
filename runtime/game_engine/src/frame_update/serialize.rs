use snapshot::{Deserialize, Deserializer, ReadUninit, Result, Serialize, Serializer, WriteUninit};

use super::*;

impl<P: Platform, G: Gpu> Serialize for FrameUpdate<P, G> {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        self.archetypes.serialize(serializer)?;
        self.system_graph.serialize(serializer)?;
        self.world.serialize(serializer)
    }
}

impl<P: Platform, G: Gpu> Deserialize for FrameUpdate<P, G> {
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
            self.archetypes.deserialize_in_place(deserializer)?;
            self.system_graph.deserialize_in_place(deserializer)?;
            self.world.deserialize_in_place(deserializer)
        }
    }
}
