//! Generates the C FFI layer from Rust code.

use std::{env::current_dir, path::Path};

use build_tools::FfiBuilder;
use generate_flat_buffers::GenerateFlatBuffers;

fn main() {
    GenerateFlatBuffers::new()
        .in_path(Path::new("events/texture_events.fbs"))
        .write();
    GenerateFlatBuffers::new()
        .in_path(Path::new("events/text_events.fbs"))
        .write();

    FfiBuilder::new()
        .input_path(&current_dir().unwrap().join("src/ecs_module.rs"))
        .add_no_mangle(false)
        .write();
}
