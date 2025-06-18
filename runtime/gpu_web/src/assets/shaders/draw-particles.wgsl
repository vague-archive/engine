// NOTE: main_bind_group.wgsl should be prepended to the contents of this file.

@group(1) @binding(0) var particleTexture: texture_2d<f32>;
// Modulation textures are optional and will only be used if the corresponding bool in
// Uniforms is set.
@group(1) @binding(1) var modulationTexture1: texture_2d<f32>;
@group(1) @binding(2) var modulationTexture2: texture_2d<f32>;

// Vertex shader

struct VertexInput {
    // 'uv' is stepped per-vertex
    @location(0) uv: vec2f,

    // The other attributes are stepped per-instance and represent per-particle 
    // data.

    @location(1) position: vec2f,
    // CCW rotation in radians
    @location(2) rotation: f32,
    // Normalized time particle has been alive (in [0,1])
    @location(3) t: f32,
    // This is randomly generated on emission so that UV scrolling starts at different 
    // offsets per particle.
    //
    // NOTE: If we want more things to be randomized per-particle, we could replace
    // this field with a random seed and generate random numbers in the vertex shader
    // starting from this seed.    
    @location(4) uv_scroll_t_offset: f32,
    // A scale applied "on top of" the particle-effect-local size computed in
    // the vertex shader. This is necessary for world-space particle emission
    // and should "no-op" (i.e., be 1.0) for local-space particle emission.
    @location(5) world_space_scale: f32,
};

struct VertexOutput {
    @builtin(position) Position: vec4f,
    @location(0) uv: vec2f,
    @location(1) alpha: f32,
    @location(3) uv_scroll_t: f32,
};

// If mix(a, b, t) = x, then invMix(a, b, x) = t
fn inv_mix(a: f32, b: f32, x: f32) -> f32 {
    return (x - a) / (b - a);
}

fn alpha_over_lifetime(endFadeIn: f32, startFadeOut: f32, t: f32) -> f32 {
    let fadeIn = clamp(inv_mix(0.0, endFadeIn, t), 0.0, 1.0);
    let fadeOut = 1.0 - clamp(inv_mix(startFadeOut, 1.0, t), 0.0, 1.0);
    return min(fadeIn, fadeOut);
}

@vertex
fn vs_main(vi: VertexInput) -> VertexOutput {
    let currentSize = mix(uniforms.start_size, uniforms.end_size, vi.t);
    let currentAlpha =
        alpha_over_lifetime(uniforms.end_fade_in, uniforms.start_fade_out, vi.t);

    // Account for world-space scale when this emitter uses
    // world-space-emission.
     let worldSpaceScale = select(
        1.0, // false
        vi.world_space_scale, // true
        uniforms.use_world_space_emission != 0);

    // Remap uv to position by flipping Y and centering on origin.
    var position = vec2f(vi.uv.x, 1.0 - vi.uv.y) - vec2f(0.5);
    // Then apply translation/rotation/scale to get final position.
    let cosR = cos(vi.rotation);
    let sinR = sin(vi.rotation);
    position = vi.position + currentSize * worldSpaceScale *
        vec2f(cosR * position.x - sinR * position.y,
              sinR * position.x + cosR * position.y);

    var out: VertexOutput;
    out.Position = uniforms.matWVP * vec4f(position, 0.0, 1.0);
    out.uv = vi.uv;
    out.alpha = currentAlpha;
    out.uv_scroll_t = vi.uv_scroll_t_offset + vi.t;

    return out;
}

// Fragment shader

@fragment
fn fs_main(vo: VertexOutput) -> @location(0) vec4f {
    // We always sample from the main particleTexture, and optionally sample from the two 
    // modulation textures. Each modulation texture scales/scrolls the UVs independently 
    // according to its own settings. When multiplying by a modulation texture, we 
    // multiply by an additional factor of 2, I believe to account for the reduction 
    // in brightness that would result from multiplying by values in [0,1], and then 
    // clamp to 1.0 when we're done. (This was taken from a similar process in the VFX 
    // tool's Godot shaders).
    //
    // NOTE: Particle textures are "alpha textures", but the alpha value is actually stored in
    // the color channels (which are identical). The actual alpha channel should not be used, 
    // as it is often 1.0 for every texel.    
    var texModulated = textureSample(particleTexture, particleTextureSampler, vo.uv).r;
    if (uniforms.modulation_texture1_is_enabled != 0u) {
        let uv = vo.uv * uniforms.modulation_texture1_uv_scale +
            vo.uv_scroll_t * uniforms.modulation_texture1_uv_speed;
        texModulated *= 2.0 * textureSample(modulationTexture1, modulationTextureSampler, uv).r;
    }
    if (uniforms.modulation_texture2_is_enabled != 0u) {
        let uv = vo.uv * uniforms.modulation_texture2_uv_scale +
            vo.uv_scroll_t * uniforms.modulation_texture2_uv_speed;
        texModulated *= 2.0 * textureSample(modulationTexture2, modulationTextureSampler, uv).r;
    }
    texModulated = min(1.0, texModulated);

    let finalAlpha = uniforms.color.a * vo.alpha;
    let premultipliedColor = vec4f(uniforms.color.rgb * finalAlpha, finalAlpha);

    return premultipliedColor * texModulated;
}
