//! # Inter Process Communication (IPC) support for game systems.
//!
//! This interface allows for sending and receiving messages with another
//! process on the same computer via the `WebSockets` protocol.
//!
//! For schema documentation, see [`./ipc.fbs`].
//!
//! ## Usage
//!
//! ### Set up
//! 1. Build this extension (crate) to generate a dynamic library (DLL on
//!    Windows, .so on Linux, etc.)
//! 2. Copy the library into the "modules/" directory next to the Fiasco
//!    platform executable. E.g.
//!    ```ignore
//!    ./game
//!    ./game/modules/ipc.dll
//!    ./game/platform_native.exe
//!    ```
//! 3. Build the [`./src/ipc.fbs`] (using flatc) to create an API for you chosen
//!    language. If using Rust that's already done as part of building this
//!    crate, just add ipc as a dependency in the Cargo.toml
//!
//! ### Your Schema
//!
//! This extension allows sending binary blobs of data back and forth. It will
//! be a nicer developer experience (your experience) if a schema is defined for
//! what your going to send and receive. This extension does not do this step
//! (it wouldn't and shouldn't know what is being sent - it's just the currier).
//!
//! Consider using something like `FlatBuffers`, `ProtoBuffers`, C-structs, or
//! similar. JSON can even be used. It may help to test your performance and
//! flexibility needs when choosing a packing scheme - some are much faster,
//! some are more flexible.
//!
//! Note: The IPC extension uses `FlatBuffers` for its schema, but that does not
//! place any burden on you to do the same. Choose the packing scheme that's
//! best for your project.
//!
//! ### API
//!
//! - Begin by sending a `PortListen` event with the desired WebSocket port
//!   number. Include a unique string (name) in the `user_id` field (e.g. the
//!   name of the module may be a good choice).
//! - When receiving events from the IPC extension, check the `user_id` field
//!   and ignore events not directed to your module.
//! - Avoid sending secrets, PII, passwords, etc. in Fiasco Events (or protect
//!   them with sufficient encryption) because any other module/system will also
//!   see the events.
//! - When a `PortListenResult` with a result `Success` is received, switch to
//!   the 'other program' (such as the Editor) and open a WebSocket to that port
//!   (likely on the localhost domain).
//! - Once the connection is detected, the IPC Agent Module will receive an
//!   `Opened` event (and the Editor, for example, would receive an onopen
//!   JavaScript event).
//! - From that point onward (until the connection is closed), send and receive
//!   `Message` events.

pub mod systems;
mod websocket_ipc;

pub mod event {
    #![allow(clippy::all, clippy::pedantic, warnings, unused, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/ipc_generated.rs"));
}
