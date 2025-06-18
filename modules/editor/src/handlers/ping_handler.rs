use crate::{
    editor_messages::Message,
    handlers::responses::{EditorResponse, PingMessageData},
};

/// Handles a ping message and returns an `EditorResponse`.
pub fn handle_ping_message(message: Message<'_>) -> EditorResponse {
    // Try to extract ping message data.
    let Some(ping) = message.message_as_ping_message() else {
        return EditorResponse::Error("Failed to extract ping message data".to_string());
    };

    let timestamp = ping.timestamp();
    let message_text = ping.message().unwrap_or("");

    log::info!(
        "Received ping message with timestamp: {} and message: {}",
        timestamp,
        message_text
    );

    EditorResponse::PingMessage(PingMessageData {
        timestamp,
        message: "Pong from server!".to_string(),
    })
}
