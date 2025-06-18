//! Message handlers for different message types.
//!
//! Each message type has its own dedicated handler module.

pub mod aspect_ratio_handler;
pub mod handle_if_input_message;
pub mod ping_handler;
pub mod process_message;
pub mod responses;
pub mod spawn_handler;

// Re-export handlers for convenience.
pub use aspect_ratio_handler::send_aspect_ratio;
pub use ping_handler::handle_ping_message;
pub use process_message::process_message;
pub use responses::{send_editor_response, EditorResponse};
pub use spawn_handler::{handle_spawn_message, EngineSpawner, Spawner};
