//! Tools for creating a Foreign Function Interface (FFI).
//!
//! The FFI will be built for items marked as ECS systems. I.e. items marked
//! `#[system]` or `#[system_once]`, along with ECS types, like components,
//! resources, async completions, etc.
//!
//! E.g. this might be called form a `build.rs` to generate FFI for a given
//! library.

use std::{
    borrow::Cow,
    env::var,
    ffi::CString,
    fmt::{self, Debug, Display},
    fs, io,
    path::{Path, PathBuf},
};

use colored::Colorize;
use iterator_helper::SplitExt;
use prettyplease::unparse;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use regex::Regex;
use syn::{
    Attribute, File, FnArg, GenericArgument, GenericParam, Ident, ImplItem, Index, Item, ItemFn,
    ItemImpl, ItemMod, LitCStr, PathArguments, Type, TypeParamBound, parse_quote, parse2,
    punctuated::Punctuated, spanned::Spanned,
};

mod iterator_helper;
pub mod platform_library;
mod syn_helper;

use std::env::{current_dir, var_os};

/// Generates the C FFI layer from Rust code.
///
/// For many Rust modules, this call will be sufficient to generate the ffi
/// wrapper for a Fiasco engine module.
///
/// It is a wrapper around `write_ffi()`.
///
/// Usage: `build_tools::FfiBuilder::new().write();`
pub struct FfiBuilder<'a> {
    pub module_name: String,
    out_dir: Option<Cow<'a, Path>>,
    input_path: Option<Cow<'a, Path>>,
    add_no_mangle: bool,
}

impl Default for FfiBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> FfiBuilder<'a> {
    /// Create a builder
    ///
    /// - `module_name` is set to the cargo `CARGO_PKG_NAME` env var, which is the value
    ///   from the `Cargo.toml` `[package]` `name` value.
    /// - `out_dir` is set to the cargo `OUT_DIR` env var (i.e. in `target/...`
    ///   somewhere).
    /// - `input_path` is set to "src/lib.rs".
    /// - `add_no_mangle` is set to true.
    ///
    /// After setting other values, call `.write()` to execute the FFI generation.
    pub fn new() -> Self {
        let module_name = var("CARGO_PKG_NAME").unwrap();
        // Check for regression.
        assert_ne!("build-tools", module_name);
        assert_ne!("build_tools", module_name);
        let add_no_mangle = true;
        Self {
            module_name,
            out_dir: None,
            input_path: None,
            add_no_mangle,
        }
    }

    pub fn module_name(mut self, module_name: &'a str) -> Self {
        self.module_name = module_name.to_string();
        self
    }

    pub fn out_dir(mut self, out_dir: &'a Path) -> Self {
        self.out_dir = Some(Cow::from(out_dir));
        self
    }

    pub fn input_path(mut self, input_path: &'a Path) -> Self {
        self.input_path = Some(Cow::from(input_path));
        self
    }

    pub fn add_no_mangle(mut self, add_no_mangle: bool) -> Self {
        self.add_no_mangle = add_no_mangle;
        self
    }

    /// If this call fails, it will panic and display an error (which is not good
    /// practice in general, but is sensible in a build script).
    pub fn write(self) {
        let out_dir = self
            .out_dir
            .unwrap_or_else(|| Cow::from(PathBuf::from(var_os("OUT_DIR").unwrap())));

        let input_path = self
            .input_path
            .unwrap_or_else(|| Cow::from(current_dir().unwrap().join("src/lib.rs")));
        write_ffi(&self.module_name, &out_dir, &input_path, self.add_no_mangle).unwrap();
    }
}

/// Errors specific to creating the FFI wrapper.
pub enum Error {
    /// Error reading the input source file to parse.
    ReadFile(io::Error),

    /// Error writing the resulting output ffi.rs file.
    WriteFile(io::Error),

    /// A syntax error in the input source file.
    ParseFile {
        error: syn::Error,
        filepath: PathBuf,
        source_code: String,
    },

    /// Error in second pass parsing.
    Parse2(Box<ParsedInfo>, syn::Error, bool),
}

impl Debug for Error {
    // If `.unwrap()` is used on the error the debug format may be called, which
    // is much harder to read than the display output. It's more helpful to pass
    // through to the display output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ReadFile(error) => write!(f, "FFI file read failed: {}", error),
            Error::WriteFile(error) => write!(f, "FFI file write failed: {}", error),
            Error::ParseFile {
                error,
                filepath,
                source_code,
            } => display_parsing_error(f, error, filepath, source_code),
            Error::Parse2(parsed_info, error, add_no_mangle) => {
                for (erroring_function_name, error) in
                    parsed_info.check_for_parsing_errors(*add_no_mangle)
                {
                    write!(f, "Error in {erroring_function_name}: {error}")?;
                }
                write!(f, "Could not convert token stream to string: {error}")
            }
        }
    }
}

/// Render a rustc-style error message.
///
/// If you don't see the pretty colors in the output, try setting env var
///  `CLICOLOR_FORCE`.
///
/// Based on example in Rust `syn` crate.
fn display_parsing_error(
    formatter: &mut fmt::Formatter<'_>,
    err: &syn::Error,
    file_path: &Path,
    code: &str,
) -> fmt::Result {
    // Windows OS terminal sometimes lack info on color. Let's try overriding
    // it. Feel free to just remove this if any issues are found.
    #[cfg(target_os = "windows")]
    colored::control::set_override(true);

    let start = err.span().start();
    let mut end = err.span().end();

    if start.line == end.line && start.column == end.column {
        return write!(
            formatter,
            "Unable to parse file (unknown start/end): {}",
            err
        );
    }

    let Some(code_line) = code.lines().nth(start.line - 1) else {
        return write!(
            formatter,
            "Unable to parse file (unknown code line): {}",
            err
        );
    };

    if end.line > start.line {
        end.line = start.line;
        end.column = code_line.len();
    }

    write!(
        formatter,
        "\n\
         {error}{header}\n\
         {indent}{arrow} {file_path}:{line}:{column}\n\
         {indent} {pipe}\n\
         {label} {pipe} {code}\n\
         {indent} {pipe} {offset}{underline} {message}\n\
         ",
        error = "error".red().bold(),
        header = ": Syn unable to parse file".bold(),
        indent = " ".repeat(start.line.to_string().len()),
        arrow = "-->".blue().bold(),
        file_path = file_path.display(),
        line = start.line,
        column = start.column,
        pipe = "|".blue().bold(),
        label = start.line.to_string().blue().bold(),
        code = code_line.trim_end(),
        offset = " ".repeat(start.column),
        underline = "^".repeat(end.column - start.column).red().bold(),
        message = err.to_string().red(),
    )
}

