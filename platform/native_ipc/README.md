# Tooling IPC (Inter-Process Communication)

Tooling in this context is referring to making tools, so this library offers
features to create tools with talk over IPC to a running game engine.

This is done by making a connection over a WebSocket and then sending (and
receiving) messages in a FlatBuffers format. The schema is defined in
[`./src/tooling_messages.fbs`].

See [`//tool/ipc_tester`] for an example.

## Features

The "Tooling IPC" allows a remote process to alter the game engine.
Contrast with:

- "modules/ipc" which provided communication to modules within the game
- "modules/editor" which allows inspecting and altering the game state

Altering the engine from the platform allows for features such as:
- loading a new module into the engine on-the-fly
- getting a list of modules in the game
- getting a list of systems currently running
- pausing engine execution (different from the user pausing the game)

## Usage

Create a WebSocket listening on a given port (call once):

```
pub use native_ipc::tooling::ToolingIpc;
let mut tooling_ipc = ToolingIpc::on_port(9002).unwrap();
```

Before each call to `engine::frame()` check whether to call it with
something like this:

```ignore
let mut engine = NativeGameEngine::new(gpu, width, height, &js_options);
loop {
  if tooling_ipc.run_frame(&mut engine) {
      engine.frame();
  }
}
```
