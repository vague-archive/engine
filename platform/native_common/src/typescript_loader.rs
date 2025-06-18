use std::sync::Arc;

use deno_ast::{
    EmitOptions, MediaType, ParseParams, TranspileModuleOptions, TranspileOptions, parse_module,
};
use deno_runtime::{
    deno_core::{
        ModuleLoadResponse, ModuleLoader, ModuleSource, ModuleSourceCode, ModuleSpecifier,
        ModuleType, RequestedModuleType, ResolutionKind, error::ModuleLoaderError, resolve_import,
        url::Url,
    },
    deno_fs::{FileSystem, RealFs},
};

use crate::js::EXTENSION_API;

pub struct TypescriptModuleLoader {
    pub fs: Arc<RealFs>,
}

impl ModuleLoader for TypescriptModuleLoader {
    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<&ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        ModuleLoadResponse::Sync(get_source(module_specifier, &self.fs, maybe_referrer))
    }

    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        Ok(resolve_import(specifier, referrer)?)
    }
}

fn transpile(
    specifier: &ModuleSpecifier,
    code: &str,
    referrer: Option<&ModuleSpecifier>,
) -> Result<ModuleSource, ModuleLoaderError> {
    let parsed_source = parse_module(ParseParams {
        specifier: specifier.clone(),
        text: code.into(),
        media_type: MediaType::TypeScript,
        capture_tokens: true,
        maybe_syntax: None,
        scope_analysis: true,
    })
    .map_err(|err| ModuleLoaderError::Unsupported {
        specifier: err.specifier.into(),
        maybe_referrer: referrer.map(|url| Box::new(url.clone())),
    })?;

    let transpiled = parsed_source
        .transpile(
            &TranspileOptions::default(),
            &TranspileModuleOptions::default(),
            &EmitOptions::default(),
        )
        .map_err(|err| {
            log::error!("failed to transpile typescript source {specifier} {err:?}");

            ModuleLoaderError::Unsupported {
                specifier: Box::new(specifier.clone()),
                maybe_referrer: referrer.map(|url| Box::new(url.clone())),
            }
        })?;

    Ok(ModuleSource::new(
        ModuleType::JavaScript,
        ModuleSourceCode::String(transpiled.into_source().text.into()),
        specifier,
        None,
    ))
}

fn get_source(
    specifier: &Url,
    fs: &Arc<RealFs>,
    referrer: Option<&ModuleSpecifier>,
) -> Result<ModuleSource, ModuleLoaderError> {
    if specifier.as_str() == "module:fiasco-entry" {
        let code = EXTENSION_API[0].load().unwrap();
        return transpile(specifier, code.as_str(), referrer);
    }

    let mut path = specifier.to_file_path().unwrap();
    let is_typescript = path.extension().is_some_and(|ext| ext == "ts");

    // If there is no extension specified, we'll assume it is typescript by default
    if path.extension().is_none() {
        path.set_extension("ts");
    }

    let cow = fs
        .read_file_sync(&path, None)
        .map_err(|_err| ModuleLoaderError::Unsupported {
            specifier: Box::new(specifier.clone()),
            maybe_referrer: referrer.map(|url| Box::new(url.clone())),
        })?;

    let contents = String::from_utf8_lossy(&cow).into_owned();

    if is_typescript {
        transpile(specifier, &contents, referrer)
    } else {
        Ok(ModuleSource::new(
            ModuleType::JavaScript,
            ModuleSourceCode::String(contents.into()),
            specifier,
            None,
        ))
    }
}
