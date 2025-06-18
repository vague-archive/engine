use glam::{Mat3, Mat4, Vec3, Vec4, quat};

/// See `EmittorDescriptor::amount` for more about how these modes work.
#[derive(Debug, Clone, Copy)]
pub enum SpawningMode {
    Burst,
    Flow,
}

#[derive(Debug, Clone, Copy)]
pub enum SpawnPattern {
    /// Particles spawn within the bounds of a circle with starting velocity moving
    /// away from the center of the circle.
    Circle,
    /// Particles spawn on the boundary of a circle with starting velocity moving
    /// away from the center of the circle.
    CircleBoundary,
    /// Particles spawn at a single point and move in a random range of directions
    /// around the emitter's +y axis (see `EmitterDescriptor::cone_spread_angle`).
    Cone,
    /// Particles spawn in the bounds of a square with starting velocity in the
    /// emitter's +y axis.
    Square,
}

#[derive(Debug, Clone)]
pub struct ModulationTexture {
    pub texture_name: String,
    /// `tile_x/y` will tile the modulation texture multiple times for values > 1 and
    /// less than one time for values < 1 (in the latter case, making the scale of
    /// the modulation texture greater than the scale of the particle texture).
    pub tile_x: f32,
    pub tile_y: f32,
    /// `scroll_speed_x/y` represent the UV units/s at which modulation texture UVs
    /// are scrolled.
    pub scroll_speed_x: f32,
    pub scroll_speed_y: f32,
}

#[derive(Debug, Clone)]
pub struct EmitterDescriptor {
    /// When set, particles are emitted (and simulated) in world-space (rather
    /// than the default ParticleEffect-local-space). This means each particle's
    /// initial position/velocity/rotation/scale are stored in world-space (by
    /// applying the current transform for the `ParticleEffect`). Simulation
    /// then proceeds in world-space, which means any updates to the
    /// `ParticleEffect`'s transform after emission are ignored.
    pub use_world_space_emission: bool,
    /// When set, all particles are destroyed immediately when this `Emitter`'s
    /// `ParticleEffect` is removed (via calling
    /// `ParticleEffectManager::remove`). If not set, emission stops, but
    /// existing particles are allowed to live out the remainder of their
    /// lifetimes.
    pub should_destroy_all_particles_on_remove: bool,
    /// Transform captures the translation/rotation/scale that is applied to the
    /// specified `spawn_pattern` (transform is only applied when generating
    /// initial particle positions/velocities).
    pub transform: Mat3,
    pub spawning_mode: SpawningMode,
    pub spawn_pattern: SpawnPattern,
    /// `SpawnPattern::Cone` emits particles moving in directions in
    /// [-`cone_spread_angle`,`cone_spread_angle`] from the emitter's +y axis. (All
    /// angles are specified in radians.)
    pub cone_spread_angle: f32,
    /// Each RGBA color is LDR (in [0,1]). When particles are rendered, their color
    /// is scaled by the multipler "1.0 + glow" to make them (potentially) HDR.
    /// Note that this color is *not* premultiplied.
    pub color: Vec4,
    pub glow: f32,
    /// This is the "main" particle texture
    pub texture_name: String,
    /// An emitter may have up to 2 modulation textures, which have independent
    /// UV scales and scroll speeds (and are sampled using "repeat" mode). These
    /// textures are multiplied into the main particle texture.
    pub modulation_texture1: Option<ModulationTexture>,
    pub modulation_texture2: Option<ModulationTexture>,
    /// This is either the number of particles emitted (for `SpawningMode::Burst`) or
    /// the rate of emission (particles/second) for `SpawningMode::Flow`.
    ///
    /// Note that this is a float because it is an emission rate when
    /// `SpawningMode::Flow` is used. The value is `floor()`'d when `SpawningMode::Burst`
    /// is used.
    pub amount: f32,
    pub lifetime: f32,
    pub start_speed: f32,
    pub gravity: f32,
    /// `start_size` and `end_size` are both in [0,1] and get scaled by `size_multiplier`
    /// to determine actual particle size.
    pub start_size: f32,
    pub end_size: f32,
    pub size_multiplier: f32,
    /// `end_fade_in` and `start_fade_out` are both in [0,1] and represent t values (over
    /// a particle's lifetime). A particle fades in over [0,`end_fade_in`] and fades
    /// out over [`start_fade_out`,1] (which means no fading is happening over
    /// [`end_fade_in`,`start_fade_out`]).
    pub end_fade_in: f32,
    pub start_fade_out: f32,
    /// All rotations are specified in radians. Particles start at `start_rotation`.
    /// They are assigned a random angular velocity in [`min_random_spin`,`max_random_spin`]
    /// (which are specified in radians/s).
    pub start_rotation: f32,
    pub min_random_spin: f32,
    pub max_random_spin: f32,
    /// If both `trail_lifetime` and `trail_width` are > 0, particle trails are
    /// enabled for this emitter. Particle trails appear as "ribbons" that track
    /// the last `trail_lifetime` seconds of particles' motion.
    ///
    /// `trail_width` controls the width of these trails at their origination (a
    /// particle's current position), and like `start/end_size` is in [0,1] and
    /// scaled by `size_multiplier`.
    ///
    /// Trails are rendered as a solid color (the `color` field). Their `alpha`
    /// is scaled by the current 'fade' alpha of the corresponding particle.
    /// Trails also grow narrower and more translucent as they age.
    pub trail_lifetime: f32,
    pub trail_width: f32,
}

