use flatbuffers::{FlatBufferBuilder, WIPOffset};
use void_public::EventWriter;

use crate::{
    editor_messages::{
        Message, MessageArgs, MessageType, PingMessage, PingMessageArgs, ResponseMessage,
        ResponseMessageArgs,
    },
    event::{MessageToRemote, MessageToRemoteArgs},
};

/// Response type for editor message responses.
#[derive(Debug)]
pub enum EditorResponse {
    Success,
    PingMessage(PingMessageData),
    Error(String),
}

/// Data structure for the ping message response.
#[derive(Debug)]
pub struct PingMessageData {
    pub timestamp: i64,
    pub message: String,
}

/// Sends the appropriate response based on the `EditorResponse` type.
pub fn send_editor_response(
    channel: u16,
    response: &EditorResponse,
    writer: &EventWriter<MessageToRemote<'_>>,
) {
    match response {
        EditorResponse::Success => {
            send_success_response(channel, writer);
        }
        EditorResponse::PingMessage(ping_data) => {
            send_ping_response(channel, ping_data, writer);
        }
        EditorResponse::Error(error_msg) => {
            send_error_response(channel, error_msg, writer);
        }
    }
}

fn send_ping_response(
    channel: u16,
    data: &PingMessageData,
    writer: &EventWriter<MessageToRemote<'_>>,
) {
    // Send a ping response using the shared message function.
    send_message(channel, writer, |fb_builder| {
        // Create response message
        let msg_offset = fb_builder.create_string(&data.message);

        // Create the ping message.
        let ping_offset = {
            let args = PingMessageArgs {
                timestamp: data.timestamp, // Echo back the original timestamp
                message: Some(msg_offset),
            };
            PingMessage::create(fb_builder, &args)
        };

        // Create and return the Message wrapper.
        Message::create(
            fb_builder,
            &MessageArgs {
                message_type: MessageType::PingMessage,
                message: Some(ping_offset.as_union_value()),
            },
        )
    });
}

/// Create a flatbuffer message and send it through the `MessageToRemote` channel.
fn send_flatbuffer_message<F>(
    channel: u16,
    writer: &EventWriter<MessageToRemote<'_>>,
    builder_fn: F,
) where
    F: for<'a> FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<Message<'a>>,
{
    writer.write_builder(|builder| {
        let mut fb_builder = FlatBufferBuilder::new();

        // Call the provided builder function to get the message
        let message = builder_fn(&mut fb_builder);

        // Finalize the buffer with minimal overhead
        fb_builder.finish_minimal(message);

        // Get the finished data
        let data = fb_builder.finished_data();

        // Create the MessageToRemote with the FlatBuffer data
        let content_offset = builder.create_vector(data);
        MessageToRemote::create(
            builder,
            &MessageToRemoteArgs {
                channel,
                content: Some(content_offset),
            },
        )
    });
}

/// Public wrapper for `send_flatbuffer_message` that can be used by other handlers.
pub fn send_message<F>(channel: u16, writer: &EventWriter<MessageToRemote<'_>>, builder_fn: F)
where
    F: for<'a> FnOnce(&mut FlatBufferBuilder<'a>) -> WIPOffset<Message<'a>>,
{
    send_flatbuffer_message(channel, writer, builder_fn);
}

/// Sends an error response using `ResponseMessage`.
fn send_error_response(channel: u16, error_msg: &str, writer: &EventWriter<MessageToRemote<'_>>) {
    log::error!("Sending error response: {}", error_msg);

    send_flatbuffer_message(channel, writer, |fb_builder| {
        // Create error message
        let msg_offset = fb_builder.create_string(error_msg);

        // Create a response message with error information
        let response_offset = ResponseMessage::create(
            fb_builder,
            &ResponseMessageArgs {
                success: false,
                message: Some(msg_offset),
            },
        );

        // Create and return the Message wrapper
        Message::create(
            fb_builder,
            &MessageArgs {
                message_type: MessageType::ResponseMessage,
                message: Some(response_offset.as_union_value()),
            },
        )
    });
}

/// Sends a success response using `ResponseMessage`.
fn send_success_response(channel: u16, writer: &EventWriter<MessageToRemote<'_>>) {
    send_flatbuffer_message(channel, writer, |fb_builder| {
        // Create a success response
        let msg_offset = fb_builder.create_string("Success");

        // Create a response message with success information
        let response_offset = ResponseMessage::create(
            fb_builder,
            &ResponseMessageArgs {
                success: true,
                message: Some(msg_offset),
            },
        );

        // Create and return the Message wrapper
        Message::create(
            fb_builder,
            &MessageArgs {
                message_type: MessageType::ResponseMessage,
                message: Some(response_offset.as_union_value()),
            },
        )
    });
}
