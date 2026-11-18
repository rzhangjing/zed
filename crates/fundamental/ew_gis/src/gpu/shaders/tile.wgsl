// 2D equirectangular tile plane. One draw call for all visible tiles.
// Each instance is one quad; `uv_rect` selects either the tile's own layer,
// a resident parent layer, or the equirectangular world layer (layer 0).

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var tiles: texture_2d_array<f32>;
@group(0) @binding(2) var tile_sampler: sampler;

struct TileInstance {
    @location(0) center: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_rect: vec4<f32>,
    @location(3) layer: u32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: TileInstance,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    let corner = corners[vertex_index];
    let normalized = corner + vec2<f32>(0.5, 0.5);
    let world = instance.center + corner * instance.size;

    var out: VsOut;
    out.clip_position = camera.view_proj * vec4<f32>(world, 0.0, 1.0);
    // v0 is north (top); quad local +y is north.
    out.uv = vec2<f32>(
        mix(instance.uv_rect.x, instance.uv_rect.z, normalized.x),
        mix(instance.uv_rect.w, instance.uv_rect.y, normalized.y),
    );
    out.layer = instance.layer;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // No tile border / graticule overlay: a seamless imagery mosaic. Fallback
    // layers remain visible as their own content until the exact tile lands.
    return textureSample(tiles, tile_sampler, in.uv, i32(in.layer));
}
