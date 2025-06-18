// This shader is reused from wgpu's 'mipmap' sample, with minor modifications.
// See: https://github.com/gfx-rs/wgpu/blob/trunk/examples/src/mipmap/blit.wgsl

struct VertexOutput {
    @builtin(position) Position: vec4f,
    @location(0) uv: vec2f,
};

// meant to be called with 3 vertex indices: 0, 1, 2
// draws one large triangle over the clip space like this:
// (the asterisks represent the clip space bounds)
//-1,1           1,1
// ---------------------------------
// |              *              .
// |              *           .
// |              *        .
// |              *      .
// |              *    . 
// |              * .
// |***************
// |            . 1,-1 
// |          .
// |       .
// |     .
// |   .
// |.
@vertex
fn vs_main(@builtin(vertex_index) VertexIndex: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = i32(VertexIndex) / 2;
    let y = i32(VertexIndex) & 1;
    let uv = vec2f(
        f32(x) * 2.0,
        f32(y) * 2.0
    );
    out.Position = vec4f(
        uv.x * 2.0 - 1.0,
        1.0 - uv.y * 2.0,
        0.0, 1.0
    );
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var inputTexture: texture_2d<f32>;
@group(0) @binding(1) var inputTextureSampler: sampler;

@fragment
fn fs_main(vo: VertexOutput) -> @location(0) vec4f {
    return textureSample(inputTexture, inputTextureSampler, vo.uv);
}
