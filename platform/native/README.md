# Native Entrypoint

This is the entrypoint for shipped games on native platforms (Windows, macOS, Linux).

## Requirements

1. [Rust](https://www.rust-lang.org/tools/install)
2. [rust-codgen](https://github.com/vaguevoid/cli-tools/blob/main/crates/codegen-rust), installed and in your PATH
3. [flatbuffer](https://github.com/google/flatbuffers/releases), installed at the expected version in you PATH. You can check for the proper version by searching for `flatbuffer` in any of the `Cargo.toml` files and using that version
