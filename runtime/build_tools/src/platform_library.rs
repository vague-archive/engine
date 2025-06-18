use std::{error::Error, ffi::CString, fs, path::Path};

use convert_case::{Case, Casing};
use json::JsonValue;
use prettyplease::unparse;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Expr, FnArg, GenericArgument, Ident, ImplItem, Item, ItemFn, ItemImpl, ItemStruct,
    LitCStr, PathArguments, Signature, Type, Visibility, parse_quote, parse2,
    punctuated::Punctuated,
};

use crate::{
    allow_attr, generate_optional_no_mangle, iterator_helper::SplitExt,
    syn_helper::replace_lifetime_if_found,
};

pub fn write_ffi(
    crate_name: &str,
    out_dir: &Path,
    source_path: &Path,
    fbs_path: &Path,
    add_no_mangle: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let dest_path = Path::new(&out_dir).join("ffi_platform.rs");
    let input = fs::read_to_string(source_path)?;

    let file = match syn::parse_file(&input) {
        Ok(file) => file,
        Err(err) => return Err(format!("{crate_name}: invalid syntax: {err}").into()),
    };

    let mut parsed_info = ParsedInfo::default();

    for item in &file.items {
        match item {
            Item::Struct(item) => parse_struct(&mut parsed_info, item),
            Item::Fn(item) => parse_free_fn(&mut parsed_info, item),
            _ => {}
        }
    }

    // parse impls after parsing structs
    for item in &file.items {
        if let Item::Impl(item) = item {
            parse_impl(&mut parsed_info, item);
        }
    }

    let token_stream = gen_ffi(&parsed_info, add_no_mangle);

    let syn_file = match parse2(token_stream) {
        Ok(file) => file,
        Err(error) => {
            panic!("Could not convert token stream to string: {error}");
        }
    };

    let pretty_file = unparse(&syn_file);

    fs::write(dest_path, pretty_file).unwrap();

    // fs::write(dest_path, token_stream.to_string()).unwrap();

    let json_path = Path::new(&out_dir).join("metadata.json");
    fs::write(json_path, gen_metadata(crate_name, fbs_path, &parsed_info))?;

    Ok(())
}

#[derive(Debug, Default)]
struct ParsedInfo {
    structs: Vec<PlatformStruct>,
    functions: Vec<PlatformFunction>,
}

#[derive(Debug)]
struct PlatformStruct {
    static_mut_var: Ident,
    path: syn::Path,
    functions: Vec<PlatformFunction>,
}

#[derive(Debug)]
struct PlatformFunction {
    /// The c-string function name.
    name: LitCStr,
    path: syn::Path,
    params: Vec<Parameter>,
}

