use void_public::EventWriter;

use crate::{
    editor_messages::{
        AspectRatio, AspectRatioMessage, AspectRatioMessageArgs, Message, MessageArgs, MessageType,
    },
    event::MessageToRemote,
    handlers::responses,
};

/// Sends an aspect ratio update to the client.
pub fn send_aspect_ratio(
    channel: u16,
    width: f32,
    height: f32,
    writer: &EventWriter<MessageToRemote<'_>>,
) {
    responses::send_message(channel, writer, |fb_builder| {
        // Create the aspect ratio message.
        let aspect_ratio_offset = {
            let aspect_ratio = AspectRatio::new(width, height);
            let args = AspectRatioMessageArgs {
                aspect_ratio: Some(&aspect_ratio),
            };
            AspectRatioMessage::create(fb_builder, &args)
        };

        // Create and return the Message wrapper.
        Message::create(
            fb_builder,
            &MessageArgs {
                message_type: MessageType::AspectRatioMessage,
                message: Some(aspect_ratio_offset.as_union_value()),
            },
        )
    });
}
