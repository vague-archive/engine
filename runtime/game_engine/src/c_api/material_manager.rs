use std::{
    ffi::{CStr, CString, c_char, c_void},
    iter::zip,
    mem::{ManuallyDrop, MaybeUninit},
    path::PathBuf,
    ptr::null_mut,
    slice::{from_raw_parts, from_raw_parts_mut},
};

use game_asset::{
    ecs_module::{GpuInterface, MaterialManager, TextAssetManager},
    resource_managers::material_manager::{
        self, fixed_size_vec::FixedSizeVec, material_parameters_extension::MaterialParametersExt,
        textures::MaterialTextures, uniforms::MaterialUniforms,
    },
};
use void_public::{
    AssetPath, EventWriter, FfiVec, Vec4,
    event::graphics::NewText,
    material::{
        AsTextureDescsLenResult, AsTextureDescsResult, AsUniformValuesLenResult,
        AsUniformValuesResult, GenerateShaderTextResult, GetIdFromTextIdResult,
        LoadMaterialFromPathResult, LoadShaderTemplateFromPathResult, MaterialId,
        MaterialParameters, MaterialsResult, RegisterMaterialFromStringResult, ShaderTemplateId,
        TextureDesc, UniformNamesDefaultValuesLenResult, UniformNamesDefaultValuesResult,
        UniformValueType, UniformValueUnion, UpdateFromTextureDescsResult,
        UpdateFromUniformValuesResult, UpdateMaterialFromStringResult,
    },
    text::TextId,
};

use super::{ffi_vec_from_vec, text_asset_manager::FfiPendingText};

#[repr(transparent)]
/// We must convert the engine side [`UniformValue`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiUniformValue(void_public::material::UniformValue);

impl FfiUniformValue {
    /// This is intended to be used at the FFI boundary. Within the Rust code it is
    /// best to use the internal `UniformValue` and not this FFI `UniformValue`,
    /// which is intended to be used in C code and **MUST** have it's memory
    /// freed via a free memory method in the C API.
    pub fn from_f32<C: Into<CString>>(uniform_name: C, value: f32) -> Self {
        let uniform_name = uniform_name.into().into_raw();

        Self(void_public::material::UniformValue {
            uniform_name,
            uniform_value_type: UniformValueType::F32,
            uniform_value: UniformValueUnion { f32_value: value },
        })
    }

    pub fn from_vec4<C: Into<CString>>(uniform_name: C, value: Vec4) -> Self {
        let uniform_name = uniform_name.into().into_raw();

        Self(void_public::material::UniformValue {
            uniform_name,
            uniform_value_type: UniformValueType::Vec4,
            uniform_value: UniformValueUnion { vec4: value },
        })
    }

    #[allow(clippy::borrowed_box)] // Rust side UniformValue is backed by Box<[Vec4]>, so this clippy is not relevant to this case.
    pub fn from_array<C: Into<CString>>(uniform_name: C, value: &Box<[Vec4]>) -> Self {
        let uniform_name = uniform_name.into().into_raw();
        let vec4_vec = value.to_vec();
        let vec4_ffi_vec: FfiVec<void_public::Vec4> = ffi_vec_from_vec(vec4_vec);

        Self(void_public::material::UniformValue {
            uniform_name,
            uniform_value_type: UniformValueType::Array,
            uniform_value: UniformValueUnion {
                vec4_array: vec4_ffi_vec,
            },
        })
    }

    pub fn from_internal_uniform_value(
        uniform_name: &str,
        internal_uniform_value: &material_manager::uniforms::UniformValue,
    ) -> Self {
        let uniform_name = CString::new::<&str>(uniform_name)
            .unwrap_or_else(|_| c"uniform_name_malformed".to_owned());
        match internal_uniform_value {
            material_manager::uniforms::UniformValue::Array(uniform_var) => {
                Self::from_array(uniform_name, &uniform_var.current_value().0)
            }
            material_manager::uniforms::UniformValue::F32(uniform_var) => {
                Self::from_f32(uniform_name, *uniform_var.current_value())
            }
            material_manager::uniforms::UniformValue::Vec4(uniform_var) => {
                Self::from_vec4(uniform_name, *uniform_var.current_value())
            }
        }
    }

