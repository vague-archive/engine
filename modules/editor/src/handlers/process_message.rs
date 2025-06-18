//! Message handler for websocket communication with editor.
//!
//! This module provides a structured approach to handling incoming messages
//! from the editor, processing them, and sending appropriate responses.

use crate::{
    editor_messages::{root_as_message, MessageType},
    handlers::{ping_handler, responses::EditorResponse, spawn_handler, EngineSpawner},
};

pub fn process_message(content: &[u8]) -> Option<EditorResponse> {
    let message = match root_as_message(content) {
        Ok(message) => message,
        Err(e) => {
            log::error!("Error parsing message: {}", e);
            return None;
        }
    };

    let message_type = message.message_type();
    log::info!("Processing message of type: {:?}", message_type);

    match message_type {
        MessageType::PingMessage => Some(ping_handler::handle_ping_message(message)),
        MessageType::SpawnMessage => {
            let spawner = EngineSpawner;
            Some(spawn_handler::handle_spawn_message(message, &spawner))
        }
        _ => None, // Ignore message types we don't handle
    }
}

#[cfg(test)]
mod tests {
    use std::{any::Any, cell::RefCell, rc::Rc};

    use flatbuffers::FlatBufferBuilder;
    use void_public::event::{Transform, Vec2, Vec3};

    use super::*;
    use crate::{
        editor_messages::{
            Color, ColorArgs, ColorRGBA, ColorRender, ColorRenderArgs, Message, MessageArgs,
            MessageType, PingMessage, PingMessageArgs, SpawnMessage, SpawnMessageArgs,
        },
        handlers::spawn_handler::{handle_spawn_message, Spawner},
    };

    #[test]
    fn test_process_ping_message() {
        let mut builder = FlatBufferBuilder::new();
        let message_str = builder.create_string("Test ping");

        let ping_message = PingMessage::create(
            &mut builder,
            &PingMessageArgs {
                timestamp: 12345,
                message: Some(message_str),
            },
        );

        let message = Message::create(
            &mut builder,
            &MessageArgs {
                message_type: MessageType::PingMessage,
                message: Some(ping_message.as_union_value()),
            },
        );

        builder.finish_minimal(message);

        let ping_message_bytes = builder.finished_data();

        let response = process_message(ping_message_bytes);

        if let Some(EditorResponse::PingMessage(ping_data)) = response {
            assert_eq!(ping_data.timestamp, 12345);
            assert_eq!(ping_data.message, "Pong from server!");
        } else {
            panic!("Expected PingMessage response, got {:?}", response);
        }
    }

    struct MockSpawner {
        was_called: Rc<RefCell<bool>>,
    }

    impl MockSpawner {
        fn new() -> Self {
            Self {
                was_called: Rc::new(RefCell::new(false)),
            }
        }

        fn was_called(&self) -> bool {
            *self.was_called.borrow()
        }
    }

    impl Spawner for MockSpawner {
        fn spawn(&self, _components: &[&dyn Any]) {
            *self.was_called.borrow_mut() = true;
        }
    }

    #[test]
    fn test_process_spawn_message() {
        let mut builder = FlatBufferBuilder::new();

        let transform = Transform::new(
            &Vec3::new(0.0, 0.0, 0.0),
            &Vec2::new(1.0, 1.0),
            &Vec2::new(0.0, 0.0),
            &Vec2::new(0.5, 0.5),
            0.0,
        );

        let color_rgba = ColorRGBA::new(1.0, 1.0, 1.0, 1.0);

        let color = Color::create(
            &mut builder,
            &ColorArgs {
                value: Some(&color_rgba),
            },
        );

        let color_render = ColorRender::create(&mut builder, &ColorRenderArgs { visible: true });

        let spawn_message = SpawnMessage::create(
            &mut builder,
            &SpawnMessageArgs {
                transform: Some(&transform),
                color: Some(color),
                color_render: Some(color_render),
            },
        );

        let message = Message::create(
            &mut builder,
            &MessageArgs {
                message_type: MessageType::SpawnMessage,
                message: Some(spawn_message.as_union_value()),
            },
        );

        builder.finish_minimal(message);

        let spawn_message_bytes = builder.finished_data();

        let mock_spawner = MockSpawner::new();
        let flatbuffer_message = root_as_message(spawn_message_bytes).unwrap();
        let response = handle_spawn_message(flatbuffer_message, &mock_spawner);

        assert!(mock_spawner.was_called(), "MockSpawner was not called");

        assert!(
            matches!(response, EditorResponse::Success),
            "Expected Success response, got {:?}",
            response
        );
    }
}
