use std::{
    borrow::Cow,
    error::Error,
    ffi::{CStr, OsStr, c_void},
    future::Future,
    io,
    mem::MaybeUninit,
    num::NonZero,
    path::Path,
    pin::Pin,
    sync::Arc,
};

use void_public::{ArgType, ComponentId, ComponentType, callable::TaskId};

pub trait Platform: Send + Sync + 'static {
    type Executor: Executor;
    type Filesystem: Filesystem;
}

pub trait Executor: Send + Sync {
    fn available_parallelism() -> NonZero<usize>;

    fn thread_index() -> usize;

    fn parallel_iter<F>(len: usize, f: F)
    where
        F: Fn(usize, usize) + Send + Sync;

    fn execute_blocking(&mut self, future: Pin<&mut (dyn Future<Output = ()> + Send)>);
}

pub trait Filesystem {
    /// Reads the bytes of a file (async).
    fn read_async<P, T: 'static>(
        path: P,
        user_data: Arc<T>,
        completion: fn(Arc<T>, io::Result<Vec<u8>>),
    ) where
        P: AsRef<Path>,
        Arc<T>: Send;
}

pub trait EcsSystemFn: Send {
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid and points to an array of
    /// pointers corresponding to the system's inputs.
    unsafe fn call(
        &mut self,
        ptr: *const *const c_void,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;
}

/// A `FnMut` which reads bytes into a provided byte buffer. Returns the number
/// of bytes read on success.
pub type DeserializeReadFn<'a> =
    &'a mut dyn FnMut(&mut [MaybeUninit<u8>]) -> Result<usize, Box<dyn Error + Send + Sync>>;

/// A `FnMut` which writes bytes from a provided byte buffer. Returns the number
/// of bytes written on success.
pub type SerializeWriteFn<'a> =
    &'a mut dyn FnMut(&[MaybeUninit<u8>]) -> Result<usize, Box<dyn Error + Send + Sync>>;

/// Interface for the engine to make use of an ECS module.
///
/// A module is a named and versioned collection of systems and components. It
/// may be used by a language integrator to add support for a new scripting
/// language, for example.
///
/// `platform_native` provides an implementation of `EcsModule` which loads
/// dynamic libraries that export a C ABI, which Rust's SDK is based on.
/// Consider reusing the same implementation for any C-based language
/// integration.
pub trait EcsModule: Sync {
    fn void_target_version(&self) -> u32;

    fn module_name(&self) -> Cow<'_, str>;

    fn set_component_id(&mut self, string_id: &CStr, component_id: ComponentId);

    fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>>;

    fn deinit(&self) -> Result<(), Box<dyn Error + Send + Sync>>;

    fn resource_init(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    fn resource_deserialize(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
        read: DeserializeReadFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    fn resource_serialize(
        &self,
        string_id: &CStr,
        val: &[MaybeUninit<u8>],
        write: SerializeWriteFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Deserialize the given JSON `json_string` into `dest_buffer`. The name of the component type is
    /// given by `string_id`. The size of the buffer is sufficient to hold the component being restored.
    fn component_deserialize_json(
        &self,
        string_id: &CStr,
        dest_buffer: &mut [MaybeUninit<u8>],
        json_string: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    fn component_string_id(&self, index: usize) -> Option<Cow<'_, CStr>>;

    fn component_size(&self, string_id: &CStr) -> usize;

    fn component_align(&self, string_id: &CStr) -> usize;

    fn component_type(&self, string_id: &CStr) -> ComponentType;

    fn component_async_completion_callable(&self, string_id: &CStr) -> Cow<'_, CStr>;

    fn systems_len(&self) -> usize;

    fn system_name(&self, system_index: usize) -> Cow<'_, CStr>;

    fn system_is_once(&self, system_index: usize) -> bool;

    fn system_fn(&self, system_index: usize) -> Box<dyn EcsSystemFn>;

    fn system_args_len(&self, system_index: usize) -> usize;

    fn system_arg_type(&self, system_index: usize, arg_index: usize) -> ArgType;

    fn system_arg_component(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr>;

    fn system_arg_event(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr>;

    fn system_query_args_len(&self, system_index: usize, arg_index: usize) -> usize;

    fn system_query_arg_type(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> ArgType;

    fn system_query_arg_component(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> Cow<'_, CStr>;
}

pub trait PlatformLibraryFn: Sync {
    /// # Safety
    ///
    /// This function calls untrusted code.
    fn call(&self, task_id: TaskId, parameter_data: &[MaybeUninit<u8>]);
}

pub trait PlatformLibrary {
    fn name(&self) -> Cow<'_, OsStr>;

    fn void_target_version(&self) -> u32;

    fn init(&mut self) -> u32;

    fn function_count(&self) -> usize;

    fn function_name(&self, function_index: usize) -> Cow<'_, CStr>;

    fn function_is_sync(&self, function_index: usize) -> bool;

    fn function(&self, function_index: usize) -> Box<dyn PlatformLibraryFn>;
}

#[cfg(feature = "test")]
pub mod test {
    use void_public::ENGINE_VERSION;

    use super::*;

    pub struct TestPlatform;

    impl Platform for TestPlatform {
        type Executor = TestExecutor;
        type Filesystem = TestFilesystem;
    }

    pub struct TestExecutor;

    impl Executor for TestExecutor {
        fn available_parallelism() -> NonZero<usize> {
            NonZero::new(1).unwrap()
        }

        fn thread_index() -> usize {
            0
        }

        fn parallel_iter<F>(len: usize, f: F)
        where
            F: Fn(usize, usize) + Send + Sync,
        {
            for i in 0..len {
                f(i, 0);
            }
        }

        fn execute_blocking(&mut self, future: Pin<&mut (dyn Future<Output = ()> + Send)>) {
            use pollster::FutureExt as _;
            future.block_on();
        }
    }

    pub struct TestFilesystem;

    impl Filesystem for TestFilesystem {
        fn read_async<P, T: 'static>(
            path: P,
            user_data: Arc<T>,
            completion: fn(Arc<T>, io::Result<Vec<u8>>),
        ) where
            P: AsRef<Path>,
            Arc<T>: Send,
        {
            completion(user_data, std::fs::read(path));
        }
    }

    pub struct TestPlatformFn;

    impl PlatformLibraryFn for TestPlatformFn {
        fn call(&self, _task_id: TaskId, _parameter_data: &[MaybeUninit<u8>]) {}
    }

    pub struct TestPlatformLibrary;

    impl PlatformLibrary for TestPlatformLibrary {
        fn name(&self) -> Cow<'_, OsStr> {
            OsStr::new("test_platform_library").into()
        }

        fn void_target_version(&self) -> u32 {
            ENGINE_VERSION
        }

        fn init(&mut self) -> u32 {
            0
        }

        fn function_count(&self) -> usize {
            0
        }

        fn function_name(&self, _function_index: usize) -> Cow<'_, CStr> {
            unreachable!()
        }

        fn function_is_sync(&self, _function_index: usize) -> bool {
            unreachable!()
        }

        fn function(&self, _function_index: usize) -> Box<dyn PlatformLibraryFn> {
            unreachable!()
        }
    }
}
