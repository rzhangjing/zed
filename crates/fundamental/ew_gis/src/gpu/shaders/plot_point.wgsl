// Batched point markers: one instanced draw, six vertices per marker.
//
// The CPU supplies the world-space centre and a diameter in pixels. The vertex
// shader projects the centre and offsets the quad in screen space, so the
// marker keeps its pixel size at every zoom and on both scenes. The fragment
// shader draws an anti-aliased disc with a darker rim from the quad-local
// coordinate, which is why the quad is [-1, 1] rather than [-0.5, 0.5].

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xy = viewport size in physical pixels, zw = its reciprocal.
    screen: vec4<f32>,
    // Clip-space depth bias (fraction of w) for fill / line / point.
    depth_bias: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct PointInstance {
    // xyz = centre, w = marker diameter in pixels.
    @location(0) center: vec4<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    // Quad-local coordinate in [-1, 1]: the marker's signed radius.
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: PointInstance) -> VsOut {
    var out: VsOut;
    out.color = instance.color;

    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];
    out.local = corner;

    let clip = camera.view_proj * vec4<f32>(instance.center.xyz, 1.0);
    if (clip.w < 0.001) {
        out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }

    let half_px = instance.center.w * 0.5;
    let clip_xy = clip.xy + corner * half_px * 2.0 * camera.screen.zw * clip.w;
    out.clip_position = vec4<f32>(
        clip_xy,
        clip.z - camera.depth_bias.z * clip.w,
        clip.w,
    );
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let radius = length(in.local);
    let aa = max(fwidth(radius), 1.0e-4);
    let disc = 1.0 - smoothstep(1.0 - aa, 1.0, radius);
    // A slightly darker rim keeps bright markers readable on light tiles.
    let rim = smoothstep(0.62 - aa, 0.62 + aa, radius) * 0.85;
    let body = mix(in.color.rgb, in.color.rgb * 0.35 + vec3<f32>(0.05, 0.07, 0.09), rim);
    let color = vec4<f32>(body, in.color.a * disc);
    if (color.a < 0.004) {
        discard;
    }
    return color;
}