impl PlatformFunction {
    fn ffi_fn_ident(&self) -> syn::Ident {
        let ident = &self.path.segments.last().unwrap().ident;
        format_ident!("{ident}_ffi")
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum Parameter {
    StructSelf,
    StructMutSelf,
    ParameterData(syn::Path),
    ReturnWriter(syn::Path),
}

fn parse_struct(parsed_info: &mut ParsedInfo, item: &ItemStruct) {
    let Some(attr) = item
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("platform"))
    else {
        return;
    };

    let ident = item.ident.clone();

    let name = if let Ok(list) = attr.meta.require_list() {
        let Ok(expr) = list.parse_args::<Expr>() else {
            panic!("fn {ident}(): attribute format must be `name = \"name\"`");
        };

        match expr {
            Expr::Assign(assign) => {
                if !matches!(&*assign.left, Expr::Path(p) if p.path.is_ident("name")) {
                    panic!("fn {ident}(): attribute format must be `name = \"name\"`");
                }

                match &*assign.right {
                    Expr::Lit(name) => {
                        format_ident!("{}", quote! { #name }.to_string().replace('"', ""))
                    }
                    _ => panic!("fn {ident}(): attribute format must be `name = \"name\"`"),
                }
            }
            _ => panic!("fn {ident}(): invalid attribute"),
        }
    } else {
        ident.clone()
    };

    let mut path = syn::Path {
        leading_colon: None,
        segments: Punctuated::new(),
    };

    let static_mut_var = format_ident!("_{}", name.to_string().to_case(Case::ScreamingSnake));

    path.segments.push(item.ident.clone().into());

    parsed_info.structs.push(PlatformStruct {
        static_mut_var,
        path,
        functions: Vec::new(),
    });
}

fn parse_impl(parsed_info: &mut ParsedInfo, item: &ItemImpl) {
    let path = match &*item.self_ty {
        Type::Path(path) => &path.path,
        _ => {
            return;
        }
    };

    let Some(parsed_struct) = parsed_info.structs.iter_mut().find(|s| s.path == *path) else {
        return;
    };

    for function in item.items.iter().filter_map(|item| match item {
        ImplItem::Fn(item) if matches!(item.vis, Visibility::Public(_)) => Some(item),
        _ => None,
    }) {
        let attr = function
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("platform"));

        let parsed_fn = parse_fn(attr, &function.sig, parsed_struct.path.clone());
        parsed_struct.functions.push(parsed_fn);
    }
}

fn parse_free_fn(parsed_info: &mut ParsedInfo, item: &ItemFn) {
    let Some(attr) = item
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("platform"))
    else {
        return;
    };

    let parsed_fn = parse_fn(
        Some(attr),
        &item.sig,
        syn::Path {
            leading_colon: None,
            segments: Punctuated::new(),
        },
    );

    parsed_info.functions.push(parsed_fn);
}

#[must_use]
fn parse_fn(
    attr: Option<&Attribute>,
    sig: &Signature,
    mut parent_path: syn::Path,
) -> PlatformFunction {
    let ident = sig.ident.clone();

    let name = if let Some(list) = attr.and_then(|attr| attr.meta.require_list().ok()) {
        let Ok(expr) = list.parse_args::<Expr>() else {
            panic!("fn {ident}(): attribute format must be `name = \"name\"`");
        };

        match expr {
            Expr::Assign(assign) => {
                if !matches!(&*assign.left, Expr::Path(p) if p.path.is_ident("name")) {
                    panic!("fn {ident}(): attribute format must be `name = \"name\"`");
                }

                match &*assign.right {
                    Expr::Lit(name) => {
                        format_ident!("{}", quote! { #name }.to_string().replace('"', ""))
                    }
                    _ => panic!("fn {ident}(): attribute format must be `name = \"name\"`"),
                }
            }
            _ => panic!("fn {ident}(): invalid attribute"),
        }
    } else {
        ident.clone()
    };

    // Prefix name with parent path.
    let name = {
        let mut path = parent_path.clone();
        path.segments.push(name.clone().into());

        LitCStr::new(
            &CString::new(path.to_token_stream().to_string().replace(" ", "")).unwrap(),
            name.span(),
        )
    };

    // Always use the actual function path for the function path.
    parent_path.segments.push(ident.clone().into());

    let mut platform_function = PlatformFunction {
        name,
        path: parent_path,
        params: Vec::new(),
    };

    for input in &sig.inputs {
        if let FnArg::Receiver(input) = input {
            if input.mutability.is_none() {
                platform_function.params.push(Parameter::StructSelf);
            } else {
                platform_function.params.push(Parameter::StructMutSelf);
            }

            continue;
        }

        let FnArg::Typed(input) = input else {
            unreachable!();
        };

        match input.ty.as_ref() {
            Type::Path(component) => {
                let param_type = component.path.segments.last().unwrap().ident.to_string();

                match &*param_type {
                    "ParameterData" | "ReturnWriter" => {
                        let PathArguments::AngleBracketed(generics) =
                            &component.path.segments.last().unwrap().arguments
                        else {
                            panic!("fn {ident}({param_type}): invalid generics");
                        };

                        let Some(generic_type) = generics.args.iter().find_map(|arg| match arg {
                            GenericArgument::Type(arg) => Some(arg),
                            _ => None,
                        }) else {
                            panic!("fn {ident}({param_type}): invalid generics");
                        };

                        let Type::Path(path) = generic_type else {
                            panic!("fn {ident}({param_type}): generic must be a path");
                        };

                        let parameter = match &*param_type {
                            "ParameterData" => Parameter::ParameterData(path.path.clone()),
                            "ReturnWriter" => Parameter::ReturnWriter(path.path.clone()),
                            _ => unreachable!(),
                        };

                        platform_function.params.push(parameter);
                    }
                    _ => {
                        panic!("fn {ident}(): unsupported function parameter: {param_type}")
                    }
                }
            }
            Type::TraitObject(_) => {
                panic!("fn {ident}(): HRTBs (for<'a>) unsupported, use function lifetime instead");
            }
            _ => panic!("fn {ident}(): parameters must be ParameterData and/or ReturnWriter"),
        }
    }

    platform_function
}

fn function_iter(
    parsed_info: &ParsedInfo,
) -> impl Iterator<Item = (Option<&PlatformStruct>, &PlatformFunction)> {
    parsed_info.functions.iter().map(|f| (None, f)).chain(
        parsed_info
            .structs
            .iter()
            .flat_map(|s| s.functions.iter().map(move |f| (Some(s), f))),
    )
}

fn gen_ffi(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let gen_set_completion_callback = gen_set_completion_callback(add_no_mangle);
    let gen_set_platform_event_callback = gen_set_platform_event_callback(add_no_mangle);
    let gen_version = gen_version(add_no_mangle);
    let gen_init = gen_init(parsed_info, add_no_mangle);
    let gen_function_count = gen_function_count(parsed_info, add_no_mangle);
    let gen_function_name = gen_function_name(parsed_info, add_no_mangle);
    let gen_function_is_sync = gen_function_is_sync(parsed_info, add_no_mangle);
    let gen_function_ptr = gen_function_ptr(parsed_info, add_no_mangle);
    let functions = parsed_info
        .functions
        .iter()
        .map(|platform_function| gen_free_function_ffi(platform_function, add_no_mangle));
    let platform_functions = parsed_info.structs.iter().flat_map(|platform_struct| {
        platform_struct
            .functions
            .iter()
            .map(|function| gen_struct_function_ffi(platform_struct, function, add_no_mangle))
    });

    quote! {
        #gen_set_completion_callback
        #gen_set_platform_event_callback
        #gen_version
        #gen_init
        #gen_function_count
        #gen_function_name
        #gen_function_is_sync
        #gen_function_ptr
        #(#functions)*
        #(#platform_functions)*
    }
}

fn gen_set_completion_callback(add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn set_completion_callback(callback: ::platform_public::CompletionCallbackFn) {
            unsafe {
                ::platform_public::_COMPLETE_TASK_FN = Some(callback);
            }
        }
    }
}

fn gen_set_platform_event_callback(add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn set_platform_event_callback(callback: ::platform_public::PlatformEventCallbackFn) {
            unsafe {
                ::platform_public::_SEND_PLATFORM_EVENT_FN = Some(callback);
            }
        }
    }
}

fn gen_version(add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn void_target_version() -> u32 {
            ::platform_public::ENGINE_VERSION
        }
    }
}

