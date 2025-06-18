//! Build compile-time requirements for this package.

use std::process::Command;

use generate_flat_buffers::GenerateFlatBuffers;

fn main() {
    GenerateFlatBuffers::new()
        .type_script()
        .in_path("../../platform/native_common/src".as_ref())
        .out_dir("client/gen".as_ref())
        .write();

    Command::new("bun")
        .arg("build")
        .arg("*.ts")
        .arg("--outdir")
        .arg("../web_root")
        .current_dir("client")
        .spawn()
        .unwrap_or_else(|e| panic!("Running bun build {e}"))
        .wait()
        .unwrap();
}
