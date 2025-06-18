//! Build compile-time requirements for this package.

use std::path::PathBuf;

use build_tools::FfiBuilder;
use generate_flat_buffers::GenerateFlatBuffers;

fn main() {
    let in_dir = PathBuf::from("../../modules/ipc/src");
    GenerateFlatBuffers::new().in_path(&in_dir).write();

    FfiBuilder::new().write();
}
