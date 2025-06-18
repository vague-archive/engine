# Editor module

A module that the Fiasco Editor uses to control the engine over IPC.

- E.g. Scene tool spawning an entity to mirror one just added to the scene.
- E.g. Loading the game the dev is editing (not yet supported).

## Usage

### Dependencies

This module requires the `flatc` flatbuffers compiler on the $PATH during build
time (only, this is not a run-time requirement). Be sure to verify that
compatible versions are used for each module accessing the same flatbuffers
formatted data and the flatbuffers libraries in use (it is easiest to simply use
the same version). Tip: `flatc --version` will output version information for
the `flatc` compiler.

The flatbuffers compiler also includes options on how data is written and read
(how utf8 is handled for example). This is not a primer on flatbuffers, but
merely some advice on things to watch for.

### Building

- Currently, the Editor module is not automatically loaded when the engine
  starts.
- To do that, create a script that will build the engine and copy over the built
  Editor module and its dependency, the build IPC Host module and then run the
  engine.
- Example for Mac -

```sh
#!/usr/bin/env bash
set -e

rm -rf target

cargo build --workspace

# Change to repo root from script location

mkdir -p target/debug/modules
mkdir -p target/debug/platform
cp target/debug/libipc.dylib target/debug/modules/. 2>/dev/null || echo "libipc.dylib not found, ignoring"
cp target/debug/libeditor.dylib target/debug/modules/. 2>/dev/null || echo "libeditor.dylib not found, ignoring"

./target/debug/platform_native
```

## Limitations

- Currently, only the `spawn` IPC message is supported.
- More messages are coming soon.
