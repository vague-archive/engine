use std::{
    ffi::OsStr,
    fs::{File, read_to_string, write},
    path::Path,
    process::Command,
};

use anyhow::{Error, Result};
use convert_case::{Case, Casing};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tempdir::TempDir;

#[derive(Serialize, Deserialize, Debug)]
pub struct PlatformLibrary {
    name: String,
    functions: Vec<Callable>,
    fbs: String,
}

impl PlatformLibrary {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn fbs(&self) -> &str {
        &self.fbs
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Callable {
    name: String,
    parameter_data: Option<String>,
    return_value: Option<String>,
}

pub fn generate_file_string(
    platform_library: &PlatformLibrary,
    additional_events: Option<&[&str]>,
) -> Result<String> {
    // generate events with flatc

    let generated_events = match generate_events(platform_library) {
        Ok(val) => val,
        Err(err) => {
            panic!("flatc error: {err}");
        }
    };

    // generate rust output

    Ok(generate_output(
        platform_library,
        additional_events,
        &generated_events,
    ))
}

pub fn parse_input_from_path(input: &Path) -> Result<PlatformLibrary> {
    if input.extension() != Some(OsStr::new("json")) {
        return Err(Error::msg("must be a json file"));
    }

    serde_json::from_reader(File::open(input)?).map_err(Error::from)
}

pub fn parse_input_from_string(input: &str) -> Result<PlatformLibrary> {
    serde_json::from_str(input).map_err(Error::from)
}

fn generate_events(platform_library: &PlatformLibrary) -> Result<String> {
    let temp_dir = TempDir::new("platform_library")?;

    let fbs_file = temp_dir.path().join("events.fbs");

    // write .fbs file from .json
    write(&fbs_file, &platform_library.fbs)?;

    #[cfg(not(target_os = "windows"))]
    const FLATC_PATH: &str = "flatc";

    #[cfg(target_os = "windows")]
    const FLATC_PATH: &str = "flatc.exe";

    // generate events .rs file
    Command::new(FLATC_PATH)
        .arg("--rust")
        .arg("-o")
        .arg(temp_dir.path())
        .arg(&fbs_file)
        .arg("--gen-object-api")
        .arg("--gen-name-strings")
        .spawn()
        .map_err(|err| Error::msg(format!("could not find flatc: {err}")))?
        .wait()?;

    let events_generated_file = fbs_file.with_file_name("events_generated.rs");

    let events_generated = read_to_string(&events_generated_file).map_err(Error::from)?;

    // cleanup generated files
    drop(fbs_file);
    drop(events_generated_file);
    temp_dir.close()?;

    Ok(events_generated)
}

fn generate_output(
    platform_library: &PlatformLibrary,
    additional_events: Option<&[&str]>,
    events_generated: &str,
) -> String {
    let mut output = String::new();

    output += "// The generated code may contain build warnings. Consider using\n";
    output += "// an annotation such as the following where this file is included:\n";
    output += "// `#![allow(clippy::all, clippy::pedantic, warnings, unused)]`\n\n";

    let mut callables_set_id_map: Vec<(String, String)> = vec![];

    // Write each of the free functions.
    for (index, function) in platform_library
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.name.contains("::"))
    {
        let function_name = function.name.to_case(Case::Pascal);

        callables_set_id_map.push(append_callable(
            &mut output,
            platform_library,
            additional_events,
            function,
            &function_name,
            index,
            "",
        ));
    }

    // Find all system structs and put their functions in a `mod`.

    let mut prefixes = platform_library
        .functions
        .iter()
        .filter_map(|f| f.name.split_once("::"))
        .map(|(s, _)| s)
        .collect::<Vec<_>>();

    prefixes.sort_unstable();
    prefixes.dedup();

