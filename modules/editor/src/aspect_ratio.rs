use void_public::EventWriter;

use crate::event::MessageToRemote;

/// Send aspect ratio update to a specific channel.
pub fn send_aspect_ratio(
    channel: u16,
    writer: &EventWriter<MessageToRemote<'_>>,
    width: f32,
    height: f32,
) {
    crate::handlers::aspect_ratio_handler::send_aspect_ratio(channel, width, height, writer);
}
