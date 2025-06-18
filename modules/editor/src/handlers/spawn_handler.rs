use void_public::{bundle, colors::Color, graphics::ColorRender, linalg, Engine, Transform};

use crate::{editor_messages::Message, handlers::responses::EditorResponse};

/// Trait for spawning entities.
pub trait Spawner {
    fn spawn(&self, components: &[&dyn std::any::Any]);
}

/// Default implementation that uses the Engine.
pub struct EngineSpawner;

impl Spawner for EngineSpawner {
    fn spawn(&self, components: &[&dyn std::any::Any]) {
        // Convert to the actual types needed by Engine::spawn
        let color_render = components[0].downcast_ref::<ColorRender>().unwrap();
        let color = components[1].downcast_ref::<Color>().unwrap();
        let transform = components[2].downcast_ref::<Transform>().unwrap();

        Engine::spawn(bundle!(color_render, color, transform,));
    }
}

/// Handles a spawn message.
pub fn handle_spawn_message<S: Spawner>(message: Message<'_>, spawner: &S) -> EditorResponse {
    // Try to extract spawn message data.
    let Some(spawn_msg) = message.message_as_spawn_message() else {
        return EditorResponse::Error("Failed to extract spawn message data".to_string());
    };

    // Extract required fields.
    let transform = spawn_msg.transform();
    let color = spawn_msg.color();
    let color_render = spawn_msg.color_render();

    let color_render_component = ColorRender {
        visible: color_render.visible(),
    };

    let transform_component = Transform {
        position: linalg::Vec3::from_xyz(
            transform.position().x(),
            transform.position().y(),
            transform.position().z(),
        ),
        scale: linalg::Vec2::from_xy(transform.scale().x(), transform.scale().y()),
        ..Default::default()
    };

    let color_component = Color::new(
        color.value().r(),
        color.value().g(),
        color.value().b(),
        color.value().a(),
    );

    spawner.spawn(&[
        &color_render_component,
        &color_component,
        &transform_component,
    ]);

    EditorResponse::Success
}