impl Default for EmitterDescriptor {
    fn default() -> Self {
        EmitterDescriptor {
            use_world_space_emission: false,
            should_destroy_all_particles_on_remove: true,
            transform: Mat3::IDENTITY,
            spawning_mode: SpawningMode::Flow,
            spawn_pattern: SpawnPattern::Cone,
            cone_spread_angle: 0.5_f32,
            color: Vec4::ONE,
            glow: 0_f32,
            texture_name: "".to_string(),
            modulation_texture1: None,
            modulation_texture2: None,
            amount: 5_f32,
            lifetime: 1.0,
            start_speed: 100_f32,
            gravity: 0_f32,
            start_size: 15_f32,
            end_size: 40_f32,
            size_multiplier: 1_f32,
            end_fade_in: 0_f32,
            start_fade_out: 0_f32,
            start_rotation: 0_f32,
            min_random_spin: 0_f32,
            max_random_spin: 0_f32,
            trail_lifetime: 0_f32,
            trail_width: 0_f32,
        }
    }
}

#[derive(Debug)]
pub struct ParticleEffectDescriptor {
    pub emitters: Vec<EmitterDescriptor>,
}

// A `ParticleEffect` can be created from a `ParticleEffectDescriptor`. The plan is to
// eventually support all functionality provided by the VFX tool.

