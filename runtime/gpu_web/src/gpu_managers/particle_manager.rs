use std::{
    cmp::min,
    collections::{BTreeSet, HashMap, HashSet},
    num::NonZero,
};

use game_asset::{
    ecs_module::GpuInterface,
    particles::{
        EmitterDescriptor, ModulationTexture, ParticleEffectDescriptor, SpawnPattern, SpawningMode,
    },
    resource_managers::texture_asset_manager::{PendingTexture, TextureAssetManager},
};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec3Swizzles};
use rand::Rng;
use snapshot::{Deserialize, Serialize};
use void_public::{
    AssetId, ComponentId, EcsType, Resource,
    graphics::{ParticleEffectHandle, ParticleManager, TextureId},
};
use wgpu::util::DeviceExt;

use crate::gpu_managers::texture_manager::GpuTextureManager;

#[derive(Copy, Clone, Default)]
struct Particle {
    position: Vec2,
    velocity: Vec2,
    rotation: f32,
    angular_velocity: f32,
    time_alive: f32,
    uv_scroll_t_offset: f32,
    /// This field is only relevant for Emitters using world-space emission. For
    /// such emitters, the world-space scale is stored on emission, as the
    /// `Emitter`'s `ParticleEffect`'s transform may change frame-to-frame.
    ///
    /// Note that only uniform scales in `ParticleEffect` transforms are
    /// supported for now.
    world_space_scale: f32,
}

/// For `Emitter`'s that use particle trails, one of these structs is kept in an
/// array parallel to the `Particle`s array. This data is separate so we don't
/// store more data needlessly for `Emitters` that don't use trails.
#[derive(Copy, Clone, Default)]
struct ParticleTrailData {
    num_samples: usize,
}

#[derive(Copy, Clone, Default)]
struct ParticleTrailSample {
    position: Vec2,
    timestamp: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleQuadVertex {
    uv: [f32; 2],
}

/// This is shipped to the GPU as per-instance particle data. Because this data is
/// stored as vertex attributes, the WebGPU alignment restrictions don't apply.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleInstanceData {
    position: [f32; 2],
    rotation: f32,
    t: f32,
    uv_scroll_t_offset: f32,
    world_space_scale: f32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleTrailVertex {
    position: [f32; 2],
    alpha: f32,
}

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mat_wvp: [[f32; 4]; 4],
    color: [f32; 4],
    modulation_texture1_uv_scale: [f32; 2],
    modulation_texture1_uv_speed: [f32; 2],
    modulation_texture2_uv_scale: [f32; 2],
    modulation_texture2_uv_speed: [f32; 2],
    modulation_texture1_is_enabled: u32,
    modulation_texture2_is_enabled: u32,
    start_size: f32,
    end_size: f32,
    end_fade_in: f32,
    start_fade_out: f32,
    use_world_space_emission: u32,
    /// Added explicit padding (needed because of alignment specifier above) to deal
    /// with compiler error:
    /// "cannot transmute between types of different sizes, or dependently-sized types"
    _padding: [u8; 4],
}

/// These are the initial allocation sizes for buffers managed by `ParticleEffectManager`.
/// buffers will grow dynamically when this capacity is exceeded. The initial capacities
/// have been set quite high because growing the buffers may cause a brief hitch, and
/// the actual space needed is low (e.g., 100k particles is currently under 2MB).
const PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_EMITTERS: usize = 1000;
const PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_PARTICLES: usize = 100000;
const PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_PARTICLE_TRAIL_VERTICES: usize = 200000;
/// Buffer allocations will grow by this factor when their capacity is exceeded.
const PARTICLE_EFFECT_MANAGER_ALLOC_GROWTH_FACTOR: f32 = 1.5;

/// Texture coordinate origin is top-left. Vertices for two triangles of quad
/// are specified in CCW order.
///
/// Note that when these positions are converted from UV -> local space in the
/// shader, the 'up' axis flips AND we flip the Y coordinates, which causes the
/// CCW order to be maintained.
const PARTICLE_QUAD_VERTICES: &[ParticleQuadVertex] = &[
    ParticleQuadVertex { uv: [0.0, 0.0] },
    ParticleQuadVertex { uv: [0.0, 1.0] },
    ParticleQuadVertex { uv: [1.0, 1.0] },
    ParticleQuadVertex { uv: [1.0, 1.0] },
    ParticleQuadVertex { uv: [1.0, 0.0] },
    ParticleQuadVertex { uv: [0.0, 0.0] },
];

const PARTICLE_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// This controls the (max) frequency at which new samples are generated for
/// particle trails. Lower sample rates use less resources but are more likely
/// to appear like a linear approximation to the particle's motion (which they
/// are) when particles change direction rapidly.
///
/// Note that (for now) particle trails are sampled at most once per frame, so
/// this constant is a max value rather than a guarantee.
///
/// This constant is also needed so we can statically size the buffers used to
/// store particle trail samples, as we don't know a priori what the update rate
/// will be.
const MAX_PARTICLE_TRAIL_MAX_SAMPLE_FREQUENCY: f32 = 30.0;
const PARTICLE_TRAIL_SAMPLE_MIN_INTERVAL: f32 = 1.0 / MAX_PARTICLE_TRAIL_MAX_SAMPLE_FREQUENCY;

/// This is used as a key into the `ParticleEffectManager` map that manages bind
/// groups for each unique texture permutation.
#[derive(Eq, Hash, PartialEq, Clone)]
struct EmitterTextureNames {
    texture_name: String,
    modulation_texture1: Option<String>,
    modulation_texture2: Option<String>,
}

struct Emitter {
    descriptor: EmitterDescriptor,
    texture_names: EmitterTextureNames,
    max_num_particles: usize,
    uses_trails: bool,
    max_num_trail_samples_per_particle: usize,
    max_num_trail_vertices: usize,
    num_active_particles: usize,
    /// This is given size `max_num_particles`. All currently-active particles are
    /// stored contiguously starting at index 0.
    particles: Vec<Particle>,
    /// `particle_trail_datas` and `trail_samples` are both parallel to the
    /// `particles` array (and are both only used if `uses_trails` is set).
    /// `trail_samples` reserves `max_num_trail_samples_per_particle`
    /// samples/particle and stores the active samples contiguously starting
    /// from the first index (oldest to newest).
    particle_trail_datas: Vec<ParticleTrailData>,
    particle_trail_samples: Vec<ParticleTrailSample>,
    has_started: bool,
    is_removed: bool,
    time_alive: f32,
    fractional_particles_accumulated_for_next_emission: f32,
}

/// If mix(a, b, t) = x, then invMix(a, b, x) = t
fn inv_mix(a: f32, b: f32, x: f32) -> f32 {
    (x - a) / (b - a)
}

impl Emitter {
    fn new(descriptor: &EmitterDescriptor) -> Self {
        let texture_names = EmitterTextureNames {
            texture_name: descriptor.texture_name.clone(),
            modulation_texture1: if descriptor.modulation_texture1.is_some() {
                Some(
                    descriptor
                        .modulation_texture1
                        .as_ref()
                        .unwrap()
                        .texture_name
                        .clone(),
                )
            } else {
                None
            },
            modulation_texture2: if descriptor.modulation_texture2.is_some() {
                Some(
                    descriptor
                        .modulation_texture2
                        .as_ref()
                        .unwrap()
                        .texture_name
                        .clone(),
                )
            } else {
                None
            },
        };

        let max_num_particles: usize = match descriptor.spawning_mode {
            SpawningMode::Burst => descriptor.amount.floor() as usize,
            // 'amount' indicates particles/s for Flow emitters, so we need to
            // allocate space for the peak number of particles that can exist
            // (which is simple to calculate because emission rate and lifetime
            // are constant).
            SpawningMode::Flow => (descriptor.amount * descriptor.lifetime).ceil() as usize,
        };

        let uses_trails = descriptor.trail_lifetime > 0.0 && descriptor.trail_width > 0.0;
        let max_num_trail_samples_per_particle = if uses_trails {
            (descriptor.trail_lifetime * MAX_PARTICLE_TRAIL_MAX_SAMPLE_FREQUENCY).ceil() as usize
        } else {
            0
        };
        let max_num_trail_vertices = if uses_trails {
            // Trails are rendered with triangle strips. Each "segment" of the
            // trail is constructed from 2 triangles. The first segment uses 4
            // vertices (because it's starting the strip), and subsequent
            // segments each require an additional 2 vertices. Because each draw
            // renders an arbitrary number of trails (strips), we "connect" the
            // strips using degenerate triangles, which are implemented via
            // duplicating the first and last vertex of each trail strip (which
            // accounts for the additional addition of 2).
            let max_num_trail_vertices_per_particle =
                2 + 2 + 2 * max_num_trail_samples_per_particle;
            max_num_particles * max_num_trail_vertices_per_particle
        } else {
            0
        };

        let particles = vec![Particle::default(); max_num_particles];
        let (particle_trail_datas, particle_trail_samples) = if uses_trails {
            (
                vec![ParticleTrailData::default(); max_num_particles],
                vec![
                    ParticleTrailSample::default();
                    max_num_particles * max_num_trail_samples_per_particle
                ],
            )
        } else {
            // Dummy empty arrays.
            (
                Vec::<ParticleTrailData>::new(),
                Vec::<ParticleTrailSample>::new(),
            )
        };

        Self {
            descriptor: descriptor.clone(),
            texture_names,
            max_num_particles,
            uses_trails,
            max_num_trail_samples_per_particle,
            max_num_trail_vertices,
            num_active_particles: 0,
            particles,
            particle_trail_datas,
            particle_trail_samples,
            has_started: false,
            is_removed: false,
            time_alive: 0.0,
            fractional_particles_accumulated_for_next_emission: 0.0,
        }
    }

