use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    error::Error,
    ffi::{CStr, CString, c_void},
    mem::{MaybeUninit, transmute},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    ops::{Deref, DerefMut},
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex, MutexGuard},
};

use deno_core::{ExtensionFileSource, ModuleSpecifier, include_js_files};
use deno_resolver::npm::{DenoInNpmPackageChecker, NpmResolver};
use deno_runtime::{
    BootstrapOptions,
    deno_fs::RealFs,
    deno_napi::v8,
    deno_permissions::{Permissions, PermissionsContainer, PermissionsOptions},
    inspector_server::InspectorServer,
    permissions::RuntimePermissionDescriptorParser,
    worker::{MainWorker, WorkerOptions, WorkerServiceOptions},
};
use game_engine::{
    platform::{DeserializeReadFn, EcsModule, EcsSystemFn, SerializeWriteFn},
    void_public::{ArgType, ComponentId, ComponentType},
};
use sys_traits::impls::RealSys;
use tokio::runtime::Runtime as TokioRuntime;

use crate::{
    GameEngine,
    deno_op::{SharedState, fiasco},
    typescript_loader::TypescriptModuleLoader,
};

#[derive(Debug)]
pub struct JsOptions {
    pub start_js_inspector: bool,
    pub js_inspector_port: u16,
    pub modules_dir: String,
}

impl Default for JsOptions {
    fn default() -> Self {
        Self {
            start_js_inspector: false,
            js_inspector_port: 8080,
            modules_dir: "modules".to_owned(),
        }
    }
}

pub fn register_js_ecs_modules(
    engine: &mut GameEngine,
    isolate: &Arc<SyncIsolate>,
    tokio_runtime: &TokioRuntime,
    env_args: &JsOptions,
) {
    let Ok(dir) = env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(&env_args.modules_dir)
        .read_dir()
    else {
        log::warn!("`modules` directory could not be found");
        return;
    };

    dir.flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "js"))
        .map(|entry| {
            isolate
                .lock()
                .load_module(&entry.path(), isolate.clone(), tokio_runtime)
        })
        .for_each(|module| engine.register_ecs_module(module));
}

pub struct SyncIsolate {
    inner: Mutex<SyncIsolateInner>,
    _inspector_server: Option<Arc<InspectorServer>>,
}

// This makes the entrypoint.ts file available at: `module:fiasco-entry`
pub const EXTENSION_API: &[ExtensionFileSource] =
    &include_js_files!(fiasco_entry dir "js/src", "module:fiasco-entry" = "entrypoint.ts");

impl SyncIsolate {
    pub fn new(env_args: &JsOptions, tokio_runtime: &TokioRuntime) -> Self {
        // The main module is the entrypoint to the file that includes all the Deno
        // extensions that get registered globally. Main module has no reference to
        // ECS modules - ECS modules are loaded as side modules.
        let main_module = deno_core::resolve_url(EXTENSION_API[0].specifier).unwrap();

        let maybe_inspector_server = if env_args.start_js_inspector {
            let inspector_adress: SocketAddr = SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                env_args.js_inspector_port,
            );

            Some(Arc::new(
                InspectorServer::new(inspector_adress, "Fiasco Inspector Server").unwrap(),
            ))
        } else {
            None
        };

        let options = WorkerOptions {
            maybe_inspector_server: maybe_inspector_server.clone(),
            bootstrap: BootstrapOptions {
                inspect: env_args.start_js_inspector,
                ..BootstrapOptions::default()
            },
            extensions: vec![fiasco::init_ops_and_esm()],
            ..WorkerOptions::default()
        };

        let fs: Arc<RealFs> = Arc::new(RealFs);

        let permission_parser = Arc::new(RuntimePermissionDescriptorParser::new(RealSys));

        // Do not allow access to anything except FFI: no file System, no env vars, no network requests, etc..
        let permission_options = PermissionsOptions {
            allow_ffi: Some(Default::default()),
            ..PermissionsOptions::default()
        };

        let perms = Permissions::from_options(&*permission_parser, &permission_options).unwrap();
        let permissions = PermissionsContainer::new(permission_parser, perms);

