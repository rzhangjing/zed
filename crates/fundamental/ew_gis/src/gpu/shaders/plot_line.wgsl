// Batched screen-space polylines: one instanced draw, six vertices per segment.
//
// Each instance carries the two segment endpoints in world space plus its half
// width in *pixels*. The shader projects both endpoints, expands the segment
// into a quad along the screen-space normal (with a half-width cap extension so
// consecutive segments overlap at joins), and converts the pixel offsets back
// to clip space by multiplying with the endpoint's own `w`, which leaves the
// depth untouched apart from the shared bias.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xy = viewport size in physical pixels, zw = its reciprocal.
    screen: vec4<f32>,
    // Clip-space depth bias (fraction of w) for fill / line / point.
    depth_bias: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct LineInstance {
    // xyz = segment start, w = half width in pixels.
    @location(0) a: vec4<f32>,
    // xyz = segment end, w unused (reserved for dashes).
    @location(1) b: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    // -1..1 across the ribbon, used for the analytic edge fade.
    @location(0) edge: f32,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: LineInstance) -> VsOut {
    var out: VsOut;
    out.color = instance.color;
    out.edge = 0.0;

    let clip_a = camera.view_proj * vec4<f32>(instance.a.xyz, 1.0);
    let clip_b = camera.view_proj * vec4<f32>(instance.b.xyz, 1.0);
    if (min(clip_a.w, clip_b.w) < 0.001) {
        // At least one endpoint is on or behind the eye plane: emit a fully
        // clipped degenerate triangle instead of a wrap-around ribbon.
        out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }

    let screen = camera.screen.xy;
    let px_a = clip_a.xy / clip_a.w * screen * 0.5;
    let px_b = clip_b.xy / clip_b.w * screen * 0.5;
    let delta = px_b - px_a;
    let length_px = length(delta);
    let axis = select(vec2<f32>(1.0, 0.0), delta / length_px, length_px > 1.0e-5);
    let normal = vec2<f32>(-axis.y, axis.x);

    // 0 = start cap, 1 = end cap; y = side of the ribbon.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];
    let at_end = corner.x > 0.5;
    let base = select(clip_a, clip_b, at_end);
    let along = select(-1.0, 1.0, at_end);
    let half_width = instance.a.w;
    let offset_px = normal * (half_width * corner.y) + axis * (half_width * along);

    let clip_xy = base.xy + offset_px * 2.0 * camera.screen.zw * base.w;
    out.clip_position = vec4<f32>(
        clip_xy,
        base.z - camera.depth_bias.y * base.w,
        base.w,
    );
    out.edge = corner.y;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Analytic 1 px edge fade: the ribbon is exactly 2 * half_width pixels wide,
    // so the derivative of `edge` is one pixel worth of ribbon.
    let aa = max(fwidth(in.edge), 1.0e-4);
    let alpha = 1.0 - smoothstep(1.0 - aa, 1.0, abs(in.edge));
    let color = vec4<f32>(in.color.rgb, in.color.a * alpha);
    if (color.a < 0.004) {
        discard;
    }
    return color;
}