fn gen_init(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();

    let struct_paths = parsed_info
        .structs
        .iter()
        .map(|parsed_struct| &parsed_struct.path);
    let struct_paths2 = struct_paths.clone();

    let static_idents = parsed_info
        .structs
        .iter()
        .map(|parsed_struct| &parsed_struct.static_mut_var);
    let static_idents2 = static_idents.clone();

    quote! {
        #(static mut #static_idents: Option<#struct_paths> = None;)*

        #no_mangle
        #allow_attr
        pub extern "C" fn init() -> u32 {
            #(#static_idents2 = Some(#struct_paths2::default());)*
            0
        }
    }
}

fn gen_function_count(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    let count = function_iter(parsed_info).count();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn function_count() -> usize {
            #count
        }
    }
}

fn gen_function_name(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();

    let function_iter_result = function_iter(parsed_info).collect::<Vec<_>>();
    let (index, function_name) = function_iter_result
        .iter()
        .map(|(_, function)| &function.name)
        .enumerate()
        .split();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn function_name(index: usize) -> *const ::std::ffi::c_char {
            match index {
                #(#index => #function_name .as_ptr(),)*
                _ => ::std::ptr::null(),
            }
        }
    }
}

fn gen_function_is_sync(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    let function_iter_result = function_iter(parsed_info).collect::<Vec<_>>();
    let (index, is_sync) = function_iter_result
        .iter()
        .map(|(_, function)| {
            !function
                .params
                .first()
                .is_some_and(|p| matches!(p, Parameter::StructMutSelf))
        })
        .enumerate()
        .split();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn function_is_sync(index: usize) -> bool {
            match index {
                #(#index => #is_sync,)*
                _ => false,
            }
        }
    }
}

fn gen_function_ptr(parsed_info: &ParsedInfo, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    let function_iter_result = function_iter(parsed_info).collect::<Vec<_>>();
    let (index, ffi_ident) = function_iter_result
        .iter()
        .map(|(struct_info, function)| {
            if let Some(struct_info) = struct_info {
                let struct_ident = struct_info
                    .path
                    .segments
                    .last()
                    .unwrap()
                    .ident
                    .to_string()
                    .to_case(Case::Snake);
                let function_ident = function.ffi_fn_ident();
                format_ident!("{struct_ident}_{function_ident}")
            } else {
                function.ffi_fn_ident()
            }
        })
        .enumerate()
        .split();

    quote! {
        #no_mangle
        #allow_attr
        pub extern "C" fn function_ptr(
            index: usize
        ) -> unsafe extern "C" fn(::platform_public::TaskId, *const ::std::ffi::c_void, usize) {
            match index {
                #(#index => #ffi_ident,)*
                _ => ::std::process::abort(),
            }
        }
    }
}