        let services: WorkerServiceOptions<DenoInNpmPackageChecker, NpmResolver<RealSys>, RealSys> =
            WorkerServiceOptions {
                module_loader: Rc::new(TypescriptModuleLoader { fs: fs.clone() }),
                permissions,
                blob_store: Default::default(),
                broadcast_channel: Default::default(),
                feature_checker: Default::default(),
                node_services: Default::default(),
                npm_process_state_provider: Default::default(),
                root_cert_store_provider: Default::default(),
                fetch_dns_resolver: Default::default(),
                shared_array_buffer_store: Default::default(),
                compiled_wasm_module_store: Default::default(),
                v8_code_cache: Default::default(),
                fs,
            };

        let mut main_worker = MainWorker::bootstrap_from_options(&main_module, services, options);

        tokio_runtime
            .block_on(main_worker.execute_main_module(&main_module))
            .unwrap();

        let mut inner = SyncIsolateInner {
            main_worker,
            js_modules: HashMap::new(),
        };

        unsafe {
            // Exit the isolate. Every subsequent access will call `enter()`.
            inner.main_worker.js_runtime.v8_isolate().exit();
        }

        Self {
            inner: inner.into(),
            _inspector_server: maybe_inspector_server,
        }
    }

    pub fn lock(&self) -> IsolateGuard<'_> {
        let mut isolate = self.inner.lock().unwrap();

        unsafe {
            // This enters the isolate on this thread, allowing it to be called.
            // The `IsolateGuard` drop implementation exits the isolate.
            isolate.main_worker.js_runtime.v8_isolate().enter();
        }

        IsolateGuard(isolate)
    }
}

impl Drop for SyncIsolate {
    fn drop(&mut self) {
        unsafe {
            // The v8 isolate expects to be entered when it is dropped.
            self.inner
                .get_mut()
                .unwrap()
                .main_worker
                .js_runtime
                .v8_isolate()
                .enter();
        }
    }
}

struct JSModule {
    /// The default export of the js module. Represents the ECS Module as a JS Object.
    default: v8::Global<v8::Object>,
    /// All the JS system functions in the module.
    system_funcs: Vec<v8::Global<v8::Function>>,
}

/// This is a sendable Deno `MainWorker`, along with any globals or other v8
/// objects which are `!Send + !Sync`. It is unsound to create anywhere except
/// within `SyncIsolate`, which provides thread synchronization and handles
/// calling `enter()` and `exit()` on the v8 isolate.
pub struct SyncIsolateInner {
    main_worker: MainWorker,
    js_modules: HashMap<usize, JSModule>,
}

unsafe impl Send for SyncIsolateInner {}

impl SyncIsolateInner {
    pub fn run_event_loop(&mut self, tokio_runtime: &TokioRuntime) {
        tokio_runtime
            .block_on(self.main_worker.run_event_loop(false))
            .unwrap();
    }

    /// `&mut self` and `Arc<SyncIsolate>` both refer to the same isolate. We
    /// only pass the `Arc` separately so that we can store it in the module.
    pub fn load_module(
        &mut self,
        path: &Path,
        isolate: Arc<SyncIsolate>,
        tokio_runtime: &TokioRuntime,
    ) -> Box<dyn EcsModule> {
        let specifier = &ModuleSpecifier::from_file_path(path).unwrap();

        let module_id = tokio_runtime.block_on(async {
            let runtime = &mut self.main_worker.js_runtime;
            let id = runtime.load_side_es_module(specifier).await.unwrap();
            runtime.mod_evaluate(id).await.unwrap();

            let module_namespace = runtime.get_module_namespace(id).unwrap();

            let op_state = runtime.op_state();
            let scope = &mut runtime.handle_scope();
            let default = get_default(scope, &module_namespace);

            let op_state = unsafe { op_state.try_borrow_unguarded() }.unwrap();
            let state = op_state.borrow::<SharedState>();

            let set_engine_args = &[v8::Local::new(scope, state.engine.as_ref().unwrap()).into()];
            get_js_fn("setEngine", scope, &default)
                .call(scope, default.into(), set_engine_args)
                .unwrap();

            // Get a count of all the systems in the ecs module so we can
            // call the systemFunction for each and cache the resulting function.
            let systems_count = get_js_fn("systemsLen", scope, &default)
                .call(scope, default.into(), &[])
                .and_then(|value| value.uint32_value(scope))
                .unwrap();

            let mut system_funcs =
                Vec::<v8::Global<v8::Function>>::with_capacity(systems_count as usize);

            for i in 0..systems_count {
                let args = &[v8::Integer::new_from_unsigned(scope, i).into()];

                let system_func = get_js_fn("systemFunction", scope, &default)
                    .call(scope, default.into(), args)
                    .map(|value| value.cast::<v8::Function>())
                    .unwrap();

                system_funcs.push(v8::Global::new(scope, system_func));
            }

            let js_module = JSModule {
                default: v8::Global::new(scope, default),
                system_funcs,
            };

            self.js_modules.insert(id, js_module);
            id
        });

        let module = JsEcsModule { isolate, module_id };

        Box::new(module)
    }

