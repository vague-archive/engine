use snapshot::{
    Deserialize, Deserializer, ReadUninit, Result, Serialize, SerializeMut, Serializer, WriteUninit,
};

use super::*;

impl SerializeMut for Callables {
    fn serialize_mut<W>(&mut self, serializer: &mut Serializer<W>) -> Result<()>
    where
        W: WriteUninit,
    {
        self.call_queue.get_mut().unwrap().serialize(serializer)?;
        self.in_flight_tasks
            .get_mut()
            .unwrap()
            .serialize(serializer)?;
        self.completions.serialize(serializer)?;
        self.next_task_id.get_mut().serialize(serializer)
    }
}

impl Deserialize for Callables {
    unsafe fn deserialize<R>(_: &mut Deserializer<R>) -> Result<Self>
    where
        R: ReadUninit,
    {
        panic!("use deserialize_in_place()!");
    }

    unsafe fn deserialize_in_place<R>(&mut self, deserializer: &mut Deserializer<R>) -> Result<()>
    where
        R: ReadUninit,
    {
        unsafe {
            self.call_queue
                .get_mut()
                .unwrap()
                .deserialize_in_place(deserializer)?;
            self.in_flight_tasks
                .get_mut()
                .unwrap()
                .deserialize_in_place(deserializer)?;
            self.completions.deserialize_in_place(deserializer)?;
            self.next_task_id
                .get_mut()
                .deserialize_in_place(deserializer)
        }
    }
}
