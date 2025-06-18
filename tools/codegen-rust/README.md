# Platform Library Codegen - Rust

This tool takes in a .json metadata file associated with a platform library, and
generates Rust bindings to safely allow the platform library functions to be
called from an ECS module.

A .json metadata file contains information on all available functions in a given
platform library, along with their optional parameter and return types. The
.json metadata also includes an inlined .fbs flatbuffers schema file, describing
all paramter and return type events.

## Usage

**This CLI tool expects the `flatc` tool (executable) to be found in a directory
on the $PATH environment variable.**

`codegen-rust --input metadata.json --output generated.rs`

The `output` flag is optional. If not provided, the output file is placed next
to the `metadata.json` file, and named after the platform library
`platform_library_name.rs`.
