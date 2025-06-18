use std::{
    borrow::Cow,
    env,
    error::Error,
    ffi::{CStr, c_char, c_void},
    mem::MaybeUninit,
    sync::Arc,
};

use game_engine::{
    c_api::get_module_api_proc_addr,
    include_module,
    platform::{DeserializeReadFn, EcsModule, EcsSystemFn, SerializeWriteFn},
    void_public::{ArgType, ComponentId, ComponentType},
};
use gpu_web::GpuWeb;
use libloading::{Library, Symbol};
use void_public_module::{
    component_deserialize_json_ffi, resource_deserialize_ffi, resource_serialize_ffi,
};

use crate::{GameEngine, Platform, get_procedure};

struct EcsSystemFnDynamic {
    func: unsafe extern "C" fn(*const *const c_void) -> i32,

    /// We save a reference to the owning library here, so that it will not be
    /// deallocated while the system function pointer is in use.
    _library: Arc<Library>,
}

impl EcsSystemFn for EcsSystemFnDynamic {
    unsafe fn call(
        &mut self,
        ptr: *const *const c_void,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let res = unsafe { (self.func)(ptr) };

        if res == 0 {
            Ok(())
        } else {
            Err(format!("error code ({res})").into())
        }
    }
}

pub fn register_ecs_modules(engine: &mut GameEngine) {
    unsafe {
        // We must initialize gpu_web's module API functions. This is a platform
        // responsibility, as the platform chooses the Gpu implementation.
        gpu_web::ecs_module::ffi::load_engine_proc_addrs(get_module_api_proc_addr_c);
    }

    unsafe {
        include_module!(
            engine,
            text_native::ecs_module::ffi,
            TextNativeModule,
            get_module_api_proc_addr_c
        );
    }

    let Ok(dir) = env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("modules")
        .read_dir()
    else {
        log::warn!("`modules` directory could not be found");
        return;
    };

    dir.flatten()
        .flat_map(|entry| unsafe { Library::new(entry.path()) })
        .map(|library| Box::new(unsafe { EcsModuleDynamic::new(library) }))
        .for_each(|module| engine.register_ecs_module(module));
}

pub struct EcsModuleDynamic {
    void_target_version: u32,

    module_name: String,

    init: Symbol<'static, extern "C" fn() -> i32>,
    deinit: Symbol<'static, extern "C" fn() -> i32>,

    resource_init:
        Symbol<'static, unsafe extern "C" fn(string_id: *const c_char, val: *mut c_void) -> i32>,
    resource_deserialize: Symbol<
        'static,
        unsafe extern "C" fn(
            string_id: *const c_char,
            val: *mut c_void,
            reader: *mut c_void,
            read: unsafe extern "C" fn(reader: *mut c_void, buf: *mut c_void, len: usize) -> isize,
        ) -> i32,
    >,
    resource_serialize: Symbol<
        'static,
        unsafe extern "C" fn(
            string_id: *const c_char,
            val: *const c_void,
            writer: *mut c_void,
            write: unsafe extern "C" fn(
                writer: *mut c_void,
                buf: *const c_void,
                len: usize,
            ) -> isize,
        ) -> i32,
    >,

    set_component_id: Symbol<'static, unsafe extern "C" fn(*const c_char, ComponentId)>,
    component_deserialize_json: Symbol<
        'static,
        unsafe extern "C" fn(
            string_id: *const c_char,
            val: *mut c_void,
            json_ptr: *const c_void,
            json_len: usize,
        ) -> i32,
    >,
    component_string_id: Symbol<'static, unsafe extern "C" fn(usize) -> *const c_char>,
    component_size: Symbol<'static, unsafe extern "C" fn(*const c_char) -> usize>,
    component_align: Symbol<'static, unsafe extern "C" fn(*const c_char) -> usize>,
    component_type: Symbol<'static, unsafe extern "C" fn(*const c_char) -> ComponentType>,
    component_async_completion_callable:
        Symbol<'static, unsafe extern "C" fn(*const c_char) -> *const c_char>,

    systems_len: Symbol<'static, unsafe extern "C" fn() -> usize>,
    system_name: Symbol<'static, unsafe extern "C" fn(usize) -> *const c_char>,
    system_is_once: Symbol<'static, unsafe extern "C" fn(usize) -> bool>,
    system_fn: Symbol<
        'static,
        unsafe extern "C" fn(usize) -> unsafe extern "C" fn(*const *const c_void) -> i32,
    >,
    system_args_len: Symbol<'static, unsafe extern "C" fn(usize) -> usize>,
    system_arg_type: Symbol<'static, unsafe extern "C" fn(usize, usize) -> ArgType>,
    system_arg_component: Symbol<'static, unsafe extern "C" fn(usize, usize) -> *const c_char>,
    system_arg_event: Symbol<'static, unsafe extern "C" fn(usize, usize) -> *const c_char>,
    system_query_args_len: Symbol<'static, unsafe extern "C" fn(usize, usize) -> usize>,
    system_query_arg_type: Symbol<'static, unsafe extern "C" fn(usize, usize, usize) -> ArgType>,
    system_query_arg_component:
        Symbol<'static, unsafe extern "C" fn(usize, usize, usize) -> *const c_char>,

