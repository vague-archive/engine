use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, quote_spanned};
use snapshot_derive_impl::{derive_deserialize_impl, derive_serialize_impl};
use syn::{
    DeriveInput, ExprLit, FnArg, Ident, ItemFn, Lit, LitStr, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

#[proc_macro_derive(EcsType)]
pub fn derive_ecs_type(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);

    let cid = Ident::new(
        &format!("_{}_CID", ident.to_string().to_uppercase()),
        Span::call_site(),
    );

    let sid = LitStr::new(&ident.to_string(), Span::call_site());

    quote!(
        static mut #cid: Option<ComponentId> = None;

        impl EcsType for #ident {
            fn id() -> ComponentId {
                unsafe { #cid.expect("ComponentId unassigned") }
            }

            fn set_id(id: ComponentId) {
                unsafe {
                    #cid = Some(id);
                }
            }

            fn string_id() -> &'static std::ffi::CStr {
                unsafe { ::std::ffi::CStr::from_bytes_with_nul_unchecked(concat!(module_path!(), "::", #sid, "\0").as_bytes()) }
            }
        }
    )
    .into()
}

#[proc_macro_derive(Component)]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, attrs, .. } = parse_macro_input!(input);

    let cid = Ident::new(
        &format!("_{}_CID", ident.to_string().to_uppercase()),
        Span::call_site(),
    );

    let sid = LitStr::new(&ident.to_string(), Span::call_site());

    let mut proper_repr_found = false;
    for attr in attrs {
        if attr.path().is_ident("repr") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("C") || meta.path.is_ident("transparent") {
                    proper_repr_found = true;
                }
                Ok(())
            });
            if proper_repr_found {
                break;
            }
        }
    }

    if !proper_repr_found {
        let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();
        if crate_name.contains("void_public") {
            let span = Span::call_site();
            return quote_spanned! {
                span => compile_error!("Every public item deriving Component must use #[repr(C)] or #[repr(transparent)]. (simply append #[repr(C)] or #[repr(transparent)] to your struct, enum or union). This will ensure a consistent ABI.")
            }.into();
        }
    }

    quote!(
        static mut #cid: Option<ComponentId> = None;

        impl Component for #ident {}

        impl EcsType for #ident {
            fn id() -> ComponentId {
                unsafe { #cid.expect("ComponentId unassigned") }
            }

            unsafe fn set_id(id: ComponentId) {
                unsafe {
                    #cid = Some(id);
                }
            }

            fn string_id() -> &'static std::ffi::CStr {
                unsafe { ::std::ffi::CStr::from_bytes_with_nul_unchecked(concat!(module_path!(), "::", #sid, "\0").as_bytes()) }
            }
        }

        impl Copy for #ident {}

        // Ignoring for the QOL of developers not having to manually implement these required
        // traits for Component
        #[allow(clippy::expl_impl_clone_on_copy)]
        impl Clone for #ident {
            fn clone(&self) -> Self {
                *self
            }
        }
    )
    .into()
}

#[proc_macro_derive(Resource)]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let deserialize_impl = derive_deserialize_impl(input.clone().into());
    let serialize_impl = derive_serialize_impl(input.clone().into());

    let DeriveInput { ident, .. } = parse_macro_input!(input);

    let cid = Ident::new(
        &format!("_{}_CID", ident.to_string().to_uppercase()),
        Span::call_site(),
    );

    let sid = LitStr::new(&ident.to_string(), Span::call_site());

    quote! {
        #deserialize_impl
        #serialize_impl

        static mut #cid: Option<ComponentId> = None;

        impl Resource for #ident {
            fn new() -> Self {
                Self::default()
            }
        }

        impl EcsType for #ident {
            fn id() -> ComponentId {
                unsafe { #cid.expect("ComponentId unassigned") }
            }

            unsafe fn set_id(id: ComponentId) {
                unsafe {
                    #cid = Some(id);
                }
            }

            fn string_id() -> &'static std::ffi::CStr {
                unsafe { ::std::ffi::CStr::from_bytes_with_nul_unchecked(concat!(module_path!(), "::", #sid, "\0").as_bytes()) }
            }
        }
    }
    .into()
}

