use std::env::current_dir;

use build_tools::FfiBuilder;

fn main() {
    FfiBuilder::new()
        .input_path(&current_dir().unwrap().join("src/lib.rs"))
        .add_no_mangle(false)
        .write();
}