    library: Arc<Library>,
}

impl EcsModuleDynamic {
    /// Create an instance of an ECS module which has been loaded from a dynamic
    /// library.
    ///
    /// # Safety
    ///
    /// The `library` must point to a valid library with a variety of functions,
    /// one of which is called here directly: `load_engine_proc_addrs`.
    ///
    /// TODO(https://www.notion.so/voidinc/Document-Module-API-1f8fa7503dbe8064a4c3f164265b7cba):
    /// Add "see <some link>" to documentation which details the requirement
    /// (call signature of each function).
    pub unsafe fn new(library: Library) -> Self {
        let void_target_version = unsafe {
            library
                .get::<unsafe extern "C" fn() -> u32>(b"void_target_version\0")
                .map(|f| f())
                .unwrap_or(0)
        };

        // Safety (here and below):
        //
        // The uses of `unsafe` are for wrapping calls raw C ABI functions.
        let init = unsafe { get_procedure(&library, c"init") };
        let deinit = unsafe { get_procedure(&library, c"deinit") };
        let module_name = unsafe {
            library
                .get::<unsafe extern "C" fn() -> *const c_char>(b"module_name\0")
                .map_or_else(
                    |_| "ERROR_FINDING_MODULENAME".to_string(),
                    |f| {
                        let c_str_ptr = f();
                        let c_str = CStr::from_ptr(c_str_ptr);
                        c_str.to_string_lossy().to_string()
                    },
                )
        };

        let resource_init = unsafe { get_procedure(&library, c"resource_init") };
        let resource_deserialize = unsafe { get_procedure(&library, c"resource_deserialize") };
        let resource_serialize = unsafe { get_procedure(&library, c"resource_serialize") };

        let set_component_id = unsafe { get_procedure(&library, c"set_component_id") };
        let component_deserialize_json =
            unsafe { get_procedure(&library, c"component_deserialize_json") };
        let component_string_id = unsafe { get_procedure(&library, c"component_string_id") };
        let component_size = unsafe { get_procedure(&library, c"component_size") };
        let component_align = unsafe { get_procedure(&library, c"component_align") };
        let component_type = unsafe { get_procedure(&library, c"component_type") };
        let component_async_completion_callable =
            unsafe { get_procedure(&library, c"component_async_completion_callable") };
        let systems_len = unsafe { get_procedure(&library, c"systems_len") };
        let system_is_once = unsafe { get_procedure(&library, c"system_is_once") };
        let system_args_len = unsafe { get_procedure(&library, c"system_args_len") };
        let system_fn = unsafe { get_procedure(&library, c"system_fn") };
        let system_name = unsafe { get_procedure(&library, c"system_name") };
        let system_arg_type = unsafe { get_procedure(&library, c"system_arg_type") };
        let system_arg_component = unsafe { get_procedure(&library, c"system_arg_component") };
        let system_query_arg_type = unsafe { get_procedure(&library, c"system_query_arg_type") };
        let system_arg_event = unsafe { get_procedure(&library, c"system_arg_event") };
        let system_query_args_len = unsafe { get_procedure(&library, c"system_query_args_len") };
        let system_query_arg_component =
            unsafe { get_procedure(&library, c"system_query_arg_component") };

        // Allow module to fetch engine function pointers.
        let load_engine_proc_addrs = unsafe {
            get_procedure::<
                unsafe extern "C" fn(unsafe extern "C" fn(*const c_char) -> *const c_void),
            >(&library, c"load_engine_proc_addrs")
        };
        unsafe { load_engine_proc_addrs(get_module_api_proc_addr_c) };

        Self {
            void_target_version,
            init,
            deinit,
            module_name,
            resource_init,
            resource_deserialize,
            resource_serialize,
            set_component_id,
            component_deserialize_json,
            component_string_id,
            component_size,
            component_align,
            component_type,
            component_async_completion_callable,
            systems_len,
            system_name,
            system_is_once,
            system_fn,
            system_args_len,
            system_arg_type,
            system_arg_component,
            system_arg_event,
            system_query_args_len,
            system_query_arg_type,
            system_query_arg_component,
            library: library.into(),
        }
    }
}