    fn scope_default(
        &mut self,
        module_id: usize,
    ) -> (v8::HandleScope<'_>, v8::Local<'_, v8::Object>) {
        let js_module = self.js_modules.get(&module_id).unwrap();
        let mut scope = self.main_worker.js_runtime.handle_scope();
        let default = v8::Local::new(&mut scope, &js_module.default);
        (scope, default)
    }

    fn module_name(&mut self, module_id: usize) -> Cow<'_, str> {
        let (mut scope, default) = self.scope_default(module_id);

        get_js_fn("moduleName", &mut scope, &default)
            .call(&mut scope, default.into(), &[])
            .map(|value| value.to_rust_string_lossy(&mut scope).into())
            .unwrap()
    }

    fn void_target_version(&mut self, module_id: usize) -> u32 {
        let (mut scope, default) = self.scope_default(module_id);

        get_js_fn("voidTargetVersion", &mut scope, &default)
            .call(&mut scope, default.into(), &[])
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap()
    }

    fn set_component_id(&mut self, module_id: usize, string_id: &CStr, component_id: ComponentId) {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::String::new(&mut scope, string_id.to_str().unwrap())
                .unwrap()
                .into(),
            v8::Number::new(&mut scope, component_id.get().into()).into(),
        ];

        get_js_fn("setComponentId", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .unwrap();
    }

    fn init(&mut self, module_id: usize) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut scope, default) = self.scope_default(module_id);

        get_js_fn("init", &mut scope, &default)
            .call(&mut scope, default.into(), &[])
            .map(|_| ())
            .ok_or("failed to call init".into())
    }

    fn deinit(&mut self, module_id: usize) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut scope, default) = self.scope_default(module_id);

        get_js_fn("deinit", &mut scope, &default)
            .call(&mut scope, default.into(), &[])
            .map(|_| ())
            .ok_or("failed to call deinit".into())
    }

    fn resource_init(
        &mut self,
        module_id: usize,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut scope, default) = self.scope_default(module_id);

        let ptr: *mut c_void = val.as_mut_ptr().cast::<c_void>();
        let args = &[
            v8::String::new(&mut scope, string_id.to_str().unwrap())
                .unwrap()
                .into(),
            v8::External::new(&mut scope, ptr).into(),
        ];

        get_js_fn("resourceInit", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|_| ())
            .ok_or("failed to call resourceInit".into())
    }

    fn component_deserialize_json(
        &mut self,
        module_id: usize,
        string_id: &CStr,
        dest_buffer: &mut [MaybeUninit<u8>],
        json_string: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut scope, default) = self.scope_default(module_id);

        let ptr = dest_buffer.as_mut_ptr().cast::<c_void>();
        let args = &[
            v8::String::new(&mut scope, string_id.to_str().unwrap())
                .unwrap()
                .into(),
            v8::External::new(&mut scope, ptr).into(),
            v8::String::new(&mut scope, json_string).unwrap().into(),
        ];

        get_js_fn("componentDeserializeJson", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|_| ())
            .ok_or("failed to call componentDeserializeJson".into())
    }

    fn resource_deserialize(
        &mut self,
        module_id: usize,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
        read: DeserializeReadFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Read length.
        let mut len_bytes = [MaybeUninit::uninit(); size_of::<u32>()];
        let bytes_read = read(&mut len_bytes)?;
        if bytes_read != len_bytes.len() {
            return Err(format!("u32: read {bytes_read} of {} bytes", len_bytes.len()).into());
        }
        let len_bytes = unsafe {
            transmute::<[MaybeUninit<u8>; size_of::<u32>()], [u8; size_of::<u32>()]>(len_bytes)
        };
        let len = u32::from_ne_bytes(len_bytes) as usize;

        // Read data.
        let mut buf = vec![MaybeUninit::uninit(); len];
        let bytes_read = read(&mut buf)?;
        if bytes_read != len {
            return Err(format!("{string_id:?}: read {bytes_read} of {} bytes", len).into());
        }
        let buf = unsafe { transmute::<Vec<MaybeUninit<u8>>, Vec<u8>>(buf) };

        let string_id = string_id.to_str().unwrap();

        let (mut scope, default) = self.scope_default(module_id);

        let store = v8::ArrayBuffer::new_backing_store_from_bytes(buf).make_shared();

        let args = &[
            v8::String::new(&mut scope, string_id).unwrap().into(),
            v8::External::new(&mut scope, val.as_ptr() as *mut c_void).into(),
            v8::ArrayBuffer::with_backing_store(&mut scope, &store).into(),
        ];

        get_js_fn("resourceDeserialize", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|_| ())
            .ok_or("failed to call resourceDeserialize".into())
    }

    fn resource_serialize(
        &mut self,
        module_id: usize,
        string_id: &CStr,
        val: &[MaybeUninit<u8>],
        write: SerializeWriteFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (mut scope, default) = self.scope_default(module_id);

        let string_id = string_id.to_str().unwrap();

        let args = &[
            v8::String::new(&mut scope, string_id).unwrap().into(),
            v8::External::new(&mut scope, val.as_ptr() as *mut c_void).into(),
        ];

        let array_buffer: v8::Local<'_, v8::ArrayBuffer> =
            get_js_fn("resourceSerialize", &mut scope, &default)
                .call(&mut scope, default.into(), args)
                .and_then(|value| value.try_into().ok())
                .unwrap();

        let byte_ptr = array_buffer.data().unwrap().as_ptr() as *const u8;
        let buf = unsafe { std::slice::from_raw_parts(byte_ptr, array_buffer.byte_length()) };

        // Write length, so that JS doesn't have to worry about reading it back.
        let len_bytes = (buf.len() as u32).to_ne_bytes();
        let bytes_written = write(slice_as_uninit(&len_bytes))?;
        if bytes_written != len_bytes.len() {
            return Err(format!("u32: wrote {bytes_written} of {} bytes", len_bytes.len()).into());
        }

        // Write data.
        let bytes_written = write(slice_as_uninit(buf))?;
        if bytes_written == buf.len() {
            Ok(())
        } else {
            Err(format!(
                "{string_id:?}: wrote {bytes_written} of {} bytes",
                buf.len()
            )
            .into())
        }
    }

    fn component_size(&mut self, module_id: usize, string_id: &CStr) -> usize {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::String::new(&mut scope, string_id.to_str().unwrap())
            .unwrap()
            .into()];

        get_js_fn("componentSize", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap() as usize
    }

    fn component_align(&mut self, module_id: usize, string_id: &CStr) -> usize {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::String::new(&mut scope, string_id.to_str().unwrap())
            .unwrap()
            .into()];

        get_js_fn("componentAlign", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap() as usize
    }

    fn component_type(&mut self, module_id: usize, string_id: &CStr) -> ComponentType {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::String::new(&mut scope, string_id.to_str().unwrap())
            .unwrap()
            .into()];

        get_js_fn("componentType", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .map(|value| match value {
                0 => ComponentType::AsyncCompletion,
                1 => ComponentType::Component,
                2 => ComponentType::Resource,
                _ => panic!(
                    "unknown component type {} returned from componentType",
                    value
                ),
            })
            .unwrap()
    }

    fn component_string_id(&mut self, module_id: usize, index: usize) -> Option<Cow<'_, CStr>> {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::Integer::new_from_unsigned(&mut scope, index as u32).into()];

        get_js_fn("componentStringId", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| {
                if value.is_undefined() {
                    None
                } else {
                    Some(value.to_rust_string_lossy(&mut scope))
                }
            })
            .map(|value| CString::new(value).unwrap().into())
    }

    fn systems_len(&mut self, module_id: usize) -> usize {
        let (mut scope, default) = self.scope_default(module_id);

        get_js_fn("systemsLen", &mut scope, &default)
            .call(&mut scope, default.into(), &[])
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap() as usize
    }

    fn system_name(&mut self, module_id: usize, system_index: usize) -> Cow<'_, CStr> {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into()];

        get_js_fn("systemName", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|value| value.to_rust_string_lossy(&mut scope))
            .map(|value| CString::new(value).unwrap().into())
            .unwrap()
    }

    fn system_is_once(&mut self, module_id: usize, system_index: usize) -> bool {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into()];

        get_js_fn("systemIsOnce", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|value| value.boolean_value(&mut scope))
            .unwrap()
    }

    fn system_args_len(&mut self, module_id: usize, system_index: usize) -> usize {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into()];

        get_js_fn("systemArgsLen", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap() as usize
    }

    fn system_arg_type(
        &mut self,
        module_id: usize,
        system_index: usize,
        arg_index: usize,
    ) -> ArgType {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, arg_index as u32).into(),
        ];

        get_js_fn("systemArgType", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .map(|value| match value {
                0 => ArgType::Completion,
                1 => ArgType::DataAccessMut,
                2 => ArgType::DataAccessRef,
                3 => ArgType::EventReader,
                4 => ArgType::EventWriter,
                5 => ArgType::Query,
                _ => panic!("unknown arg type {} returned from systemArgType", value),
            })
            .unwrap()
    }

    fn system_arg_component(
        &mut self,
        module_id: usize,
        system_index: usize,
        arg_index: usize,
    ) -> Cow<'_, CStr> {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, arg_index as u32).into(),
        ];

        get_js_fn("systemArgComponent", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|value| value.to_rust_string_lossy(&mut scope))
            .map(|value| CString::new(value).unwrap().into())
            .unwrap()
    }

    fn system_arg_event(
        &mut self,
        module_id: usize,
        system_index: usize,
        arg_index: usize,
    ) -> Cow<'_, CStr> {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, arg_index as u32).into(),
        ];

        get_js_fn("systemArgEvent", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|value| value.to_rust_string_lossy(&mut scope))
            .map(|value| CString::new(value).unwrap().into())
            .unwrap()
    }

    fn system_query_args_len(
        &mut self,
        module_id: usize,
        system_index: usize,
        arg_index: usize,
    ) -> usize {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, arg_index as u32).into(),
        ];

        get_js_fn("systemQueryArgsLen", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .unwrap() as usize
    }

    fn system_query_arg_type(
        &mut self,
        module_id: usize,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> ArgType {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, system_arg_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, query_arg_index as u32).into(),
        ];

        get_js_fn("systemQueryArgType", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .and_then(|value| value.uint32_value(&mut scope))
            .map(|value| match value {
                0 => ArgType::Completion,
                1 => ArgType::DataAccessMut,
                2 => ArgType::DataAccessRef,
                3 => ArgType::EventReader,
                4 => ArgType::EventWriter,
                5 => ArgType::Query,
                _ => panic!("unknown arg type {} returned from systemArgType", value),
            })
            .unwrap()
    }

    fn system_query_arg_component(
        &mut self,
        module_id: usize,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> Cow<'_, CStr> {
        let (mut scope, default) = self.scope_default(module_id);

        let args = &[
            v8::Integer::new_from_unsigned(&mut scope, system_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, system_arg_index as u32).into(),
            v8::Integer::new_from_unsigned(&mut scope, query_arg_index as u32).into(),
        ];

        get_js_fn("systemQueryArgComponent", &mut scope, &default)
            .call(&mut scope, default.into(), args)
            .map(|value| value.to_rust_string_lossy(&mut scope))
            .map(|value| CString::new(value).unwrap().into())
            .unwrap()
    }

    pub fn run_system(
        &mut self,
        ptr: *const *const c_void,
        module_id: usize,
        system_index: usize,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Run system.

        let res = {
            let js_module = self.js_modules.get(&module_id).unwrap();
            let scope = &mut self.main_worker.js_runtime.handle_scope();

            let undefined = v8::undefined(scope);
            let system_func = v8::Local::new(scope, &js_module.system_funcs[system_index]);
            let args = &[v8::External::new(scope, ptr as *mut c_void).into()];

            system_func
                .call(scope, undefined.into(), args)
                .map(|_| ())
                .ok_or(format!("failed to call systemFunction({system_index})").into())
        };

        // Adjust amount of external allocated memory.

        let op_state = self.main_worker.js_runtime.op_state();
        let mut op_state = op_state.borrow_mut();
        let shared_state = op_state.borrow_mut::<SharedState>();

        let change_in_bytes = shared_state.change_in_external_memory;
        shared_state.change_in_external_memory = 0;

        self.main_worker
            .js_runtime
            .v8_isolate()
            .adjust_amount_of_external_allocated_memory(change_in_bytes as i64);

        res
    }
}

