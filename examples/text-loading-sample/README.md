# Prerequisites

* Install [Rust](https://www.rust-lang.org/tools/install)
* Install [cargo-fiasco](https://github.com/vaguevoid/cargo-fiasco/)
* Recommended: Use VSCode

## Usage

* Build platform native (from engine root, run `cargo build`)
* Manual installation and run instructions....
  * `cd` to this directory
  * `cp -R assets ../../target/debug`, note you probably need to only do this once
    * On Windows, `cp assets ../../target/debug`
  * `cargo-fiasco build --copy ../../target/debug --symbols` // You will need to re-run this on any modifications to this module
    * On Windows `cargo-fiasco.exe build --copy ../../target/debug --symbols`
  * `cd` back to engine root
  * `cargo run --bin platform_native`
* Automatic with VSCode
  * You may need to open a VSCode window into this directory only
  * Hit F5
    * You can also hit the Run and Debug icon in the left sidebar and then hit the play button
