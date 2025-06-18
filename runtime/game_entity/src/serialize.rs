use snapshot::{Deserialize, Deserializer, ReadUninit, Result, Serialize, Serializer, WriteUninit};

use super::*;

// Manual implementation of `Serialize` and `Deserialize` for `EntityId`, in
// order to take advantage of the `serialize_pod()` optimization on the whole
// struct.

impl Serialize for EntityId {
    fn serialize<W>(&self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        serializer.serialize_pod(self)
    }
}

impl Deserialize for EntityId {
    unsafe fn deserialize<R>(deserializer: &mut Deserializer<R>) -> Result<Self>
    where
        R: ReadUninit,
    {
        unsafe { deserializer.deserialize_pod() }
    }
}
