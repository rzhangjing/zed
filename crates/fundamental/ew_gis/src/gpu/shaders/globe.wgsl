// Quadtree LOD globe. One indexed draw over a merged mesh of WGS84 ellipsoid
// tiles; every vertex carries the texture-array layer it must sample, so a
// single `texture_2d_array` bind group serves exact tiles, ancestor fallbacks,
// and the equirectangular polar caps.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var tiles: texture_2d_array<f32>;
@group(0) @binding(2) var tile_sampler: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) layer: u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_position: vec3<f32>,
    // Integer varyings must be flat: the whole triangle samples one layer.
    @location(3) @interpolate(flat) layer: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.uv = in.uv;
    out.world_position = in.position;
    out.layer = in.layer;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let albedo = textureSampleLevel(tiles, tile_sampler, in.uv, i32(in.layer), 0.0).rgb;
    let normal = normalize(in.normal);
    let light_dir = normalize(vec3<f32>(-0.35, 0.62, 0.70));
    let view_dir = normalize(camera.eye.xyz - in.world_position);

    let diffuse = max(dot(normal, light_dir), 0.0);
    let rim = pow(1.0 - clamp(dot(normal, view_dir), 0.0, 1.0), 3.0);
    let day = 0.13 + 0.92 * diffuse;
    var color = albedo * day;
    color += vec3<f32>(0.08, 0.20, 0.36) * rim;

    // A little warm sun scattering on the terminator.
    let terminator = smoothstep(0.0, 0.22, diffuse) * (1.0 - smoothstep(0.22, 0.55, diffuse));
    color += vec3<f32>(0.12, 0.055, 0.018) * terminator;

    return vec4<f32>(color, 1.0);
}
