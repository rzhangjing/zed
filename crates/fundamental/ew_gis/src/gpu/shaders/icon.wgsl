// Instanced textured icon quads. CPU supplies a per-instance T * R * S matrix
// (quaternion rotation around X/Y/Z) plus a tint color.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var icon_texture: texture_2d<f32>;
@group(0) @binding(2) var icon_sampler: sampler;

struct IconInstance {
    @location(0) model_0: vec4<f32>,
    @location(1) model_1: vec4<f32>,
    @location(2) model_2: vec4<f32>,
    @location(3) model_3: vec4<f32>,
    @location(4) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: IconInstance,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    let local = corners[vertex_index];
    let model = mat4x4<f32>(
        instance.model_0,
        instance.model_1,
        instance.model_2,
        instance.model_3,
    );
    let world = model * vec4<f32>(local.x, local.y, 0.0, 1.0);

    var out: VsOut;
    out.clip_position = camera.view_proj * world;
    out.uv = vec2<f32>(local.x + 0.5, 0.5 - local.y);
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(icon_texture, icon_sampler, in.uv);
    let color = texel * in.color;
    if color.a < 0.01 {
        discard;
    }
    return color;
}