fn gen_free_function_ffi(function: &PlatformFunction, add_no_mangle: bool) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    let function_ident = function.path.clone();
    let function_name = function.ffi_fn_ident();

    let parameter_data = if let Some(param) = function
        .params
        .iter()
        .find(|p| matches!(p, Parameter::ParameterData(_)))
    {
        let Parameter::ParameterData(parameter_path) = param else {
            unreachable!();
        };

        let parameter_path = {
            let mut parameter_path = parameter_path.clone();
            replace_lifetime_if_found(&mut parameter_path, "'_");
            parameter_path
        };

        quote! {
            let parameter_data = ::std::slice::from_raw_parts(parameter_data_ptr.cast(), parameter_data_size);
            let parameters = ::flatbuffers::root_unchecked::<#parameter_path>(parameter_data);
        }
    } else {
        quote! {}
    };

    let params = function.params.iter().map(|param| match param {
        Parameter::ParameterData(_) => {
            quote! {ParameterData::new(parameters)}
        }
        Parameter::ReturnWriter(_) => {
            quote! {ReturnWriter::new(task_id)}
        }
        _ => unreachable!(),
    });

    quote! {
        #no_mangle
        #allow_attr
        pub unsafe extern "C" fn #function_name(
            task_id: ::platform_public::TaskId,
            parameter_data_ptr: *const ::std::ffi::c_void,
            parameter_data_size: usize,
        ) {
            #parameter_data
            #function_ident(
                #(#params,)*
            );
        }
    }
}

fn gen_struct_function_ffi(
    platform_struct: &PlatformStruct,
    function: &PlatformFunction,
    add_no_mangle: bool,
) -> TokenStream {
    let no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();

    let struct_ident = platform_struct
        .path
        .segments
        .last()
        .unwrap()
        .ident
        .to_string()
        .to_case(Case::Snake);
    let ffi_ident = format_ident!("{struct_ident}_{}", function.ffi_fn_ident());

    let parameter_data = if let Some(param) = function
        .params
        .iter()
        .find(|p| matches!(p, Parameter::ParameterData(_)))
    {
        let Parameter::ParameterData(parameter_path) = param else {
            unreachable!();
        };

        let parameter_path = {
            let mut parameter_path = parameter_path.clone();
            replace_lifetime_if_found(&mut parameter_path, "'_");
            parameter_path
        };

        quote! {
            let parameter_data = ::std::slice::from_raw_parts(parameter_data_ptr.cast(), parameter_data_size);
            let parameters = ::flatbuffers::root_unchecked::<#parameter_path>(parameter_data);
        }
    } else {
        quote! {}
    };

    let function_ident = {
        let var_name = &platform_struct.static_mut_var;

        let unwrap_text: Expr = if matches!(function.params[0], Parameter::StructSelf) {
            parse_quote! { as_ref() }
        } else {
            parse_quote! { as_mut() }
        };

        let function_ident = &function.path.segments.last().unwrap().ident;

        quote! {
            #var_name.#unwrap_text.unwrap().#function_ident
        }
    };

    let params = function.params.iter().filter_map(|param| match param {
        Parameter::ParameterData(_) => Some(quote! {ParameterData::new(parameters)}),
        Parameter::ReturnWriter(_) => Some(quote! {ReturnWriter::new(task_id)}),
        _ => None,
    });

    quote! {
        #no_mangle
        #allow_attr
        #[allow(static_mut_refs)]
        pub unsafe extern "C" fn #ffi_ident(
            task_id: ::platform_public::TaskId,
            parameter_data_ptr: *const ::std::ffi::c_void,
            parameter_data_size: usize,
        ) {
            #parameter_data
            #function_ident(
                #(#params,)*
            );
        }
    }
}

fn gen_metadata(crate_name: &str, fbs_path: &Path, parsed_info: &ParsedInfo) -> String {
    let mut object = JsonValue::new_object();
    let mut functions = JsonValue::new_array();

    for (_, function) in function_iter(parsed_info) {
        let mut value = JsonValue::new_object();
        value["name"] = function.name.value().to_string_lossy().to_string().into();

        if let Some(param) = function
            .params
            .iter()
            .find(|p| matches!(p, Parameter::ParameterData(_)))
        {
            let Parameter::ParameterData(parameter_path) = param else {
                unreachable!();
            };

            value["parameter_data"] = parameter_path
                .to_token_stream()
                .to_string()
                .replace("event::", "")
                .replace(" ", "")
                .into();
        }

        if let Some(param) = function
            .params
            .iter()
            .find(|p| matches!(p, Parameter::ReturnWriter(_)))
        {
            let Parameter::ReturnWriter(parameter_path) = param else {
                unreachable!();
            };

            value["return_value"] = parameter_path
                .to_token_stream()
                .to_string()
                .replace("event::", "")
                .replace(" ", "")
                .into();
        }

        functions.push(value).unwrap();
    }

    object["name"] = crate_name.into();
    object["functions"] = functions;
    object["fbs"] = fs::read_to_string(fbs_path)
        .expect("error reading fbs file")
        .into();

    object.pretty(4)
}
