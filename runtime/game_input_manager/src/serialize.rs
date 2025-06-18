use snapshot::{Deserialize, Deserializer, ReadUninit, Result, Serialize, Serializer, WriteUninit};

use super::*;

impl Serialize for InputManager {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        serializer.serialize_pod_vec(&self.buffer)?;
        self.gamepads.serialize(serializer)
    }
}

impl Deserialize for InputManager {
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
            deserializer.deserialize_pod_vec_in_place(&mut self.buffer)?;
            self.gamepads.deserialize_in_place(deserializer)
        }
    }
}
