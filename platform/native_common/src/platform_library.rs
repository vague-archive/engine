use std::{
    borrow::Cow,
    env,
    ffi::{CStr, OsStr, OsString, c_char, c_void},
    mem::MaybeUninit,
    sync::Arc,
};

use game_engine::{
    platform::{self, PlatformLibraryFn},
    void_public::callable::TaskId,
};
use gpu_web::platform_ecs::GpuEcsPlatformLibrary;
use libloading::{Library, Symbol};
use platform_public::{CompletionCallbackFn, PlatformEventCallbackFn};
use text_native::platform_ecs::TextNativePlatformLibrary;

use crate::{GameEngine, async_task_complete_callback, get_procedure, platform_event_callback};

pub fn register_platform_libraries(engine: &mut GameEngine) {
    // Here we are registering the platform libraries within the engine, as
    // opposed to those read from dynamic libraries.
    engine.register_platform_library(Box::new(GpuEcsPlatformLibrary));
    engine.register_platform_library(Box::new(TextNativePlatformLibrary));

    gpu_web::platform_ecs::set_completion_callback(async_task_complete_callback);
    gpu_web::platform_ecs::set_platform_event_callback(platform_event_callback);

    text_native::platform_ecs::set_completion_callback(async_task_complete_callback);
    text_native::platform_ecs::set_platform_event_callback(platform_event_callback);

    let Ok(dir) = env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("platform")
        .read_dir()
    else {
        log::info!("`platform` directory could not be found");
        return;
    };

    dir.flatten()
        .flat_map(|entry| unsafe { Library::new(entry.path()).map(|lib| (lib, entry.file_name())) })
        .map(|(library, name)| Box::new(unsafe { PlatformLibrary::new(name, library) }))
        .for_each(|library| engine.register_platform_library(library));
}

pub struct PlatformFn {
    func: unsafe extern "C" fn(TaskId, *const c_void, usize),

    /// We save a reference to the owning library here, so that it will not be
    /// deallocated while the system function pointer is in use.
    _library: Arc<Library>,
}

impl platform::PlatformLibraryFn for PlatformFn {
    fn call(&self, task_id: TaskId, parameter_data: &[MaybeUninit<u8>]) {
        unsafe {
            (self.func)(
                task_id,
                parameter_data.as_ptr().cast(),
                parameter_data.len(),
            );
        };
    }
}

pub struct PlatformLibrary {
    name: OsString,
    void_target_version: u32,

    set_completion_callback: Symbol<'static, unsafe extern "C" fn(CompletionCallbackFn)>,
    set_platform_event_callback: Symbol<'static, unsafe extern "C" fn(PlatformEventCallbackFn)>,
    init: Symbol<'static, extern "C" fn() -> u32>,
    function_count: Symbol<'static, extern "C" fn() -> usize>,
    function_name: Symbol<'static, extern "C" fn(usize) -> *const c_char>,
    function_is_sync: Symbol<'static, extern "C" fn(usize) -> bool>,
    function_ptr:
        Symbol<'static, extern "C" fn(usize) -> unsafe extern "C" fn(TaskId, *const c_void, usize)>,

    library: Arc<Library>,
}

impl PlatformLibrary {
    unsafe fn new(name: OsString, library: Library) -> Self {
        let void_target_version = unsafe {
            library
                .get::<unsafe extern "C" fn() -> u32>(b"void_target_version\0")
                .map(|f| f())
                .unwrap_or(0)
        };

        let set_completion_callback =
            unsafe { get_procedure(&library, c"set_completion_callback") };
        let set_platform_event_callback =
            unsafe { get_procedure(&library, c"set_platform_event_callback") };
        let init = unsafe { get_procedure(&library, c"init") };
        let function_count = unsafe { get_procedure(&library, c"function_count") };
        let function_name = unsafe { get_procedure(&library, c"function_name") };
        let function_is_sync = unsafe { get_procedure(&library, c"function_is_sync") };
        let function_ptr = unsafe { get_procedure(&library, c"function_ptr") };

        Self {
            name,
            void_target_version,
            set_completion_callback,
            set_platform_event_callback,
            init,
            function_count,
            function_name,
            function_is_sync,
            function_ptr,
            library: library.into(),
        }
    }
}

impl platform::PlatformLibrary for PlatformLibrary {
    fn name(&self) -> Cow<'_, OsStr> {
        (&self.name).into()
    }

    fn void_target_version(&self) -> u32 {
        self.void_target_version
    }

    fn init(&mut self) -> u32 {
        unsafe {
            (self.set_completion_callback)(async_task_complete_callback);
            (self.set_platform_event_callback)(platform_event_callback);
            (self.init)()
        }
    }

    fn function_count(&self) -> usize {
        (self.function_count)()
    }

    fn function_name(&self, function_index: usize) -> Cow<'_, CStr> {
        unsafe { CStr::from_ptr((self.function_name)(function_index)).into() }
    }

    fn function_is_sync(&self, function_index: usize) -> bool {
        (self.function_is_sync)(function_index)
    }

    fn function(&self, function_index: usize) -> Box<dyn PlatformLibraryFn> {
        Box::new(PlatformFn {
            func: (self.function_ptr)(function_index),
            _library: self.library.clone(),
        })
    }
}