    pub fn internal_uniform_into_public_uniform_value(
        internal_uniform: material_manager::uniforms::UniformValue,
        uniform_name: &str,
    ) -> Self {
        let uniform_name = CString::new::<&str>(uniform_name)
            .unwrap_or_else(|_| c"uniform_name_malformed".to_owned());
        match internal_uniform {
            material_manager::uniforms::UniformValue::Array(uniform_var) => {
                Self::from_array(uniform_name, &uniform_var.current_value().0)
            }
            material_manager::uniforms::UniformValue::F32(uniform_var) => {
                Self::from_f32(uniform_name, *uniform_var.current_value())
            }
            material_manager::uniforms::UniformValue::Vec4(uniform_var) => {
                Self::from_vec4(uniform_name, *uniform_var.current_value())
            }
        }
    }

    pub fn into_internal_uniform_value(&self) -> material_manager::uniforms::UniformValue {
        match &self.0.uniform_value_type {
            void_public::material::UniformValueType::F32 => {
                unsafe { self.0.uniform_value.f32_value }.into()
            }
            void_public::material::UniformValueType::Vec4 => {
                unsafe { self.0.uniform_value.vec4 }.into()
            }
            void_public::material::UniformValueType::Array => {
                let len = unsafe { self.0.uniform_value.vec4_array.len };
                let array = unsafe { from_raw_parts(self.0.uniform_value.vec4_array.ptr, len) };
                FixedSizeVec::<Vec4>::new(array).into()
            }
        }
    }

    pub fn into_material_uniforms(
        material_id: MaterialId,
        uniform_values: &[Self],
    ) -> material_manager::uniforms::MaterialUniforms {
        MaterialUniforms::new_from_iter(
            material_id,
            uniform_values.iter().map(|uniform_value| {
                let name =
                    unsafe { CStr::from_ptr(uniform_value.0.uniform_name) }.to_string_lossy();
                let uniform_value = Self::into_internal_uniform_value(uniform_value);
                (name, uniform_value)
            }),
        )
    }

    pub fn into_inner(self) -> void_public::material::UniformValue {
        let value = ManuallyDrop::new(self);
        void_public::material::UniformValue {
            uniform_value: value.0.uniform_value,
            uniform_name: value.0.uniform_name,
            uniform_value_type: value.0.uniform_value_type,
        }
    }
}

impl From<void_public::material::UniformValue> for FfiUniformValue {
    fn from(value: void_public::material::UniformValue) -> Self {
        Self(value)
    }
}

impl AsRef<void_public::material::UniformValue> for FfiUniformValue {
    fn as_ref(&self) -> &void_public::material::UniformValue {
        &self.0
    }
}

impl Drop for FfiUniformValue {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.uniform_name.cast_mut()) };
        if matches!(self.0.uniform_value_type, UniformValueType::Array) {
            let len = unsafe { self.0.uniform_value.vec4_array.len };
            let capacity = unsafe { self.0.uniform_value.vec4_array.capacity };
            let _ =
                unsafe { Vec::from_raw_parts(self.0.uniform_value.vec4_array.ptr, len, capacity) };
        }
    }
}

#[repr(transparent)]
/// We must convert the engine side [`TextureDesc`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
pub struct FfiTextureDesc(void_public::material::TextureDesc);

impl From<void_public::material::TextureDesc> for FfiTextureDesc {
    fn from(value: void_public::material::TextureDesc) -> Self {
        Self(value)
    }
}

impl AsRef<void_public::material::TextureDesc> for FfiTextureDesc {
    fn as_ref(&self) -> &void_public::material::TextureDesc {
        &self.0
    }
}

impl Drop for FfiTextureDesc {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.name.cast_mut()) };
    }
}

/// We must convert the engine side [`Material`] types to their publically
/// facing, C structs in `void_public`. Ensuring we do not run into leaking
/// memory or deallocating memory with the incorrect allocator is tricky.
/// `into_raw` on Rust's [`CString`] API *MUST* be freed with the corresponding
/// Rust API `from_raw`. Attempting to free this memory in C with `free` is
/// undefined behavior.
///
/// It is a user's responsibility to drop this type if they have used a public
/// API to generate it. We provide functions to free these.
#[repr(transparent)]
#[derive(Debug)]
pub struct FfiMaterial(void_public::material::Material);

