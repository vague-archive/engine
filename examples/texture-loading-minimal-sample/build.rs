//! Build compile-time requirements for this package.

use build_tools::FfiBuilder;

fn main() {
    FfiBuilder::new().write();
}
