// The main bind group (#0) is shared between pipelines and should be prepended
// to their source.

struct Uniforms {
    // Using mat4x4 instead of 3x3 for now to avoid weird size/alignment issues with 3x3.
    matWVP: mat4x4f,
    // This is *not* premultiplied and incorporates Emitter "glow"
    color: vec4f,
    // uv_speed is measured in "UV units" per lifetime (rather than per second)
    modulation_texture1_uv_scale: vec2f,
    modulation_texture1_uv_speed: vec2f,
    modulation_texture2_uv_scale: vec2f,
    modulation_texture2_uv_speed: vec2f,
    // bool can't be used in uniforms, so use u32 for these flags.
    modulation_texture1_is_enabled: u32,
    modulation_texture2_is_enabled: u32,
    // start_size/end_size incorporate Emitter size_multiplier
    start_size: f32,
    end_size: f32,
    // end_fade_in/end_fade_out are 't' values (in [0,1])
    end_fade_in: f32,
    start_fade_out: f32,
    // This is set to 1 if the emitter uses world-space emission, 0 otherwise.
    use_world_space_emission: u32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var particleTextureSampler: sampler;
@group(0) @binding(2) var modulationTextureSampler: sampler;
