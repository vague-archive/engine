// NOTE: main_bind_group.wgsl should be prepended to the contents of this file.

// Vertex shader

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) alpha: f32,
};

struct VertexOutput {
    @builtin(position) Position: vec4f,
    @location(0) alpha: f32,
};

@vertex
fn vs_main(vi: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.Position = uniforms.matWVP * vec4f(vi.position, 0.0, 1.0);
    out.alpha = vi.alpha;
    return out;
}

// Fragment shader

@fragment
fn fs_main(vo: VertexOutput) -> @location(0) vec4f {
    let finalAlpha = uniforms.color.a * vo.alpha;
    let premultipliedColor = vec4f(uniforms.color.rgb * finalAlpha, finalAlpha);
    return premultipliedColor;
}