    fn texture_names(&self) -> &EmitterTextureNames {
        &self.texture_names
    }

    fn max_num_particles(&self) -> usize {
        self.max_num_particles
    }

    fn max_num_trail_vertices(&self) -> usize {
        self.max_num_trail_vertices
    }

    /// When a `ParticleEffect` is removed, it notifies all of its `Emitter`s.
    /// When notified, an `Emitter` destroys all of its active particles if
    /// `EmitterDescriptor::should_destroy_all_particles_on_remove` is set, and
    /// otherwise it stops emission but continues to simulate its currently
    /// active particles until they die.
    ///
    /// Only a `SpawningMode::Burst` emitter can "finish" without being removed,
    /// and that happens when all of its particles have died.
    fn is_finished(&self) -> bool {
        (self.is_removed && self.num_active_particles == 0)
            || (matches!(self.descriptor.spawning_mode, SpawningMode::Burst)
                && self.has_started
                && self.num_active_particles == 0)
    }

    fn notify_remove(&mut self) {
        self.is_removed = true;
        if self.descriptor.should_destroy_all_particles_on_remove {
            self.num_active_particles = 0;
        }
    }

    /// The world transform is passed in here so it can be used if this Emitter
    /// uses world-space emission. It's ignored if that's not the case.
    fn update(&mut self, delta_time: f32, mat_w: &Mat3) {
        if self.has_started {
            self.time_alive += delta_time;
        }

        // First update existing particles (removing dead ones)

        let mut next_particle_index = 0;
        for i in 0..self.num_active_particles {
            let particle = &self.particles[i];

            let time_alive = particle.time_alive + delta_time;
            if time_alive >= self.descriptor.lifetime {
                // Remove particle. Note that next_particle_index is not incremented.
                continue;
            }

            // When using world-space emission, gravity must be scaled by the
            // particle's world-space-scale so it has the same magnitude as if
            // it were applied before the world transform.
            let gravity_scale = if self.descriptor.use_world_space_emission {
                particle.world_space_scale
            } else {
                1.0
            };

            let position = particle.position + particle.velocity * delta_time;
            let velocity = particle.velocity
                + Vec2::new(0.0, self.descriptor.gravity * gravity_scale * delta_time);
            let rotation = particle.rotation + particle.angular_velocity * delta_time;

            self.particles[next_particle_index] = Particle {
                position,
                velocity,
                rotation,
                angular_velocity: particle.angular_velocity,
                time_alive,
                uv_scroll_t_offset: particle.uv_scroll_t_offset,
                world_space_scale: particle.world_space_scale,
            };

            if self.uses_trails {
                let particle_trail_data = &self.particle_trail_datas[i];

                assert!(particle_trail_data.num_samples > 0);
                // We should create a new sample this frame if the most-recent
                // sample (which is last in the sub-array for this trail) is
                // older than the specified interval AND the particle has moved
                // from that sample's position.
                let first_sample_index = i * self.max_num_trail_samples_per_particle;
                let most_recent_sample = &self.particle_trail_samples
                    [first_sample_index + particle_trail_data.num_samples - 1];
                let should_create_new_sample = self.time_alive - most_recent_sample.timestamp
                    >= PARTICLE_TRAIL_SAMPLE_MIN_INTERVAL
                    && position != most_recent_sample.position;
                let timestamp_cutoff = self.time_alive - self.descriptor.trail_lifetime;

                // Iterate over the existing trail samples (in order from oldest
                // to newest), removing any that have grown too old (but see the
                // caveat within the loop).
                //
                // The general idea here is to read samples starting from
                // first_sample_index and write samples starting from
                // new_first_sample_index (which will always be <=
                // first_sample_index)
                let new_first_sample_index =
                    next_particle_index * self.max_num_trail_samples_per_particle;
                let mut new_num_samples = 0;
                for sample_num in 0..particle_trail_data.num_samples {
                    let sample = &self.particle_trail_samples[first_sample_index + sample_num];
                    // Drop this sample if it's older than the cutoff UNLESS it
                    // is the last (most-recent) sample and we're NOT supposed
                    // to create a new sample this frame. This is done to
                    // maintain the guarantee that there is always at least 1
                    // sample.
                    //
                    // Note that we may not create a sample even when
                    // `should_create_new_sample` is set (see the condition
                    // below), but if we drop a particle here because it's set,
                    // we're guaranteed to have space for the new one, so things
                    // work out.
                    if sample.timestamp < timestamp_cutoff
                        && (sample_num != particle_trail_data.num_samples - 1
                            || should_create_new_sample)
                    {
                        continue;
                    }

                    self.particle_trail_samples[new_first_sample_index + new_num_samples] = *sample;
                    new_num_samples += 1;
                }

                // Create a new sample if it's time to *and* we have room.
                // Because of variable frame times and floating point
                // imprecision, we may have to wait a frame before we've removed
                // the oldest to make room for the new one.
                if should_create_new_sample
                    && new_num_samples < self.max_num_trail_samples_per_particle
                {
                    self.particle_trail_samples[new_first_sample_index + new_num_samples] =
                        ParticleTrailSample {
                            position,
                            timestamp: self.time_alive,
                        };
                    new_num_samples += 1;
                }

                assert!(new_num_samples > 0);
                self.particle_trail_datas[next_particle_index] = ParticleTrailData {
                    num_samples: new_num_samples,
                };
            }

            next_particle_index += 1;
        }
        self.num_active_particles = next_particle_index;

        // Then emit particles. Do this after updating to reduce the likelihood
        // we would overrun max_num_particles.
        //
        // Note that emission is stopped when notify_removed is called.

        if !self.is_removed {
            match self.descriptor.spawning_mode {
                SpawningMode::Burst => {
                    // Burst emitters emit all of their particles on the first
                    // frame.
                    // (fractional_particles_accumulated_for_next_emission is
                    // ignored for this mode.)
                    if !self.has_started {
                        let num_particles_to_emit = self.descriptor.amount.floor() as i32;
                        let num_particles_emitted =
                            self.emit(num_particles_to_emit, 0.0, 0.0, mat_w);
                        assert!(num_particles_emitted == num_particles_to_emit);
                    }
                }
                SpawningMode::Flow => {
                    // Accumulate "fractional" particles each frame using
                    // 'amount' as the emission rate. As soon as we've
                    // accumulated >= 1 particles, emit those particles.

                    self.fractional_particles_accumulated_for_next_emission +=
                        self.descriptor.amount * delta_time;
                    // Kick start emitters that emit at low rates by having them
                    // emit their first particle on the first frame, rather than
                    // needing to wait until they've accumulated enough time.
                    if !self.has_started
                        && self.fractional_particles_accumulated_for_next_emission < 1.0
                    {
                        self.fractional_particles_accumulated_for_next_emission = 1.0;
                    }

                    let num_particles_to_emit: i32 = self
                        .fractional_particles_accumulated_for_next_emission
                        .floor() as i32;
                    if num_particles_to_emit > 0 {
                        // To prevent permanent clumpy emission of particles
                        // after long frames, pass these values to emit() so it
                        // can compute when each particle would have been
                        // emitted in the past. See more discussion in the
                        // comment above the emit() method.
                        let first_particle_time_already_alive =
                            (self.fractional_particles_accumulated_for_next_emission - 1.0)
                                / self.descriptor.amount;
                        let time_between_particles = 1.0 / self.descriptor.amount;

                        let num_particles_emitted = self.emit(
                            num_particles_to_emit,
                            first_particle_time_already_alive,
                            time_between_particles,
                            mat_w,
                        );
                        self.fractional_particles_accumulated_for_next_emission -=
                            num_particles_emitted as f32;
                    }
                }
            }
        }

        self.has_started = true;
    }