/// This version of `Resource` is used internally to allow resources to opt out
/// of state snapshot serialization.
///
/// This should eventually be made private to the engine.
/// GitHub issue: <https://github.com/vaguevoid/engine/issues/310>
#[proc_macro_derive(ResourceWithoutSerialize)]
pub fn derive_resource_without_serialize(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident, generics, ..
    } = parse_macro_input!(input);

    let cid = Ident::new(
        &format!("_{}_CID", ident.to_string().to_uppercase()),
        Span::call_site(),
    );

    let sid = LitStr::new(&ident.to_string(), Span::call_site());

    let generic_type_idents = generics.type_params().map(|param| &param.ident);
    let generic_type_idents2 = generics.type_params().map(|param| &param.ident);

    quote! {
        static mut #cid: Option<ComponentId> = None;

        impl Resource for #ident {
            fn new() -> Self {
                Self::default()
            }
        }

        impl EcsType for #ident {
            fn id() -> ComponentId {
                unsafe { #cid.expect("ComponentId unassigned") }
            }

            unsafe fn set_id(id: ComponentId) {
                unsafe {
                    #cid = Some(id);
                }
            }

            fn string_id() -> &'static std::ffi::CStr {
                unsafe { ::std::ffi::CStr::from_bytes_with_nul_unchecked(concat!(module_path!(), "::", #sid, "\0").as_bytes()) }
            }
        }

        impl #generics ::snapshot::Serialize for #ident <#(#generic_type_idents,)*> {
            #[inline]
            fn serialize<W>(&self, _: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
            where
                W: ::snapshot::WriteUninit,
            {
                Ok(())
            }
        }

        impl #generics ::snapshot::Deserialize for #ident <#(#generic_type_idents2,)*> {
            #[inline]
            unsafe fn deserialize<R>(_: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<Self>
            where
                R: ::snapshot::ReadUninit,
            {
                panic!("use deserialize_in_place()!")
            }

            #[inline]
            unsafe fn deserialize_in_place<R>(&mut self, _: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<()>
            where
            R: ::snapshot::ReadUninit,
            {
                Ok(())
            }
        }
    }
    .into()
}

/// This macro exists because the `serde` implementation for the `glam`
/// structures only allows support for the JSON array representation. (e.g.
/// `"my_field": [0,0,0]`). The JS side requires the object representation (e.g.
/// `"my_field"`: { "x": 0, "y": 0, "z": 0 }) so we need to wrap the `glam`
/// structures in an explicit access wrapper so that we can have complete
/// control over its serialization and deserialization.
#[proc_macro_derive(Vector)]
pub fn derive_vector_impl(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);
    quote!(
        impl #ident {
            pub const ZERO: Self = Self(glam::#ident::ZERO);

            #[inline(always)]
            pub const fn new(value: glam::#ident) -> Self {
                Self(value)
            }

            #[inline]
            pub const fn splat(value: f32) -> Self {
                Self(glam::#ident::splat(value))
            }
        }

        impl ::std::ops::Deref for #ident {
            type Target = glam::#ident;
            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl ::std::ops::DerefMut for #ident {
            #[inline(always)]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl From<glam::#ident> for #ident {
            #[inline]
            fn from(value: glam::#ident) -> Self {
                Self(value)
            }
        }

        impl From<#ident> for glam::#ident {
            #[inline]
            fn from(value: #ident) -> Self {
                value.0
            }
        }

        impl ::std::ops::Add<#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn add(self, rhs: #ident) -> Self::Output {
                (*self + *rhs).into()
            }
        }

        impl ::std::ops::Add<&#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn add(self, rhs: &#ident) -> Self::Output {
                self + (*rhs)
            }
        }

        impl ::std::ops::Add<#ident> for &#ident {
            type Output = #ident;
            #[inline]
            fn add(self, rhs: #ident) -> Self::Output {
                *self + rhs
            }
        }

        impl ::std::ops::Add<&#ident> for &#ident {
            type Output = #ident;
            #[inline]
            fn add(self, rhs: &#ident) -> Self::Output {
                (*self + *rhs).into()
            }
        }

        impl ::std::ops::AddAssign<#ident> for #ident {
            #[inline]
            fn add_assign(&mut self, rhs: #ident) {
                self.0.add_assign(*rhs)
            }
        }

        impl ::std::ops::AddAssign<&#ident> for #ident {
            #[inline]
            fn add_assign(&mut self, rhs: &#ident) {
                self.0.add_assign(**rhs)
            }
        }

        impl ::std::ops::AddAssign<glam::#ident> for #ident {
            #[inline]
            fn add_assign(&mut self, rhs: glam::#ident) {
                self.0.add_assign(rhs)
            }
        }

        impl ::std::ops::AddAssign<&glam::#ident> for #ident {
            #[inline]
            fn add_assign(&mut self, rhs: &glam::#ident) {
                self.0.add_assign(rhs)
            }
        }

        impl ::std::ops::Sub<#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn sub(self, rhs: #ident) -> Self::Output {
                (*self - *rhs).into()
            }
        }

        impl ::std::ops::Sub<&#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn sub(self, rhs: &#ident) -> Self::Output {
                self - *rhs
            }
        }

        impl ::std::ops::Sub<#ident> for &#ident {
            type Output = #ident;
            #[inline]
            fn sub(self, rhs: #ident) -> Self::Output {
                *self - rhs
            }
        }

        impl ::std::ops::Sub<&#ident> for &#ident {
            type Output = #ident;
            #[inline]
            fn sub(self, rhs: &#ident) -> Self::Output {
                *self - *rhs
            }
        }

        impl ::std::ops::SubAssign<#ident> for #ident {
            #[inline]
            fn sub_assign(&mut self, rhs: #ident) {
                self.0.sub_assign(*rhs)
            }
        }

        impl ::std::ops::SubAssign<&#ident> for #ident {
            #[inline]
            fn sub_assign(&mut self, rhs: &#ident) {
                self.0.sub_assign(**rhs)
            }
        }

        impl ::std::ops::SubAssign<glam::#ident> for #ident {
            #[inline]
            fn sub_assign(&mut self, rhs: glam::#ident) {
                self.0.sub_assign(rhs)
            }
        }

        impl ::std::ops::SubAssign<&glam::#ident> for #ident {
            #[inline]
            fn sub_assign(&mut self, rhs: &glam::#ident) {
                self.0.sub_assign(rhs)
            }
        }

        impl ::std::ops::Mul<f32> for #ident {
            type Output = #ident;
            #[inline]
            fn mul(self, rhs: f32) -> Self::Output {
                (*self * rhs).into()
            }
        }

        impl ::std::ops::Mul<f32> for &#ident {
            type Output = #ident;
            #[inline]
            fn mul(self, rhs: f32) -> Self::Output {
                *self * rhs
            }
        }

        impl ::std::ops::MulAssign<f32> for #ident {
            #[inline]
            fn mul_assign(&mut self, rhs: f32) {
                self.0.mul_assign(rhs)
            }
        }

        impl ::std::ops::MulAssign<&f32> for #ident {
            #[inline]
            fn mul_assign(&mut self, rhs: &f32) {
                self.0.mul_assign(*rhs)
            }
        }

        impl ::std::ops::Div<f32> for #ident {
            type Output = #ident;
            #[inline]
            fn div(self, rhs: f32) -> Self::Output {
                (*self / rhs).into()
            }
        }

        impl ::std::ops::Div<f32> for &#ident {
            type Output = #ident;
            #[inline]
            fn div(self, rhs: f32) -> Self::Output {
                *self / rhs
            }
        }

        impl ::std::ops::DivAssign<f32> for #ident {
            #[inline]
            fn div_assign(&mut self, rhs: f32) {
                self.0.div_assign(rhs)
            }
        }

        impl ::std::ops::DivAssign<&f32> for #ident {
            #[inline]
            fn div_assign(&mut self, rhs: &f32) {
                self.0.div_assign(*rhs)
            }
        }
    )
    .into()
}