pub fn slice_as_uninit<T>(v: &[T]) -> &[MaybeUninit<T>] {
    unsafe { transmute::<&[T], &[MaybeUninit<T>]>(v) }
}

/// Returns the default export of a JS file, example: The `foo` in `export default foo`.
fn get_default<'a>(
    scope: &mut v8::HandleScope<'a>,
    module_namespace: &v8::Global<v8::Object>,
) -> v8::Local<'a, v8::Object> {
    let local = v8::Local::new(scope, module_namespace);
    let default_name = v8::String::new(scope, "default").unwrap();
    let default_object = local.get(scope, default_name.into()).unwrap();
    v8::Local::<v8::Object>::try_from(default_object).unwrap()
}

/// Takes an object and string representing a key on the object.
/// Returns the value for the given key, expecting it to be a function.
fn get_js_fn<'a>(
    str: &str,
    scope: &mut v8::HandleScope<'a>,
    obj: &v8::Local<'_, v8::Object>,
) -> v8::Local<'a, v8::Function> {
    v8::String::new(scope, str)
        .and_then(|id| obj.get(scope, id.into()))
        .map_or_else(
            || panic!("Module expected to have key '{str}' with type Function"),
            |value| value.cast::<v8::Function>(),
        )
}

pub struct IsolateGuard<'a>(MutexGuard<'a, SyncIsolateInner>);

