use void_public::input::InputState;

use crate::{
    editor_messages::{root_as_message, MessageType},
    handlers::responses::EditorResponse,
};

pub fn process(bytes: &[u8], input_state: &mut InputState) -> Option<EditorResponse> {
    match root_as_message(bytes) {
        Ok(message) => {
            match message.message_type() {
                MessageType::InputMessage => {
                    let input_msg = message.message_as_input_message()?;
                    process_keyboard_input_message(&input_msg, input_state);
                    Some(EditorResponse::Success)
                }
                _ => None, // Not an input message.
            }
        }
        Err(e) => {
            log::error!("Error parsing message: {}", e);
            None
        }
    }
}

fn process_keyboard_input_message(
    input_msg: &crate::editor_messages::InputMessage<'_>,
    input_state: &mut void_public::input::InputState,
) {
    if let Some(keyboard) = input_msg.input_as_keyboard_input_message() {
        input_state.keys[keyboard.key_code()].set_pressed(keyboard.is_pressed());
    }
}

#[cfg(test)]
mod tests {
    use flatbuffers::FlatBufferBuilder;
    use void_public::{event::input::KeyCode, input::InputState};

    use super::*;
    use crate::editor_messages::{
        InputMessage, InputMessageArgs, InputMessageType, KeyboardInputMessage,
        KeyboardInputMessageArgs, Message, MessageArgs,
    };

    /// Creates a keyboard input message with the specified key and pressed state.
    /// Returns the serialized message bytes.
    fn create_keyboard_input_message(key_code: KeyCode, is_pressed: bool) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();

        // Create keyboard input message.
        let keyboard_input = KeyboardInputMessage::create(
            &mut builder,
            &KeyboardInputMessageArgs {
                key_code,
                is_pressed,
            },
        );

        // Create input message with keyboard input.
        let input_message = InputMessage::create(
            &mut builder,
            &InputMessageArgs {
                input_type: InputMessageType::KeyboardInputMessage,
                input: Some(keyboard_input.as_union_value()),
            },
        );

        // Create message with input message.
        let message = Message::create(
            &mut builder,
            &MessageArgs {
                message_type: MessageType::InputMessage,
                message: Some(input_message.as_union_value()),
            },
        );

        builder.finish_minimal(message);
        builder.finished_data().to_vec()
    }

    #[test]
    fn test_process_input_message_space_key_pressed() {
        let space_key_code = KeyCode::Space;
        let mut input_state = InputState::default();

        let response = process(
            &create_keyboard_input_message(space_key_code, true),
            &mut input_state,
        );

        // Check response is success.
        assert!(matches!(response, Some(EditorResponse::Success)));

        // Check input state has space key pressed.
        assert!(input_state.keys[space_key_code].pressed());
    }
}