fn no_mangle_attr() -> TokenStream {
    quote! { #[unsafe(no_mangle)] }
}

pub(crate) fn generate_optional_no_mangle(add_no_mangle: bool) -> TokenStream {
    if add_no_mangle {
        no_mangle_attr()
    } else {
        quote! {}
    }
}

fn allow_attr() -> TokenStream {
    quote! { #[allow(unused, unsafe_op_in_unsafe_fn, clippy::all, clippy::pedantic)] }
}

/// The primary entry point to create a foreign file interface (ffi).
///
/// Parse the Rust source code at `input_path`, looking for items marked as
/// systems, and write the resulting Rust (ffi) code to `out_dir` under the
/// filename "ffi.rs".
///
/// `add_no_mangle` can be used so that the exported symbol names in a library
/// are not mangled by the compiler, so that they can be looked up with the
/// expected name. i.e. Symbols in the FFI are tagged with a request not
/// 'mangle' (encode) symbol names and instead write the names as they appear in
/// the source code. See <https://www.google.com/search?q=name+mangling> for more.
fn write_ffi(
    module_name: &str,
    out_dir: &Path,
    input_path: &Path,
    add_no_mangle: bool,
) -> Result<(), Error> {
    let dest_path = Path::new(&out_dir).join("ffi.rs");
    let input = fs::read_to_string(input_path).map_err(Error::ReadFile)?;

    let mut parsed_info = ParsedInfo::new(module_name);

    // Parse the file with `syn`.
    let file = syn::parse_file(&input).map_err(|error| Error::ParseFile {
        error,
        filepath: input_path.to_path_buf(),
        source_code: input.clone(),
    })?;

    let mod_path = syn::Path {
        leading_colon: None,
        segments: Punctuated::new(),
    };

    // Naive search for any usage of `Callable` types.
    let re = Regex::new(r"Engine::call.*::<(.+)>").unwrap();
    for capture in re.captures_iter(&input) {
        let callable_path = capture.get(1).unwrap().as_str();

        let ident = Ident::new(callable_path, file.span());
        let mut path = mod_path.clone();
        path.segments.push(ident.into());

        parsed_info.imported_ecs_types.push(path);
    }

    // Parse rest of file.
    for item in &file.items {
        parsed_info.parse_item(item, input_path.parent().unwrap(), &mod_path);
    }

    let token_stream = parsed_info.gen_ffi(add_no_mangle);

    let syn_file = parse2(token_stream)
        .map_err(|error| Error::Parse2(Box::new(parsed_info), error, add_no_mangle))?;

    let pretty_file = unparse(&syn_file);

    fs::write(dest_path, pretty_file.as_str()).map_err(Error::WriteFile)
}

/// Compound type for components or resources in native code generation.
///
/// The engine handles `struct` and `enum` identically. Having two separate
/// types would be code duplication, so either type is handled by this [`EcsType`]
/// enum. Any type that can derive to Copy + 'static can be an [`EcsType`], for
/// Rust this means both `struct` and `enum`s.
#[derive(Debug)]
enum EcsType {
    AsyncCompletion { callable: syn::Path },
    Component,
    Resource,
}

impl EcsType {
    pub fn to_ecs_type_variant(&self) -> syn::Ident {
        match self {
            EcsType::AsyncCompletion { callable: _ } => parse_quote! { AsyncCompletion },
            EcsType::Component => parse_quote! { Component },
            EcsType::Resource => parse_quote! { Resource },
        }
    }
}

#[derive(Debug)]
enum ArgType {
    Completion,
    DataAccessDirect,
    EventReader { input: syn::Path },
    EventWriter { input: syn::Path },
    Query { inputs: Vec<SystemInputInfo> },
}

/// A collection of items this FFI is interested in.
///
/// As items are found/parsed, the information will be collected here.
/// Ultimately used to generate the body of the FFI.
#[derive(Debug)]
pub struct ParsedInfo {
    module_name: String,
    systems: Vec<SystemInfo>,
    init_functions: Vec<syn::Path>,
    deinit_functions: Vec<syn::Path>,
    ecs_types: Vec<EcsTypeInfo>,
    /// A list of paths to all ECS types used but not declared by this crate.
    imported_ecs_types: Vec<syn::Path>,
}

impl ParsedInfo {
    fn new(module_name: &str) -> Self {
        Self {
            module_name: module_name.to_string(),
            systems: Vec::new(),
            init_functions: Vec::new(),
            deinit_functions: Vec::new(),
            ecs_types: Vec::new(),
            // Until we get better parsing, include all `void_public`
            // components. We must do this in case `void_public` components are
            // used with i.e. `Engine::spawn()`, where they are referred to via
            // their `EntityId`. Otherwise, their ID will not be assigned.
            imported_ecs_types: vec![
                parse_quote!(::void_public::Transform),
                parse_quote!(::void_public::LocalToWorld),
                parse_quote!(::void_public::Camera),
                parse_quote!(::void_public::colors::Color),
                parse_quote!(::void_public::graphics::CircleRender),
                parse_quote!(::void_public::graphics::TextRender),
                parse_quote!(::void_public::graphics::ColorRender),
                parse_quote!(::void_public::graphics::TextureRender),
                parse_quote!(::void_public::material::MaterialParameters),
            ],
        }
    }
}

#[derive(Debug)]
struct SystemInfo {
    path: syn::Path,
    is_once: bool,
    takes_platform_generic: bool,
    inputs: Vec<SystemInputInfo>,
}

fn generate_ffi_function_name_from_path(path: &syn::Path) -> Ident {
    let mut ident = path
        .to_token_stream()
        .to_string()
        .replace(" ", "")
        .replace("::", "_");
    ident.push_str("_ffi");

    format_ident!("{ident}")
}

impl SystemInfo {
    fn name(&self) -> LitCStr {
        let path = self.path.to_token_stream().to_string().replace(" ", "");

        LitCStr::new(&CString::new(path).unwrap(), self.path.span())
    }

    fn ffi_ident(&self) -> Ident {
        generate_ffi_function_name_from_path(&self.path)
    }
}

#[derive(Debug)]
struct SystemInputInfo {
    path: syn::Path,
    arg_type: ArgType,
    mutable: bool,
}

impl SystemInputInfo {
    pub fn is_component(&self) -> bool {
        self.path.segments.last().unwrap().ident != "Completion"
            && self.path.segments.last().unwrap().ident != "Query"
            && self.path.segments.last().unwrap().ident != "EventReader"
            && self.path.segments.last().unwrap().ident != "EventWriter"
    }
}

#[derive(Debug)]
struct EcsTypeInfo {
    path: syn::Path,
    ecs_type: EcsType,
}

impl ParsedInfo {
    /// Parse a top-level file or mod.
    fn parse_item(&mut self, item: &Item, working_dir: &Path, mod_path: &syn::Path) {
        match item {
            Item::Fn(item) => {
                self.parse_fn(item, mod_path);
            }
            Item::Mod(item) => {
                self.parse_mod(item, working_dir, mod_path.clone());
            }
            Item::Struct(item) => {
                self.parse_struct_or_enum(&item.ident, &item.attrs, mod_path);
            }
            Item::Enum(item) => {
                self.parse_struct_or_enum(&item.ident, &item.attrs, mod_path);
            }
            Item::Impl(item) => {
                self.parse_impl(item, mod_path);
            }
            _ => {}
        }
    }

    /// Collect information from a function (fn) which are marked `#[system]` or
    /// `#[system_once]`.
    fn parse_fn(&mut self, item: &ItemFn, mod_path: &syn::Path) {
        let is_system = item.attrs.iter().any(|attr| attr.path().is_ident("system"));
        let is_system_once = item
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("system_once"));
        let is_init = item.attrs.iter().any(|attr| attr.path().is_ident("init"));
        let is_deinit = item.attrs.iter().any(|attr| attr.path().is_ident("deinit"));

        if is_system || is_system_once {
            self.parse_system_fn(item, mod_path, is_system_once);
        } else if is_init {
            self.parse_init_fn(item, mod_path);
        } else if is_deinit {
            self.parse_deinit_fn(item, mod_path);
        }
    }

    fn parse_system_fn(&mut self, item: &ItemFn, mod_path: &syn::Path, is_system_once: bool) {
        let takes_platform_generic = item
            .sig
            .generics
            .params
            .first()
            .and_then(|param| {
                if let GenericParam::Type(param) = param {
                    Some(param)
                } else {
                    None
                }
            })
            .and_then(|param| param.bounds.first())
            .and_then(|bound| {
                if let TypeParamBound::Trait(bound) = bound {
                    Some(bound)
                } else {
                    None
                }
            })
            .and_then(|bound| bound.path.segments.first())
            .is_some_and(|segment| segment.ident == "Platform");

        let ident = item.sig.ident.clone();

        let mut inputs = Vec::new();

        for input in &item.sig.inputs {
            let FnArg::Typed(input) = input else {
                panic!("fn {ident}(): systems cannot take self");
            };

            let system_input = match input.ty.as_ref() {
                Type::Path(component) => {
                    let param_type = component.path.segments.last().unwrap().ident.clone();

                    // Query or EventReader
                    match param_type.to_string().as_str() {
                        "EventReader" | "EventWriter" => {
                            let PathArguments::AngleBracketed(inputs) =
                                &component.path.segments.last().unwrap().arguments
                            else {
                                panic!("fn {ident}(): invalid EventReader/EventWriter generics")
                            };

                            assert_eq!(
                                inputs.args.len(),
                                1,
                                "EventReader/EventWriter may only take a single generic"
                            );

                            let GenericArgument::Type(input) = inputs.args.first().unwrap() else {
                                panic!("EventReader/EventWriter generic must be a value type");
                            };

                            let Type::Path(path) = input else {
                                panic!("EventReader/EventWriter generic must be a path");
                            };

                            let arg_type = if param_type == "EventReader" {
                                ArgType::EventReader {
                                    input: path.path.clone(),
                                }
                            } else {
                                ArgType::EventWriter {
                                    input: path.path.clone(),
                                }
                            };

                            SystemInputInfo {
                                path: component.path.clone(),
                                arg_type,
                                mutable: false,
                            }
                        }
                        "Query" => {
                            let PathArguments::AngleBracketed(query_inputs) =
                                &component.path.segments.last().unwrap().arguments
                            else {
                                panic!("fn {ident}(): invalid query generics")
                            };

                            let inputs = query_inputs
                            .args
                            .iter()
                            .flat_map(|input| {
                                let GenericArgument::Type(input) = input else {
                                    panic!("fn {ident}(): invalid query generics")
                                };

                                if let Type::Reference(ty) = input {
                                    let Type::Path(component) = ty.elem.as_ref() else {
                                        panic!("fn {ident}(): unsupported query input type")
                                    };

                                    Vec::from([SystemInputInfo {
                                        path: component.path.clone(),
                                        arg_type: ArgType::DataAccessDirect,
                                        mutable: ty.mutability.is_some(),
                                    }])
                                } else {
                                    let Type::Tuple(tuple) = input else {
                                        panic!("fn {ident}(): unsupported query input type")
                                    };

                                    tuple
                                        .elems
                                        .iter()
                                        .map(|elem| {
                                            let Type::Reference(ty) = elem else {
                                                panic!("fn {ident}(): system inputs must be references")
                                            };

                                            let Type::Path(component) = ty.elem.as_ref() else {
                                                panic!("fn {ident}(): unsupported system input type")
                                            };

                                            SystemInputInfo {
                                                path: component.path.clone(),
                                                arg_type: ArgType::DataAccessDirect,
                                                mutable: ty.mutability.is_some(),
                                            }
                                        })
                                        .collect()
                                }
                            })
                            .collect();

                            SystemInputInfo {
                                path: component.path.clone(),
                                arg_type: ArgType::Query { inputs },
                                mutable: false,
                            }
                        }
                        "Completion" => SystemInputInfo {
                            path: component.path.clone(),
                            arg_type: ArgType::Completion,
                            mutable: false,
                        },
                        _ => {
                            panic!(
                                "fn {ident}(): unsupported system input type: {param_type}. Hint: resource inputs must be references."
                            )
                        }
                    }
                }

                Type::Reference(ty) => {
                    // Component or Resource

                    let Type::Path(component) = ty.elem.as_ref() else {
                        panic!("fn {ident}(): unsupported system input type")
                    };

                    let param_type = component.path.segments.last().unwrap().ident.clone();

                    if param_type == "Query" {
                        panic!("fn {ident}(): query inputs must be taken by value");
                    }

                    if param_type == "EventReader" {
                        panic!("fn {ident}(): EventReader inputs must be taken by value");
                    }

                    if param_type == "EventWriter" {
                        panic!("fn {ident}(): EventWriter inputs must be taken by value");
                    }

                    SystemInputInfo {
                        path: component.path.clone(),
                        arg_type: ArgType::DataAccessDirect,
                        mutable: ty.mutability.is_some(),
                    }
                }
                _ => panic!("fn {ident}(): system inputs must be references"),
            };

            inputs.push(system_input);
        }

        let mut path = mod_path.clone();
        path.segments.push(ident.into());

        self.systems.push(SystemInfo {
            path,
            is_once: is_system_once,
            takes_platform_generic,
            inputs,
        });
    }

    fn parse_init_fn(&mut self, item: &ItemFn, mod_path: &syn::Path) {
        let ident = item.sig.ident.clone();
        let mut path = mod_path.clone();
        path.segments.push(ident.into());
        self.init_functions.push(path);
    }

    fn parse_deinit_fn(&mut self, item: &ItemFn, mod_path: &syn::Path) {
        let ident = item.sig.ident.clone();
        let mut path = mod_path.clone();
        path.segments.push(ident.into());
        self.deinit_functions.push(path);
    }

    /// Collect information from a Rust module.
    fn parse_mod(&mut self, item: &ItemMod, working_dir: &Path, mut mod_path: syn::Path) {
        let parent_mod_ident = match mod_path.segments.last() {
            Some(segment) => segment.ident.to_string(),
            None => "".to_owned(),
        };

        // Push new mod into the mod path and shadow to make read-only.
        mod_path.segments.push(item.ident.clone().into());
        let mod_path = mod_path;

        // Check if the mod is inline in the parent file.
        if let Some((_, items)) = &item.content {
            for item in items {
                self.parse_item(item, working_dir, &mod_path);
            }

            return;
        }

        // Chech if the mod file is in the same dir.
        let path = working_dir
            .join(item.ident.to_string())
            .with_extension("rs");
        if let Ok(mod_file) = fs::read_to_string(path) {
            for item in &syn::parse_file(&mod_file).unwrap().items {
                self.parse_item(item, working_dir, &mod_path);
            }

            return;
        }

        // Check if the mod file is in a subdir named after parent mod.
        let path = working_dir
            .join(&parent_mod_ident)
            .join(item.ident.to_string())
            .with_extension("rs");
        if let Ok(mod_file) = fs::read_to_string(path) {
            let working_dir = working_dir.join(&parent_mod_ident);
            for item in &syn::parse_file(&mod_file).unwrap().items {
                self.parse_item(item, &working_dir, &mod_path);
            }

            return;
        }

        // Check if mod file is in subdir named after child mod.
        let path = working_dir.join(item.ident.to_string()).join("mod.rs");
        if let Ok(mod_file) = fs::read_to_string(path) {
            let working_dir = working_dir.join(item.ident.to_string());
            for item in &syn::parse_file(&mod_file).unwrap().items {
                self.parse_item(item, &working_dir, &mod_path);
            }
        }
    }

    fn parse_struct_or_enum(
        &mut self,
        item_ident: &Ident,
        item_attributes: &[Attribute],
        mod_path: &syn::Path,
    ) {
        let ecs_type = item_attributes
            .iter()
            .filter(|attr| attr.path().is_ident("derive"))
            .find_map(|attr| {
                let mut ecs_type = None;
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("Component") {
                        ecs_type = Some(EcsType::Component);
                    }

                    if meta.path.is_ident("Resource")
                        || meta.path.is_ident("ResourceWithoutSerialize")
                    {
                        ecs_type = Some(EcsType::Resource);
                    }

                    Ok(())
                })
                .unwrap();

                ecs_type
            });

        let Some(ecs_type) = ecs_type else {
            return;
        };

        let mut path = mod_path.clone();
        path.segments.push(item_ident.clone().into());

        self.ecs_types.push(EcsTypeInfo { path, ecs_type });
    }

    /// Parse any impl blocks for `EcsType` structs. This is currently only used
    /// to parse `AsyncCompletion` trait implementations and imported `Callable`
    /// ECS types.
    fn parse_impl(&mut self, item: &ItemImpl, mod_path: &syn::Path) {
        // Check for types which implement `Callable`.
        if item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|segment| segment.ident == "Callable")
        {
            let Type::Path(type_path) = &*item.self_ty else {
                return;
            };

            let mut path = mod_path.clone();
            path.segments
                .extend(type_path.path.segments.iter().cloned());

            self.imported_ecs_types.push(path);
            return;
        }

        // Check for types which implement `AsyncCompletion`.
        if item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_none_or(|segment| segment.ident != "AsyncCompletion")
        {
            return;
        }

        let Type::Path(struct_ident) = &*item.self_ty else {
            return;
        };

        let mut path = mod_path.clone();
        path.segments
            .extend(struct_ident.path.segments.iter().cloned());

        let function_type = item
            .items
            .iter()
            .find(|item| matches!(item, ImplItem::Type(ty) if ty.ident == "Function"))
            .expect("`AsyncCompletion` does not contain `Function` type??");

        let ImplItem::Type(impl_item) = function_type else {
            unreachable!(); // just checked in find()
        };

        let Type::Path(callable_path) = &impl_item.ty else {
            panic!("AsyncCompletion Function type must be a path");
        };

        let mut callable = mod_path.clone();
        callable
            .segments
            .extend(callable_path.path.segments.iter().cloned());

        self.ecs_types.push(EcsTypeInfo {
            path,
            ecs_type: EcsType::AsyncCompletion { callable },
        });
    }

    pub fn check_for_parsing_errors(&self, add_no_mangle: bool) -> Vec<(String, syn::Error)> {
        let mut output = vec![];

        macro_rules! error_collector{
            ($(($my_self:ident, $function_name:ident, $function_name_str:literal)),*) => {
                $(
                    if let Err(error) = parse2::<File>($my_self.$function_name(add_no_mangle)) {
                        output.push(($function_name_str.into(), error));
                    }

                )*
            }
        }

        if let Err(error) = parse2::<File>(gen_version(add_no_mangle)) {
            output.push(("gen_version".into(), error));
        }

        if let Err(error) = parse2::<File>(Self::gen_load_engine_proc_addrs(add_no_mangle)) {
            output.push(("gen_load_engine_proc_addrs".into(), error));
        }

        if let Err(error) = parse2::<File>(self.gen_system_fn_ffi()) {
            output.push(("gen_system_fn_ffi".into(), error));
        }

        error_collector!(
            (self, gen_init_functions, "gen_init_functions"),
            (self, gen_init, "gen_init"),
            (self, gen_deinit, "gen_deinit"),
            (self, gen_module_info, "gen_module_info"),
            (self, gen_module_name, "gen_module_name"),
            (self, gen_components, "gen_components"),
            (self, gen_resource_init, "gen_resource_init"),
            (self, gen_systems, "gen_systems"),
            (self, gen_set_component_id, "gen_set_component_id"),
            (self, gen_component_string_id, "gen_component_string_id"),
            (self, gen_component_size, "gen_component_size"),
            (self, gen_component_align, "gen_component_align"),
            (self, gen_component_type, "gen_component_type"),
            (
                self,
                gen_component_async_completion_callable,
                "gen_component_async_completion_callable"
            ),
            (self, gen_systems_len, "gen_systems_len"),
            (self, gen_system_name, "gen_system_name"),
            (self, gen_system_is_once, "gen_system_is_once"),
            (self, gen_system_fn, "gen_system_fn"),
            (self, gen_system_args_len, "gen_system_args_len"),
            (self, gen_system_arg_type, "gen_system_arg_type"),
            (self, gen_system_arg_component, "gen_system_arg_component"),
            (self, gen_system_arg_event, "gen_system_arg_event"),
            (self, gen_system_query_args_len, "gen_system_query_args_len"),
            (self, gen_system_query_arg_type, "gen_system_query_arg_type"),
            (
                self,
                gen_system_query_arg_component,
                "gen_system_query_arg_component"
            )
        );
        output
    }

    fn gen_ffi(&self, add_no_mangle: bool) -> TokenStream {
        let gen_version = gen_version(add_no_mangle);
        let gen_init_functions = self.gen_init_functions(add_no_mangle);
        let gen_module_info = self.gen_module_info(add_no_mangle);
        let gen_resources = self.gen_resources(add_no_mangle);
        let gen_components = self.gen_components(add_no_mangle);
        let gen_systems = self.gen_systems(add_no_mangle);
        let gen_load_engine_proc_addrs = Self::gen_load_engine_proc_addrs(add_no_mangle);

        quote! {
            #gen_version
            #gen_init_functions
            #gen_module_info
            #gen_resources
            #gen_components
            #gen_systems
            #gen_load_engine_proc_addrs
        }
    }

    fn gen_init_functions(&self, add_no_mangle: bool) -> TokenStream {
        let gen_init = self.gen_init(add_no_mangle);
        let gen_deinit = self.gen_deinit(add_no_mangle);

        quote! {
            #gen_init
            #gen_deinit
        }
    }

    fn gen_init(&self, add_no_mangle: bool) -> TokenStream {
        let optional_add_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attrs = allow_attr();
        let body = self.init_functions.iter().fold(
            TokenStream::new(),
            |token_stream, init_function_path| {
                quote! {
                    #token_stream
                    #init_function_path();
                }
            },
        );

        quote! {
            #optional_add_no_mangle
            #allow_attrs
            pub extern "C" fn init() -> i32 {
                ::std::panic::catch_unwind(|| {
                    #body
                })
                .map(|_| 0)
                .unwrap_or(1)
            }
        }
    }

    fn gen_deinit(&self, add_no_mangle: bool) -> TokenStream {
        let optional_add_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attrs = allow_attr();
        let body = self.deinit_functions.iter().fold(
            TokenStream::new(),
            |token_stream, deinit_function_path| {
                quote! {
                    #token_stream
                    #deinit_function_path();
                }
            },
        );

        quote! {
            #optional_add_no_mangle
            #allow_attrs
            pub extern "C" fn deinit() -> i32 {
                ::std::panic::catch_unwind(|| {
                    #body
                })
                .map(|_| 0)
                .unwrap_or(1)
            }
        }
    }

    fn gen_module_info(&self, add_no_mangle: bool) -> TokenStream {
        let gen_module_name = self.gen_module_name(add_no_mangle);

        quote! {
            #gen_module_name
        }
    }

    fn gen_resources(&self, add_no_mangle: bool) -> TokenStream {
        let gen_resource_init = self.gen_resource_init(add_no_mangle);
        let gen_resource_deserialize = self.gen_resource_deserialize(add_no_mangle);
        let gen_resource_serialize = self.gen_resource_serialize(add_no_mangle);

        quote! {
            #gen_resource_init
            #gen_resource_deserialize
            #gen_resource_serialize
        }
    }

    fn gen_components(&self, add_no_mangle: bool) -> TokenStream {
        let gen_set_component_id = self.gen_set_component_id(add_no_mangle);
        let gen_component_deserialize_json = self.gen_component_deserialize_json(add_no_mangle);
        let gen_component_string_id = self.gen_component_string_id(add_no_mangle);
        let gen_component_size = self.gen_component_size(add_no_mangle);
        let gen_component_align = self.gen_component_align(add_no_mangle);
        let gen_component_type = self.gen_component_type(add_no_mangle);
        let gen_component_async_completion_callable =
            self.gen_component_async_completion_callable(add_no_mangle);

        quote! {
            #gen_set_component_id
            #gen_component_deserialize_json
            #gen_component_string_id
            #gen_component_size
            #gen_component_align
            #gen_component_type
            #gen_component_async_completion_callable
        }
    }

    fn gen_module_name(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let module_name = LitCStr::new(
            &CString::new(&*self.module_name).unwrap(),
            self.module_name.span(),
        );
        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn module_name() -> *const ::std::ffi::c_char {
                #module_name.as_ptr()
            }
        }
    }

    fn gen_component_string_id(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, type_path) = self
            .ecs_types
            .iter()
            .map(|ecs_type| &ecs_type.path)
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_string_id(index: usize) -> *const ::std::ffi::c_char {
                match index {
                    #(#index => #type_path::string_id().as_ptr(),)*
                    _ => ::std::ptr::null(),
                }
            }
        }
    }

    fn gen_component_size(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let body = if self.ecs_types.is_empty() {
            quote! {
                ::std::process::abort();
            }
        } else {
            let mut ecs_type_path = self
                .ecs_types
                .iter()
                .map(|ecs_type_info| &ecs_type_info.path);
            let first_ecs_type_path = ecs_type_path.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_ecs_type_path::string_id() {
                    ::std::mem::size_of::<#first_ecs_type_path>()
                } #(else if string_id == #ecs_type_path::string_id() {
                    ::std::mem::size_of::<#ecs_type_path>()
                })* else {
                    ::std::process::abort();
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_size(string_id: *const ::std::ffi::c_char) -> usize {
                #body
            }
        }
    }

    fn gen_component_align(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let body = if self.ecs_types.is_empty() {
            quote! {
                ::std::process::abort();
            }
        } else {
            let mut ecs_type_path = self
                .ecs_types
                .iter()
                .map(|ecs_type_info| &ecs_type_info.path);
            let first_ecs_type_path = ecs_type_path.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_ecs_type_path::string_id() {
                    ::std::mem::align_of::<#first_ecs_type_path>()
                } #(else if string_id == #ecs_type_path::string_id() {
                    ::std::mem::align_of::<#ecs_type_path>()
                })* else {
                    ::std::process::abort();
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_align(string_id: *const ::std::ffi::c_char) -> usize {
                #body
            }
        }
    }

    fn gen_component_type(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let body = if self.ecs_types.is_empty() {
            quote! {
                ::std::process::abort();
            }
        } else {
            let type_enum_prefix: syn::Path = parse_quote! { ::void_public::ComponentType };
            let mut ecs_type_iter = self.ecs_types.iter();

            let first_ecs_type = ecs_type_iter.next().unwrap();
            let first_ecs_type_path = &first_ecs_type.path;

            let first_ecs_type_variant = first_ecs_type.ecs_type.to_ecs_type_variant();
            let first_ecs_type_variant_path: syn::Path =
                parse_quote!(#type_enum_prefix::#first_ecs_type_variant);

            let (ecs_type_path, ecs_type_variant_path) = ecs_type_iter
                .map(|ecs_type_info| -> (_, syn::Path) {
                    let ecs_type_variant = ecs_type_info.ecs_type.to_ecs_type_variant();
                    (
                        &ecs_type_info.path,
                        parse_quote!(#type_enum_prefix::#ecs_type_variant),
                    )
                })
                .split();

            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_ecs_type_path::string_id() {
                    #first_ecs_type_variant_path
                } #(else if string_id == #ecs_type_path::string_id() {
                    #ecs_type_variant_path
                })* else {
                    ::std::process::abort();
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_type(string_id: *const ::std::ffi::c_char) -> ::void_public::ComponentType {
                #body
            }
        }
    }

    fn gen_component_async_completion_callable(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let ecs_types = self
            .ecs_types
            .iter()
            .filter_map(|ecs_type| {
                if let EcsType::AsyncCompletion { callable } = &ecs_type.ecs_type {
                    Some((&ecs_type.path, callable))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let body = if ecs_types.is_empty() {
            quote! {
                ::std::process::abort();
            }
        } else {
            let mut ecs_types = ecs_types.into_iter();
            let (first_ecs_struct_path, first_ecs_callable_path) = ecs_types.next().unwrap();
            let (struct_path, callable_path) = ecs_types.split();

            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_ecs_struct_path::string_id() {
                    #first_ecs_callable_path::string_id().as_ptr()
                } #(else if string_id == #struct_path::string_id() {
                    #callable_path::string_id().as_ptr()
                })* else {
                    ::std::process::abort();
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_async_completion_callable(
                string_id: *const ::std::ffi::c_char,
            ) -> *const ::std::ffi::c_char
            {
                #body
            }
        }
    }

    fn gen_set_component_id(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let mut components = self
            .systems
            .iter()
            .flat_map(|s| &s.inputs)
            .filter_map(|i| {
                if i.is_component() {
                    Some(&i.path)
                } else {
                    None
                }
            })
            .chain(self.ecs_types.iter().map(|s| &s.path))
            .chain(&self.imported_ecs_types)
            .collect::<Vec<_>>();

        components.sort_by_key(|a| a.to_token_stream().to_string());
        components.dedup();

        let body = if components.is_empty() {
            quote! {}
        } else {
            let mut components_iter = components.iter();
            let first_component = components_iter.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_component::string_id() {
                    unsafe { #first_component::set_id(id); }
                } #(else if string_id == #components_iter::string_id() {
                    unsafe { #components_iter::set_id(id); }
                })*
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn set_component_id(string_id: *const ::std::ffi::c_char, id: ::void_public::ComponentId) {
                #body
            }
        }
    }

    fn gen_resource_init(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let resources = self
            .ecs_types
            .iter()
            .filter(|s| matches!(s.ecs_type, EcsType::Resource))
            .map(|ecs_type_info| &ecs_type_info.path)
            .collect::<Vec<_>>();

        let body = if resources.is_empty() {
            quote! { 1 }
        } else {
            let mut resources = resources.into_iter();
            let first_resource = resources.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                if string_id == #first_resource::string_id() {
                    val.cast::<#first_resource>().write(Default::default());
                } #(else if string_id == #resources::string_id() {
                    val.cast::<#resources>().write(Default::default());
                })* else {
                    return 1;
                }

                0
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn resource_init(
                string_id: *const ::std::ffi::c_char,
                val: *mut ::std::ffi::c_void,
            ) -> i32 {
                #body
            }
        }
    }

    fn gen_resource_deserialize(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let resources = self
            .ecs_types
            .iter()
            .filter(|s| matches!(s.ecs_type, EcsType::Resource))
            .map(|ecs_type_info| &ecs_type_info.path)
            .collect::<Vec<_>>();

        let body = if resources.is_empty() {
            quote! { 1 }
        } else {
            let mut resources = resources.into_iter();
            let first_resource = resources.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                let res = if string_id == #first_resource::string_id() {
                    <#first_resource as ::snapshot::Deserialize>::deserialize_in_place(
                        val.cast::<#first_resource>().as_mut().unwrap(),
                        &mut ::snapshot::Deserializer::new(::snapshot::FfiReader::new(
                            reader,
                            read,
                        ))
                    )
                } #(else if string_id == #resources::string_id() {
                    <#resources as ::snapshot::Deserialize>::deserialize_in_place(
                        val.cast::<#resources>().as_mut().unwrap(),
                        &mut ::snapshot::Deserializer::new(::snapshot::FfiReader::new(
                            reader,
                            read,
                        ))
                    )
                })* else {
                    return 1;
                };

                if res.is_ok() {
                    0
                } else {
                    1
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn resource_deserialize(
                string_id: *const ::std::ffi::c_char,
                val: *mut ::std::ffi::c_void,
                reader: *mut ::std::ffi::c_void,
                read: unsafe extern "C" fn(
                    reader: *mut ::std::ffi::c_void,
                    buf: *mut ::std::ffi::c_void,
                    len: usize
                ) -> isize,
            ) -> i32 {
                #body
            }
        }
    }

    fn gen_component_deserialize_json(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let components = self
            .ecs_types
            .iter()
            .filter(|s| matches!(s.ecs_type, EcsType::Component))
            .map(|ecs_type_info| &ecs_type_info.path)
            .collect::<Vec<_>>();

        let body = if components.is_empty() {
            quote! {
                eprintln!("Unknown component {:?}", string_id);
                1
            }
        } else {
            let mut components = components.into_iter();
            let first_component = components.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                let res: Result<_, Box<dyn ::std::error::Error>> = if string_id == #first_component::string_id() {
                    let json_str = std::string::String::from_utf8_lossy(std::slice::from_raw_parts::<u8>(json_ptr.cast(), json_len));
                    match serde_json::from_str(&json_str) {
                        Ok(obj) => {
                            val.cast::<#first_component>().write(obj);
                            Ok(())
                            },
                        Err(e) => { Err(Box::new(e)) }
                    }
                } #(else if string_id == #components::string_id() {
                    let json_str = std::string::String::from_utf8_lossy(std::slice::from_raw_parts::<u8>(json_ptr.cast(), json_len));
                    match serde_json::from_str(&json_str) {
                        Ok(obj) => {
                            val.cast::<#components>().write(obj);
                            Ok(())
                            },
                        Err(e) => { Err(Box::new(e)) }
                    }
                })* else {
                    Err(format!("Unknown component {:?}", string_id).into())
                };

                if let Err(e) = res {
                    eprintln!("Error: {e}");
                    return 1;
                }

                0
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn component_deserialize_json(
                string_id: *const ::std::ffi::c_char,
                val: *mut ::std::ffi::c_void,
                json_ptr: *const ::std::ffi::c_void,
                json_len: usize,
            ) -> i32 {
                #body
            }
        }
    }

    fn gen_resource_serialize(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let resources = self
            .ecs_types
            .iter()
            .filter(|s| matches!(s.ecs_type, EcsType::Resource))
            .map(|ecs_type_info| &ecs_type_info.path)
            .collect::<Vec<_>>();

        let body = if resources.is_empty() {
            quote! { 1 }
        } else {
            let mut resources = resources.into_iter();
            let first_resource = resources.next().unwrap();
            quote! {
                let string_id = ::std::ffi::CStr::from_ptr(string_id);

                let res = if string_id == #first_resource::string_id() {
                    <#first_resource as ::snapshot::Serialize>::serialize(
                        val.cast::<#first_resource>().as_ref().unwrap(),
                        &mut ::snapshot::Serializer::new(::snapshot::FfiWriter::new(
                            writer,
                            write,
                        ))
                    )
                } #(else if string_id == #resources::string_id() {
                    <#resources as ::snapshot::Serialize>::serialize(
                        val.cast::<#resources>().as_ref().unwrap(),
                        &mut ::snapshot::Serializer::new(::snapshot::FfiWriter::new(
                            writer,
                            write,
                        ))
                    )
                })* else {
                    return 1;
                };

                if res.is_ok() {
                    0
                } else {
                    1
                }
            }
        };

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn resource_serialize(
                string_id: *const ::std::ffi::c_char,
                val: *const ::std::ffi::c_void,
                writer: *mut ::std::ffi::c_void,
                write: unsafe extern "C" fn(
                    writer: *mut ::std::ffi::c_void,
                    buf: *const ::std::ffi::c_void,
                    len: usize
                ) -> isize,
            ) -> i32 {
                #body
            }
        }
    }

    fn gen_systems(&self, add_no_mangle: bool) -> TokenStream {
        let gen_system_fn_ffi = self.gen_system_fn_ffi();
        let gen_systems_len = self.gen_systems_len(add_no_mangle);
        let gen_system_name = self.gen_system_name(add_no_mangle);
        let gen_system_is_once = self.gen_system_is_once(add_no_mangle);
        let gen_system_fn = self.gen_system_fn(add_no_mangle);
        let gen_system_args_len = self.gen_system_args_len(add_no_mangle);
        let gen_system_arg_type = self.gen_system_arg_type(add_no_mangle);
        let gen_system_arg_component = self.gen_system_arg_component(add_no_mangle);
        let gen_system_arg_event = self.gen_system_arg_event(add_no_mangle);

        let gen_system_query_args_len = self.gen_system_query_args_len(add_no_mangle);
        let gen_system_query_arg_type = self.gen_system_query_arg_type(add_no_mangle);
        let gen_system_query_arg_component = self.gen_system_query_arg_component(add_no_mangle);

        quote! {
            #gen_system_fn_ffi
            #gen_systems_len
            #gen_system_name
            #gen_system_is_once
            #gen_system_fn
            #gen_system_args_len
            #gen_system_arg_type
            #gen_system_arg_component
            #gen_system_arg_event
            #gen_system_query_args_len
            #gen_system_query_arg_type
            #gen_system_query_arg_component
        }
    }

    fn gen_system_fn_ffi(&self) -> TokenStream {
        self.systems.iter().fold(quote! {}, |token_stream, system| {
            let allow_attr = allow_attr();
            let ffi_ident = system.ffi_ident();
            let ffi_ident = if system.takes_platform_generic {
                quote!(#ffi_ident<P: ::platform::Platform>)
            } else {
                ffi_ident.to_token_stream()
            };
            let function_path = if system.takes_platform_generic {
                let path = &system.path;
                quote!(#path::<P>)
            } else {
                system.path.to_token_stream()
            };
            let args = system.inputs.iter().enumerate().fold(quote! {}, |token_stream, (index, input)| {
                let index = index as isize;
                let this_arg = match input.arg_type {
                    ArgType::Completion => quote! {::void_public::callable::Completion::new(), },
                    ArgType::DataAccessDirect => {
                        let reference = if input.mutable {
                            quote! {&mut}
                        } else {
                            quote! {&}
                        };
                        let pointer_type = if input.mutable {
                            quote! {mut}
                        } else {
                            quote! {const}
                        };
                        let ident = input.path.clone();

                        quote! {#reference *(*data.offset(#index) as *#pointer_type #ident),}
                    },
                    ArgType::EventReader { .. } => quote! {::void_public::EventReader::new(*data.offset(#index)),},
                    ArgType::EventWriter { .. } => quote! {::void_public::EventWriter::new(*data.offset(#index)),},
                    ArgType::Query { .. } => quote! {::void_public::Query::new(*data.offset(#index) as *mut ::std::ffi::c_void),},
                };

                quote! {
                    #token_stream
                    #this_arg
                }
            });

            quote! {
                #token_stream

                #allow_attr
                unsafe extern "C" fn #ffi_ident(data: *const *const ::std::ffi::c_void) -> i32 {
                        ::std::panic::catch_unwind(||{
                        #function_path(
                            #args
                        );
                    })
                    .map(|_| 0)
                    .unwrap_or(1)
                }
            }
        })
    }

    fn gen_systems_len(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let systems_len = self.systems.len();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn systems_len() -> usize {
                #systems_len
            }
        }
    }

    fn gen_system_name(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_ident) = self
            .systems
            .iter()
            .map(SystemInfo::name)
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_name(system_index: usize) -> *const ::std::ffi::c_char {

                match system_index {
                    #(#index => #system_ident.as_ptr(),)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_is_once(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, is_system_once) = self
            .systems
            .iter()
            .map(|system| system.is_once)
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_is_once(system_index: usize) -> bool {
                match system_index {
                    #(#index => #is_system_once,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_fn(&self, add_no_mangle: bool) -> TokenStream {
        let has_any_generic = self
            .systems
            .iter()
            .any(|system_info| system_info.takes_platform_generic);
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle && !has_any_generic);
        let allow_attr = allow_attr();

        let system_fn = if has_any_generic {
            quote! {system_fn<P: ::platform::Platform>}
        } else {
            quote! {system_fn}
        };

        let (index, function_name) = self
            .systems
            .iter()
            .map(|system_info| {
                let ident = system_info.ffi_ident();
                if system_info.takes_platform_generic {
                    quote!(#ident::<P>)
                } else {
                    ident.to_token_stream()
                }
            })
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn #system_fn(system_index: usize) -> unsafe extern "C" fn(*const *const ::std::ffi::c_void) -> i32 {
                match system_index {
                    #(#index => #function_name,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_args_len(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        let (index, system_inputs_length) = self
            .systems
            .iter()
            .map(|system_info| system_info.inputs.len())
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_args_len(system_index: usize) -> usize {
                match system_index {
                    #(#index => #system_inputs_length,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_arg_type(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_args) = self
            .systems
            .iter()
            .map(|system_info| {
                let (index, arg_type) = system_info
                    .inputs
                    .iter()
                    .map(|input| -> syn::Path {
                        let arg_type: Ident = match input.arg_type {
                            ArgType::Completion => parse_quote!(Completion),
                            ArgType::DataAccessDirect if input.mutable => {
                                parse_quote!(DataAccessMut)
                            }
                            ArgType::DataAccessDirect => parse_quote!(DataAccessRef),
                            ArgType::EventReader { .. } => parse_quote!(EventReader),
                            ArgType::EventWriter { .. } => parse_quote!(EventWriter),
                            ArgType::Query { .. } => parse_quote!(Query),
                        };
                        parse_quote!(::void_public::ArgType::#arg_type)
                    })
                    .enumerate()
                    .split();
                quote! {
                    match arg_index {
                        #(#index => #arg_type,)*
                        _ => std::process::abort(),
                    }
                }
            })
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_arg_type(system_index: usize, arg_index: usize) -> ::void_public::ArgType {
                match system_index {
                    #(#index => #system_args,)*
                    _ => std::process::abort(),
                }
            }
        }
    }

    fn gen_system_arg_component(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_args) = self
            .systems
            .iter()
            .map(|system_info| {
                let (index, arg_types) = system_info
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, system_input_info)| {
                        if matches!(system_input_info.arg_type, ArgType::DataAccessDirect) {
                            let ident = system_input_info.path.clone();
                            Some((index, quote! { #ident::string_id().as_ptr()}))
                        } else {
                            None
                        }
                    })
                    .split();

                quote! {
                    match arg_index {
                        #(#index => #arg_types,)*
                        _ => ::std::process::abort(),
                    }
                }
            })
            .enumerate()
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_arg_component(system_index: usize, arg_index: usize) -> *const ::std::ffi::c_char {
                match system_index {
                    #(#index => #system_args,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_arg_event(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_args) = self.systems
                .iter()
                .map(|system_info| {
                    let (index, arg_types) =
                        system_info.inputs.iter().enumerate().filter_map(
                            |(index, system_input_info)| match &system_input_info.arg_type {
                                ArgType::EventReader { input } | ArgType::EventWriter { input } => {
                                    let input = quote!(#input).to_string();
                                    let input_generic_removed = match input.find("<") {
                                        Some(index) => input[..(index - 1)].to_string(),
                                        None => input
                                    };
                                    let input_generic_removed = format_ident!("{}", input_generic_removed);
                                    Some((
                                        index,
                                        quote! { ::void_public::event_name!(#input_generic_removed).as_ptr()},
                                    ))
                                }
                                _ => None,
                            },
                        ).split();

                    quote! {
                        match arg_index {
                            #(#index => #arg_types,)*
                            _ => ::std::process::abort(),
                        }
                    }
                })
                .enumerate().split();
        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_arg_event(system_index: usize, arg_index: usize) -> *const ::std::ffi::c_char {
                match system_index {
                    #(#index => #system_args,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_query_args_len(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_args) = self
            .systems
            .iter()
            .enumerate()
            .filter(|(_, system_info)| {
                system_info
                    .inputs
                    .iter()
                    .any(|input| matches!(input.arg_type, ArgType::Query { .. }))
            })
            .map(|(outer_index, system_info)| {
                let (index, arg_types) = system_info
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(inner_index, input)| {
                        if let ArgType::Query { inputs } = &input.arg_type {
                            Some((inner_index, inputs.len()))
                        } else {
                            None
                        }
                    })
                    .split();

                let token_stream = quote! {
                    match arg_index {
                        #(#index => #arg_types,)*
                        _ => ::std::process::abort(),
                    }
                };
                (outer_index, token_stream)
            })
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_query_args_len(system_index: usize, arg_index: usize) -> usize {
                match system_index {
                    #(#index => #system_args,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_query_arg_type(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();
        let (index, system_args) = self
            .systems
            .iter()
            .enumerate()
            .filter(|(_, system)| {
                system
                    .inputs
                    .iter()
                    .any(|input| matches!(input.arg_type, ArgType::Query { .. }))
            })
            .map(|(outer_index, system_info)| {
                let (index, arg_types) = system_info
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(middle_index, input)| {
                        if let ArgType::Query { inputs } = &input.arg_type {
                            let (index, query_types) = inputs
                                .iter()
                                .map(|system_input_info| -> syn::Path {
                                    let ident = format_ident!(
                                        "DataAccess{}",
                                        if system_input_info.mutable {
                                            "Mut"
                                        } else {
                                            "Ref"
                                        }
                                    );
                                    parse_quote!(::void_public::ArgType::#ident)
                                })
                                .enumerate()
                                .split();
                            let token_stream = quote! {
                                match query_index {
                                    #(#index => #query_types,)*
                                    _ => ::std::process::abort(),
                                }
                            };
                            Some((middle_index, token_stream))
                        } else {
                            None
                        }
                    })
                    .split();

                let token_stream = quote! {
                    match arg_index {
                        #(#index => #arg_types,)*
                        _ => ::std::process::abort(),
                    }
                };
                (outer_index, token_stream)
            })
            .split();
        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_query_arg_type(
                system_index: usize,
                arg_index: usize,
                query_index: usize,
            ) -> ::void_public::ArgType {
                match system_index {
                    #(#index => #system_args,)*
                    _ => ::std::process::abort(),
                }
            }
        }
    }

    fn gen_system_query_arg_component(&self, add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        // SAFETY: verify tuple layout
        let layout_check = self.systems.iter().flat_map(|system| {
            system
                .inputs
                .iter()
                .filter_map(|input| match &input.arg_type {
                    ArgType::Query { inputs } if inputs.len() > 1 => Some(inputs),
                    _ => None,
                })
                .map(|query_inputs| {
                    let (query_input_params, input_type) = query_inputs.iter().map(|input| {
                        let mut_or_const = if input.mutable { quote!(*mut) } else { quote!(*const) };
                        let input_ident = input.path.clone();
                        let query_input_params = quote!(#mut_or_const #input_ident);
                        (query_input_params, quote!(::std::ptr::null_mut()))
                    }).split();
                    let layout_check_index = (1..query_inputs.len()).map(Index::from);
                    let layout_check_index_minus_one = (0..(query_inputs.len() - 1)).map(Index::from);
                    quote! {
                        let layout_check: (#(#query_input_params,)*)= (#(#input_type,)*);
                        #(assert_eq!(&layout_check.#layout_check_index as *const _ as usize - &layout_check.#layout_check_index_minus_one as *const _ as usize, ::std::mem::size_of::<*const ::std::ffi::c_void>());)*
                    }
                })
            });

        let (index, system_args) = self
            .systems
            .iter()
            .enumerate()
            .filter(|(_, system)| {
                system
                    .inputs
                    .iter()
                    .any(|input| matches!(input.arg_type, ArgType::Query { .. }))
            })
            .map(|(outer_index, system_info)| {
                let (index, arg_types) = system_info
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(middle_index, input)| {
                        if let ArgType::Query { inputs } = &input.arg_type {
                            let (index, query_idents) = inputs
                                .iter()
                                .map(|system_input_info| system_input_info.path.clone())
                                .enumerate()
                                .split();
                            let token_stream = quote! {
                                match query_index {
                                    #(#index => #query_idents::string_id().as_ptr(),)*
                                    _ => ::std::process::abort(),
                                }
                            };
                            Some((middle_index, token_stream))
                        } else {
                            None
                        }
                    })
                    .split();

                let token_stream = quote! {
                    match arg_index {
                        #(#index => #arg_types,)*
                        _ => ::std::process::abort(),
                    }
                };
                (outer_index, token_stream)
            })
            .split();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub extern "C" fn system_query_arg_component(
                system_index: usize,
                arg_index: usize,
                query_index: usize,
            ) -> *const ::std::ffi::c_char {
                #(#layout_check)*

                match system_index {
                    #(#index => #system_args,)*
                    _ => std::process::abort(),
                }
            }
        }
    }

    fn gen_load_engine_proc_addrs(add_no_mangle: bool) -> TokenStream {
        let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
        let allow_attr = allow_attr();

        quote! {
            #optional_no_mangle
            #allow_attr
            pub unsafe extern "C" fn load_engine_proc_addrs(
                get_proc_addr: unsafe extern "C" fn(*const ::std::ffi::c_char) -> *const ::std::ffi::c_void
            ) {
                use ::std::mem::transmute;
                use ::void_public::{*, callable::*};
                // Core
                _ADD_COMPONENTS_FN = transmute(get_proc_addr(c"add_components".as_ptr()));
                _CALL_FN = transmute(get_proc_addr(c"call".as_ptr()));
                _CALL_ASYNC_FN = transmute(get_proc_addr(c"call_async".as_ptr()));
                _COMPLETION_COUNT_FN = transmute(get_proc_addr(c"completion_count".as_ptr()));
                _COMPLETION_GET_FN = transmute(get_proc_addr(c"completion_get".as_ptr()));
                _DESPAWN = transmute(get_proc_addr(c"despawn".as_ptr()));
                _ENTITY_LABEL_FN = transmute(get_proc_addr(c"entity_label".as_ptr()));
                _EVENT_COUNT_FN = transmute(get_proc_addr(c"event_count".as_ptr()));
                _EVENT_GET_FN = transmute(get_proc_addr(c"event_get".as_ptr()));
                _EVENT_SEND_FN = transmute(get_proc_addr(c"event_send".as_ptr()));
                _GET_PARENT_FN = transmute(get_proc_addr(c"get_parent".as_ptr()));
                _LOAD_SCENE = transmute(get_proc_addr(c"load_scene".as_ptr()));
                _SET_ENTITY_LABEL_FN = transmute(get_proc_addr(c"set_entity_label".as_ptr()));
                _SET_PARENT_FN = transmute(get_proc_addr(c"set_parent".as_ptr()));
                _SET_SYSTEM_ENABLED_FN = transmute(get_proc_addr(c"set_system_enabled".as_ptr()));
                _SPAWN = transmute(get_proc_addr(c"spawn".as_ptr()));
                _QUERY_FOR_EACH_FN = transmute(get_proc_addr(c"query_for_each".as_ptr()));
                _QUERY_GET_FN = transmute(get_proc_addr(c"query_get".as_ptr()));
                _QUERY_GET_ENTITY_FN = transmute(get_proc_addr(c"query_get_entity".as_ptr()));
                _QUERY_GET_LABEL_FN = transmute(get_proc_addr(c"query_get_label".as_ptr()));
                _QUERY_LEN_FN = transmute(get_proc_addr(c"query_len".as_ptr()));
                _QUERY_PAR_FOR_EACH_FN = transmute(get_proc_addr(c"query_par_for_each".as_ptr()));
                _REMOVE_COMPONENTS_FN = transmute(get_proc_addr(c"remove_components".as_ptr()));
            }
        }
    }
}

fn gen_version(add_no_mangle: bool) -> TokenStream {
    let optional_no_mangle = generate_optional_no_mangle(add_no_mangle);
    let allow_attr = allow_attr();
    quote! {
        #optional_no_mangle
        #allow_attr
        pub extern "C" fn void_target_version() -> u32 {
            ::void_public::ENGINE_VERSION
        }
    }
}
