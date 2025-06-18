# IPC Agent Example

A simple example to see the WebSocket IPC interface (at ../../modules/ipc) in
action.

See [`../../modules/ipc/README.md`] for information on the IPC Host module.

## Scope

The objective of this example is to show messages going to/from a remote process
through the IPC Host module. It is not a best-practices guide for writing an IPC
protocol.

The minimal (silly) protocol herein uses the first byte of each message to
determine the message content type (kind of message).

Both the local and remote code have custom packing/unpacking for the message
types. Which is an example of what not to do as far as the protocol. In a real
implementation, it would be *much* better if both the local and remote used a
common (and singular) definition (a schema) of the protocol.

## Building

To make use of this example, you'll need the Fiasco engine, the IPC module, and
the ipc_agent module. That last one is this module. The native Fiasco engine can
be built from [`../../platform/native`] and the IPC module is in
[`../../modules/ipc`].

## Usage

Once the `Building` of the game is done and the pieces arranged, simply execute
the platform_native executable.

When the native platform game starts up, it will look in the "modules" folder
and load the IPC Host module and IPC Agent module and start them automatically.

## Concepts covered in this example

This example builds on the concepts covered in [`../moving_box`]. Here are some
of the additional concepts in this example.

The exercises provided in the source are intended to be fun and help understand
these topics.

### Events

- Reading and writing events
- Events are broadcast to all modules in the game

### IPC

- The WebSocket IPC module communicates through engine events
- The `user_id` field is used to uniquely identify modules in events
- The `channel` field is used to uniquely identify IPC connections in events.

### FFI

- The ffi.rs can be imported into a `mod ffi` or similar to group that API
  together

### FlatBuffers

- The `flatc` compiler can be run during built-time with `build.rs` to generate
  flatbuffer definitions for custom messages.
- The FlatBuffer format is used for communicating with the IPC module
  through events.

### Messages

- Separate messages are passed between the two 'ends' of the IPC connection
- While the FlatBuffer format is used for the IPC module, the contents of those
  messages may be some other format (or can be defined with FlatBuffers as
  well).

### WebSocket connection from JavaScript in a web browser

- The [`web_root/index.html`] and [`web_root/websocket_example.js`] open a
  WebSocket from JavaScript which can connect to a native running game. The
  [`../../modules/ipc`] module acts as a WebSocket server for the browser to
  connect to.
