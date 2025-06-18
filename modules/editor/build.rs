use std::path::{Path, PathBuf};

use build_tools::FfiBuilder;
use generate_flat_buffers::GenerateFlatBuffers;

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema_path = crate_dir.join("src").join("editor_messages.fbs");
    GenerateFlatBuffers::new().in_path(&schema_path).write();
    GenerateFlatBuffers::new()
        .in_path(Path::new("../../modules/ipc/src"))
        .write();

    // Generate FFI.
    FfiBuilder::new().add_no_mangle(false).write();
}