impl Drop for IsolateGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.main_worker.js_runtime.v8_isolate().exit();
        }
    }
}

impl Deref for IsolateGuard<'_> {
    type Target = SyncIsolateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for IsolateGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct JsEcsModule {
    isolate: Arc<SyncIsolate>,
    module_id: usize,
}

impl EcsModule for JsEcsModule {
    fn void_target_version(&self) -> u32 {
        self.isolate.lock().void_target_version(self.module_id)
    }

    fn module_name(&self) -> Cow<'_, str> {
        self.isolate
            .lock()
            .module_name(self.module_id)
            .into_owned()
            .into()
    }

    fn set_component_id(&mut self, string_id: &CStr, component_id: ComponentId) {
        self.isolate
            .lock()
            .set_component_id(self.module_id, string_id, component_id);
    }

    fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate.lock().init(self.module_id)
    }

    fn deinit(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate.lock().deinit(self.module_id)
    }

    fn resource_init(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate
            .lock()
            .resource_init(self.module_id, string_id, val)
    }

    fn component_deserialize_json(
        &self,
        string_id: &CStr,
        dest_buffer: &mut [MaybeUninit<u8>],
        json_string: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate.lock().component_deserialize_json(
            self.module_id,
            string_id,
            dest_buffer,
            json_string,
        )
    }

    fn resource_deserialize(
        &self,
        string_id: &CStr,
        val: &mut [MaybeUninit<u8>],
        read: DeserializeReadFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate
            .lock()
            .resource_deserialize(self.module_id, string_id, val, read)
    }

    fn resource_serialize(
        &self,
        string_id: &CStr,
        val: &[MaybeUninit<u8>],
        write: SerializeWriteFn<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate
            .lock()
            .resource_serialize(self.module_id, string_id, val, write)
    }

    fn component_string_id(&self, index: usize) -> Option<Cow<'_, CStr>> {
        self.isolate
            .lock()
            .component_string_id(self.module_id, index)
            .map(|id| id.into_owned().into())
    }

    fn component_size(&self, string_id: &CStr) -> usize {
        self.isolate
            .lock()
            .component_size(self.module_id, string_id)
    }

    fn component_align(&self, string_id: &CStr) -> usize {
        self.isolate
            .lock()
            .component_align(self.module_id, string_id)
    }

    fn component_type(&self, string_id: &CStr) -> ComponentType {
        self.isolate
            .lock()
            .component_type(self.module_id, string_id)
    }

    fn component_async_completion_callable(&self, string_id: &CStr) -> Cow<'_, CStr> {
        string_id.to_owned().into()
    }

    fn systems_len(&self) -> usize {
        self.isolate.lock().systems_len(self.module_id)
    }

    fn system_name(&self, system_index: usize) -> Cow<'_, CStr> {
        self.isolate
            .lock()
            .system_name(self.module_id, system_index)
            .into_owned()
            .into()
    }

    fn system_is_once(&self, system_index: usize) -> bool {
        self.isolate
            .lock()
            .system_is_once(self.module_id, system_index)
    }

    fn system_fn(&self, system_index: usize) -> Box<dyn EcsSystemFn> {
        let system = JsEcsSystemFn {
            isolate: self.isolate.clone(),
            module_id: self.module_id,
            system_index,
        };

        Box::new(system)
    }

    fn system_args_len(&self, system_index: usize) -> usize {
        self.isolate
            .lock()
            .system_args_len(self.module_id, system_index)
    }

    fn system_arg_type(&self, system_index: usize, arg_index: usize) -> ArgType {
        self.isolate
            .lock()
            .system_arg_type(self.module_id, system_index, arg_index)
    }

    fn system_arg_component(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
        self.isolate
            .lock()
            .system_arg_component(self.module_id, system_index, arg_index)
            .into_owned()
            .into()
    }

    fn system_arg_event(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
        self.isolate
            .lock()
            .system_arg_event(self.module_id, system_index, arg_index)
            .into_owned()
            .into()
    }

    fn system_query_args_len(&self, system_index: usize, arg_index: usize) -> usize {
        self.isolate
            .lock()
            .system_query_args_len(self.module_id, system_index, arg_index)
    }

    fn system_query_arg_type(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> ArgType {
        self.isolate.lock().system_query_arg_type(
            self.module_id,
            system_index,
            system_arg_index,
            query_arg_index,
        )
    }

    fn system_query_arg_component(
        &self,
        system_index: usize,
        system_arg_index: usize,
        query_arg_index: usize,
    ) -> Cow<'_, CStr> {
        self.isolate
            .lock()
            .system_query_arg_component(
                self.module_id,
                system_index,
                system_arg_index,
                query_arg_index,
            )
            .into_owned()
            .into()
    }
}

