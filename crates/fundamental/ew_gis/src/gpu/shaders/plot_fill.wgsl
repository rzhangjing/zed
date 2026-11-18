// Batched vector fill. One draw over every polygon / circle / sector triangle;
// the CPU tessellates the shapes, the shader only projects and tints them.
//
// The uniform is the plot-specific `PlotUniform` (camera + viewport + depth
// bias), shared by all three plot pipelines. Tile/globe/icon keep the older
// 80-byte `CameraUniform` unchanged.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xy = viewport size in physical pixels, zw = its reciprocal.
    screen: vec4<f32>,
    // Clip-space depth bias (fraction of w) for fill / line / point.
    depth_bias: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var clip = camera.view_proj * vec4<f32>(in.position, 1.0);
    // In the 2D plane the fill is exactly coplanar with the tile quads, and at
    // a 9,000 km eye altitude one f32 depth step is ~10 km of ground distance,
    // so a world-space lift cannot separate them. A clip-space bias (the same
    // term the hardware polygon offset uses) can, and it is projection
    // independent: on the globe it simply makes the already lifted patch win
    // over the ellipsoid shell.
    clip.z = clip.z - camera.depth_bias.x * clip.w;

    var out: VsOut;
    out.clip_position = clip;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.color.a < 0.004) {
        discard;
    }
    return in.color;
}
