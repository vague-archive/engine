struct GlobalUniforms {
    inv_screen_dimensions: vec4f,
    camera_transform:  vec4f,
}
@group(0) @binding(0) var<uniform> global_uniforms: GlobalUniforms;

%uniforms

// Constant data for post processes
@group(1) @binding(0) var<storage, read> scene_instances: array<SceneInstance>;

// Maps draw instances to their index into scene_instances
@group(1) @binding(1) var<storage, read> scene_indices: array<u32>;

struct VertexInput {
    @location(0) color: vec4f,
    @location(1) position: vec3f,
    @location(2) tex_coords: vec2f,
    @builtin(instance_index) instance_idx: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) tex_coords: vec2f,
    @location(1) color: vec4f,
    @location(2) instance_idx: u32,
}

fn get_world_offset(uv0: vec2f, instance_index: u32) -> vec2f {
    let scene_instance = scene_instances[instance_index];
    %get_world_offset
}

@vertex
fn vs_main(
    vertex: VertexInput
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex.tex_coords;
    out.clip_position = vec4f(vertex.position, 1);
    out.color = vec4f(1.0, 1.0, 1.0, 1.0);
    out.instance_idx = vertex.instance_idx;
    return out;
}

%textures

fn get_fragment_color(uv0: vec2f, instance_idx: u32, vertex_color: vec4f) -> vec4f {
    let scene_instance = scene_instances[instance_idx];
    %get_fragment_color
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    var fragment_color = get_fragment_color(in.tex_coords.xy, in.instance_idx, in.color);

    return fragment_color;
}
