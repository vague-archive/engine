use std::{error::Error, ffi::c_void};

use platform::EcsSystemFn;

/// This macro statically links an ECS module with a C API. It generates an
/// `EcsModule` implementation for the module, loads procedure addresses, and
/// registers the module with the engine.
///
/// # Safety
///
/// The caller must provide a valid `load_engine_proc_addrs` C function.
#[macro_export]
macro_rules! include_module {
    ($engine:expr, $($module_path:ident)::*, $module_name:ident, $load_proc_addr_fn:expr) => (
        {
            // Manually call `load_engine_proc_addrs` for statically-linked C
            // ECS modules.
            $($module_path ::)* load_engine_proc_addrs(
                $load_proc_addr_fn
            );

            $engine.register_ecs_module(Box::new($module_name));

            pub struct $module_name;

            impl $crate::platform::EcsModule for $module_name {
                fn void_target_version(&self) -> u32 {
                    $($module_path ::)* void_target_version()
                }

                fn init(&self) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    let res = $($module_path ::)*init();

                    if res == 0 {
                        Ok(())
                    } else {
                        Err(format!("error code ({res})").into())
                    }
                }

                fn deinit(&self) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    let res = $($module_path ::)*deinit();

                    if res == 0 {
                        Ok(())
                    } else {
                        Err(format!("error code ({res})").into())
                    }
                }

                fn module_name(&self) -> ::std::borrow::Cow<'_, str> {
                    unsafe { ::std::ffi::CStr::from_ptr($($module_path ::)*module_name()).to_string_lossy() }
                }

                fn set_component_id(&mut self, string_id: &::std::ffi::CStr, component_id: $crate::void_public::ComponentId) {
                    unsafe {
                        $($module_path ::)*set_component_id(string_id.as_ptr(), component_id);
                    }
                }

                fn resource_init(
                    &self,
                    string_id: &::std::ffi::CStr,
                    val: &mut [::std::mem::MaybeUninit<u8>],
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    let res = unsafe { $($module_path ::)*resource_init(string_id.as_ptr(), val.as_mut_ptr().cast()) };

                    if res == 0 {
                        Ok(())
                    } else {
                        Err(format!("error code ({res})").into())
                    }
                }

                fn resource_deserialize(
                    &self,
                    string_id: &::std::ffi::CStr,
                    val: &mut [::std::mem::MaybeUninit<u8>],
                    read: $crate::platform::DeserializeReadFn<'_>,
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    $crate::void_public_module::resource_deserialize_ffi($($module_path ::)*resource_deserialize, string_id, val, read)
                }

                fn resource_serialize(
                    &self,
                    string_id: &::std::ffi::CStr,
                    val: &[::std::mem::MaybeUninit<u8>],
                    write: $crate::platform::SerializeWriteFn<'_>,
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    $crate::void_public_module::resource_serialize_ffi($($module_path ::)*resource_serialize, string_id, val, write)
                }

                fn component_deserialize_json(
                    &self,
                    string_id: &::std::ffi::CStr,
                    dest_buffer: &mut [::std::mem::MaybeUninit<u8>],
                    json_string: &str,
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    $crate::void_public_module::component_deserialize_json_ffi(
                        $($module_path ::)*component_deserialize_json,
                        string_id,
                        dest_buffer,
                        json_string,
                    )
                }

                fn component_string_id(&self, index: usize) -> Option<::std::borrow::Cow<'_, ::std::ffi::CStr>> {
                    unsafe {
                        let ptr = $($module_path ::)*component_string_id(index);
                        if ptr.is_null() {
                            None
                        } else {
                            Some(::std::ffi::CStr::from_ptr(ptr).into())
                        }
                    }
                }

                fn component_size(&self, string_id: &::std::ffi::CStr) -> usize {
                    unsafe { $($module_path ::)*component_size(string_id.as_ptr()) }
                }

                fn component_align(&self, string_id: &::std::ffi::CStr) -> usize {
                    unsafe { $($module_path ::)*component_align(string_id.as_ptr()) }
                }

                fn component_type(&self, string_id: &::std::ffi::CStr) -> $crate::void_public::ComponentType {
                    unsafe { $($module_path ::)*component_type(string_id.as_ptr()) }
                }

                fn component_async_completion_callable(&self, string_id: &::std::ffi::CStr) -> ::std::borrow::Cow<'_, ::std::ffi::CStr> {
                    unsafe {
                        let ptr = $($module_path ::)*component_async_completion_callable(string_id.as_ptr());
                        assert!(
                            !ptr.is_null(),
                            "component_async_completion_callable returned null"
                        );
                        ::std::ffi::CStr::from_ptr(ptr).into()
                    }
                }

                fn systems_len(&self) -> usize {
                    $($module_path ::)*systems_len()
                }

                fn system_name(&self, system_index: usize) -> ::std::borrow::Cow<'_, ::std::ffi::CStr> {
                    unsafe { ::std::ffi::CStr::from_ptr($($module_path ::)*system_name(system_index)).into() }
                }

                fn system_is_once(&self, system_index: usize) -> bool {
                    $($module_path ::)*system_is_once(system_index)
                }

                fn system_fn(&self, system_index: usize) -> Box<dyn $crate::platform::EcsSystemFn> {
                    Box::new($crate::include_module_macro::EcsSystemFnC($($module_path ::)*system_fn(system_index)))
                }

                fn system_args_len(&self, system_index: usize) -> usize {
                    $($module_path ::)*system_args_len(system_index)
                }

                fn system_arg_type(&self, system_index: usize, arg_index: usize) -> $crate::void_public::ArgType {
                    $($module_path ::)*system_arg_type(system_index, arg_index)
                }

                fn system_arg_component(&self, system_index: usize, arg_index: usize) -> ::std::borrow::Cow<'_, ::std::ffi::CStr> {
                    unsafe { ::std::ffi::CStr::from_ptr($($module_path ::)*system_arg_component(system_index, arg_index)).into() }
                }

                fn system_arg_event(&self, system_index: usize, arg_index: usize) -> ::std::borrow::Cow<'_, ::std::ffi::CStr> {
                    unsafe { ::std::ffi::CStr::from_ptr($($module_path ::)*system_arg_event(system_index, arg_index)).into() }
                }

                fn system_query_args_len(&self, system_index: usize, arg_index: usize) -> usize {
                    $($module_path ::)*system_query_args_len(system_index, arg_index)
                }

                fn system_query_arg_type(
                    &self,
                    system_index: usize,
                    arg_index: usize,
                    query_index: usize,
                ) -> $crate::void_public::ArgType {
                    $($module_path ::)*system_query_arg_type(system_index, arg_index, query_index)
                }

                fn system_query_arg_component(
                    &self,
                    system_index: usize,
                    arg_index: usize,
                    query_index: usize,
                ) -> ::std::borrow::Cow<'_, ::std::ffi::CStr> {
                    unsafe {
                        ::std::ffi::CStr::from_ptr($($module_path ::)*system_query_arg_component(
                            system_index,
                            arg_index,
                            query_index,
                        ))
                        .into()
                    }
                }
            }
        }
    )
}

pub struct EcsSystemFnC(pub unsafe extern "C" fn(*const *const c_void) -> i32);

impl EcsSystemFn for EcsSystemFnC {
    unsafe fn call(
        &mut self,
        ptr: *const *const c_void,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let res = unsafe { (self.0)(ptr) };

        if res == 0 {
            Ok(())
        } else {
            Err(format!("error code ({res})").into())
        }
    }
}