    /// This tries to emit 'n' particles, but if doing so would overrun
    /// `max_num_particles` will emit fewer than 'n'. Returns the actual number of
    /// particles emitted.
    ///
    /// Frame timing differences and floating point inaccuracies may cause the
    /// `Emitter` to try to emit more particles in a frame than would fit in
    /// the 'particles' vector - if this would happen, the particles that
    /// aren't emitted are not forgotten about and will be emitted on later
    /// frames (likely the next frame).
    ///
    /// `first_particle_time_already_alive` and `time_between_particles` are used
    /// for `Flow` emitters to smooth over the effects of long frames. If all
    /// particles emitted by a `Flow` emitter were given a starting `time_alive`
    /// of 0.0, this creates clumpiness whenever there is a single long frame
    /// (e.g., due to loading a new asset or resizing the window). The issue
    /// is that the following frame will emit many more particles than usual
    /// (all with `time_alive`=0.0). All of those particles will then be killed
    /// on the same frame, which will then cause a large emission of
    /// accumulated particles on the next frame, etc. The fix is to give
    /// each particle a `time_alive` equal to how long it would have been alive
    /// had it been emitted at the exact point the Emitter had accumulated
    /// enough time to emit it, assuming an infinite framerate. This means
    /// that particles emitted on the same frame will have different lifetimes,
    /// which means they'll die on different frames.
    ///
    /// Note that the 100% correct way to solve this would be to simulate
    /// what each particle's state should be given it's >0.0 `time_alive`
    /// (e.g., where it's position should be given its starting velocity
    /// and gravity). For now, we don't address this which means there will
    /// be temporary visual gap in Flow emitters when there is a long frame.
    fn emit(
        &mut self,
        n: i32,
        first_particle_time_already_alive: f32,
        time_between_particles: f32,
        mat_w: &Mat3,
    ) -> i32 {
        assert!(self.num_active_particles <= self.max_num_particles);
        let num_to_emit: usize = min(
            n as usize,
            self.max_num_particles - self.num_active_particles,
        );

        let world_space_scale = if self.descriptor.use_world_space_emission {
            // We expect that only uniform scales will be used for emitters
            // using world-space emission, as this feature doesn't really make
            // sense for non-uniform scales. We could assert this here, but it
            // seems better not to crash and rather just draw something
            // incorrect.
            //
            // Note because the method of creating world transform matrices
            // (which is outside this module) creates the transform using TSR
            // order, the X-scale is applied to the first row (and Y-scale to
            // the second).
            Vec2::new(mat_w.x_axis.x, mat_w.y_axis.x).length()
        } else {
            // This field should no-op for local-space emission.
            1.0
        };

        let rotation = if self.descriptor.use_world_space_emission {
            // Extract the (CCW) rotation from the passed-in mat_w and then add
            // that to the emitter's start_rotation. This converts the rotation
            // to world-space, which is where we need it when doing world-space
            // emission.
            let x_axis_after_mat_w_rotation = Vec2::new(mat_w.x_axis.x, mat_w.x_axis.y).normalize();
            let mat_w_rotation = x_axis_after_mat_w_rotation
                .y
                .atan2(x_axis_after_mat_w_rotation.x);
            mat_w_rotation + self.descriptor.start_rotation
        } else {
            self.descriptor.start_rotation
        };

        let mut rng = rand::thread_rng();
        // This constant comes from the VFX tool's shader. The exact value used
        // here shouldn't matter much, as long as it's big enough to approximate
        // uniformly sampling the space of "UV offset combinations" between
        // any scrolling modulation textures.
        let max_uv_scroll_t_offset = 1137.2932 / self.descriptor.lifetime;
        for i in 0..num_to_emit {
            let (mut position, mut velocity) =
                self.generate_initial_particle_position_and_velocity();
            let angular_velocity =
                rng.gen_range(self.descriptor.min_random_spin..=self.descriptor.max_random_spin);
            let uv_scroll_t_offset = rng.gen_range(0.0..max_uv_scroll_t_offset);

            if self.descriptor.use_world_space_emission {
                position = mat_w.mul_vec3(Vec3::new(position.x, position.y, 1.0)).xy();
                velocity = mat_w.mul_vec3(Vec3::new(velocity.x, velocity.y, 0.0)).xy();
            }

            let particle_index = self.num_active_particles + i;

            self.particles[particle_index] = Particle {
                position,
                velocity,
                rotation,
                angular_velocity,
                time_alive: first_particle_time_already_alive + (i as f32) * time_between_particles,
                uv_scroll_t_offset,
                world_space_scale,
            };

            if self.uses_trails {
                // Append to `particle_trail_datas`, which is parallel to
                // `particles`. Also, add the first sample for the particle
                // immediately when it's emitted. We do this so we can draw the
                // first trail segment from the particle's current position to
                // this sample before a second sample has been added, which
                // could be multiple frames in the future.
                let first_sample_index = particle_index * self.max_num_trail_samples_per_particle;
                self.particle_trail_samples[first_sample_index] = ParticleTrailSample {
                    position,
                    timestamp: self.time_alive,
                };
                self.particle_trail_datas[particle_index] = ParticleTrailData { num_samples: 1 };
            }
        }
        self.num_active_particles += num_to_emit;

        num_to_emit as i32
    }

    /// This applies the emitter's spawning pattern (along with related params like
    /// transform) to (randomly) generate a new position and velocity.
    fn generate_initial_particle_position_and_velocity(&self) -> (Vec2, Vec2) {
        let mut rng = rand::thread_rng();

        // TODO: determine if we want to uniformly sample the post-transform
        // shape, even if the case of non-uniform scales (in which case this
        // function will need to be modified).
        let (local_position, local_direction) = match self.descriptor.spawn_pattern {
            SpawnPattern::Circle => {
                // The somewhat obscure 'r' calculation here comes from the compute
                // shader for the VFX tool (which is uncommented). Note that this doesn't
                // uniformly sample points from within a circle, but biases toward the
                // center.
                let s = rng.r#gen::<f32>();
                let theta = rng.gen_range(0.0..(2.0 * std::f32::consts::PI));
                let p = rng.r#gen::<f32>();
                let r = (1.0 - s * s).sqrt() * p;
                let local_position = Vec2::new(r * theta.cos(), r * theta.sin());
                // Circle emitter particles move away from the center of the circle.
                let local_direction = local_position.normalize();

                (local_position, local_direction)
            }
            SpawnPattern::CircleBoundary => {
                let theta = rng.gen_range(0.0..(2.0 * std::f32::consts::PI));
                let local_position = Vec2::new(theta.cos(), theta.sin());
                // Circle emitter particles move away from the center of the circle.
                let local_direction = local_position;

                (local_position, local_direction)
            }
            SpawnPattern::Cone => {
                let random_spread_angle = rng.gen_range(
                    -self.descriptor.cone_spread_angle..=self.descriptor.cone_spread_angle,
                );
                // Cone spread is relative to +y, so rotate +90 degrees.
                let angle = std::f32::consts::FRAC_PI_2 + random_spread_angle;

                // Cone emitter particles start at (local) origin and move in the
                // chosen direction (within the cone).
                let local_position = Vec2::new(0.0, 0.0);
                let local_direction = Vec2::new(angle.cos(), angle.sin());

                (local_position, local_direction)
            }
            SpawnPattern::Square => {
                // Generate a random point in a square of side-length 2 centered on
                // the origin.
                let x = rng.gen_range(-1.0..1.0);
                let y = rng.gen_range(-1.0..1.0);
                let local_position = Vec2::new(x, y);
                // Square emitter particles move in the (local) +y direction.
                let local_direction = Vec2::new(0.0, 1.0);

                (local_position, local_direction)
            }
        };

        let position =
            (self.descriptor.transform * Vec3::new(local_position.x, local_position.y, 1.0)).xy();
        let velocity = self.descriptor.start_speed
            * ((self.descriptor.transform * Vec3::new(local_direction.x, local_direction.y, 0.0))
                .xy()
                .normalize());

        (position, velocity)
    }

