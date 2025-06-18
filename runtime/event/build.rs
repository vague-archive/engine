//! Build compile-time requirements for this package.

use build_tools::FfiBuilder;
use generate_flat_buffers::GenerateFlatBuffers;

fn main() {
    GenerateFlatBuffers::new().write();
    FfiBuilder::new().write();
}