impl ParticleEffectDescriptor {
    /// This parses the JSON format output by the VFX tool and returns a `ParticleEffectDescriptor`
    /// that can be cached/reused to create the same effect multiple times.
    ///
    /// TODO(miketuritzin): move this parsing code to Typescript, as we want descriptors to
    /// be created/modified on that side.
    pub fn from_json(json: &str) -> ParticleEffectDescriptor {
        let mut emitter_descriptors = Vec::<EmitterDescriptor>::new();

        let parsed_json: serde_json::Value = serde_json::from_str(json).unwrap();
        let parsed_json_emitters = parsed_json["emitters"].as_array().unwrap();

        // Emitter parsing code

        let vector3_from_parsed_json = |v: &serde_json::Value| {
            Vec3::new(
                v["x"].as_f64().unwrap() as f32,
                v["y"].as_f64().unwrap() as f32,
                v["z"].as_f64().unwrap() as f32,
            )
        };

        let quaternion_from_parsed_json = |v: &serde_json::Value| {
            quat(
                v["x"].as_f64().unwrap() as f32,
                v["y"].as_f64().unwrap() as f32,
                v["z"].as_f64().unwrap() as f32,
                v["w"].as_f64().unwrap() as f32,
            )
        };

        let modulation_texture_from_parsed_json = |mt: &serde_json::Value| -> ModulationTexture {
            ModulationTexture {
                texture_name: String::from(mt["texture_name"].as_str().unwrap()),
                tile_x: mt["tile_x"].as_f64().unwrap() as f32,
                tile_y: mt["tile_y"].as_f64().unwrap() as f32,
                scroll_speed_x: mt["speed_x"].as_f64().unwrap() as f32,
                scroll_speed_y: mt["speed_y"].as_f64().unwrap() as f32,
            }
        };

        for parsed_json_emitter in parsed_json_emitters {
            // Use a default of 'false' for use_world_space_emission, as it's
            // not provided currently by the vfx16 tool (but can obviously be
            // added to JSON manually).
            let use_world_space_emission = parsed_json_emitter["use_world_space_emission"]
                .as_bool()
                .unwrap_or(false);

            // Use a default of 'true' for should_destroy_all_particles_on_remove.
            // This value isn't provided by vfx-16, and things will not look
            // good if it's set to false across the board, as some particles
            // with very long lifetimes are currently expected to be destroyed
            // immediately.
            let should_destroy_all_particles_on_remove =
                parsed_json_emitter["should_destroy_all_particles_on_remove"]
                    .as_bool()
                    .unwrap_or(true);

            let position = vector3_from_parsed_json(&parsed_json_emitter["transform_position"]);
            let rotation = quaternion_from_parsed_json(&parsed_json_emitter["transform_rotation"]);
            let scale = vector3_from_parsed_json(&parsed_json_emitter["transform_scale"]);

            // The JSON format specifies 3D position/rotation/scale even for 2D emitters (though the transform in
            // practice should end up as 2D-only). Because quaternions are involved, create a 4x4 matrix and then
            // convert that to 3x3 by ignoring the z components of translation/rotation/scale in the matrix.
            let transform_mat4 = Mat4::from_scale_rotation_translation(scale, rotation, position);
            #[rustfmt::skip]
            let transform = Mat3::from_cols_array(&[
                transform_mat4.x_axis.x, transform_mat4.x_axis.y, 0.0,
                transform_mat4.y_axis.x, transform_mat4.y_axis.y, 0.0,
                transform_mat4.w_axis.x, transform_mat4.w_axis.y, 1.0,
            ]);

            let spawning_mode = match parsed_json_emitter["spawning_mode"].as_str().unwrap() {
                // "BURST_REPEAT" is a legacy mode that's still output by the VFX tool - it should be treated
                // identically to "BURST".
                "BURST" | "BURST_REPEAT" => SpawningMode::Burst,
                "FLOW" => SpawningMode::Flow,
                _ => panic!("Unrecognized 'spawning_mode'"),
            };

            let spawn_pattern = match parsed_json_emitter["pattern"].as_str().unwrap() {
                "SPHERE" => SpawnPattern::Circle,
                "SPHERE_SURFACE" => SpawnPattern::CircleBoundary,
                "CONE" => SpawnPattern::Cone,
                "BOX" => SpawnPattern::Square,
                _ => panic!("Unrecognized 'pattern'"),
            };

            let cone_spread_angle = match spawn_pattern {
                // Convert all angles from degrees to radians
                SpawnPattern::Cone => {
                    parsed_json_emitter["spread"].as_f64().unwrap().to_radians() as f32
                }
                _ => 0.0,
            };

            // "color" is a string that is 8 chars long, in RGBA format with two (hex) chars for
            // each channel.
            let color_hex_str = parsed_json_emitter["color"].as_str().unwrap();
            let color = Vec4::new(
                (i32::from_str_radix(&color_hex_str[0..2], 16).unwrap() as f32) / 255.0,
                (i32::from_str_radix(&color_hex_str[2..4], 16).unwrap() as f32) / 255.0,
                (i32::from_str_radix(&color_hex_str[4..6], 16).unwrap() as f32) / 255.0,
                (i32::from_str_radix(&color_hex_str[6..8], 16).unwrap() as f32) / 255.0,
            );

            let textures = parsed_json_emitter["textures"].as_array().unwrap();
            // At least 1 texture must be provided, and the first is the "main" particle texture.
            // Though the JSON format includes modulation-texture-related fields for this
            // texture, they are always set to defaults and should be ignored.
            let texture_name = String::from(textures[0]["texture_name"].as_str().unwrap());
            // The VFX tool supports up to 2 additional textures, which are "modulation" textures
            // that are treated differently from the "main" texture.
            let modulation_texture1 = if textures.len() > 1 {
                Some(modulation_texture_from_parsed_json(&textures[1]))
            } else {
                None
            };
            let modulation_texture2 = if textures.len() > 2 {
                Some(modulation_texture_from_parsed_json(&textures[2]))
            } else {
                None
            };

            let amount = match spawning_mode {
                SpawningMode::Burst => parsed_json_emitter["burst"].as_i64().unwrap() as f32,
                SpawningMode::Flow => parsed_json_emitter["rate"].as_f64().unwrap() as f32,
            };

            emitter_descriptors.push(EmitterDescriptor {
                use_world_space_emission,
                should_destroy_all_particles_on_remove,
                transform,
                spawning_mode,
                spawn_pattern,
                cone_spread_angle,
                color,
                glow: (parsed_json_emitter["glow"].as_f64().unwrap() as f32),
                texture_name,
                modulation_texture1,
                modulation_texture2,
                amount,
                lifetime: (parsed_json_emitter["lifetime"].as_f64().unwrap() as f32),
                start_speed: (parsed_json_emitter["speed"].as_f64().unwrap() as f32),
                gravity: (parsed_json_emitter["gravity"].as_f64().unwrap() as f32),
                start_size: (parsed_json_emitter["start_size"].as_f64().unwrap() as f32),
                end_size: (parsed_json_emitter["end_size"].as_f64().unwrap() as f32),
                size_multiplier: (parsed_json_emitter["size_multiplier"].as_f64().unwrap() as f32),
                end_fade_in: (parsed_json_emitter["fade_in_time"].as_f64().unwrap() as f32),
                start_fade_out: (parsed_json_emitter["fade_out_time"].as_f64().unwrap() as f32),
                // Convert all angles from degrees to radians
                //
                // Negate these values because the VFX tool intends positive rotations
                // to be clockwise. Also note that we intentionally swap min/max_rotation_s
                // because of this sign flipping.
                start_rotation: -(parsed_json_emitter["start_rotation"]
                    .as_f64()
                    .unwrap()
                    .to_radians() as f32),
                min_random_spin: -(parsed_json_emitter["max_rotation_s"]
                    .as_f64()
                    .unwrap()
                    .to_radians() as f32),
                max_random_spin: -(parsed_json_emitter["min_rotation_s"]
                    .as_f64()
                    .unwrap()
                    .to_radians() as f32),
                trail_lifetime: parsed_json_emitter["trail_lifetime"].as_f64().unwrap() as f32,
                trail_width: parsed_json_emitter["trail_width"].as_f64().unwrap() as f32,
            });
        }

        ParticleEffectDescriptor {
            emitters: emitter_descriptors,
        }
    }
}
