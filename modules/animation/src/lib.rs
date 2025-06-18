use anyhow::{bail, Result};
use game_module_macro::{system, Component};
use glam::UVec2;
use void_public::{
    graphics::{Rect, TextureRender},
    Component, ComponentId, EcsType, FrameConstants, Query,
};

#[repr(C)]
#[derive(Clone, Copy, Default, serde::Deserialize)]
pub enum PlaybackDirection {
    #[default]
    Forward,
    Reverse,
}

#[repr(C)]
#[derive(Component, Default, serde::Deserialize)]
pub struct SpriteAnimation {
    #[serde(default)]
    pub playback_direction: PlaybackDirection,

    #[serde(default)]
    pub frames_per_second: f32,

    /// `frame_dimensions` contains the # of columns and rows in the sprite sheet
    #[serde(default)]
    pub frame_dimensions: UVec2,

    #[serde(skip_deserializing)]
    pub current_time: f32,

    #[serde(default)]
    pub looping: bool,

    #[serde(default)]
    pub running: bool,
}

impl SpriteAnimation {
    pub fn new(frame_dimensions: UVec2) -> Self {
        SpriteAnimation {
            playback_direction: PlaybackDirection::Forward,
            frames_per_second: 9.0,
            frame_dimensions,
            current_time: 0.,
            looping: true,
            running: true,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        if self.running {
            bail!("SpriteAnimation::start() - Animation is already running.");
        }

        self.current_time = 0.;
        self.running = true;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.running {
            bail!("SpriteAnimation::stop() - Animation is not running.");
        }

        self.running = false;

        Ok(())
    }

    fn update_animation(&mut self, dt: f32, sprite_render: &mut TextureRender) {
        if !self.running {
            return;
        }

        self.current_time += dt;

        let num_frames = self.frame_dimensions.x * self.frame_dimensions.y;
        let current_frame = {
            let current_frame = f32::round(self.current_time * self.frames_per_second) as u32;
            if self.looping {
                current_frame % num_frames
            } else if current_frame >= num_frames {
                0_u32
            } else {
                current_frame
            }
        };

        let current_frame = match self.playback_direction {
            PlaybackDirection::Forward => current_frame,
            PlaybackDirection::Reverse => num_frames - 1 - current_frame,
        };

        let x = 1. / self.frame_dimensions.x as f32;
        let y = 1. / self.frame_dimensions.y as f32;
        let x_offset = x * (current_frame % self.frame_dimensions.x) as f32;
        let y_offset = y * (current_frame / self.frame_dimensions.x) as f32;

        sprite_render.uv_region = Rect::new(x_offset, y_offset, x, y);
    }
}

#[system]
fn update_animations(
    frame_constants: &FrameConstants,
    mut texture_query: Query<(&mut TextureRender, &mut SpriteAnimation)>,
) {
    texture_query.for_each(|(sprite_render, sprite_anim)| {
        if sprite_anim.running {
            sprite_anim.update_animation(frame_constants.delta_time, sprite_render);
        }
    });
}

pub mod ffi {
    #![allow(clippy::all, clippy::pedantic, warnings, unused, unused_imports)]
    use super::*;

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
}
