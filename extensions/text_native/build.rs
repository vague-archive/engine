//! Generates the C FFI layer from Rust code.

use std::{
    env::{current_dir, var_os},
    fs::{read_to_string, write},
    path::PathBuf,
};

use build_tools::FfiBuilder;
use codegen_rust::{generate_file_string, parse_input_from_string};

fn main() {
    let out_dir = PathBuf::from(var_os("OUT_DIR").unwrap());

    FfiBuilder::new()
        .input_path(&current_dir().unwrap().join("src/ecs_module.rs"))
        .add_no_mangle(false)
        .write();

    if let Err(error) = build_tools::platform_library::write_ffi(
        "text_native_platform",
        &out_dir,
        &current_dir().unwrap().join("src/lib.rs"),
        &current_dir()
            .unwrap()
            .join("src/assets/platform_events/events.fbs"),
        false,
    ) {
        println!("cargo:warning=Error writing platform ffi files in text_native: {error}");
        panic!();
    }

    let metadata_file = out_dir.join("metadata.json");

    let metadata_contents = match read_to_string(metadata_file) {
        Ok(contents) => contents,
        Err(err) => {
            println!("cargo:warning=Error reading metadata.json file: {err}");
            panic!();
        }
    };

    let platform_library = match parse_input_from_string(&metadata_contents) {
        Ok(platform_library) => platform_library,
        Err(err) => {
            println!("cargo:warning=Error converting metadata.json to PlatformLibrary: {err}");
            panic!();
        }
    };

    let platform_generated_file_contents =
        match generate_file_string(&platform_library, Some(&["HandleTextEventParameters"])) {
            Ok(file_contents) => file_contents,
            Err(err) => {
                println!(
                    "cargo:warning=Error generating platform_generated.rs file contents: {err}"
                );
                panic!();
            }
        };

    let platform_generated_file = out_dir.join("platform_generated.rs");

    if let Err(err) = write(
        platform_generated_file.as_path(),
        platform_generated_file_contents,
    ) {
        println!("cargo:warning=Error writing platform_generated.rs file: {err}");
        panic!();
    }
}