    // TODO(miketuritzin): move some of these args into a struct if that seems worthwhile.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        draw_particles_render_pipeline: &wgpu::RenderPipeline,
        draw_particle_trails_render_pipeline: &wgpu::RenderPipeline,
        main_bind_group: &wgpu::BindGroup,
        texture_to_bind_group: &HashMap<EmitterTextureNames, wgpu::BindGroup>,
        mat_vp: &Mat4,
        mat_w: &Mat4,
        cpu_uniform_buffer: &mut [u8],
        next_uniform_buffer_offset: &mut usize,
        uniform_buffer_stride: usize,
        cpu_particle_instance_data_buffer: &mut [ParticleInstanceData],
        next_particle_instance_data_buffer_index: &mut usize,
        cpu_particle_trail_vertex_buffer: &mut [ParticleTrailVertex],
        next_particle_trail_vertex_buffer_index: &mut usize,
    ) {
        // When world-space emission is used, the world transform was already
        // applied to each particle (at its point of emission), so we don't want
        // to apply it again here.
        let mat_wvp = if self.descriptor.use_world_space_emission {
            mat_vp
        } else {
            &mat_vp.mul_mat4(mat_w)
        };

        // Look up the specific BindGroup for the texture this Emitter uses. It may not be
        // available yet (if all of the textures haven't finished loading), so if
        // that's the case, we bail without rendering anything.
        let texture_bind_group_option = &texture_to_bind_group.get(&self.texture_names);
        if texture_bind_group_option.is_none() {
            return;
        }
        let texture_bind_group = texture_bind_group_option.unwrap();

        // Generate Uniforms struct and copy it into the next slot in the cpu_uniform_buffer.

        // Returns (is_enabled, uv_scale, uv_speed)
        //
        // Note that uv_speed is in "UV units" per lifetime
        let modulation_texture_uniforms =
            |o_mt: &Option<ModulationTexture>| -> (u32, [f32; 2], [f32; 2]) {
                if o_mt.is_none() {
                    // uv_scale / uv_speed are unused (by the shader) in this case
                    return (0, [1.0, 1.0], [0.0, 0.0]);
                }
                let mt = o_mt.as_ref().unwrap();
                (
                    1,
                    [mt.tile_x, mt.tile_y],
                    [
                        mt.scroll_speed_x * self.descriptor.lifetime,
                        mt.scroll_speed_y * self.descriptor.lifetime,
                    ],
                )
            };
        let (mt1_is_enabled, mt1_uv_scale, mt1_uv_speed) =
            modulation_texture_uniforms(&self.descriptor.modulation_texture1);
        let (mt2_is_enabled, mt2_uv_scale, mt2_uv_speed) =
            modulation_texture_uniforms(&self.descriptor.modulation_texture2);

        let glow_factor = 1.0 + self.descriptor.glow;

        let uniforms = Uniforms {
            mat_wvp: (*mat_wvp).to_cols_array_2d(),
            color: [
                self.descriptor.color.x * glow_factor,
                self.descriptor.color.y * glow_factor,
                self.descriptor.color.z * glow_factor,
                self.descriptor.color.w,
            ],
            modulation_texture1_uv_scale: mt1_uv_scale,
            modulation_texture1_uv_speed: mt1_uv_speed,
            modulation_texture2_uv_scale: mt2_uv_scale,
            modulation_texture2_uv_speed: mt2_uv_speed,
            modulation_texture1_is_enabled: mt1_is_enabled,
            modulation_texture2_is_enabled: mt2_is_enabled,
            start_size: self.descriptor.start_size * self.descriptor.size_multiplier,
            end_size: self.descriptor.end_size * self.descriptor.size_multiplier,
            end_fade_in: self.descriptor.end_fade_in,
            start_fade_out: self.descriptor.start_fade_out,
            use_world_space_emission: self.descriptor.use_world_space_emission as u32,
            _padding: [0u8; 4],
        };

        let uniform_buffer_offset = *next_uniform_buffer_offset;
        *next_uniform_buffer_offset += uniform_buffer_stride;
        cpu_uniform_buffer
            [uniform_buffer_offset..(uniform_buffer_offset + std::mem::size_of::<Uniforms>())]
            .copy_from_slice(bytemuck::cast_slice(&[uniforms]));

        // Emit per-particle dynamic data that will be uploaded to the GPU:
        //
        // - A `ParticleInstanceData` for each active particle (which goes into
        //   `cpu_particle_instance_data_buffer`).
        // - If trails are enabled for this emitter, multiple
        //   `ParticleTrailVertex`s (which go into
        //   `cpu_particle_trail_vertex_buffer`)

        let first_particle_instance_data_buffer_index = *next_particle_instance_data_buffer_index;
        let first_particle_trail_vertex_buffer_index = *next_particle_trail_vertex_buffer_index;
        let trail_half_width = 0.5 * self.descriptor.trail_width * self.descriptor.size_multiplier;

        for i in 0..self.num_active_particles {
            let particle = &self.particles[i];

            let particle_t = particle.time_alive / self.descriptor.lifetime;
            cpu_particle_instance_data_buffer[*next_particle_instance_data_buffer_index] =
                ParticleInstanceData {
                    position: particle.position.into(),
                    rotation: particle.rotation,
                    t: particle_t,
                    uv_scroll_t_offset: particle.uv_scroll_t_offset,
                    world_space_scale: particle.world_space_scale,
                };
            *next_particle_instance_data_buffer_index += 1;

            if self.uses_trails {
                let particle_trail_data = &self.particle_trail_datas[i];
                assert!(particle_trail_data.num_samples > 0);
                let first_sample_index = i * self.max_num_trail_samples_per_particle;

                let sample_for_current_position = ParticleTrailSample {
                    position: particle.position,
                    timestamp: self.time_alive,
                };
                // Skip the first segment if it would be of 0 length. (We only
                // need to do this check here, as all adjacent samples in the
                // samples array are guaranteed to be not-equal, as we test for
                // that before adding them.)
                let should_ignore_most_recent_sample = sample_for_current_position.position
                    == self.particle_trail_samples
                        [first_sample_index + particle_trail_data.num_samples - 1]
                        .position;
                // Check if we have any segments to render.
                if !should_ignore_most_recent_sample || particle_trail_data.num_samples > 1 {
                    let most_recent_sample_num = if should_ignore_most_recent_sample {
                        particle_trail_data.num_samples - 2
                    } else {
                        particle_trail_data.num_samples - 1
                    };

                    let particle_alpha = Self::particle_alpha_over_lifetime(
                        self.descriptor.end_fade_in,
                        self.descriptor.start_fade_out,
                        particle_t,
                    );
                    let scale = if self.descriptor.use_world_space_emission {
                        trail_half_width * particle.world_space_scale
                    } else {
                        trail_half_width
                    };

                    let calc_normal_dir = |prev_pos: &Vec2, next_pos: &Vec2| -> Vec2 {
                        let tangent_dir = (next_pos - prev_pos).normalize();
                        // Normal is tangent rotated 90 degrees CCW.
                        //
                        // NOTE: see the `scale` multiplier, which effectively
                        // flips this rotation to be CW when using world-space
                        // emission.
                        Vec2::new(-tangent_dir.y, tangent_dir.x)
                    };

                    // Draw the trail from "head" (most recent) to "tail" (least
                    // recent).

                    // First emit the 2 vertices for the current position (which
                    // starts the trail). This also handles emitting extra
                    // vertices to produce 2 degenerate triangles between
                    // trails, which is what allows us to render all trails for
                    // an emitter as a single triangle strip.
                    {
                        let is_first_trail = *next_particle_trail_vertex_buffer_index == 0;
                        if !is_first_trail {
                            cpu_particle_trail_vertex_buffer
                                [*next_particle_trail_vertex_buffer_index] =
                                cpu_particle_trail_vertex_buffer
                                    [*next_particle_trail_vertex_buffer_index - 1];
                            *next_particle_trail_vertex_buffer_index += 1;
                        }

                        let most_recent_sample = &self.particle_trail_samples
                            [first_sample_index + most_recent_sample_num];
                        let current_position_normal_dir = calc_normal_dir(
                            &sample_for_current_position.position,
                            &most_recent_sample.position,
                        );
                        let current_position_first_vertex = ParticleTrailVertex {
                            position: (sample_for_current_position.position
                                + scale * current_position_normal_dir)
                                .into(),
                            alpha: particle_alpha,
                        };
                        let current_position_second_vertex = ParticleTrailVertex {
                            position: (sample_for_current_position.position
                                - scale * current_position_normal_dir)
                                .into(),
                            alpha: particle_alpha,
                        };

                        if !is_first_trail {
                            cpu_particle_trail_vertex_buffer
                                [*next_particle_trail_vertex_buffer_index] =
                                current_position_first_vertex;
                            *next_particle_trail_vertex_buffer_index += 1;
                        }
                        cpu_particle_trail_vertex_buffer
                            [*next_particle_trail_vertex_buffer_index] =
                            current_position_first_vertex;
                        *next_particle_trail_vertex_buffer_index += 1;
                        cpu_particle_trail_vertex_buffer
                            [*next_particle_trail_vertex_buffer_index] =
                            current_position_second_vertex;
                        *next_particle_trail_vertex_buffer_index += 1;
                    }
                    // Now emit two additional vertices for each sample. Iterate
                    // backward through the array, as it's ordered from least-
                    // to most-recent.
                    for sample_num in (0..=most_recent_sample_num).rev() {
                        let sample = &self.particle_trail_samples[first_sample_index + sample_num];
                        let prev_sample = if sample_num == particle_trail_data.num_samples - 1 {
                            &sample_for_current_position
                        } else {
                            &self.particle_trail_samples[first_sample_index + sample_num + 1]
                        };
                        let next_sample_num = if sample_num > 0 { sample_num - 1 } else { 0 };
                        let next_sample =
                            &self.particle_trail_samples[first_sample_index + next_sample_num];

                        let normal_dir =
                            calc_normal_dir(&prev_sample.position, &next_sample.position);

                        let sample_t =
                            (self.time_alive - sample.timestamp) / self.descriptor.trail_lifetime;
                        let one_minus_sample_t = 1.0 - sample_t;
                        let sample_alpha = one_minus_sample_t * particle_alpha;
                        let sample_scale = one_minus_sample_t * scale;

                        cpu_particle_trail_vertex_buffer
                            [*next_particle_trail_vertex_buffer_index] = ParticleTrailVertex {
                            position: (sample.position + sample_scale * normal_dir).into(),
                            alpha: sample_alpha,
                        };
                        *next_particle_trail_vertex_buffer_index += 1;
                        cpu_particle_trail_vertex_buffer
                            [*next_particle_trail_vertex_buffer_index] = ParticleTrailVertex {
                            position: (sample.position - sample_scale * normal_dir).into(),
                            alpha: sample_alpha,
                        };
                        *next_particle_trail_vertex_buffer_index += 1;
                    }
                }
            }
        }

        // Issue rendering commands

        // We must rebind main_bind_group for each draw to set the dynamic
        // offset for the uniform buffer.
        //
        // Note that this bind group is used by both draw-particles and
        // draw-particle-trails.
        render_pass.set_bind_group(
            0,
            main_bind_group,
            // Specify offset into uniform buffer as a "dynamic offset".
            &[uniform_buffer_offset as wgpu::DynamicOffset],
        );
        // This bind group is only used by draw-particles.
        render_pass.set_bind_group(1, texture_bind_group, &[]);

        // Draw trails first so they appear visually behind all particles for
        // the emitter.

        if self.uses_trails
            && *next_particle_trail_vertex_buffer_index > first_particle_trail_vertex_buffer_index
        {
            // The expectation is that the draw-particles pipeline has been set
            // on the render-pass prior to this function being called. So we
            // switch to the draw-particle-trails pipeline, draw the trails, and
            // then switch back to draw-particles.
            render_pass.set_pipeline(draw_particle_trails_render_pipeline);
            render_pass.draw(
                (first_particle_trail_vertex_buffer_index as u32)
                    ..(*next_particle_trail_vertex_buffer_index as u32),
                0..1,
            );
            render_pass.set_pipeline(draw_particles_render_pipeline);
        }

        // Now draw particles.
        //
        // (See note above regarding how the render-pass's pipeline is managed.)

        // Draw one quad per particle, using instancing to render multiple particles.
        render_pass.draw(
            0..(PARTICLE_QUAD_VERTICES.len() as u32),
            // All particle instance data (across all Emitters) are stored in a single
            // buffer, so need to set the first instance index to the first index in this
            // buffer.
            (first_particle_instance_data_buffer_index as u32)
                ..(*next_particle_instance_data_buffer_index as u32),
        );
    }

    // This is the same as alpha_over_lifetime in draw-particles.wgsl. We need
    // to compute the current particle alpha CPU-side (for now) for rendering
    // trails.
    fn particle_alpha_over_lifetime(end_fade_in: f32, start_fade_out: f32, t: f32) -> f32 {
        let fade_in = inv_mix(0.0, end_fade_in, t).clamp(0.0, 1.0);
        let fade_out = 1.0 - inv_mix(start_fade_out, 1.0, t).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }
}