impl EcsModule for EcsModuleDynamic {
    fn void_target_version(&self) -> u32 {
        self.void_target_version
    }

    fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let res = (self.init)();

        if res == 0 {
            Ok(())
        } else {
            Err(format!("error code ({res})").into())
        }
    }

    fn deinit(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let res = (self.deinit)();

        if res == 0 {
            Ok(())
        } else {
            Err(format!("error code ({res})").into())
        }
    }

    fn module_name(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.module_name)
    }

    fn set_component_id(&mut self, string_id: &CStr, component_id: ComponentId) {
        unsafe {
            (self.set_component_id)(string_id.as_ptr(), component_id);
        }
    }

    fn resource_init(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let res = unsafe { (self.resource_init)(string_id.as_ptr(), val.as_mut_ptr().cast()) };

        if res == 0 {
            Ok(())
        } else {
            Err(format!("error code ({res})").into())
        }
    }

    fn resource_deserialize(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
        read: DeserializeReadFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        resource_deserialize_ffi(*self.resource_deserialize, string_id, val, read)
    }

    fn resource_serialize(
        &self,
        string_id: &CStr,
        val: &[MaybeUninit<u8>],
        write: SerializeWriteFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        resource_serialize_ffi(*self.resource_serialize, string_id, val, write)
    }

    fn component_deserialize_json(
        &self,
        string_id: &CStr,
        dest_buffer: &mut [MaybeUninit<u8>],
        json_string: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        component_deserialize_json_ffi(
            *self.component_deserialize_json,
            string_id,
            dest_buffer,
            json_string,
        )
    }

    fn component_string_id(&self, index: usize) -> Option<Cow<'_, CStr>> {
        unsafe {
            let ptr = (self.component_string_id)(index);
            if ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(ptr).into())
            }
        }
    }

    fn component_size(&self, string_id: &CStr) -> usize {
        unsafe { (self.component_size)(string_id.as_ptr()) }
    }

    fn component_align(&self, string_id: &CStr) -> usize {
        unsafe { (self.component_align)(string_id.as_ptr()) }
    }

    fn component_type(&self, string_id: &CStr) -> ComponentType {
        unsafe { (self.component_type)(string_id.as_ptr()) }
    }

    fn component_async_completion_callable(&self, string_id: &CStr) -> Cow<'_, CStr> {
        unsafe {
            let ptr = (self.component_async_completion_callable)(string_id.as_ptr());
            assert!(
                !ptr.is_null(),
                "component_async_completion_callable returned null"
            );
            CStr::from_ptr(ptr).into()
        }
    }

    fn systems_len(&self) -> usize {
        unsafe { (self.systems_len)() }
    }

    fn system_name(&self, system_index: usize) -> Cow<'_, CStr> {
        unsafe {
            let ptr = (self.system_name)(system_index);
            CStr::from_ptr(ptr).into()
        }
    }

    fn system_is_once(&self, system_index: usize) -> bool {
        unsafe { (self.system_is_once)(system_index) }
    }

    fn system_fn(&self, system_index: usize) -> Box<dyn EcsSystemFn> {
        let func = unsafe { (self.system_fn)(system_index) };
        Box::new(EcsSystemFnDynamic {
            func,
            _library: self.library.clone(),
        })
    }

    fn system_args_len(&self, system_index: usize) -> usize {
        unsafe { (self.system_args_len)(system_index) }
    }

    fn system_arg_type(&self, system_index: usize, arg_index: usize) -> ArgType {
        unsafe { (self.system_arg_type)(system_index, arg_index) }
    }

    fn system_arg_component(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
        unsafe { CStr::from_ptr((self.system_arg_component)(system_index, arg_index)).into() }
    }

    fn system_arg_event(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
        unsafe { CStr::from_ptr((self.system_arg_event)(system_index, arg_index)).into() }
    }

    fn system_query_args_len(&self, system_index: usize, query_index: usize) -> usize {
        unsafe { (self.system_query_args_len)(system_index, query_index) }
    }

    fn system_query_arg_type(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> ArgType {
        unsafe { (self.system_query_arg_type)(system_index, system_arg_index, query_arg_index) }
    }

    fn system_query_arg_component(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> Cow<'_, CStr> {
        unsafe {
            CStr::from_ptr((self.system_query_arg_component)(
                system_index,
                system_arg_index,
                query_arg_index,
            ))
            .into()
        }
    }
}

unsafe extern "C" fn get_module_api_proc_addr_c(proc_name: *const c_char) -> *const c_void {
    let proc_name = unsafe { CStr::from_ptr(proc_name) };
    get_module_api_proc_addr::<Platform, GpuWeb>(proc_name)
}