impl Drop for FfiMaterial {
    fn drop(&mut self) {
        let _ = unsafe { CString::from_raw(self.0.name.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.get_world_offset_body.cast_mut()) };
        let _ = unsafe { CString::from_raw(self.0.get_fragment_color_body.cast_mut()) };
        if self.0.uniform_types_with_defaults.len > 0 {
            let uniform_values = unsafe {
                Vec::from_raw_parts(
                    self.0.uniform_types_with_defaults.ptr,
                    self.0.uniform_types_with_defaults.len,
                    self.0.uniform_types_with_defaults.capacity,
                )
            };

            for uniform_value in uniform_values {
                let _ = FfiUniformValue::from(uniform_value);
            }
        }
        if self.0.texture_descs.len > 0 {
            let texture_descs = unsafe {
                Vec::from_raw_parts(
                    self.0.texture_descs.ptr,
                    self.0.texture_descs.len,
                    self.0.texture_descs.capacity,
                )
            };
            for texture_desc in texture_descs {
                let _ = FfiTextureDesc::from(texture_desc);
            }
        }
    }
}

impl From<&material_manager::materials::Material> for FfiMaterial {
    fn from(value: &material_manager::materials::Material) -> Self {
        let uniform_types_with_defaults: FfiVec<void_public::material::UniformValue> = match value
            .uniform_types_with_defaults()
        {
            Some(material_uniforms) => ffi_vec_from_vec(
                material_uniforms
                    .iter()
                    .map(|(uniform_name, uniform)| {
                        FfiUniformValue::from_internal_uniform_value(uniform_name.as_ref(), uniform)
                            .into_inner()
                    })
                    .collect::<Vec<_>>(),
            ),
            None => ffi_vec_from_vec(vec![]),
        };
        let texture_descs: FfiVec<TextureDesc> = match value.texture_descs() {
            Some(texture_descs) => ffi_vec_from_vec(
                texture_descs
                    .iter()
                    .map(|(texture_material_spec, texture_id)| {
                        texture_material_spec.into_public_texture_desc(*texture_id)
                    })
                    .collect::<Vec<_>>(),
            ),
            None => ffi_vec_from_vec(vec![]),
        };
        let ffi_material = void_public::material::Material {
            material_id: value.material_id(),
            shader_template_id: value.shader_template_id(),
            name: CString::new(value.name())
                .unwrap_or_else(|_| c"material_name_malformed".to_owned())
                .into_raw(),
            uniform_types_with_defaults,
            texture_descs,
            get_world_offset_body: CString::new(value.world_offset_body())
                .unwrap_or_else(|_| c"world_offset_body_malformed".to_owned())
                .into_raw(),
            get_fragment_color_body: CString::new(value.fragment_color_body())
                .unwrap_or_else(|_| c"fragment_color_body_malformed".to_owned())
                .into_raw(),
        };
        Self(ffi_material)
    }
}

/// Gives a list of FFI version of [`Material`]s, returns the length of the
/// array.
///
/// # Safety
///
/// * `material_manager` must be a valid pointer to the [`Resource`]
///   [`MaterialManager`]
pub unsafe extern "C" fn material_manager_materials_len(
    material_manager: *const MaterialManager,
) -> usize {
    unsafe { material_manager.as_ref() }
        .unwrap()
        .materials()
        .len()
}

/// Gives a list of FFI version of [`Material`]s, returns the length of the
/// array.
///
/// # Errors
///
/// * If the user's `expected_len` does match the actual length, we do not
///   allocate the array and return 1.
///
/// # Safety
///
/// * `material_manager` must be a valid pointer to the [`Resource`].
/// * `materials` must be allocated for the Ffi type [`Material`] to the length
///   of materials. The `material_manager_materials_len` function should provide
///   this, and this function should return a failure if the lengths do not match.
pub unsafe extern "C" fn material_manager_materials(
    material_manager: *const MaterialManager,
    materials: *mut MaybeUninit<FfiMaterial>,
    materials_len: usize,
) -> MaterialsResult {
    let material_manager = unsafe { material_manager.as_ref().unwrap() };
    let count = material_manager.materials().len();
    if count != materials_len {
        return MaterialsResult::IncorrectLen;
    }
    let output_array_slice = unsafe { from_raw_parts_mut(materials, count) };

    for (output, material) in zip(output_array_slice, material_manager.materials()) {
        output.write(material.into());
    }

    MaterialsResult::Success
}

/// # Safety
///
/// The pointer must be of type [`FfiMaterial`].
pub unsafe extern "C" fn material_manager_free_material(ptr: *mut FfiMaterial) {
    if ptr.is_null() {
        return;
    }

    unsafe { ptr.read() };
}

/// # Safety
///
/// The pointer must be of type [`FfiUniformValue`].
pub unsafe extern "C" fn material_manager_free_uniform_value(ptr: *mut FfiUniformValue) {
    if ptr.is_null() {
        return;
    }

    unsafe { ptr.read() };
}