    for prefix in prefixes {
        output += &format!("pub mod {} {{\n", prefix.to_case(Case::Pascal));
        output += "    use super::*;\n\n";

        // iterate struct impl functions
        for (index, function) in platform_library
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.name.starts_with(prefix))
        {
            let function_name = function
                .name
                .split_once("::")
                .unwrap()
                .1
                .to_case(Case::Pascal);

            callables_set_id_map.push(append_callable(
                &mut output,
                platform_library,
                additional_events,
                function,
                &function_name,
                index,
                "    ",
            ));
        }

        output += "}\n\n";
    }

    output += "\n\n";

    output += events_generated;

    output
}

/// Writes a callable, which is a struct definition + impls
fn append_callable(
    output: &mut String,
    platform_library: &PlatformLibrary,
    additional_events: Option<&[&str]>,
    function: &Callable,
    function_name: &str,
    function_index: usize,
    indentation: &str,
) -> (String, String) {
    // generate struct and Callable impl

    let parameters = function.parameter_data.as_ref().map_or_else(
        || "::void_public::callable::Pod<()>".to_owned(),
        |ident| {
            let mut ident = ident.clone();

            let regex = Regex::new(&format!("table\\s{ident}")).unwrap();
            if regex.is_match(&platform_library.fbs)
                || additional_events
                    .is_some_and(|additional_events| additional_events.contains(&ident.as_str()))
            {
                ident.push_str("<'a>");
            }

            ident
        },
    );

    let return_value = function.return_value.as_ref().map_or_else(
        || "::void_public::callable::Pod<()>".to_owned(),
        |ident| {
            let mut ident = ident.clone();

            let regex = Regex::new(&format!("table\\s{ident}")).unwrap();
            if regex.is_match(&platform_library.fbs) {
                ident.push_str("<'a>");
            }

            ident
        },
    );

    *output += &format!("{indentation}pub struct {function_name}Fn;\n\n");
    *output +=
        &format!("{indentation}impl ::void_public::callable::Callable for {function_name}Fn {{\n");
    *output += &format!("{indentation}    type Parameters<'a> = {parameters};\n");
    *output += &format!("{indentation}    type ReturnValue<'a> = {return_value};\n");
    *output += &format!("{indentation}}}\n\n");

    // generate EcsType impl

    let mut cid_var_name = function_name.to_case(Case::ScreamingSnake);
    cid_var_name.insert(0, '_');
    cid_var_name.push_str("_CID");

    let mut component_string_name = platform_library.name.clone();
    component_string_name.push_str("::");
    component_string_name.push_str(&function.name);

    *output += &format!(
        "{indentation}static mut {}: Option<::void_public::ComponentId> = None;\n\n",
        cid_var_name
    );

    *output += &format!("{indentation}impl ::void_public::EcsType for {function_name}Fn {{\n");
    *output += &format!("{indentation}    fn id() -> ::void_public::ComponentId {{\n");
    *output += &format!(
        "{indentation}        unsafe {{ {cid_var_name}.expect(\"ComponentId unassigned\") }}\n"
    );
    *output += &format!("{indentation}    }}\n\n");
    *output += &format!("{indentation}    unsafe fn set_id(id: ::void_public::ComponentId) {{\n");
    *output += &format!("{indentation}        unsafe {{ {cid_var_name} = Some(id); }}\n");
    *output += &format!("{indentation}    }}\n\n");
    *output += &format!("{indentation}    fn string_id() -> &'static ::std::ffi::CStr {{\n");
    *output += &format!("{indentation}        c\"{component_string_name}\"\n");
    *output += &format!("{indentation}    }}\n");
    *output += &format!("{indentation}}}\n\n");

    let function_name_prefix = match platform_library.functions[function_index]
        .name
        .split_once("::")
    {
        Some((mod_name, _)) => format!("{mod_name}::"),
        None => "".to_string(),
    };

    (
        format!("b\"{component_string_name}\""),
        format!("{function_name_prefix}{function_name}Fn::set_id"),
    )
}