pub struct JsEcsSystemFn {
    isolate: Arc<SyncIsolate>,
    module_id: usize,
    system_index: usize,
}

impl EcsSystemFn for JsEcsSystemFn {
    unsafe fn call(
        &mut self,
        ptr: *const *const c_void,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.isolate
            .lock()
            .run_system(ptr, self.module_id, self.system_index)
    }
}

#[cfg(test)]
mod test {
    use std::thread;

    use super::*;

    fn test_options() -> JsOptions {
        JsOptions {
            start_js_inspector: false,
            js_inspector_port: 0,
            modules_dir: "modules".into(),
        }
    }

    #[test]
    fn isolate_lifecycle_single_thread_drop() {
        let runtime = TokioRuntime::new().unwrap();

        // Create and drop isolate on the same thread.
        SyncIsolate::new(&test_options(), &runtime);
    }

    #[test]
    fn isolate_lifecycle_multi_thread_drop() {
        let runtime = TokioRuntime::new().unwrap();

        // Create an isolate and drop it on a different thread.
        let isolate = SyncIsolate::new(&test_options(), &runtime);

        thread::spawn(move || {
            drop(isolate);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn isolate_lifecycle_multi_thread_access() {
        let runtime = TokioRuntime::new().unwrap();

        let isolate = Arc::new(SyncIsolate::new(&test_options(), &runtime));

        let isolate2 = isolate.clone();
        thread::spawn(move || {
            // Access the isolate on a different thread.
            isolate2.lock();
        })
        .join()
        .unwrap();

        // Drop the isolate on the original thread.
    }
}