struct ParticleEffect {
    transform: Mat3,
    emitters: Vec<Emitter>,
}

impl ParticleEffect {
    fn new(descriptor: &ParticleEffectDescriptor, transform: &Mat3) -> Self {
        let mut emitters = Vec::<Emitter>::with_capacity(descriptor.emitters.len());

        for emitter_descriptor in &descriptor.emitters {
            emitters.push(Emitter::new(emitter_descriptor));
        }

        Self {
            transform: *transform,
            emitters,
        }
    }

    fn num_emitters(&self) -> usize {
        self.emitters.len()
    }

    fn max_num_particles(&self) -> usize {
        let mut max_num_particles: usize = 0;
        for emitter in &self.emitters {
            max_num_particles += emitter.max_num_particles();
        }
        max_num_particles
    }

    fn max_num_trail_vertices(&self) -> usize {
        let mut max_num_trail_vertices: usize = 0;
        for emitter in &self.emitters {
            max_num_trail_vertices += emitter.max_num_trail_vertices();
        }
        max_num_trail_vertices
    }

    fn is_finished(&self) -> bool {
        // Can just check whether the list of Emitters is empty, as update() removes
        // Emitters as soon as they are finished.
        self.emitters.is_empty()
    }

    fn set_transform(&mut self, transform: &Mat3) {
        self.transform = *transform;
    }

    fn notify_remove(&mut self) {
        for emitter in &mut self.emitters {
            emitter.notify_remove();
        }
    }

    fn update(&mut self, delta_time: f32) {
        // Update all Emitters and remove the ones that have finished.
        self.emitters.retain_mut(|emitter| {
            emitter.update(delta_time, &self.transform);

            !emitter.is_finished()
        });
    }

    // TODO(miketuritzin): move some of these args into a struct if that seems worthwhile.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        draw_particles_render_pipeline: &wgpu::RenderPipeline,
        draw_particle_trails_render_pipeline: &wgpu::RenderPipeline,
        main_bind_group: &wgpu::BindGroup,
        texture_to_bind_group: &HashMap<EmitterTextureNames, wgpu::BindGroup>,
        mat_vp: &Mat4,
        cpu_uniform_buffer: &mut [u8],
        next_uniform_buffer_offset: &mut usize,
        uniform_buffer_stride: usize,
        cpu_particle_instance_data_buffer: &mut [ParticleInstanceData],
        next_particle_instance_data_buffer_index: &mut usize,
        cpu_particle_trail_vertex_buffer: &mut [ParticleTrailVertex],
        next_particle_trail_vertex_buffer_index: &mut usize,
    ) {
        // Convert the 3x3 matrix specified in the transform to 4x4. We assume the
        // transform is affine (i.e., its 3rd row is [0, 0, 1]).
        #[rustfmt::skip]
        let mat_w = &Mat4::from_cols_slice(&[
            self.transform.x_axis.x, self.transform.x_axis.y, 0.0, 0.0,
            self.transform.y_axis.x, self.transform.y_axis.y, 0.0, 0.0,
            0.0,                     0.0,                     1.0, 0.0,
            self.transform.z_axis.x, self.transform.z_axis.y, 0.0, 1.0,
        ]);

        for emitter in &self.emitters {
            emitter.render(
                render_pass,
                draw_particles_render_pipeline,
                draw_particle_trails_render_pipeline,
                main_bind_group,
                texture_to_bind_group,
                mat_vp,
                mat_w,
                cpu_uniform_buffer,
                next_uniform_buffer_offset,
                uniform_buffer_stride,
                cpu_particle_instance_data_buffer,
                next_particle_instance_data_buffer_index,
                cpu_particle_trail_vertex_buffer,
                next_particle_trail_vertex_buffer_index,
            );
        }
    }
}

pub(crate) struct ParticleEffectManager {
    particle_effects: HashMap<ParticleEffectHandle, ParticleEffect>,

    // `previous_visible_effects` are the visible `ParticleEffect`s from the previous frame.
    // `current_visible_effects` are the visible `ParticleEffect`s from this frame.
    // A `GpuWeb` ECS query adds `ParticleRender` handles to `current_visible_effects` while removing them from `previous_visible_effects`
    // The leftover handles in `previous_visible_effects` are stale and cleaned up in `ParticleEffectManager::post_update_effects`
    previous_visible_effects: BTreeSet<ParticleEffectHandle>,
    current_visible_effects: BTreeSet<ParticleEffectHandle>,

    next_effect_handle: ParticleEffectHandle,

    /// `current_frame_num_emitters` and `current_frame_max_num_particles` are updated
    /// in each call to `update()` and used in each (following) call to `render()`
    current_frame_num_emitters: usize,
    current_frame_max_num_particles: usize,
    current_frame_max_num_trail_vertices: usize,
    next_uniform_buffer_offset: usize,
    next_particle_instance_data_buffer_index: usize,
    next_particle_trail_vertex_buffer_index: usize,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_render_pipeline: wgpu::RenderPipeline,
    blit_input_texture_sampler: wgpu::Sampler,
    draw_particles_main_bind_group_layout: wgpu::BindGroupLayout,
    draw_particles_texture_bind_group_layout: wgpu::BindGroupLayout,
    draw_particles_render_pipeline: wgpu::RenderPipeline,
    draw_particle_trails_render_pipeline: wgpu::RenderPipeline,
    particle_quad_vertex_buffer: wgpu::Buffer,
    uniform_buffer_capacity: usize,
    uniform_buffer_stride: usize,
    uniform_buffer: wgpu::Buffer,
    /// This must unfortunately be a byte vector because of the
    /// `min_uniform_buffer_offset_alignment` restriction.
    cpu_uniform_buffer: Vec<u8>,
    particle_instance_data_buffer_capacity: usize,
    particle_instance_data_buffer: wgpu::Buffer,
    cpu_particle_instance_data_buffer: Vec<ParticleInstanceData>,
    particle_trail_vertex_buffer_capacity: usize,
    particle_trail_vertex_buffer: wgpu::Buffer,
    cpu_particle_trail_vertex_buffer: Vec<ParticleTrailVertex>,
    particle_texture_sampler: wgpu::Sampler,
    modulation_texture_sampler: wgpu::Sampler,
    draw_particles_main_bind_group: wgpu::BindGroup,
    /// We keep this map so we can create a `BindGroup` on-demand for any permutation of
    /// textures an `Emitter` may need.
    texture_name_to_texture_id: HashMap<String, TextureId>,
    /// Used to look up the cached `BindGroup` for each permutation of texture names.
    texture_to_bind_group: HashMap<EmitterTextureNames, wgpu::BindGroup>,
    /// When a `ParticleEffect` is added, if we don't already have a `BindGroup` for any
    /// of its `Emitter`s, we add the corresponding `EmitterTextureNames` to this set so
    /// that the corresponding `BindGroup`s are created before they're needed for
    /// rendering. We do this rather than creating the `BindGroup`s immediately in
    /// `add()` so we don't need to hold a persistent reference to `wgpu::Device`.
    pending_texture_bind_groups: HashSet<EmitterTextureNames>,
}

static mut PARTICLE_EFFECT_MANAGER_CID: Option<ComponentId> = None;

impl Resource for ParticleEffectManager {
    fn new() -> Self {
        unreachable!()
    }
}

impl EcsType for ParticleEffectManager {
    fn id() -> ComponentId {
        unsafe { PARTICLE_EFFECT_MANAGER_CID.expect("ComponentId unassigned") }
    }

    unsafe fn set_id(id: ComponentId) {
        unsafe {
            PARTICLE_EFFECT_MANAGER_CID = Some(id);
        }
    }

    fn string_id() -> &'static std::ffi::CStr {
        c"vfx::ParticleEffectManager"
    }
}

impl Serialize for ParticleEffectManager {
    fn serialize<W>(&self, _: &mut snapshot::Serializer<W>) -> snapshot::Result<()>
    where
        W: snapshot::WriteUninit,
    {
        Ok(())
    }
}

impl Deserialize for ParticleEffectManager {
    unsafe fn deserialize<R>(_: &mut snapshot::Deserializer<R>) -> snapshot::Result<Self>
    where
        R: snapshot::ReadUninit,
    {
        panic!("use deserialize_in_place()!");
    }

    unsafe fn deserialize_in_place<R>(
        &mut self,
        _: &mut snapshot::Deserializer<R>,
    ) -> snapshot::Result<()>
    where
        R: snapshot::ReadUninit,
    {
        Ok(())
    }
}

impl ParticleManager for ParticleEffectManager {
    fn next_effect_handle(&mut self) -> ParticleEffectHandle {
        self.next_effect_handle = self
            .next_effect_handle
            .map(|h| {
                h.checked_add(1)
                    .expect("ParticleEffectManager::next_effect_handle() - Handle overflow")
            })
            .or_else(|| NonZero::<u64>::new(1));

        self.next_effect_handle
    }
}

// Web MVP is singlethreaded, so this is fine
#[cfg(target_family = "wasm")]
unsafe impl Send for ParticleEffectManager {}
#[cfg(target_family = "wasm")]
unsafe impl Sync for ParticleEffectManager {}

fn align_to_next_multiple(v: usize, factor: usize) -> usize {
    v.div_ceil(factor) * factor
}

/// Grow 'capacity' by `growth_factor` multiples until it reaches or exceeds
/// `needed_capacity`.
fn grow_capacity(capacity: usize, needed_capacity: usize, growth_factor: f32) -> usize {
    assert!(growth_factor > 1.0);
    let mut new_capacity = capacity;
    while new_capacity < needed_capacity {
        new_capacity = ((new_capacity as f32) * growth_factor).ceil() as usize;
    }
    new_capacity
}

