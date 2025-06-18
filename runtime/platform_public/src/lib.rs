use std::{
    ffi::{CString, c_char, c_void},
    marker::PhantomData,
    ops::Deref,
};

use flatbuffers::{FlatBufferBuilder, Follow, Push, WIPOffset};
pub use void_public::{ENGINE_VERSION, callable::TaskId};

pub struct Engine;

impl Engine {
    /// # Safety
    ///
    /// The identifier `fully_qualified_name` (pulled from the flatbuffers codegen) **must** match `event`.
    #[inline]
    pub unsafe fn send_platform_event<T>(fully_qualified_name: &str, event: T)
    where
        T: Push,
    {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let event = builder.push(event);
        builder.finish_minimal(event);

        let ident = CString::new(fully_qualified_name).unwrap();
        let data = builder.finished_data();

        unsafe {
            _SEND_PLATFORM_EVENT_FN.unwrap_unchecked()(
                ident.as_ptr(),
                data.as_ptr().cast(),
                data.len(),
            );
        }
    }

    /// # Safety
    ///
    /// The identifier `fully_qualified_name` (pulled from the flatbuffers codegen) **must** match the event.
    pub unsafe fn send_platform_event_builder<'a, T>(
        fully_qualified_name: &str,
        f: impl FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<T>,
    ) where
        T: Follow<'a>,
    {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let val = f(&mut builder);
        builder.finish_minimal(val);

        let ident = CString::new(fully_qualified_name).unwrap();
        let data = builder.finished_data();

        unsafe {
            _SEND_PLATFORM_EVENT_FN.unwrap_unchecked()(
                ident.as_ptr(),
                data.as_ptr().cast(),
                data.len(),
            );
        }
    }
}

pub struct ParameterData<'a, T: Follow<'a>>(T::Inner);

impl<'a, T: Follow<'a>> ParameterData<'a, T> {
    pub fn new(fb: T::Inner) -> Self {
        Self(fb)
    }
}

impl<'a, T: Follow<'a>> Deref for ParameterData<'a, T> {
    type Target = T::Inner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct ReturnWriter<T> {
    task_id: u32,
    marker: PhantomData<T>,
}

impl<T> ReturnWriter<T> {
    pub fn new(task_id: TaskId) -> Self {
        Self {
            task_id,
            marker: PhantomData,
        }
    }

    #[inline]
    pub fn write(self, return_value: T)
    where
        T: Push,
    {
        let mut fbb = FlatBufferBuilder::new();
        let return_value = fbb.push(return_value);
        fbb.finish_minimal(return_value);
        let return_value_data = fbb.finished_data();

        unsafe {
            _COMPLETE_TASK_FN.unwrap_unchecked()(
                self.task_id,
                return_value_data.as_ptr().cast(),
                return_value_data.len(),
            );
        }
    }

    pub fn write_builder<'a, F>(self, f: F)
    where
        T: Follow<'a>,
        F: FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<T>,
    {
        let mut fbb = FlatBufferBuilder::new();
        let offset = f(&mut fbb);
        fbb.finish_minimal(offset);
        let return_value_data = fbb.finished_data();

        unsafe {
            _COMPLETE_TASK_FN.unwrap_unchecked()(
                self.task_id,
                return_value_data.as_ptr().cast(),
                return_value_data.len(),
            );
        }
    }
}

pub type CompletionCallbackFn = unsafe extern "C" fn(u32, *const c_void, usize);
pub static mut _COMPLETE_TASK_FN: Option<CompletionCallbackFn> = None;

pub type PlatformEventCallbackFn = unsafe extern "C" fn(*const c_char, *const c_void, usize);
pub static mut _SEND_PLATFORM_EVENT_FN: Option<PlatformEventCallbackFn> = None;