/// # Safety
///
/// The pointer must be of type [`FfiTextureDesc`].
pub unsafe extern "C" fn material_manager_free_texture_desc(ptr: *mut FfiTextureDesc) {
    if ptr.is_null() {
        return;
    }

    unsafe { ptr.read() };
}

/// # Safety
///
/// The pointer must be a c style string that *was originally generated in a
/// Rust function.* Do not use this for `malloc`ed c style strings.
pub unsafe extern "C" fn free_rust_generated_c_str(ptr: *mut c_char) {
    let _ = unsafe { CString::from_raw(ptr) };
}

/// Begins the process of asynchronously loading a shader template from a .fsh
/// text file. The user should pass in a [`PendingText`] pointer, which they can
/// use with the [`TextAssetManager`] to find out when the text has loaded.
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`]. This pointer
/// is equivalent to Rust's mutable reference, so no other references to
/// [`MaterialManager`] should be alive during this call.
/// `new_text_event_writer_handle` should point to a valid
/// [`EventWriter<NewText>`] handle. `text_asset_manager` must point to a valid
/// [`TextAssetManager`]. This pointer is equivalent to Rust's mutable
/// reference, so no other references to [`TextAssetManager`] should be alive
/// during this call.
pub unsafe extern "C" fn material_manager_load_shader_template_from_path(
    material_manager: *mut MaterialManager,
    name: *const c_char,
    path: *const c_char,
    new_text_event_writer_handle: *const c_void,
    text_asset_manger: *mut TextAssetManager,
    output_pending_text: *mut MaybeUninit<FfiPendingText>,
) -> LoadShaderTemplateFromPathResult {
    let new_text_event_writer =
        unsafe { EventWriter::<NewText<'_>>::new(new_text_event_writer_handle) };
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let shader_template_path =
        PathBuf::from(unsafe { CStr::from_ptr(path) }.to_string_lossy().as_ref());
    let Some(material_manager) = (unsafe { material_manager.as_mut() }) else {
        return LoadShaderTemplateFromPathResult::MaterialManagerNull;
    };
    let Some(text_asset_manager) = (unsafe { text_asset_manger.as_mut() }) else {
        return LoadShaderTemplateFromPathResult::TextAssetManagerNull;
    };
    let Some(output_pending_text) = (unsafe { output_pending_text.as_mut() }) else {
        return LoadShaderTemplateFromPathResult::OutputPendingTextNull;
    };

    match material_manager.load_shader_template_from_path(
        name.as_ref(),
        &AssetPath::from(shader_template_path),
        &new_text_event_writer,
        text_asset_manager,
    ) {
        Ok(pending_text) => {
            let pending_text: FfiPendingText = pending_text.into();
            output_pending_text.write(pending_text);
            LoadShaderTemplateFromPathResult::Success
        }
        Err(err) => {
            log::error!("Error loading shader template {name} from path: {err}");
            LoadShaderTemplateFromPathResult::ShaderTemplateLoadError
        }
    }
}

/// Begins the process of asychronously loading a material from a TOML
/// definition file. The user should pass in a [`PendingText`] pointer, which they can
/// use with the [`TextAssetManager`] to find out when the text has loaded.
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`]. This pointer
/// is equivalent to Rust's mutable reference, so no other references to
/// [`MaterialManager`] should be alive during this call.
/// `new_text_event_writer_handle` should point to a valid
/// [`EventWriter<NewText>`] handle. `text_asset_manager` must point to a valid
/// [`TextAssetManager`]. This pointer is equivalent to Rust's mutable
/// reference, so no other references to [`TextAssetManager`] should be alive
/// during this call.
pub unsafe extern "C" fn material_manager_load_material_from_path(
    material_manager: *mut MaterialManager,
    shader_template_id: ShaderTemplateId,
    material_name: *const c_char,
    path: *const c_char,
    set_up_watcher: bool,
    new_text_event_writer_handle: *const c_void,
    text_asset_manger: *mut TextAssetManager,
    output_pending_text: *mut MaybeUninit<FfiPendingText>,
) -> LoadMaterialFromPathResult {
    let new_text_event_writer =
        unsafe { EventWriter::<NewText<'_>>::new(new_text_event_writer_handle) };
    let material_name = unsafe { CStr::from_ptr(material_name) }.to_string_lossy();
    let material_definition_path =
        PathBuf::from(unsafe { CStr::from_ptr(path) }.to_string_lossy().as_ref());
    let Some(material_manager) = (unsafe { material_manager.as_mut() }) else {
        return LoadMaterialFromPathResult::MaterialManagerNull;
    };
    let Some(text_asset_manager) = (unsafe { text_asset_manger.as_mut() }) else {
        return LoadMaterialFromPathResult::TextAssetManagerNull;
    };
    let Some(output_pending_text) = (unsafe { output_pending_text.as_mut() }) else {
        return LoadMaterialFromPathResult::OutputPendingTextNull;
    };

    match material_manager.load_material_from_path(
        shader_template_id,
        material_name.as_ref(),
        &material_definition_path.into(),
        set_up_watcher,
        &new_text_event_writer,
        text_asset_manager,
    ) {
        Ok(pending_text) => {
            let pending_text = pending_text.into();
            output_pending_text.write(pending_text);
            LoadMaterialFromPathResult::Success
        }
        Err(err) => {
            log::error!("Error loading material definition {err}");
            LoadMaterialFromPathResult::MaterialLoadError
        }
    }
}

/// This registers a material from a string.
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`]. This pointer
/// is equivalent to Rust's mutable reference, so no other references to
/// [`MaterialManager`] should live. `material_name` and `toml_string` should be
/// nul terminated c strings. `output_material_id` should be a pointer to
/// [`MaterialId`].
pub unsafe extern "C" fn material_manager_register_material_from_string(
    material_manager: *mut MaterialManager,
    shader_template_id: ShaderTemplateId,
    material_name: *const c_char,
    toml_string: *const c_char,
    output_material_id: *mut MaybeUninit<MaterialId>,
) -> RegisterMaterialFromStringResult {
    let material_name = unsafe { CStr::from_ptr(material_name) }.to_string_lossy();
    let toml_string = unsafe { CStr::from_ptr(toml_string) }.to_string_lossy();
    let Some(material_manager) = (unsafe { material_manager.as_mut() }) else {
        return RegisterMaterialFromStringResult::MaterialManagerNull;
    };
    let Some(output_material_id) = (unsafe { output_material_id.as_mut() }) else {
        return RegisterMaterialFromStringResult::OutputMaterialIdNull;
    };

    match material_manager.register_material_from_string(
        shader_template_id,
        material_name.as_ref(),
        toml_string.as_ref(),
    ) {
        Ok(material_id) => {
            output_material_id.write(material_id);
            RegisterMaterialFromStringResult::Success
        }
        Err(err) => {
            log::error!("Error registering material from string: {err}");
            RegisterMaterialFromStringResult::RegisterMaterialError
        }
    }
}

/// This gets the length of the uniform names for creating an uninitialized array to
/// be passed into `material_manager_uniform_names_and_default_values`.
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`].
/// `len` should be a pointer to [`usize`].
pub unsafe extern "C" fn material_manager_uniform_names_and_default_values_len(
    material_manager: *const MaterialManager,
    material_id: MaterialId,
    output_len: *mut MaybeUninit<usize>,
) -> UniformNamesDefaultValuesLenResult {
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return UniformNamesDefaultValuesLenResult::MaterialManagerNull;
    };
    let Some(output_len) = (unsafe { output_len.as_mut() }) else {
        return UniformNamesDefaultValuesLenResult::OutputLenNull;
    };

    let Ok(uniform_names) = material_manager.uniform_names_and_default_values(material_id) else {
        return UniformNamesDefaultValuesLenResult::IdNotFound;
    };

    match uniform_names {
        Some(uniform_names) => {
            output_len.write(uniform_names.len());
            UniformNamesDefaultValuesLenResult::Success
        }
        None => UniformNamesDefaultValuesLenResult::NoUniforms,
    }
}