#[proc_macro_derive(Matrix)]
pub fn derive_matrix_impl(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);
    quote!(
        impl #ident {
            pub const IDENTITY: Self = Self(glam::#ident::IDENTITY);

            #[inline(always)]
            pub const fn new(value: glam::#ident) -> Self {
                Self(value)
            }
        }

        impl ::std::ops::Deref for #ident {
            type Target = glam::#ident;
            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl ::std::ops::DerefMut for #ident {
            #[inline(always)]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl From<glam::#ident> for #ident {
            #[inline]
            fn from(value: glam::#ident) -> Self {
                Self(value)
            }
        }

        impl From<#ident> for glam::#ident {
            #[inline]
            fn from(value: #ident) -> Self {
                value.0
            }
        }

        impl ::std::ops::Mul<#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn mul(self, rhs: #ident) -> Self::Output {
                (*self * *rhs).into()
            }
        }

        impl ::std::ops::Mul<&#ident> for #ident {
            type Output = #ident;
            #[inline]
            fn mul(self, rhs: &#ident) -> Self::Output {
                self * *rhs
            }
        }
    )
    .into()
}

/// `system_once` is a marker attribute for FFI codegen.
#[proc_macro_attribute]
pub fn system_once(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// `system` is a marker attribute for FFI codegen.
#[proc_macro_attribute]
pub fn system(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// `init` is a marker attribute to run the function when the module starts.
/// `init` functions should take no parameters and not have a return type.
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_function = parse_macro_input!(item as ItemFn);

    let has_params = input_function
        .sig
        .inputs
        .iter()
        .any(|argument| match argument {
            FnArg::Receiver(_) | FnArg::Typed(_) => true,
        });

    if has_params {
        return syn::Error::new_spanned(
            input_function.sig.fn_token,
            "init function must not have any parameters",
        )
        .to_compile_error()
        .into();
    }

    let returns_only_unit = match &input_function.sig.output {
        syn::ReturnType::Default => true,
        syn::ReturnType::Type(_, return_type) => {
            matches!(**return_type, Type::Tuple(ref tuple) if tuple.elems.is_empty())
        }
    };

    if !returns_only_unit {
        return syn::Error::new_spanned(
            &input_function.sig.output,
            "Function must return nothing or the unit type",
        )
        .to_compile_error()
        .into();
    }

    quote! {
        #input_function
    }
    .into()
}

/// `deinit` is a marker attribute to run a function when a module is unloaded.
/// The intention is to allow a caller to clean up anything that was in a
/// corresponding `init` function. `deinit` functions should take no parameters
/// and not have a return type.
#[proc_macro_attribute]
pub fn deinit(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_function = parse_macro_input!(item as ItemFn);

    let has_params = input_function
        .sig
        .inputs
        .iter()
        .any(|argument| match argument {
            FnArg::Receiver(_) | FnArg::Typed(_) => true,
        });

    if has_params {
        return syn::Error::new_spanned(
            input_function.sig.fn_token,
            "deinit function must not have any parameters",
        )
        .to_compile_error()
        .into();
    }

    let returns_only_unit = match &input_function.sig.output {
        syn::ReturnType::Default => true,
        syn::ReturnType::Type(_, return_type) => {
            matches!(**return_type, Type::Tuple(ref tuple) if tuple.elems.is_empty())
        }
    };

    if !returns_only_unit {
        return syn::Error::new_spanned(
            &input_function.sig.output,
            "Function must return nothing or the unit type",
        )
        .to_compile_error()
        .into();
    }

    quote! {
        #input_function
    }
    .into()
}

struct SetSystemEnabledInput {
    system_enabled_bool_flag: bool,
    func_idents: Vec<Ident>,
}

impl Parse for SetSystemEnabledInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let bool_expr_lit: ExprLit = input.parse()?;

        let system_enabled_bool_flag = match &bool_expr_lit.lit {
            Lit::Bool(boolean) => boolean.value(),
            _ => {
                panic!("First parameter must be a bool for whether the system is enabled or not");
            }
        };
        input.parse::<Token![,]>()?;
        let mut func_idents = vec![input.parse::<Ident>()?];
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            func_idents.push(input.parse::<Ident>()?);
        }
        Ok(Self {
            system_enabled_bool_flag,
            func_idents,
        })
    }
}

/// Helper around [`Engine::set_system_enabled`]
#[proc_macro]
pub fn set_system_enabled(input: TokenStream) -> TokenStream {
    let SetSystemEnabledInput {
        func_idents,
        system_enabled_bool_flag,
    } = parse_macro_input!(input as SetSystemEnabledInput);
    // let func_name_str = func_ident.to_string();

    let system_enabled_functions: proc_macro2::TokenStream = func_idents.iter().fold(proc_macro2::TokenStream::new(), |token_stream, system_name_ident| {
        let system_name = system_name_ident.to_string();
        quote! {
            #token_stream

            let system_name = ::std::ffi::CStr::from_bytes_with_nul(concat!(#system_name, "\0").as_bytes()).unwrap();
            let full_system_name = ::void_public::system::system_name_generator_c(unsafe { ::std::ffi::CStr::from_ptr(module_name()) }, system_name);
            unsafe {
                (::void_public::_SET_SYSTEM_ENABLED_FN).unwrap_unchecked()(full_system_name.as_ptr(), enabled);
            }
        }
    });

    quote! {
        let enabled = #system_enabled_bool_flag;
        #system_enabled_functions
    }
    .into()
}