impl ParticleEffectManager {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        target_sample_count: u32,
    ) -> Self {
        let particle_effects = HashMap::<ParticleEffectHandle, ParticleEffect>::new();

        // Create resources for mipmap downsampling

        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    // inputTexture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    // inputTextureSampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the texture bindings.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("ParticleEffectManager: blit"),
            });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleEffectManager: blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/shaders/blit.wgsl").into()),
        });

        let blit_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ParticleEffectManager: blit"),
                bind_group_layouts: &[&blit_bind_group_layout],
                push_constant_ranges: &[],
            });

        let blit_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ParticleEffectManager: blit"),
            layout: Some(&blit_render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                // No vertex buffers needed - vertex shader specifies fullscreen triangle vertices
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PARTICLE_TEXTURE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let blit_input_texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create resources for rendering particles / particle trails

        let draw_color_targets = &[Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let draw_multisample_state = wgpu::MultisampleState {
            count: target_sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };

        // draw-particles render pipeline

        // Note that this is shared by draw-particle-trails
        let draw_particles_main_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    // uniforms
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            // The uniform buffer is an array of structs, one per draw, so
                            // we need to set a dynamic offset for each draw.
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<Uniforms>() as u64,
                            ),
                        },
                        count: None,
                    },
                    // particleTextureSampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the texture bindings.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // modulationTextureSampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the texture bindings.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("ParticleEffectManager: draw-particles main"),
            });

        // Use a separate bind group for setting the particle texture so we don't need
        // to recreate a BindGroup for every texture when buffers are realloc'd.
        let draw_particles_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    // particleTexture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    // modulationTexture1
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    // modulationTexture2
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                ],
                label: Some("ParticleEffectManager: draw-particles texture"),
            });

        let draw_particles_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ParticleEffectManager: draw-particles"),
                bind_group_layouts: &[
                    &draw_particles_main_bind_group_layout,
                    &draw_particles_texture_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });

        let draw_particles_shader_module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ParticleEffectManager: draw-particles"),
                source: wgpu::ShaderSource::Wgsl(
                    concat!(
                        include_str!("../assets/shaders/main_bind_group.wgsl"),
                        include_str!("../assets/shaders/draw-particles.wgsl")
                    )
                    .into(),
                ),
            });

        let draw_particles_render_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ParticleEffectManager: draw-particles"),
                layout: Some(&draw_particles_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &draw_particles_shader_module,
                    entry_point: Some("vs_main"),
                    buffers: &[
                        // Slot #0: particle quad vertices
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ParticleQuadVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[
                                // uv
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32x2,
                                    offset: memoffset::offset_of!(ParticleQuadVertex, uv) as u64,
                                    shader_location: 0,
                                },
                            ],
                        },
                        // Slot #1: per-particle instance data
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ParticleInstanceData>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &[
                                // position
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32x2,
                                    offset: memoffset::offset_of!(ParticleInstanceData, position)
                                        as u64,
                                    shader_location: 1,
                                },
                                // rotation
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32,
                                    offset: memoffset::offset_of!(ParticleInstanceData, rotation)
                                        as u64,
                                    shader_location: 2,
                                },
                                // t
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32,
                                    offset: memoffset::offset_of!(ParticleInstanceData, t) as u64,
                                    shader_location: 3,
                                },
                                // uv_scroll_t_offset
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32,
                                    offset: memoffset::offset_of!(
                                        ParticleInstanceData,
                                        uv_scroll_t_offset
                                    ) as u64,
                                    shader_location: 4,
                                },
                                // world_space_scale
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32,
                                    offset: memoffset::offset_of!(
                                        ParticleInstanceData,
                                        world_space_scale
                                    ) as u64,
                                    shader_location: 5,
                                },
                            ],
                        },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_particles_shader_module,
                    entry_point: Some("fs_main"),
                    targets: draw_color_targets,
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: draw_multisample_state,
                multiview: None,
                cache: None,
            });

        // draw-particle-trails render pipeline

        // Use the same layout (and binding) for slot #0 as for draw-particles
        // to minimize uniform updates / binding.
        let draw_particle_trails_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ParticleEffectManager: draw-particle-trails"),
                bind_group_layouts: &[&draw_particles_main_bind_group_layout],
                push_constant_ranges: &[],
            });

        let draw_particle_trails_shader_module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ParticleEffectManager: draw-particle-trails"),
                source: wgpu::ShaderSource::Wgsl(
                    concat!(
                        include_str!("../assets/shaders/main_bind_group.wgsl"),
                        include_str!("../assets/shaders/draw-particle-trails.wgsl")
                    )
                    .into(),
                ),
            });

        let draw_particle_trails_render_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ParticleEffectManager: draw-particle-trails"),
                layout: Some(&draw_particle_trails_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &draw_particle_trails_shader_module,
                    entry_point: Some("vs_main"),
                    buffers: &[
                        // Slot #0 is reserved for draw-particles, and so is
                        // empty here.
                        //
                        // HACKY: Unfortunately, it appears the only way to
                        // specify an empty slot in wgpu right now is via a
                        // `VertexBufferLayout` with a dummy attribute that
                        // isn't used in the shader. Rendering just silently
                        // fails if specifying an empty array of attributes.
                        wgpu::VertexBufferLayout {
                            array_stride: 0,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 0,
                                shader_location: 2, // unused
                            }],
                        },
                        // Slot #1 is also reserved for draw-particles.
                        wgpu::VertexBufferLayout {
                            array_stride: 0,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 0,
                                shader_location: 3, // unused
                            }],
                        },
                        // // Slot #2: particle trail vertices
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ParticleTrailVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[
                                // position
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32x2,
                                    offset: memoffset::offset_of!(ParticleTrailVertex, position)
                                        as u64,
                                    shader_location: 0,
                                },
                                // alpha
                                wgpu::VertexAttribute {
                                    format: wgpu::VertexFormat::Float32,
                                    offset: memoffset::offset_of!(ParticleTrailVertex, alpha)
                                        as u64,
                                    shader_location: 1,
                                },
                            ],
                        },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_particle_trails_shader_module,
                    entry_point: Some("fs_main"),
                    targets: draw_color_targets,
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: draw_multisample_state,
                multiview: None,
                cache: None,
            });

        // Other rendering-related resources

        let particle_quad_vertex_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ParticleEffectManager: particle quad"),
                contents: bytemuck::cast_slice(PARTICLE_QUAD_VERTICES),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // Because we're binding uniforms using dynamic offsets into the uniform buffer,
        // we need to obey the min_uniform_buffer_offset_alignment restriction (which
        // means that cpu_uniform_buffer can't be a simple Vec<Uniforms> because of this
        // additional alignment requirement).
        let uniform_buffer_stride = align_to_next_multiple(
            std::mem::size_of::<Uniforms>(),
            device.limits().min_uniform_buffer_offset_alignment as usize,
        );

        // Each Emitter emits a single draw each frame, which uses a single instance of
        // Uniforms. All of these Uniforms instances are appended to this buffer, which is
        // updated (GPU-side) once per frame.
        let uniform_buffer_capacity = PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_EMITTERS;
        let uniform_buffer_size_in_bytes = uniform_buffer_capacity * uniform_buffer_stride;
        let uniform_buffer = Self::alloc_gpu_uniform_buffer(device, uniform_buffer_size_in_bytes);
        // This is a byte vector because of the stride requirement (default struct alignment
        // is not enough).
        let cpu_uniform_buffer = vec![0u8; uniform_buffer_size_in_bytes];

        let particle_instance_data_buffer_capacity =
            PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_PARTICLES;
        let particle_instance_data_buffer = Self::alloc_gpu_particle_instance_data_buffer(
            device,
            particle_instance_data_buffer_capacity,
        );
        let cpu_particle_instance_data_buffer =
            vec![ParticleInstanceData::default(); particle_instance_data_buffer_capacity];

        let particle_trail_vertex_buffer_capacity =
            PARTICLE_EFFECT_MANAGER_INITIAL_ALLOC_NUM_PARTICLE_TRAIL_VERTICES;
        let particle_trail_vertex_buffer = Self::alloc_gpu_particle_trail_vertex_buffer(
            device,
            particle_trail_vertex_buffer_capacity,
        );
        let cpu_particle_trail_vertex_buffer =
            vec![ParticleTrailVertex::default(); particle_trail_vertex_buffer_capacity];

        let particle_texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Modulation textures use 'repeat' sampling because repeats are expected when
        // using tiling and/or scrolling.
        let modulation_texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let draw_particles_main_bind_group = Self::create_draw_particles_main_bind_group(
            device,
            &draw_particles_main_bind_group_layout,
            &uniform_buffer,
            &particle_texture_sampler,
            &modulation_texture_sampler,
        );

        // For now, particle textures are loaded on-demand when Emitters are added that
        // use them. In the future, there should be some mechanism for preloading to avoid
        // runtime hitches.
        let texture_name_to_texture_id = HashMap::<String, TextureId>::new();

        // This map starts out empty and gets filled in on-demand as Emitters are added
        // that need particular permutations of textures.
        let texture_to_bind_group = HashMap::<EmitterTextureNames, wgpu::BindGroup>::new();

        let emitter_texture_names_that_need_bind_groups_set = HashSet::<EmitterTextureNames>::new();

        Self {
            particle_effects,
            previous_visible_effects: BTreeSet::<ParticleEffectHandle>::new(),
            current_visible_effects: BTreeSet::<ParticleEffectHandle>::new(),
            next_effect_handle: None,
            current_frame_num_emitters: 0,
            current_frame_max_num_particles: 0,
            current_frame_max_num_trail_vertices: 0,
            next_uniform_buffer_offset: 0,
            next_particle_instance_data_buffer_index: 0,
            next_particle_trail_vertex_buffer_index: 0,
            blit_bind_group_layout,
            blit_render_pipeline,
            blit_input_texture_sampler,
            draw_particles_main_bind_group_layout,
            draw_particles_texture_bind_group_layout,
            draw_particles_render_pipeline,
            draw_particle_trails_render_pipeline,
            particle_quad_vertex_buffer,
            uniform_buffer_capacity,
            uniform_buffer_stride,
            uniform_buffer,
            cpu_uniform_buffer,
            particle_instance_data_buffer_capacity,
            particle_instance_data_buffer,
            cpu_particle_instance_data_buffer,
            particle_trail_vertex_buffer_capacity,
            particle_trail_vertex_buffer,
            cpu_particle_trail_vertex_buffer,
            particle_texture_sampler,
            modulation_texture_sampler,
            draw_particles_main_bind_group,
            texture_name_to_texture_id,
            texture_to_bind_group,
            pending_texture_bind_groups: emitter_texture_names_that_need_bind_groups_set,
        }
    }

    /// This takes `size_in_bytes` (rather than capacity) because there is a separate
    /// stride calculation determining how many bytes to alloc per uniform struct.
    fn alloc_gpu_uniform_buffer(device: &wgpu::Device, size_in_bytes: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleEffectManager: uniforms"),
            size: size_in_bytes as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn alloc_gpu_particle_instance_data_buffer(
        device: &wgpu::Device,
        capacity: usize,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleEffectManager: instance data"),
            size: (capacity * std::mem::size_of::<ParticleInstanceData>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn alloc_gpu_particle_trail_vertex_buffer(
        device: &wgpu::Device,
        capacity: usize,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleEffectManager: trail vertices"),
            size: (capacity * std::mem::size_of::<ParticleTrailVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn resize_uniform_buffers(&mut self, device: &wgpu::Device, capacity: usize) {
        assert!(capacity != self.uniform_buffer_capacity);

        let size_in_bytes = capacity * self.uniform_buffer_stride;

        self.uniform_buffer_capacity = capacity;
        self.cpu_uniform_buffer.resize(size_in_bytes, 0u8);
        // Allocate a new GPU-side buffer with the new capacity (WebGPU buffers can't
        // be resized).
        self.uniform_buffer = Self::alloc_gpu_uniform_buffer(device, size_in_bytes);
    }

    fn resize_particle_instance_data_buffers(&mut self, device: &wgpu::Device, capacity: usize) {
        assert!(capacity != self.particle_instance_data_buffer_capacity);

        self.particle_instance_data_buffer_capacity = capacity;
        self.cpu_particle_instance_data_buffer.resize(
            capacity,
            ParticleInstanceData {
                position: [0.0, 0.0],
                rotation: 0.0,
                t: 0.0,
                uv_scroll_t_offset: 0.0,
                world_space_scale: 0.0,
            },
        );
        // Allocate a new GPU-side buffer with the new capacity (WebGPU buffers can't
        // be resized).
        self.particle_instance_data_buffer =
            Self::alloc_gpu_particle_instance_data_buffer(device, capacity);
    }

    fn resize_particle_trail_vertex_buffers(&mut self, device: &wgpu::Device, capacity: usize) {
        assert!(capacity != self.particle_trail_vertex_buffer_capacity);

        self.particle_trail_vertex_buffer_capacity = capacity;
        self.cpu_particle_trail_vertex_buffer.resize(
            capacity,
            ParticleTrailVertex {
                position: [0.0, 0.0],
                alpha: 0.0,
            },
        );
        // Allocate a new GPU-side buffer with the new capacity (WebGPU buffers can't
        // be resized).
        self.particle_trail_vertex_buffer =
            Self::alloc_gpu_particle_trail_vertex_buffer(device, capacity);
    }

    /// This needs to be called whenever any of the passed-in wgpu resources are
    /// reallocated, as these are all referenced in the main `BindGroup` (this is
    /// relevant for growing the capacities of `Buffer`(s) right now).
    fn create_draw_particles_main_bind_group(
        device: &wgpu::Device,
        main_bind_group_layout: &wgpu::BindGroupLayout,
        uniform_buffer: &wgpu::Buffer,
        particle_texture_sampler: &wgpu::Sampler,
        modulation_texture_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: main_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniform_buffer,
                        // Using per-draw dynamic offsets, so just use offset 0 here.
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(particle_texture_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(modulation_texture_sampler),
                },
            ],
            label: Some("ParticleEffectManager: main"),
        })
    }

    /// Creates a `BindGroup` for the passed-in permutation of `EmitterTextureNames`
    /// *if* all of the associated textures are ready. Returns None without doing
    /// anything if that's not the case.
    fn create_draw_particles_texture_bind_group_if_all_textures_ready(
        device: &wgpu::Device,
        draw_particles_texture_bind_group_layout: &wgpu::BindGroupLayout,
        texture_name_to_texture_id: &HashMap<String, TextureId>,
        gpu_texture_manager: &mut GpuTextureManager,
        emitter_texture_names: &EmitterTextureNames,
    ) -> Option<wgpu::BindGroup> {
        let texture_view = &if !emitter_texture_names.texture_name.is_empty() {
            // The texture is ready when it can be retrieved from `GpuTextureManager`
            let texture_id = texture_name_to_texture_id.get(&emitter_texture_names.texture_name)?;
            gpu_texture_manager.get_texture(*texture_id)?
        } else {
            gpu_texture_manager.get_texture_or_missing(TextureAssetManager::missing_texture_id())
        }
        .view;

        let mut modulation_texture1_view_option = Option::<&wgpu::TextureView>::None;
        if let Some(tex_id) = emitter_texture_names
            .modulation_texture1
            .as_ref()
            .and_then(|modulation_texture| texture_name_to_texture_id.get(modulation_texture))
        {
            let texture = gpu_texture_manager.get_texture(*tex_id)?;
            modulation_texture1_view_option = Some(&texture.view);
        }

        let mut modulation_texture2_view_option = Option::<&wgpu::TextureView>::None;
        if let Some(tex_id) = emitter_texture_names
            .modulation_texture2
            .as_ref()
            .and_then(|modulation_texture| texture_name_to_texture_id.get(modulation_texture))
        {
            let texture = gpu_texture_manager.get_texture(*tex_id)?;
            modulation_texture2_view_option = Some(&texture.view);
        }

        // It appears we can't bind nothing in cases where modulation textures
        // are unused, so just bind the particle texture multiple times. We
        // could also create a 'dummy' 1px texture and use that - not sure
        // if it matters since this texture isn't sampled when unused.
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: draw_particles_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        if let Some(modulation_texture1_view) = modulation_texture1_view_option {
                            modulation_texture1_view
                        } else {
                            texture_view
                        },
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        if let Some(modulation_texture2_view) = modulation_texture2_view_option {
                            modulation_texture2_view
                        } else {
                            texture_view
                        },
                    ),
                },
            ],
            label: Some("ParticleEffectManager: texture"),
        }))
    }

    /// This generates (through repeated downsampling) the mip chain for `texture`, which is assumed to have had
    /// its mip level 0 already initialized.
    /// Todo: Move this to `GpuTextureManager` or somewhere more general
    #[allow(dead_code)]
    fn generate_mipmaps(
        &self,
        device: &wgpu::Device,
        command_encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) {
        let mip_level_texture_views = (0..texture.mip_level_count())
            .map(|mip_level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: None,
                    format: None,
                    dimension: None,
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip_level,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: None,
                })
            })
            .collect::<Vec<_>>();

        // Progressively downsample, starting from level 0. Each iteration of this loop uses the output of the
        // previous iteration as input.
        for target_mip_level in 1..texture.mip_level_count() {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &self.blit_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &mip_level_texture_views[target_mip_level as usize - 1],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.blit_input_texture_sampler),
                    },
                ],
                label: None,
            });

            let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mip_level_texture_views[target_mip_level as usize],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            render_pass.set_pipeline(&self.blit_render_pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            // Draw 3 vertices for blit.wgsl's "fullscreen triangle".
            render_pass.draw(0..3, 0..1);
        }
    }

    fn create_effect(
        &mut self,
        gpu_interface: &mut GpuInterface,
        handle: ParticleEffectHandle,
        particle_effect: ParticleEffect,
    ) {
        for emitter in &particle_effect.emitters {
            let mut request_texture_name_if_applicable = |texture_name: &String| {
                let texture_path = &format!("{}.png", texture_name);
                if gpu_interface
                    .texture_asset_manager
                    .get_texture_by_path(&texture_path.into())
                    .is_some()
                {
                    return;
                }

                let texture_id = gpu_interface
                    .texture_asset_manager
                    .register_next_texture_id();
                let pending_texture = PendingTexture::new(texture_id, &texture_path.into(), false);
                gpu_interface
                    .texture_asset_manager
                    .add_to_batched_textures(pending_texture);

                self.texture_name_to_texture_id
                    .insert(texture_name.clone(), texture_id);
            };

            let emitter_texture_names = emitter.texture_names();
            request_texture_name_if_applicable(&emitter_texture_names.texture_name);
            if emitter_texture_names.modulation_texture1.is_some() {
                request_texture_name_if_applicable(
                    emitter_texture_names.modulation_texture1.as_ref().unwrap(),
                );
            }
            if emitter_texture_names.modulation_texture2.is_some() {
                request_texture_name_if_applicable(
                    emitter_texture_names.modulation_texture2.as_ref().unwrap(),
                );
            }

            // Check whether each of the EmitterTextureNames permutations already
            // has a BindGroup cached, and if not, record that we need to create one
            // before rendering. These can only be created once all of their associated
            // textures have been loaded/created.
            if !self
                .texture_to_bind_group
                .contains_key(emitter_texture_names)
            {
                self.pending_texture_bind_groups
                    .insert(emitter_texture_names.clone());
            }
        }

        self.particle_effects.insert(handle, particle_effect);
    }

    #[allow(unused)]
    pub fn create_effect_from_descriptors(
        &mut self,
        gpu_interface: &mut GpuInterface,
        handle: ParticleEffectHandle,
        emitters: Vec<EmitterDescriptor>,
    ) {
        // Create a default descriptor containing just the passed in`emitters` vec
        let particle_desc = ParticleEffectDescriptor { emitters };
        let particle_effect = ParticleEffect::new(&particle_desc, &Mat3::IDENTITY);
        self.create_effect(gpu_interface, handle, particle_effect);
    }

    pub fn create_effect_from_id(
        &mut self,
        gpu_interface: &mut GpuInterface,
        handle: ParticleEffectHandle,
        descriptor_id: &AssetId,
        transform: &Mat3,
    ) {
        let Some(descriptor) = gpu_interface.particle_effect_descriptors.get(descriptor_id) else {
            log::warn!(
                "ParticleEffectManager::create_effect_from_id() - failed to add particle with descriptor id {descriptor_id}"
            );
            return;
        };

        let particle_effect = ParticleEffect::new(descriptor, transform);
        self.create_effect(gpu_interface, handle, particle_effect);
    }

    pub fn contains_effect(&self, handle: &ParticleEffectHandle) -> bool {
        self.particle_effects.contains_key(handle)
    }

    pub fn destroy_effect(&mut self, handle: &ParticleEffectHandle) {
        let result = self.particle_effects.get_mut(handle);
        if let Some(particle_effect) = result {
            // Remove the PE on the next update where ParticleEffect::is_finished returns true.
            particle_effect.notify_remove();
        } else {
            log::warn!(
                "ParticleEffectManager::destroy_effect() - Handle {:?} not found",
                handle
            );
        }
    }

    pub fn set_transform(&mut self, handle: &ParticleEffectHandle, transform: &Mat3) {
        let result = self.particle_effects.get_mut(handle);
        if let Some(particle_effect) = result {
            particle_effect.set_transform(transform);
        } else {
            log::warn!(
                "ParticleEffectManager::set_transform() - Handle {:?} not found",
                handle
            );
        }
    }

    pub fn begin_frame(&mut self) {
        self.current_frame_num_emitters = 0;
        self.current_frame_max_num_particles = 0;
        self.current_frame_max_num_trail_vertices = 0;
        self.next_uniform_buffer_offset = 0;
        self.next_particle_instance_data_buffer_index = 0;
        self.next_particle_trail_vertex_buffer_index = 0;

        // Swap last frame's `current_visible_effects` to this frame's `previous_visible_effects`
        std::mem::swap(
            &mut self.current_visible_effects,
            &mut self.previous_visible_effects,
        );
        self.current_visible_effects.clear();
    }

    pub fn update_effect(&mut self, handle: &ParticleEffectHandle, delta_time: f32) -> bool {
        if let Some(effect) = self.particle_effects.get_mut(handle) {
            effect.update(delta_time);

            self.current_frame_num_emitters += effect.num_emitters();
            self.current_frame_max_num_particles += effect.max_num_particles();
            self.current_frame_max_num_trail_vertices += effect.max_num_trail_vertices();

            if !effect.is_finished() {
                self.current_visible_effects.insert(*handle);
                self.previous_visible_effects.remove(handle);
            }

            return !effect.is_finished();
        }
        false
    }

    /// This should be called after all `ParticleEffects` have been ticked via `ParticleEffectManager::update_effect`
    /// and before `render_effect` calls.
    pub fn post_update_effects(
        &mut self,
        device: &wgpu::Device,
        gpu_texture_manager: &mut GpuTextureManager,
    ) {
        // Remove stale effects
        for effect in &self.previous_visible_effects {
            self.particle_effects.remove_entry(effect);
        }

        // Create any BindGroups that have been marked as needed as soon as all
        // of their associated textures have finished loading.

        self.pending_texture_bind_groups
            .retain(|emitter_texture_names| {
                let Some(draw_particles_texture_bind_group) =
                    Self::create_draw_particles_texture_bind_group_if_all_textures_ready(
                        device,
                        &self.draw_particles_texture_bind_group_layout,
                        &self.texture_name_to_texture_id,
                        gpu_texture_manager,
                        emitter_texture_names,
                    )
                else {
                    return true;
                };

                let prev = self.texture_to_bind_group.insert(
                    emitter_texture_names.clone(),
                    draw_particles_texture_bind_group,
                );

                assert!(prev.is_none());

                false
            });
    }

    // This should be called after all `ParticleEffects` have been rendered via `ParticleEffectManager::render_effect`
    pub fn end_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        num_camera_passes: usize,
    ) {
        // First, grow our buffer capacities if the number of active emitters or
        // the maximum particles that may currently be alive exceeds their current
        // capacities.

        let new_uniform_buffer_capacity = grow_capacity(
            self.uniform_buffer_capacity,
            self.current_frame_num_emitters,
            PARTICLE_EFFECT_MANAGER_ALLOC_GROWTH_FACTOR,
        );
        if new_uniform_buffer_capacity > self.uniform_buffer_capacity {
            self.resize_uniform_buffers(device, new_uniform_buffer_capacity);

            // Important: when we realloc the uniform buffer, we need to
            // recreate the main BindGroup, as otherwise we'll render with one
            // that still reference the old buffer!
            self.draw_particles_main_bind_group = Self::create_draw_particles_main_bind_group(
                device,
                &self.draw_particles_main_bind_group_layout,
                &self.uniform_buffer,
                &self.particle_texture_sampler,
                &self.modulation_texture_sampler,
            );
        }

        let new_particle_instance_data_buffer_capacity = grow_capacity(
            self.particle_instance_data_buffer_capacity,
            self.current_frame_max_num_particles,
            PARTICLE_EFFECT_MANAGER_ALLOC_GROWTH_FACTOR,
        );
        if new_particle_instance_data_buffer_capacity > self.particle_instance_data_buffer_capacity
        {
            self.resize_particle_instance_data_buffers(
                device,
                new_particle_instance_data_buffer_capacity,
            );
        }

        let new_particle_trail_vertex_buffer_capacity = grow_capacity(
            self.particle_trail_vertex_buffer_capacity,
            self.current_frame_max_num_trail_vertices,
            PARTICLE_EFFECT_MANAGER_ALLOC_GROWTH_FACTOR,
        );
        if new_particle_trail_vertex_buffer_capacity > self.particle_trail_vertex_buffer_capacity {
            self.resize_particle_trail_vertex_buffers(
                device,
                new_particle_trail_vertex_buffer_capacity,
            );
        }

        // render()

        // Update the dynamic buffers so they have the expected data when the
        // render pass / command buffer are submitted.
        //
        // Note that we upload only the portions of the buffers that will be
        // used to render this frame, not the entire (CPU-side) buffers.

        if self.next_uniform_buffer_offset > 0 {
            assert!(
                self.next_uniform_buffer_offset
                    <= num_camera_passes * self.uniform_buffer.size() as usize
            );
            queue.write_buffer(
                &self.uniform_buffer,
                0,
                &self.cpu_uniform_buffer[0..self.next_uniform_buffer_offset],
            );
        }

        if self.next_particle_instance_data_buffer_index > 0 {
            assert!(
                self.next_particle_instance_data_buffer_index
                    <= self.current_frame_max_num_particles * num_camera_passes
            );
            queue.write_buffer(
                &self.particle_instance_data_buffer,
                0,
                bytemuck::cast_slice(
                    &self.cpu_particle_instance_data_buffer
                        [0..self.next_particle_instance_data_buffer_index],
                ),
            );
        }

        if self.next_particle_trail_vertex_buffer_index > 0 {
            assert!(
                self.next_particle_trail_vertex_buffer_index
                    <= self.current_frame_max_num_trail_vertices
            );
            queue.write_buffer(
                &self.particle_trail_vertex_buffer,
                0,
                bytemuck::cast_slice(
                    &self.cpu_particle_trail_vertex_buffer
                        [0..self.next_particle_trail_vertex_buffer_index],
                ),
            );
        }
    }

    pub fn render_effect(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'_>,
        handle: &ParticleEffectHandle,
        mat_vp: &Mat4,
    ) {
        let particle_effect = self.particle_effects.get(handle);
        if particle_effect.is_none() {
            log::warn!(
                "ParticleEffectManager::render_effect() - Handle {:?} not found",
                handle
            );
            return;
        }
        let particle_effect = particle_effect.unwrap();

        render_pass.set_vertex_buffer(0, self.particle_quad_vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.particle_instance_data_buffer.slice(..));
        render_pass.set_vertex_buffer(2, self.particle_trail_vertex_buffer.slice(..));
        render_pass.set_pipeline(&self.draw_particles_render_pipeline);

        particle_effect.render(
            render_pass,
            &self.draw_particles_render_pipeline,
            &self.draw_particle_trails_render_pipeline,
            &self.draw_particles_main_bind_group,
            &self.texture_to_bind_group,
            mat_vp,
            &mut self.cpu_uniform_buffer,
            &mut self.next_uniform_buffer_offset,
            self.uniform_buffer_stride,
            &mut self.cpu_particle_instance_data_buffer,
            &mut self.next_particle_instance_data_buffer_index,
            &mut self.cpu_particle_trail_vertex_buffer,
            &mut self.next_particle_trail_vertex_buffer_index,
        );
    }
}