/// Gets the uniform names and the default values for a [`Material`].
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`].
/// should be a pointer to [`usize`].
pub unsafe extern "C" fn material_manager_uniform_names_and_default_values(
    material_manager: *const MaterialManager,
    material_id: MaterialId,
    uniform_values_array: *mut MaybeUninit<FfiUniformValue>,
    expected_len: usize,
) -> UniformNamesDefaultValuesResult {
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return UniformNamesDefaultValuesResult::MaterialManagerNull;
    };
    let uniform_values_array = unsafe { from_raw_parts_mut(uniform_values_array, expected_len) };

    let Ok(uniform_names) = material_manager.uniform_names_and_default_values(material_id) else {
        return UniformNamesDefaultValuesResult::IdNotFound;
    };

    match uniform_names {
        Some(uniform_names) => {
            if uniform_names.len() != expected_len {
                log::error!(
                    "Attempted to get uniform names and default values for id {material_id}, but output array is len {expected_len} but actual len is {}",
                    uniform_names.len()
                );
                return UniformNamesDefaultValuesResult::InputArrayIncorrectLen;
            }
            for (ffi_uniform_value, (uniform_name, uniform_value)) in
                zip(uniform_values_array, uniform_names.iter())
            {
                ffi_uniform_value.write(FfiUniformValue::from_internal_uniform_value(
                    uniform_name,
                    uniform_value,
                ));
            }
            UniformNamesDefaultValuesResult::Success
        }
        None => UniformNamesDefaultValuesResult::NoUniforms,
    }
}

/// Generates the shader text for a [`Material`].
///
/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`]. `result` must
/// point to a valid [`GenerateShaderTextResult`]. `output_len` must point to a
/// valid [`usize`]. The output from this function must be freed with
/// [`free_rust_generated_c_str`], do **not** use C's free.
pub unsafe extern "C" fn material_manager_generate_shader_text(
    material_manager: *const MaterialManager,
    material_id: MaterialId,
    output_string: *mut MaybeUninit<*const c_char>,
    output_len: *mut MaybeUninit<usize>,
) -> GenerateShaderTextResult {
    let Some(output_string) = (unsafe { output_string.as_mut() }) else {
        return GenerateShaderTextResult::OutputStringNull;
    };
    let Some(output_len) = (unsafe { output_len.as_mut() }) else {
        return GenerateShaderTextResult::OutputLenNull;
    };
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        output_len.write(0);
        return GenerateShaderTextResult::MaterialManagerNull;
    };

    match material_manager.generate_shader_text(material_id) {
        Ok(shader_text) => {
            let Ok(shader_text) = CString::new(&*shader_text) else {
                output_len.write(0);
                return GenerateShaderTextResult::ErrorConvertingTextToCString;
            };
            let len = shader_text.as_bytes_with_nul().len();
            output_string.write(shader_text.into_raw());
            output_len.write(len);
            GenerateShaderTextResult::Success
        }
        Err(err) => {
            log::error!("Error generating shader text {err}");
            output_len.write(0);
            GenerateShaderTextResult::ErrorConvertingTextToCString
        }
    }
}

