struct VertexInput {
    @builtin(vertex_index) index: u32,
    @location(0) color: vec4f,
    @location(1) top_left: vec3f,
    @location(2) bottom_right: vec2f,
    @location(3) uv_top_left: vec2f,
    @location(4) uv_bottom_right: vec2f,
}

struct BrushBuffer {
    transform: mat4x4f,
}
@group(0) @binding(0) var<uniform> brush_uniforms: BrushBuffer;


struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    var position: vec2f;
    var left = in.top_left.x;
    var right = in.bottom_right.x;
    var top = in.top_left.y;
    var bottom = in.bottom_right.y;

    switch(in.index) {
        case 0u: {
            position = vec2f(left, top);
            out.uv = in.uv_top_left;
            break;
        }
        case 1u: {
            position = vec2f(right, top);
            out.uv = vec2f(in.uv_bottom_right.x, in.uv_top_left.y);
            break;
        }
        case 2u: {
            position = vec2f(left, bottom);
            out.uv = vec2f(in.uv_top_left.x, in.uv_bottom_right.y);
            break;
        }
        case 3u: {
            position = vec2f(right, bottom);
            out.uv = in.uv_bottom_right;
            break;
        }
        default {}
    }

    out.clip_position = brush_uniforms.transform * vec4f(position, in.top_left.z, 1.0);
    out.color = in.color;

    return out;
}

@group(0) @binding(1) var t_color: texture_2d<f32>;
@group(0) @binding(2) var s_color: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    var alpha = textureSample(t_color, s_color, in.uv).r;
    return vec4f(in.color.rgb, in.color.a * alpha);
}