/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`].
/// `output_material_id` should be a pointer to [`MaterialId`].
pub unsafe extern "C" fn material_manager_get_id_from_text_id(
    material_manager: *const MaterialManager,
    text_id: TextId,
    output_material_id: *mut MaybeUninit<MaterialId>,
) -> GetIdFromTextIdResult {
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return GetIdFromTextIdResult::MaterialManagerNull;
    };
    let Some(output_material_id) = (unsafe { output_material_id.as_mut() }) else {
        return GetIdFromTextIdResult::OutputMaterialIdNull;
    };

    match material_manager.get_material_id_from_text_id(text_id) {
        Some(material_id) => {
            output_material_id.write(*material_id);
            GetIdFromTextIdResult::Success
        }
        None => GetIdFromTextIdResult::TextIdNotFound,
    }
}

/// # Safety
///
/// `material_manager` must point to a valid [`MaterialManager`]. This pointer
/// is equivalent to Rust's mutable reference, so no other references to
/// [`MaterialManager`] should live. `name` and `material_toml_str` should
/// either be a null pointer or a valid nul terminated c style string.
pub unsafe extern "C" fn material_manager_update_material_from_string(
    material_manager: *mut MaterialManager,
    material_id: MaterialId,
    name: *const c_char,
    material_toml_str: *const c_char,
) -> UpdateMaterialFromStringResult {
    let Some(material_manager) = (unsafe { material_manager.as_mut() }) else {
        return UpdateMaterialFromStringResult::MaterialManagerNull;
    };

    if name.is_null() && material_toml_str.is_null() {
        return UpdateMaterialFromStringResult::NameAndTomlNull;
    }

    let name = if name.is_null() {
        None
    } else {
        let name = unsafe { CStr::from_ptr(name.cast_mut()) }.to_string_lossy();
        Some(name.to_string())
    };

    let material_toml_str = if material_toml_str.is_null() {
        None
    } else {
        let material_toml_str =
            unsafe { CStr::from_ptr(material_toml_str.cast_mut()) }.to_string_lossy();
        Some(material_toml_str.to_string())
    };

    match material_manager.update_material_from_string(
        material_id,
        name.as_deref(),
        material_toml_str.as_deref(),
    ) {
        Ok(_) => UpdateMaterialFromStringResult::Success,
        Err(err) => {
            log::error!("Error updating material {material_id}: {err}");
            UpdateMaterialFromStringResult::ErrorUpdatingMaterial
        }
    }
}

/// Gets the len of [`UniformValue`]s for a given [`MaterialParameters`].
///
/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `material_manager` must point to a valid [`MaterialManager`].
/// `output_len` should be a pointer to `usize`.
pub unsafe extern "C" fn material_params_as_uniform_values_len(
    material_parameters: *const MaterialParameters,
    material_manager: *const MaterialManager,
    output_len: *mut MaybeUninit<usize>,
) -> AsUniformValuesLenResult {
    let Some(material_paramaters) = (unsafe { material_parameters.as_ref() }) else {
        return AsUniformValuesLenResult::MaterialParametersNull;
    };
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return AsUniformValuesLenResult::MaterialManagerNull;
    };
    let Some(output_len) = (unsafe { output_len.as_mut() }) else {
        return AsUniformValuesLenResult::OutputLenNull;
    };

    match material_paramaters.as_material_uniforms(material_manager) {
        Ok(material_uniforms) => {
            output_len.write(material_uniforms.len());
            AsUniformValuesLenResult::Success
        }
        Err(err) => {
            log::error!(
                "Error converting MaterialParameters buffer to Material Uniforms for len: {err}"
            );
            AsUniformValuesLenResult::ErrorInAsMaterialUniforms
        }
    }
}

/// Gets the [`UniformValue`]s for a given [`MaterialParameters`].
///
/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `material_manager` must point to a valid [`MaterialManager`].
/// `output_len` should be a pointer to `usize`.
pub unsafe extern "C" fn material_params_as_uniform_values(
    material_parameters: *const MaterialParameters,
    material_manager: *const MaterialManager,
    expected_len: usize,
    output_uniform_values: *mut MaybeUninit<FfiUniformValue>,
) -> AsUniformValuesResult {
    let Some(material_paramaters) = (unsafe { material_parameters.as_ref() }) else {
        return AsUniformValuesResult::MaterialParametersNull;
    };
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return AsUniformValuesResult::MaterialManagerNull;
    };
    let output_uniform_values = unsafe { from_raw_parts_mut(output_uniform_values, expected_len) };

    match material_paramaters.as_material_uniforms(material_manager) {
        Ok(material_uniforms) => {
            let found_len = material_uniforms.len();
            if expected_len != found_len {
                log::error!(
                    "Expected a len of {expected_len} for MaterialParameters, but actual length is {found_len}"
                );
                return AsUniformValuesResult::InputArrayIncorrectLen;
            }
            for (output_uniform_value, (uniform_name, uniform_value)) in
                zip(output_uniform_values, material_uniforms.iter())
            {
                output_uniform_value.write(FfiUniformValue::from_internal_uniform_value(
                    uniform_name,
                    uniform_value,
                ));
            }
            AsUniformValuesResult::Success
        }
        Err(err) => {
            log::error!(
                "Error converting MaterialParameters buffer to Material Uniforms for len: {err}"
            );
            AsUniformValuesResult::ErrorInAsMaterialUniforms
        }
    }
}

/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `uniform_values` must point to an array of [`FfiUniformValue`]s and must be
/// of len `uniform_values_len`.
pub unsafe extern "C" fn material_params_update_from_uniform_values(
    material_parameters: *mut MaterialParameters,
    uniform_values: *const FfiUniformValue,
    uniform_values_len: usize,
) -> UpdateFromUniformValuesResult {
    let Some(material_parameters) = (unsafe { material_parameters.as_mut() }) else {
        return UpdateFromUniformValuesResult::MaterialParametersNull;
    };

    let uniform_values = unsafe { from_raw_parts(uniform_values, uniform_values_len) };
    let material_uniforms =
        FfiUniformValue::into_material_uniforms(material_parameters.material_id(), uniform_values);

    match material_parameters.update_from_material_uniforms(&material_uniforms) {
        Ok(_) => UpdateFromUniformValuesResult::Success,
        Err(err) => {
            log::error!("Error updating MaterialParameters from MaterialUniforms: {err}");
            UpdateFromUniformValuesResult::UpdateFailed
        }
    }
}

/// Gets the len of [`UniformValue`]s for a given [`MaterialParameters`].
///
/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `material_manager` must point to a valid [`MaterialManager`].
/// `output_len` should be a pointer to `usize`.
pub unsafe extern "C" fn material_params_as_texture_descs_len(
    material_parameters: *const MaterialParameters,
    material_manager: *const MaterialManager,
    output_len: *mut MaybeUninit<usize>,
) -> AsTextureDescsLenResult {
    let Some(material_paramaters) = (unsafe { material_parameters.as_ref() }) else {
        return AsTextureDescsLenResult::MaterialParametersNull;
    };
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return AsTextureDescsLenResult::MaterialManagerNull;
    };
    let Some(output_len) = (unsafe { output_len.as_mut() }) else {
        return AsTextureDescsLenResult::OutputLenNull;
    };

    match material_paramaters.as_material_textures(material_manager) {
        Ok(material_textures) => {
            output_len.write(material_textures.len());
            AsTextureDescsLenResult::Success
        }
        Err(err) => {
            log::error!(
                "Error converting MaterialParameters buffer to Material Uniforms for len: {err}"
            );
            AsTextureDescsLenResult::ErrorInAsTextureDescs
        }
    }
}

/// Gets the [`FfiTextureDesc`]s for a given [`MaterialParameters`].
///
/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `material_manager` must point to a valid [`MaterialManager`].
/// `output_len` should be a pointer to `usize`.
pub unsafe extern "C" fn material_params_as_texture_descs(
    material_parameters: *const MaterialParameters,
    material_manager: *const MaterialManager,
    expected_len: usize,
    output_texture_descs: *mut MaybeUninit<FfiTextureDesc>,
) -> AsTextureDescsResult {
    let Some(material_paramaters) = (unsafe { material_parameters.as_ref() }) else {
        return AsTextureDescsResult::MaterialParametersNull;
    };
    let Some(material_manager) = (unsafe { material_manager.as_ref() }) else {
        return AsTextureDescsResult::MaterialManagerNull;
    };
    let output_texture_descs = unsafe { from_raw_parts_mut(output_texture_descs, expected_len) };

    match material_paramaters.as_material_textures(material_manager) {
        Ok(material_textures) => {
            let found_len = material_textures.len();
            if expected_len != found_len {
                log::error!(
                    "Expected a len of {expected_len} for MaterialParameters, but actual length is {found_len}"
                );
                return AsTextureDescsResult::InputArrayIncorrectLen;
            }
            for (output_texture_desc, (material_spec, texture_id)) in
                zip(output_texture_descs, material_textures.iter())
            {
                output_texture_desc.write(FfiTextureDesc(
                    material_spec.into_public_texture_desc(*texture_id),
                ));
            }
            AsTextureDescsResult::Success
        }
        Err(err) => {
            log::error!(
                "Error converting MaterialParameters buffer to Material Uniforms for len: {err}"
            );
            AsTextureDescsResult::ErrorInAsTextureDescs
        }
    }
}

/// # Safety
///
/// `material_parameters` must point to a valid [`MaterialParameters`].
/// `texture_descs` must point to an array of [`FfiTextureDesc`]s and must be
/// of len `texture_descs_len`.
pub unsafe extern "C" fn material_params_update_from_texture_descs(
    material_parameters: *mut MaterialParameters,
    texture_descs: *const FfiTextureDesc,
    texture_descs_len: usize,
) -> UpdateFromTextureDescsResult {
    let Some(material_parameters) = (unsafe { material_parameters.as_mut() }) else {
        return UpdateFromTextureDescsResult::MaterialParametersNull;
    };

    let texture_descs = unsafe { from_raw_parts(texture_descs, texture_descs_len) };
    let material_textures = MaterialTextures::from_public_texture_descs(
        material_parameters.material_id(),
        texture_descs,
    );

    match material_parameters.update_from_material_textures(&material_textures) {
        Ok(_) => UpdateFromTextureDescsResult::Success,
        Err(err) => {
            log::error!("Error updating MaterialParameters from MaterialUniforms: {err}");
            UpdateFromTextureDescsResult::UpdateFailed
        }
    }
}

/// # Safety
///
/// `gpu_interface` can be a null pointer, but then it will return a null
/// pointer. However, the lifetime of [`MaterialManger`] must be the same
/// or less than [`GpuInterface`].
pub unsafe extern "C" fn gpu_interface_get_material_manager_mut(
    gpu_interface: *mut GpuInterface,
) -> *mut MaterialManager {
    if gpu_interface.is_null() {
        return null_mut();
    }
    let gpu_interface = unsafe { gpu_interface.as_mut().unwrap() };
    &mut gpu_interface.material_manager as *mut MaterialManager
}
