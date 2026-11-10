//! The GPU map backend: plays a [`rustgis_render::gpu_scene::GpuScene`] into an offscreen wgpu
//! render pass. This is a port of RustGIS 0.6.0's `crates/ui-egui/src/mapgpu.rs` (Apache-2.0,
//! see `LICENSE-RUSTGIS` at the repository root) to a host that is not egui:
//!
//! * **Modified**: the `egui_wgpu::CallbackTrait` callback is replaced by [`GpuMap`], which owns
//!   an offscreen 4× MSAA target, records the scene's pass, resolves it, and presents it into a
//!   host surface (the map's own child window) or reads it back as BGRA/RGBA for a host image.
//! * **Modified**: `rustgis_engine::render::…` paths become the L2/L3 crates this adapter
//!   depends on directly, and the live raster-tile layer path (egui-only pacing, `LiveTiles`)
//!   is dropped: this host builds its scenes with `Backend::Gpu`, where raster tiles arrive as
//!   ordinary [`DrawCmd::Tiles`] and vector tiles as [`DrawCmd::TileMeshes`].
//! * **Unchanged**: the wgpu 30 API (this crate depends on `wgpu30`, RustGIS's own version;
//!   Zed's gpui keeps its wgpu 29), the shader (`mapgpu.wgsl`, byte-identical apart from this
//!   note), the camera/marker math, the pipeline states, the stencil trick for vector tile
//!   squares, the depth-tested extrusions and the shadow stencil.
//!
//! `view_proj` maps origin-relative map-unit vertices to clip space and is golden-tested
//! against `Camera::to_screen` — the CPU path's own transform — to 1/100 px.

use wgpu30 as wgpu;

use rustgis_map::Camera;

/// Clip-space transform for one draw command: vertices are f32 offsets from the command's
/// `origin` (map units), so the f64 `(origin - center)` subtraction happens on the CPU where
/// the precision still is; a vertex's third coordinate is its height above the ground (map
/// units). Matches `Camera::to_screen` for a flat, turned or tilted camera: the ground offsets
/// `(u, v)` from the centre (logical px, turned to the bearing) and the height `up` go to
/// `x = 2u/w`, `y = -2(v cos p - up sin p)/h` over `w' = 1 - (v sin p + up cos p)/d`, where `p`
/// is the pitch and `d` the eye's distance from the centre (`Camera::project_raised`). `w'` is
/// a point's depth (1 at the centre); clip z is `w' - NEAR`, so the near plane cuts what is
/// behind the eye as `Camera::try_to_screen` leaves it out.
pub(crate) fn view_proj(cam: &Camera, w: f64, h: f64, origin: [f64; 2]) -> [[f32; 4]; 4] {
    let a = 1.0 / cam.res;
    let (sb, cb) = if cam.bearing.is_finite() { cam.bearing.to_radians().sin_cos() } else { (0.0, 1.0) };
    let (dx, dy) = (origin[0] - cam.center.x, origin[1] - cam.center.y);
    // Ground offsets and height as linear forms over (x, y, up, 1).
    let u = [a * cb, -a * sb, 0.0, a * (cb * dx - sb * dy)];
    let v = [-a * sb, -a * cb, 0.0, -a * (sb * dx + cb * dy)];
    let up = [0.0, 0.0, a, 0.0];
    let pitch = rustgis_map::camera::normalize_pitch(cam.pitch);
    let (sp, cp, inv_d) = if pitch == 0.0 {
        (0.0, 1.0, 0.0)
    } else {
        let (s, c) = pitch.to_radians().sin_cos();
        (s, c, 2.0 * rustgis_map::camera::TAN_HALF_FOV / h.max(1e-9))
    };
    let row = |f: &dyn Fn(usize) -> f64| -> [f64; 4] { std::array::from_fn(f) };
    let x = row(&|i| 2.0 / w.max(1e-9) * u[i]);
    let y = row(&|i| -2.0 / h * (cp * v[i] - sp * up[i]));
    let depth = row(&|i| if i == 3 { 1.0 } else { 0.0 } - inv_d * (sp * v[i] + cp * up[i]));
    let z = row(&|i| depth[i] - if i == 3 { NEAR_DEPTH } else { 0.0 });
    // Column-major, as WGSL's mat4x4f.
    std::array::from_fn(|col| [x[col] as f32, y[col] as f32, z[col] as f32, depth[col] as f32])
}

/// The nearest depth drawn (a fraction of the eye's distance from the centre), as
/// `Camera::try_to_screen` leaves out what is nearer.
const NEAR_DEPTH: f64 = rustgis_map::camera::NEAR;

/// Host-side mirror of the marker vertex shader's displacement: a unit-space template point
/// becomes a pixel offset, then a clip offset. Templates are defined y-down (screen), clip y
/// is up — hence the negated y. Test-only: the shader is the production side of the mirror.
#[cfg(test)]
pub(crate) fn marker_displace(template: [f32; 2], radius_px: f32, viewport_px: [f32; 2]) -> [f32; 2] {
    [template[0] * radius_px * 2.0 / viewport_px[0], -template[1] * radius_px * 2.0 / viewport_px[1]]
}

// ---- wgpu playback of a GpuScene ----

use std::sync::Arc;

use wgpu::util::DeviceExt;

use rustgis_render::gpu_scene::{self, CmdClip, DrawCmd, GpuScene, MeshTile, SceneTilt, TileMeshVertex, TileQuad};
use rustgis_render::rustgis_tiles::cache::Payload;
use rustgis_render::tile_mesh::TileMesh;

/// Vertices per marker shape slot in the template bank (the circle's 24 fan triangles fill it
/// exactly; smaller shapes pad with centre points — zero-area triangles are discarded).
const TEMPLATE_VERTS: usize = 72;
const SHAPES: usize = 5;

/// The five marker templates as triangle-fan lists (unit space, y-down like the CPU path's
/// tiny-skia marker shapes at `render::lib.rs` `Ctx::marker`).
fn marker_templates() -> [[f32; 4]; TEMPLATE_VERTS * SHAPES] {
    let mut out = [[0.0; 4]; TEMPLATE_VERTS * SHAPES];
    // Writes one shape's triangle fan (centre + consecutive outline pairs) into its slot.
    fn fan(outline: &[(f32, f32)], dst: &mut [[f32; 4]]) {
        for (i, &a) in outline.iter().enumerate() {
            let b = outline[(i + 1) % outline.len()];
            for (j, p) in [(0.0f32, 0.0f32), a, b].into_iter().enumerate() {
                if let Some(slot) = dst.get_mut(i * 3 + j) {
                    *slot = [p.0, p.1, 0.0, 0.0];
                }
            }
        }
    }
    // Circle: a 24-gon.
    let circle: Vec<(f32, f32)> = (0..24)
        .map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            (a.cos(), a.sin())
        })
        .collect();
    fan(&circle, &mut out[0..TEMPLATE_VERTS]);
    fan(&[(-0.9, -0.9), (0.9, -0.9), (0.9, 0.9), (-0.9, 0.9)], &mut out[TEMPLATE_VERTS..2 * TEMPLATE_VERTS]);
    fan(&[(0.0, -1.1), (1.0, 0.75), (-1.0, 0.75)], &mut out[2 * TEMPLATE_VERTS..3 * TEMPLATE_VERTS]);
    fan(&[(0.0, -1.15), (0.9, 0.0), (0.0, 1.15), (-0.9, 0.0)], &mut out[3 * TEMPLATE_VERTS..4 * TEMPLATE_VERTS]);
    let star: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { 1.2 } else { 0.5 };
            (r * a.cos(), r * a.sin())
        })
        .collect();
    fan(&star, &mut out[4 * TEMPLATE_VERTS..5 * TEMPLATE_VERTS]);
    out
}

/// Per-draw-call uniforms (WGSL `CmdUniforms`: 144 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CmdUniforms {
    view_proj: [[f32; 4]; 4],
    /// The depth row of the camera the scene was drawn for: a tilted scene's band keeps to its
    /// own ground ([`CmdClip::depth`]) however the camera has moved since.
    scene_depth: [f32; 4],
    /// The fog a tilted view's ground fades into (premultiplied).
    fog: [f32; 4],
    /// `x`: map units per physical pixel (scales stroke normals); `y`: 1 for upright markers,
    /// else 0; `z`, `w`: the band's nearest and farthest depths.
    params: [f32; 4],
    /// `x`, `y`: physical pixel size of the map view (scales marker pixel offsets); `z`: 1 when
    /// tilted (fog, the reach, band clipping), else 0; `w`: the reach's depth.
    viewport: [f32; 4],
    /// Extrusions: the light's direction (map units, unit length) and how far a shadow falls per
    /// map unit of height (map units), both turned with the view as MapLibre's light is.
    light: [f32; 4],
}

impl CmdUniforms {
    fn new(cam: &Camera, logical_size: [f64; 2], ppp: f32, origin: [f64; 2], tilt: Option<&SceneTilt>, clip: CmdClip) -> Self {
        let [w, h] = logical_size;
        let scene = tilt.map_or(*cam, |t| t.camera);
        let s = view_proj(&scene, w, h, origin);
        CmdUniforms {
            view_proj: view_proj(cam, w, h, origin),
            scene_depth: [s[0][3], s[1][3], s[2][3], s[3][3]],
            fog: tilt.map_or([0.0; 4], |t| t.fog),
            params: [(cam.res / f64::from(ppp)) as f32, if clip.upright { 1.0 } else { 0.0 }, clip.depth[0], clip.depth[1]],
            viewport: [
                (w * f64::from(ppp)) as f32,
                (h * f64::from(ppp)) as f32,
                if rustgis_map::camera::normalize_pitch(cam.pitch) != 0.0 { 1.0 } else { 0.0 },
                rustgis_map::camera::FAR as f32,
            ],
            light: extrusion_light(cam),
        }
    }
}

/// A vector tile mesh's uniforms (WGSL `TileMeshUniforms`): the command's, how far the camera's
/// zoom is past the mesh's level and the pixels per point, and the tile's square.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileMeshUniforms {
    cmd: CmdUniforms,
    zoom: [f32; 4],
    square: [f32; 4],
}

impl TileMeshUniforms {
    fn new(cam: &Camera, logical_size: [f64; 2], ppp: f32, tile: &MeshTile, tilt: Option<&SceneTilt>) -> Self {
        TileMeshUniforms {
            cmd: CmdUniforms::new(cam, logical_size, ppp, tile.origin, tilt, CmdClip::ALL),
            zoom: [(camera_zoom(cam) - f32::from(tile.level)).clamp(0.0, 1.0), ppp, 0.0, 0.0],
            square: tile.clip,
        }
    }
}

/// A camera's zoom as a style takes it (MapLibre's, for 512-pixel tiles).
fn camera_zoom(cam: &Camera) -> f32 {
    rustgis_render::rustgis_tiles::template::style_zoom(cam.res)
}

/// The extrusions' light for a camera turned to its bearing (see [`CmdUniforms::light`]):
/// `render::EXTRUSION_LIGHT` is given in the view's ground frame, x right and y down.
fn extrusion_light(cam: &Camera) -> [f32; 4] {
    let (l0, l1, l2) = rustgis_render::EXTRUSION_LIGHT;
    let (s, c) = if cam.bearing.is_finite() { cam.bearing.to_radians().sin_cos() } else { (0.0, 1.0) };
    // The view's frame to the map's (as `Camera::unrotate`, then y up).
    let to_map = |u: f64, v: f64| (u * c - v * s, -(u * s + v * c));
    let (lx, ly) = to_map(l0, l1);
    let len = lx.hypot(ly).max(1e-9);
    let (fx, fy) = to_map(-l0 / l2, -l1 / l2);
    [(lx / len) as f32, (ly / len) as f32, fx as f32, fy as f32]
}

/// The backdrop of a tilted view (WGSL `Backdrop`): water fading into fog toward the reach,
/// and the sky above it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BackdropUniforms {
    /// The view's height (logical px), the eye's distance from the centre, the pitch's cosine
    /// and sine.
    tilt: [f32; 4],
    /// Physical pixels per point, the reach's screen row (logical px), the reach's depth.
    params: [f32; 4],
    water: [f32; 4],
    fog: [f32; 4],
    sky: [f32; 4],
}

impl BackdropUniforms {
    fn new(cam: &Camera, logical_size: [f64; 2], ppp: f32, water: [f32; 4], t: &SceneTilt) -> Self {
        let h = logical_size[1];
        let pitch = rustgis_map::camera::normalize_pitch(cam.pitch).to_radians();
        let d = h / 2.0 / rustgis_map::camera::TAN_HALF_FOV;
        let sky_y = cam.sky_y(h).unwrap_or(0.0).max(1e-9);
        BackdropUniforms {
            tilt: [h as f32, d as f32, pitch.cos() as f32, pitch.sin() as f32],
            params: [ppp, sky_y as f32, rustgis_map::camera::FAR as f32, 0.0],
            water,
            fog: t.fog,
            sky: t.sky,
        }
    }
}

/// The scene's depth buffer, for the extrusions, and its stencil, so their shadows darken each
/// pixel once however their pieces overlap.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Premultiplied-alpha blending, the same arithmetic tiny-skia composites with.
pub(crate) const PREMULTIPLIED_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Pipelines, layouts and shared resources, created once per wgpu device. The scene draws
/// into an offscreen 4× MSAA target (anti-aliased like the CPU renderer); its resolved
/// texture is then read back by the host (or blitted, when the host's target format differs).
pub(crate) struct GpuMapRenderer {
    mesh_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    backdrop_pipeline: wgpu::RenderPipeline,
    /// Extruded features' roofs and walls (depth-tested and written), and their shadows.
    extrude_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    /// Raster tiles, each its own texture placed on the map.
    tile_pipeline: wgpu::RenderPipeline,
    /// Vector tile meshes, drawn where their square marked the stencil, and those squares.
    tile_mesh_pipeline: wgpu::RenderPipeline,
    tile_square_pipeline: wgpu::RenderPipeline,
    /// Presents a resolved scene in a host target format other than `Rgba8Unorm`.
    blit_pipeline: wgpu::RenderPipeline,
    /// Draws the placed-label layer into a rectangle of the map that a gesture moves.
    label_pipeline: wgpu::RenderPipeline,
    cmd_layout: wgpu::BindGroupLayout,
    tile_mesh_layout: wgpu::BindGroupLayout,
    backdrop_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    templates: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// MSAA sample count of the offscreen scene target.
    samples: u32,
    /// The tile pictures uploaded so far, kept across scenes.
    tile_textures: std::sync::Mutex<TileTextures>,
    /// The vector tile meshes uploaded so far, kept across scenes.
    tile_meshes: std::sync::Mutex<TileMeshBuffers>,
}

impl GpuMapRenderer {
    /// Builds the pipelines: the scene pipelines against the offscreen `Rgba8Unorm` MSAA
    /// target, the blit pipeline against the host's own target format.
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mapgpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mapgpu.wgsl").into()),
        });
        let uniform_buf = |visibility: wgpu::ShaderStages, binding: u32, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let cmd_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mapgpu_cmd"),
            entries: &[
                uniform_buf(wgpu::ShaderStages::VERTEX_FRAGMENT, 0, std::mem::size_of::<CmdUniforms>() as u64),
                uniform_buf(wgpu::ShaderStages::VERTEX, 1, (TEMPLATE_VERTS * SHAPES * 16) as u64),
            ],
        });
        let tile_mesh_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mapgpu_tile_mesh"),
            entries: &[uniform_buf(wgpu::ShaderStages::VERTEX_FRAGMENT, 0, std::mem::size_of::<TileMeshUniforms>() as u64)],
        });
        let backdrop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mapgpu_backdrop"),
            entries: &[uniform_buf(wgpu::ShaderStages::FRAGMENT, 0, std::mem::size_of::<BackdropUniforms>() as u64)],
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mapgpu_image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mapgpu_blit_uv"),
            entries: &[uniform_buf(wgpu::ShaderStages::VERTEX, 0, 16)],
        });
        let templates = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mapgpu_templates"),
            contents: bytemuck::cast_slice(&marker_templates()),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mapgpu_image"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[wgpu::VertexBufferLayout<'_>],
                        layouts: &[&wgpu::BindGroupLayout],
                        format: wgpu::TextureFormat,
                        samples: u32,
                        depth: Option<wgpu::DepthStencilState>| {
            let layouts: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(|l| Some(*l)).collect();
            let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = buffers.iter().map(|b| Some(b.clone())).collect();
            let target = [Some(wgpu::ColorTargetState { format, blend: Some(PREMULTIPLIED_BLEND), write_mask: wgpu::ColorWrites::ALL })];
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &target,
                }),
                primitive: wgpu::PrimitiveState::default(), // triangle list, no culling
                depth_stencil: depth,
                multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh_buffers = [wgpu::VertexBufferLayout {
            array_stride: 40,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 32, shader_location: 3 },
            ],
        }];
        let point_buffers = [wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 24, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 28, shader_location: 3 },
            ],
        }];
        let extrude_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<rustgis_render::gpu_scene::ExtrudeVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 20, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 36, shader_location: 3 },
            ],
        }];
        let tile_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TileVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 16, shader_location: 2 },
            ],
        }];
        let tile_mesh_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TileMeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 20, shader_location: 3 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 4 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 32, shader_location: 5 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 36, shader_location: 6 },
            ],
        }];
        let samples = 4;
        let scene_format = wgpu::TextureFormat::Rgba8Unorm;
        // Everything but the extrusions draws in order, ignoring the depth buffer; extrusions test
        // and write it, so the nearest roof or wall shows whatever order they come in.
        let depth = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };
        let flat = depth(false, wgpu::CompareFunction::Always);
        // A shadow darkens a pixel only while its stencil is still clear, and marks it.
        let once = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::IncrementClamp,
        };
        let shadows = flat.clone().map(|d| wgpu::DepthStencilState {
            stencil: wgpu::StencilState { front: once, back: once, read_mask: 0xff, write_mask: 0xff },
            ..d
        });
        // A tile's square writes its number into the stencil; its mesh draws only where it is.
        let stencil = |face: wgpu::StencilFaceState, write_mask: u32| {
            flat.clone().map(|d| wgpu::DepthStencilState {
                stencil: wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask },
                ..d
            })
        };
        let mark = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Always,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::Replace,
        };
        let inside = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Equal, pass_op: wgpu::StencilOperation::Keep, ..mark };
        let tile_mesh_pipeline = pipeline(
            "mapgpu_tile_mesh",
            "vs_tile_mesh",
            "fs_tile_mesh",
            &tile_mesh_buffers,
            &[&tile_mesh_layout],
            scene_format,
            samples,
            stencil(inside, 0),
        );
        let tile_square_pipeline = pipeline(
            "mapgpu_tile_square",
            "vs_tile_square",
            "fs_tile_square",
            &[],
            &[&tile_mesh_layout],
            scene_format,
            samples,
            stencil(mark, 0xff),
        );
        let mesh_pipeline =
            pipeline("mapgpu_mesh", "vs_mesh", "fs_mesh", &mesh_buffers, &[&cmd_layout], scene_format, samples, flat.clone());
        let point_pipeline =
            pipeline("mapgpu_points", "vs_marker", "fs_marker", &point_buffers, &[&cmd_layout], scene_format, samples, flat.clone());
        let tile_pipeline = pipeline(
            "mapgpu_tiles",
            "vs_tile",
            "fs_tile",
            &tile_buffers,
            &[&cmd_layout, &image_layout],
            scene_format,
            samples,
            flat.clone(),
        );
        let image_pipeline = pipeline("mapgpu_image", "vs_image", "fs_image", &[], &[&image_layout], scene_format, samples, flat.clone());
        let backdrop_pipeline =
            pipeline("mapgpu_backdrop", "vs_backdrop", "fs_backdrop", &[], &[&backdrop_layout], scene_format, samples, flat.clone());
        let shadow_pipeline =
            pipeline("mapgpu_shadow", "vs_extrude", "fs_extrude", &extrude_buffers, &[&cmd_layout], scene_format, samples, shadows);
        let extrude_pipeline = pipeline(
            "mapgpu_extrude",
            "vs_extrude",
            "fs_extrude",
            &extrude_buffers,
            &[&cmd_layout],
            scene_format,
            samples,
            depth(true, wgpu::CompareFunction::Less),
        );
        // The blit pipeline presents the resolved scene texture in a host target format other
        // than `Rgba8Unorm` (1×: the scene's own pipelines render into the 4× MSAA target).
        let blit_pipeline = pipeline("mapgpu_blit", "vs_blit", "fs_blit", &[], &[&image_layout, &blit_layout], target_format, 1, None);
        // The label layer is drawn where its quad says (a gesture moves the layer's rectangle),
        // with the whole texture in it: `vs_labels` places the quad, `fs_image` samples it.
        let label_pipeline =
            pipeline("mapgpu_labels", "vs_labels", "fs_image", &[], &[&image_layout, &blit_layout], scene_format, samples, flat.clone());
        GpuMapRenderer {
            mesh_pipeline,
            point_pipeline,
            image_pipeline,
            backdrop_pipeline,
            extrude_pipeline,
            shadow_pipeline,
            tile_pipeline,
            tile_mesh_pipeline,
            tile_square_pipeline,
            blit_pipeline,
            label_pipeline,
            cmd_layout,
            tile_mesh_layout,
            backdrop_layout,
            image_layout,
            blit_layout,
            templates,
            sampler,
            samples,
            tile_textures: Default::default(),
            tile_meshes: Default::default(),
        }
    }
}

/// One corner of a tile (WGSL `TileIn`): origin-relative map units, where in the tile's picture,
/// and the layer's opacity.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileVertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

/// The two triangles of a tile.
fn tile_vertices(q: &TileQuad) -> [TileVertex; 6] {
    let [x0, y0, x1, y1] = q.rect;
    let [u0, v0, u1, v1] = q.uv;
    // The picture's top row is the square's top (greatest y).
    let c = |x: f32, y: f32, u: f32, v: f32| TileVertex { position: [x, y], uv: [u, v], opacity: q.opacity };
    let (tl, tr, bl, br) = (c(x0, y1, u0, v0), c(x1, y1, u1, v0), c(x0, y0, u0, v1), c(x1, y0, u1, v1));
    [bl, br, tr, bl, tr, tl]
}

/// A GPU buffer or texture, destroyed as soon as it is dropped. Dropping a wgpu resource
/// frees its memory natively, but in the browser only once JavaScript's garbage collector
/// finds its handle, and the handles are so small that it seldom looks: a map drawn again and
/// again (each scene uploads its own images and meshes) ran the GPU out of memory
/// (`VK_ERROR_OUT_OF_DEVICE_MEMORY`) and crashed the page. Work already submitted still
/// completes: WebGPU and wgpu free the memory once it has.
///
/// Nothing is destroyed between recording a frame and submitting it: a [`SceneCallback`] holds
/// its scene (and through it the tile caches' textures and meshes it draws, shared by `Arc`) in
/// egui's paint list until egui-wgpu has submitted the frame, and the [`SceneTarget`] replaced in
/// `prepare` was last drawn in an earlier frame (each view has its own).
pub(crate) struct Owned<T: Destroy>(T);

/// What [`Owned`] destroys.
pub(crate) trait Destroy {
    fn destroy(&self);
}

impl Destroy for wgpu::Buffer {
    fn destroy(&self) {
        wgpu::Buffer::destroy(self);
    }
}

impl Destroy for wgpu::Texture {
    fn destroy(&self) {
        wgpu::Texture::destroy(self);
    }
}

impl<T: Destroy> Drop for Owned<T> {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

impl<T: Destroy> std::ops::Deref for Owned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

fn owned_buffer(device: &wgpu::Device, desc: &wgpu::util::BufferInitDescriptor<'_>) -> Owned<wgpu::Buffer> {
    Owned(device.create_buffer_init(desc))
}

fn owned_texture(device: &wgpu::Device, desc: &wgpu::TextureDescriptor<'_>) -> Owned<wgpu::Texture> {
    Owned(device.create_texture(desc))
}

/// How many bytes of tile textures are kept besides the latest scene's (a few hundred 256-pixel
/// tiles, or a quarter as many 512-pixel ones).
const KEPT_TILE_BYTES: u64 = 64 << 20;

/// The tile pictures on the GPU, one texture each, kept across scenes (as MapLibre keeps its
/// tiles): a map that moves or is drawn again finds the tiles it showed already uploaded. A
/// texture goes when the tile cache drops its picture, or when it is among the longest unused.
#[derive(Default)]
struct TileTextures {
    /// By the picture's address, with the picture itself held weakly: the tile cache keeps the
    /// picture alive, and once it lets go the texture goes too (the weak reference keeps the
    /// address from being reused until then).
    by_picture: std::collections::HashMap<usize, TileTexture>,
    /// Counts scenes: when each texture was last drawn.
    scene: u64,
}

struct TileTexture {
    picture: std::sync::Weak<Payload>,
    bind: wgpu::BindGroup,
    texture: Arc<Owned<wgpu::Texture>>,
    used: u64,
    bytes: u64,
}

impl TileTextures {
    /// The bind group of a tile's picture, uploaded the first time it is drawn; `None` when it
    /// is no picture or the device can't hold it.
    fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        r: &GpuMapRenderer,
        q: &TileQuad,
    ) -> Option<(wgpu::BindGroup, Arc<Owned<wgpu::Texture>>)> {
        let key = Arc::as_ptr(&q.picture) as usize;
        if let Some(t) = self.by_picture.get_mut(&key)
            && t.picture.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &q.picture))
        {
            t.used = self.scene;
            return Some((t.bind.clone(), t.texture.clone()));
        }
        let pm = q.pixmap()?;
        let (w, h) = (pm.width(), pm.height());
        let max = device.limits().max_texture_dimension_2d;
        if !(1..=max).contains(&w) || !(1..=max).contains(&h) || pm.data().len() as u64 != u64::from(w) * u64::from(h) * 4 {
            return None;
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = owned_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("mapgpu_tile"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            pm.data(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mapgpu_tile_bind"),
            layout: &r.image_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&r.sampler) },
            ],
        });
        let texture = Arc::new(texture);
        let t = TileTexture {
            picture: Arc::downgrade(&q.picture),
            bind: bind.clone(),
            texture: texture.clone(),
            used: self.scene,
            bytes: u64::from(w) * u64::from(h) * 4,
        };
        self.by_picture.insert(key, t);
        Some((bind, texture))
    }

    /// Ends a scene: drops the textures of pictures the tile cache let go, and past
    /// [`KEPT_TILE_BYTES`] those unused longest, but never one the scene just drew.
    fn end_scene(&mut self) {
        self.by_picture.retain(|_, t| t.picture.strong_count() > 0);
        let mut old: Vec<(u64, u64, usize)> =
            self.by_picture.iter().filter(|(_, t)| t.used != self.scene).map(|(k, t)| (t.used, t.bytes, *k)).collect();
        if old.iter().map(|o| o.1).sum::<u64>() > KEPT_TILE_BYTES {
            old.sort_unstable_by_key(|o| std::cmp::Reverse(o.0));
            let mut kept = 0u64;
            for (_, bytes, k) in old {
                kept = kept.saturating_add(bytes);
                if kept > KEPT_TILE_BYTES {
                    self.by_picture.remove(&k);
                }
            }
        }
        self.scene += 1;
    }
}

/// How many bytes of vector tile meshes are kept on the GPU besides the latest scene's.
const KEPT_MESH_BYTES: u64 = if cfg!(target_arch = "wasm32") { 64 << 20 } else { 256 << 20 };

/// The vector tile meshes on the GPU, kept across scenes as [`TileTextures`] keeps tile
/// pictures: by the mesh's address, held weakly (the mesh cache keeps it alive).
#[derive(Default)]
struct TileMeshBuffers {
    by_mesh: std::collections::HashMap<usize, MeshBuffers>,
    scene: u64,
}

struct MeshBuffers {
    mesh: std::sync::Weak<TileMesh>,
    vertices: Arc<Owned<wgpu::Buffer>>,
    indices: Arc<Owned<wgpu::Buffer>>,
    used: u64,
    bytes: u64,
}

impl TileMeshBuffers {
    /// A mesh's vertex and index buffers, uploaded the first time it is drawn.
    fn buffers(&mut self, device: &wgpu::Device, mesh: &Arc<TileMesh>) -> (Arc<Owned<wgpu::Buffer>>, Arc<Owned<wgpu::Buffer>>) {
        let key = Arc::as_ptr(mesh) as usize;
        if let Some(b) = self.by_mesh.get_mut(&key)
            && b.mesh.upgrade().is_some_and(|m| Arc::ptr_eq(&m, mesh))
        {
            b.used = self.scene;
            return (b.vertices.clone(), b.indices.clone());
        }
        let vertices = owned_buffer(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mapgpu_tile_mesh_vertices"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let indices = owned_buffer(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mapgpu_tile_mesh_indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            },
        );
        let (vertices, indices) = (Arc::new(vertices), Arc::new(indices));
        let bytes = mesh.bytes() as u64;
        let b = MeshBuffers { mesh: Arc::downgrade(mesh), vertices: vertices.clone(), indices: indices.clone(), used: self.scene, bytes };
        self.by_mesh.insert(key, b);
        (vertices, indices)
    }

    /// Ends a scene: drops the buffers of meshes gone, and past [`KEPT_MESH_BYTES`] those unused
    /// longest, but never one the scene just drew.
    fn end_scene(&mut self) {
        self.by_mesh.retain(|_, b| b.mesh.strong_count() > 0);
        let mut old: Vec<(u64, u64, usize)> =
            self.by_mesh.iter().filter(|(_, b)| b.used != self.scene).map(|(k, b)| (b.used, b.bytes, *k)).collect();
        if old.iter().map(|o| o.1).sum::<u64>() > KEPT_MESH_BYTES {
            old.sort_unstable_by_key(|o| std::cmp::Reverse(o.0));
            let mut kept = 0u64;
            for (_, bytes, k) in old {
                kept = kept.saturating_add(bytes);
                if kept > KEPT_MESH_BYTES {
                    self.by_mesh.remove(&k);
                }
            }
        }
        self.scene += 1;
    }
}

/// One tile of an uploaded vector tile layer.
pub(crate) struct MeshTileDraw {
    mesh: Arc<TileMesh>,
    tile: MeshTile,
    vertices: Arc<Owned<wgpu::Buffer>>,
    indices: Arc<Owned<wgpu::Buffer>>,
    uniforms: Owned<wgpu::Buffer>,
    bind: wgpu::BindGroup,
}

/// The offscreen scene target: a 4× MSAA color attachment plus the plain texture it resolves
/// to (the read-back source, and the blit source when the host format is not `Rgba8Unorm`).
/// Recreated when the map view's physical size changes.
pub(crate) struct SceneTarget {
    msaa_view: wgpu::TextureView,
    /// The extrusions' depth buffer, multisampled like the colour.
    depth_view: wgpu::TextureView,
    /// The textures behind the views, destroyed with the target.
    _msaa_tex: Owned<wgpu::Texture>,
    _depth_tex: Owned<wgpu::Texture>,
    resolve_tex: Owned<wgpu::Texture>,
    resolve_view: wgpu::TextureView,
    pub resolve_bind: wgpu::BindGroup,
    /// The blit's source viewport (the whole resolve texture) and its bind group.
    /// The blit viewport uniform; the bind group holds it too, the field just keeps its label.
    _blit_uniforms: Owned<wgpu::Buffer>,
    blit_bind: wgpu::BindGroup,
    pub size: [u32; 2],
}

impl SceneTarget {
    pub(crate) fn new(device: &wgpu::Device, r: &GpuMapRenderer, size: [u32; 2]) -> Option<SceneTarget> {
        let [w, h] = size;
        if w == 0 || h == 0 {
            return None;
        }
        let msaa_tex = owned_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("mapgpu_msaa"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: r.samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        );
        let depth_tex = owned_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("mapgpu_depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: r.samples,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        );
        let resolve_tex = owned_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("mapgpu_resolve"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        );
        let resolve_view = resolve_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let resolve_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mapgpu_resolve_bind"),
            layout: &r.image_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolve_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&r.sampler) },
            ],
        });
        let blit_uniforms = owned_buffer(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mapgpu_blit_uv"),
                contents: bytemuck::cast_slice(&[0.0f32, 0.0, 1.0, 1.0]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mapgpu_blit_uv_bind"),
            layout: &r.blit_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: blit_uniforms.as_entire_binding() }],
        });
        Some(SceneTarget {
            msaa_view: msaa_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            _msaa_tex: msaa_tex,
            _depth_tex: depth_tex,
            resolve_tex,
            resolve_view,
            resolve_bind,
            _blit_uniforms: blit_uniforms,
            blit_bind,
            size,
        })
    }
}

/// Records the scene's MSAA render pass into `enc` (resolves into `tgt.resolve_tex`), with the
/// label layer over it when there is one.
fn record_scene(
    r: &GpuMapRenderer,
    draws: &[SceneDraw],
    clear: [f32; 4],
    tgt: &SceneTarget,
    enc: &mut wgpu::CommandEncoder,
    overlay: Option<&Overlay>,
) {
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("mapgpu_scene"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &tgt.msaa_view,
            depth_slice: None,
            resolve_target: Some(&tgt.resolve_view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: clear[3] as f64 }),
                store: wgpu::StoreOp::Discard,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &tgt.depth_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
        }),
        ..Default::default()
    });
    paint_draws(r, draws, &mut rp);
    if let Some(overlay) = overlay {
        rp.set_pipeline(&r.label_pipeline);
        rp.set_bind_group(0, &overlay.bind, &[]);
        rp.set_bind_group(1, &overlay.quad_bind, &[]);
        rp.draw(0..6, 0..1);
    }
}

/// Blits the resolved scene into a host target (a surface texture, or a read-back texture) of
/// the renderer's target format.
fn blit(r: &GpuMapRenderer, tgt: &SceneTarget, view: &wgpu::TextureView, enc: &mut wgpu::CommandEncoder) {
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("mapgpu_blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    });
    rp.set_pipeline(&r.blit_pipeline);
    rp.set_bind_group(0, &tgt.resolve_bind, &[]);
    rp.set_bind_group(1, &tgt.blit_bind, &[]);
    rp.draw(0..6, 0..1);
}

/// One uploaded draw command.
pub(crate) enum SceneDraw {
    Mesh {
        vertices: Owned<wgpu::Buffer>,
        indices: Owned<wgpu::Buffer>,
        index_count: u32,
        uniforms: Owned<wgpu::Buffer>,
        bind: wgpu::BindGroup,
        origin: [f64; 2],
        clip: CmdClip,
    },
    Points {
        instances: Owned<wgpu::Buffer>,
        count: u32,
        uniforms: Owned<wgpu::Buffer>,
        bind: wgpu::BindGroup,
        origin: [f64; 2],
        clip: CmdClip,
    },
    /// Extruded features (depth-tested), or their shadows.
    Extrusion {
        vertices: Owned<wgpu::Buffer>,
        indices: Owned<wgpu::Buffer>,
        index_count: u32,
        uniforms: Owned<wgpu::Buffer>,
        bind: wgpu::BindGroup,
        origin: [f64; 2],
        clip: CmdClip,
        shadows: bool,
    },
    /// A vector tile layer's meshes: what shows of them depends on the camera's zoom (its bits,
    /// rewritten by `set_camera`).
    TileMeshes {
        tiles: Vec<MeshTileDraw>,
        zoom: std::sync::atomic::AtomicU32,
    },
    /// A tilted view's water, fog and sky, under everything.
    Backdrop {
        uniforms: Owned<wgpu::Buffer>,
        bind: wgpu::BindGroup,
    },
    Image {
        bind: wgpu::BindGroup,
        _texture: Owned<wgpu::Texture>,
    },
    /// A raster tile layer: its tiles' corners in one buffer, six per tile, and each tile's
    /// texture (from [`TileTextures`]) with the first of its corners.
    Tiles {
        vertices: Owned<wgpu::Buffer>,
        textures: Vec<(u32, wgpu::BindGroup, Arc<Owned<wgpu::Texture>>)>,
        uniforms: Owned<wgpu::Buffer>,
        bind: wgpu::BindGroup,
        origin: [f64; 2],
        clip: CmdClip,
    },
}

/// A whole scene on the GPU, plus the view parameters needed to recompute per-draw uniforms
/// for a moved camera (`set_camera`).
pub(crate) struct SceneBuffers {
    pub draws: Vec<SceneDraw>,
    /// The scene's clear colour (the basemap's water), used as the MSAA pass's load op.
    pub clear: [f32; 4],
    pub size: [f64; 2],
    pub ppp: f32,
    /// Scenes containing CPU image runs move as a cached picture on north-up flat maps;
    /// other scenes can re-project their vertices for a camera move.
    pub has_images: bool,
    /// Raster tiles, drawn for the camera's zoom: zoomed in far past it they would show blurred.
    pub has_tiles: bool,
    /// A turned or tilted scene: the camera it was drawn for, its sky and fog.
    pub tilt: Option<SceneTilt>,
    /// The farthest a vertex lies from its origin (map units, [`gpu_scene::reach`]): zoomed in
    /// past what f32 holds of it, the scene is drawn again rather than moved (#902).
    pub reach: f64,
}

impl SceneBuffers {
    /// Whether a camera move to `cam` may re-project the scene: a flat, north-up scene for a
    /// flat, north-up view; a turned or tilted one for about the same bearing and pitch, a zoom
    /// and a pan within reach (it covers the ground its own camera showed: a turned view's north-up
    /// box, a tilted view's bands out to its reach), and not zoomed in past what f32 holds of its
    /// vertices; else it is drawn again.
    pub(crate) fn fits(&self, cam: &Camera) -> bool {
        let pitch = rustgis_map::camera::normalize_pitch(cam.pitch);
        let precise = gpu_scene::f32_holds(self.reach, cam.res / f64::from(self.ppp.max(0.1)));
        precise
            && match &self.tilt {
                None => cam.bearing == 0.0 && pitch == 0.0,
                Some(t) => {
                    let turn = rustgis_map::camera::normalize_bearing(cam.bearing - t.camera.bearing).abs();
                    let zoom = cam.res / t.camera.res.max(1e-12);
                    (t.camera.bearing != 0.0) == (cam.bearing != 0.0)
                    && (t.camera.pitch != 0.0) == (pitch != 0.0)
                    && turn < 0.5
                    && (pitch - t.camera.pitch).abs() < 0.5
                    && (0.7..1.4).contains(&zoom)
                    // Panned less than a quarter of the view: the ground it drew still covers it.
                    && (cam.center.x - t.camera.center.x).hypot(cam.center.y - t.camera.center.y)
                        < 0.25 * self.size[0].min(self.size[1]) * t.camera.res
                }
            }
    }
}

/// Whether one draw command fits the device: each of its buffers within `max_buffer_size`,
/// its index/instance count within u32 (they become draw counts), and images non-empty,
/// within `max_texture_dimension_2d`, with exactly `w*h*4` pixels — anything else would only
/// raise wgpu validation errors (fatal under eframe), so the caller renders the frame with
/// the CPU backend instead.
fn cmd_fits(limits: &wgpu::Limits, cmd: &DrawCmd) -> bool {
    let fits = |bytes: usize| u64::try_from(bytes).is_ok_and(|b| b <= limits.max_buffer_size);
    let max_dim = limits.max_texture_dimension_2d;
    match cmd {
        DrawCmd::Mesh { vertices, indices, .. } => {
            fits(std::mem::size_of_val(vertices.as_slice()))
                && fits(std::mem::size_of_val(indices.as_slice()))
                && u32::try_from(indices.len()).is_ok()
        }
        DrawCmd::Points { instances, .. } => fits(std::mem::size_of_val(instances.as_slice())) && u32::try_from(instances.len()).is_ok(),
        DrawCmd::Extrusion { vertices, indices, .. } => {
            fits(std::mem::size_of_val(vertices.as_slice()))
                && fits(std::mem::size_of_val(indices.as_slice()))
                && u32::try_from(indices.len()).is_ok()
        }
        DrawCmd::Image { pixmap } => {
            let (w, h) = (pixmap.width(), pixmap.height());
            (1..=max_dim).contains(&w) && (1..=max_dim).contains(&h) && pixmap.data().len() as u64 == w as u64 * h as u64 * 4
        }
        // A live tile layer is an egui view optimization this host does not build (`Backend::Gpu`).
        DrawCmd::LiveTiles { .. } => true,
        // A tile the device can't hold is left out (`TileTextures::bind`).
        DrawCmd::Tiles { tiles, .. } => tiles.len().checked_mul(6 * std::mem::size_of::<TileVertex>()).is_some_and(fits),
        DrawCmd::TileMeshes { tiles, .. } => tiles.iter().all(|t| {
            fits(std::mem::size_of_val(t.mesh.vertices.as_slice()))
                && fits(std::mem::size_of_val(t.mesh.indices.as_slice()))
                && u32::try_from(t.mesh.indices.len()).is_ok()
        }),
    }
}

impl SceneBuffers {
    /// Uploads a scene's draw commands (meshes, marker instances, CPU images) and bakes the
    /// initial per-command camera uniforms. `None` when the scene exceeds the device's limits
    /// (`cmd_fits`) — the caller renders it with the CPU backend instead.
    pub(crate) fn from_scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        r: &GpuMapRenderer,
        scene: &GpuScene,
        camera: Camera,
        size: [f64; 2],
        ppp: f32,
    ) -> Option<SceneBuffers> {
        let limits = device.limits();
        if !scene.cmds.iter().all(|c| cmd_fits(&limits, c)) {
            return None;
        }
        let mut draws = Vec::with_capacity(scene.cmds.len() + 1);
        let mut has_images = false;
        let mut has_tiles = false;
        let tilt = scene.tilt.as_ref();
        let mut tile_textures = r.tile_textures.lock().unwrap_or_else(|e| e.into_inner());
        let mut tile_meshes = r.tile_meshes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = tilt.filter(|t| t.camera.pitch != 0.0) {
            let u = BackdropUniforms::new(&camera, size, ppp, scene.clear, t);
            let uniforms = owned_buffer(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("mapgpu_backdrop_uniforms"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            );
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mapgpu_backdrop_bind"),
                layout: &r.backdrop_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
            });
            draws.push(SceneDraw::Backdrop { uniforms, bind });
        }
        for (i, cmd) in scene.cmds.iter().enumerate() {
            let clip = scene.clips.get(i).copied().unwrap_or(CmdClip::ALL);
            match cmd {
                DrawCmd::Mesh { origin, vertices, indices } => {
                    if indices.is_empty() {
                        continue;
                    }
                    let uniforms = CmdUniforms::new(&camera, size, ppp, *origin, tilt, clip);
                    let uniforms = owned_buffer(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("mapgpu_cmd_uniforms"),
                            contents: bytemuck::bytes_of(&uniforms),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        },
                    );
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mapgpu_mesh_bind"),
                        layout: &r.cmd_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: r.templates.as_entire_binding() },
                        ],
                    });
                    draws.push(SceneDraw::Mesh {
                        vertices: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_mesh_vertices"),
                                contents: bytemuck::cast_slice(vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            },
                        ),
                        indices: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_mesh_indices"),
                                contents: bytemuck::cast_slice(indices),
                                usage: wgpu::BufferUsages::INDEX,
                            },
                        ),
                        index_count: indices.len() as u32,
                        uniforms,
                        bind,
                        origin: *origin,
                        clip,
                    });
                }
                DrawCmd::Points { origin, instances } => {
                    if instances.is_empty() {
                        continue;
                    }
                    let uniforms = CmdUniforms::new(&camera, size, ppp, *origin, tilt, clip);
                    let uniforms = owned_buffer(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("mapgpu_cmd_uniforms"),
                            contents: bytemuck::bytes_of(&uniforms),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        },
                    );
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mapgpu_points_bind"),
                        layout: &r.cmd_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: r.templates.as_entire_binding() },
                        ],
                    });
                    draws.push(SceneDraw::Points {
                        instances: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_point_instances"),
                                contents: bytemuck::cast_slice(instances),
                                usage: wgpu::BufferUsages::VERTEX,
                            },
                        ),
                        count: instances.len() as u32,
                        uniforms,
                        bind,
                        origin: *origin,
                        clip,
                    });
                }
                DrawCmd::Extrusion { origin, vertices, indices, shadows } => {
                    if indices.is_empty() {
                        continue;
                    }
                    let uniforms = CmdUniforms::new(&camera, size, ppp, *origin, tilt, clip);
                    let uniforms = owned_buffer(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("mapgpu_cmd_uniforms"),
                            contents: bytemuck::bytes_of(&uniforms),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        },
                    );
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mapgpu_extrude_bind"),
                        layout: &r.cmd_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: r.templates.as_entire_binding() },
                        ],
                    });
                    draws.push(SceneDraw::Extrusion {
                        vertices: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_extrude_vertices"),
                                contents: bytemuck::cast_slice(vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            },
                        ),
                        indices: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_extrude_indices"),
                                contents: bytemuck::cast_slice(indices),
                                usage: wgpu::BufferUsages::INDEX,
                            },
                        ),
                        index_count: indices.len() as u32,
                        uniforms,
                        bind,
                        origin: *origin,
                        clip,
                        shadows: *shadows,
                    });
                }
                DrawCmd::Image { pixmap } => {
                    let texture = owned_texture(
                        device,
                        &wgpu::TextureDescriptor {
                            label: Some("mapgpu_image"),
                            size: wgpu::Extent3d { width: pixmap.width(), height: pixmap.height(), depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            // Linear, like every egui texture: values pass through unconverted.
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        },
                    );
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        pixmap.data(),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(pixmap.width() * 4),
                            rows_per_image: Some(pixmap.height()),
                        },
                        wgpu::Extent3d { width: pixmap.width(), height: pixmap.height(), depth_or_array_layers: 1 },
                    );
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mapgpu_image_bind"),
                        layout: &r.image_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&r.sampler) },
                        ],
                    });
                    draws.push(SceneDraw::Image { bind, _texture: texture });
                    has_images = true;
                }
                // Live tile layers are not part of a `Backend::Gpu` scene: skip one if it appears.
                DrawCmd::LiveTiles { .. } => continue,
                DrawCmd::Tiles { origin, tiles } => {
                    let mut vertices = Vec::with_capacity(tiles.len() * 6);
                    let mut textures = Vec::with_capacity(tiles.len());
                    for q in tiles {
                        if let Some((bind, texture)) = tile_textures.bind(device, queue, r, q) {
                            textures.push((vertices.len() as u32, bind, texture));
                            vertices.extend(tile_vertices(q));
                        }
                    }
                    if textures.is_empty() {
                        continue;
                    }
                    let uniforms = CmdUniforms::new(&camera, size, ppp, *origin, tilt, clip);
                    let uniforms = owned_buffer(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("mapgpu_cmd_uniforms"),
                            contents: bytemuck::bytes_of(&uniforms),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        },
                    );
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mapgpu_tiles_bind"),
                        layout: &r.cmd_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: r.templates.as_entire_binding() },
                        ],
                    });
                    draws.push(SceneDraw::Tiles {
                        vertices: owned_buffer(
                            device,
                            &wgpu::util::BufferInitDescriptor {
                                label: Some("mapgpu_tile_vertices"),
                                contents: bytemuck::cast_slice(&vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            },
                        ),
                        textures,
                        uniforms,
                        bind,
                        origin: *origin,
                        clip,
                    });
                    has_tiles = true;
                }
                DrawCmd::TileMeshes { tiles } => {
                    let tile_draws: Vec<MeshTileDraw> = tiles
                        .iter()
                        .map(|t| {
                            let (vertices, indices) = tile_meshes.buffers(device, &t.mesh);
                            let u = TileMeshUniforms::new(&camera, size, ppp, t, tilt);
                            let uniforms = owned_buffer(
                                device,
                                &wgpu::util::BufferInitDescriptor {
                                    label: Some("mapgpu_tile_mesh_uniforms"),
                                    contents: bytemuck::bytes_of(&u),
                                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                                },
                            );
                            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("mapgpu_tile_mesh_bind"),
                                layout: &r.tile_mesh_layout,
                                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
                            });
                            MeshTileDraw { mesh: t.mesh.clone(), tile: t.clone(), vertices, indices, uniforms, bind }
                        })
                        .collect();
                    if !tile_draws.is_empty() {
                        let zoom = std::sync::atomic::AtomicU32::new(camera_zoom(&camera).to_bits());
                        draws.push(SceneDraw::TileMeshes { tiles: tile_draws, zoom });
                    }
                }
            }
        }
        tile_textures.end_scene();
        tile_meshes.end_scene();
        let reach = gpu_scene::reach(&scene.cmds);
        Some(SceneBuffers { draws, clear: scene.clear, size, ppp, has_images, has_tiles, tilt: scene.tilt, reach })
    }

    /// Rewrites every command's camera uniforms for a moved camera — pan/zoom without
    /// re-tessellation (meshes re-project in the vertex shader). wgpu buffer writes go through
    /// the queue, so `&self` suffices. Callers check `has_images` first: image runs don't
    /// re-project and need a full re-render.
    pub(crate) fn set_camera(&self, queue: &wgpu::Queue, cam: &Camera) {
        for d in &self.draws {
            let (uniforms, origin, clip) = match d {
                SceneDraw::Mesh { uniforms, origin, clip, .. }
                | SceneDraw::Points { uniforms, origin, clip, .. }
                | SceneDraw::Extrusion { uniforms, origin, clip, .. }
                | SceneDraw::Tiles { uniforms, origin, clip, .. } => (uniforms, origin, clip),
                SceneDraw::Backdrop { uniforms, .. } => {
                    if let Some(t) = &self.tilt {
                        let u = BackdropUniforms::new(cam, self.size, self.ppp, self.clear, t);
                        queue.write_buffer(uniforms, 0, bytemuck::bytes_of(&u));
                    }
                    continue;
                }
                SceneDraw::TileMeshes { tiles, zoom } => {
                    zoom.store(camera_zoom(cam).to_bits(), std::sync::atomic::Ordering::Relaxed);
                    for t in tiles {
                        let u = TileMeshUniforms::new(cam, self.size, self.ppp, &t.tile, self.tilt.as_ref());
                        queue.write_buffer(&t.uniforms, 0, bytemuck::bytes_of(&u));
                    }
                    continue;
                }
                SceneDraw::Image { .. } => continue,
            };
            let u = CmdUniforms::new(cam, self.size, self.ppp, *origin, self.tilt.as_ref(), *clip);
            queue.write_buffer(uniforms, 0, bytemuck::bytes_of(&u));
        }
    }
}

/// The draw loop (painter's algorithm; the depth buffer is used by extrusions only).
fn paint_draws(renderer: &GpuMapRenderer, draws: &[SceneDraw], rp: &mut wgpu::RenderPass<'_>) {
    for d in draws {
        match d {
            SceneDraw::Mesh { vertices, indices, index_count, bind, .. } => {
                rp.set_pipeline(&renderer.mesh_pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, vertices.slice(..));
                rp.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..*index_count, 0, 0..1);
            }
            SceneDraw::Points { instances, count, bind, .. } => {
                rp.set_pipeline(&renderer.point_pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, instances.slice(..));
                rp.draw(0..TEMPLATE_VERTS as u32, 0..*count);
            }
            SceneDraw::Extrusion { vertices, indices, index_count, bind, shadows, .. } => {
                rp.set_pipeline(if *shadows { &renderer.shadow_pipeline } else { &renderer.extrude_pipeline });
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, vertices.slice(..));
                rp.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..*index_count, 0, 0..1);
            }
            SceneDraw::Backdrop { bind, .. } => {
                rp.set_pipeline(&renderer.backdrop_pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.draw(0..6, 0..1);
            }
            SceneDraw::Image { bind, .. } => {
                rp.set_pipeline(&renderer.image_pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.draw(0..6, 0..1);
            }
            SceneDraw::TileMeshes { tiles, zoom, .. } => {
                let zoom = f32::from_bits(zoom.load(std::sync::atomic::Ordering::Relaxed));
                // Each tile's square marks the stencil with its number, its mesh draws only
                // there, and the squares clear it again for what comes next (a byte holds 255).
                for batch in tiles.chunks(255) {
                    let squares = |rp: &mut wgpu::RenderPass<'_>, mark: bool| {
                        rp.set_pipeline(&renderer.tile_square_pipeline);
                        for (i, t) in batch.iter().enumerate() {
                            rp.set_stencil_reference(if mark { i as u32 + 1 } else { 0 });
                            rp.set_bind_group(0, &t.bind, &[]);
                            rp.draw(0..6, 0..1);
                        }
                    };
                    squares(rp, true);
                    rp.set_pipeline(&renderer.tile_mesh_pipeline);
                    for (i, t) in batch.iter().enumerate() {
                        rp.set_stencil_reference(i as u32 + 1);
                        rp.set_bind_group(0, &t.bind, &[]);
                        rp.set_vertex_buffer(0, t.vertices.slice(..));
                        rp.set_index_buffer(t.indices.slice(..), wgpu::IndexFormat::Uint32);
                        for run in t.mesh.shown(zoom) {
                            rp.draw_indexed(run, 0, 0..1);
                        }
                    }
                    squares(rp, false);
                }
            }
            SceneDraw::Tiles { vertices, textures, bind, .. } => {
                rp.set_pipeline(&renderer.tile_pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.set_vertex_buffer(0, vertices.slice(..));
                for (first, texture, _) in textures {
                    rp.set_bind_group(1, texture, &[]);
                    rp.draw(*first..first + 6, 0..1);
                }
            }
        }
    }
}

/// The GPU map: the pipelines of one device plus the offscreen target a host draws into.
///
/// This is the port of RustGIS 0.6.0's `egui_wgpu::CallbackTrait` implementation for a host
/// that is not egui. The scene is drawn into an offscreen 4× MSAA `Rgba8Unorm` target, resolved,
/// and — when the host's format is not `Rgba8Unorm` (Zed reads **BGRA** for GPUI's image atlas) —
/// blitted once into that format, so `render` hands the host bytes it can upload as they are.
///
/// One instance per window and device; it keeps the tile textures and vector tile mesh buffers
/// across frames, as the egui renderer did. Not `Sync`, since it owns the target and the last
/// scene; drive it from the one thread that draws.
pub struct GpuMap {
    renderer: GpuMapRenderer,
    /// The offscreen target, recreated when the physical size changes.
    target: Option<SceneTarget>,
    /// The scene uploaded by [`GpuMap::prepare`], or kept across a camera move.
    scene: Option<SceneBuffers>,
    /// Physical (device-pixel) size of the target.
    size: [u32; 2],
    /// The format [`GpuMap::render`] returns, and the format of the surface [`GpuMap::present`]
    /// draws into.
    format: wgpu::TextureFormat,
    /// The placed-label text layer, drawn over the map in the scene's own pass.
    overlay: Option<Overlay>,
}

/// A premultiplied RGBA layer over the map (the labels), uploaded from the CPU at device
/// resolution and drawn with the renderer's premultiplied-alpha blend.
///
/// It is drawn into a rectangle of the map (`quad`, in clip space): the whole frame at rest, and
/// a moved and scaled rectangle while a gesture runs, so the labels follow the map without being
/// rasterised again (see `GpuMap::set_overlay_rect`).
struct Overlay {
    texture: Owned<wgpu::Texture>,
    bind: wgpu::BindGroup,
    /// The texture's size, which is the target's at rest.
    size: [u32; 2],
    /// The destination rectangle's clip-space `[left, top, right, bottom]`.
    quad: Owned<wgpu::Buffer>,
    quad_bind: wgpu::BindGroup,
}

impl Overlay {
    /// Writes the layer's destination rectangle, in device pixels of a `size` target, as the
    /// clip-space quad `vs_labels` draws.
    fn set_rect(&self, queue: &wgpu::Queue, rect: [f32; 4], size: [u32; 2]) {
        let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
        let [x0, y0, x1, y1] = rect;
        // Clip space: x right, y up; the top edge is the larger y.
        let quad = [2.0 * x0 / w - 1.0, 1.0 - 2.0 * y0 / h, 2.0 * x1 / w - 1.0, 1.0 - 2.0 * y1 / h];
        queue.write_buffer(&self.quad, 0, bytemuck::cast_slice(&quad));
    }
}

impl GpuMap {
    /// Build the pipelines. `size` is the target's physical (device-pixel) size; `format` is the
    /// byte order [`GpuMap::render`] returns (`Rgba8Unorm` straight RGBA, `Bgra8Unorm` for
    /// GPUI's atlas). The target itself is created by the first [`GpuMap::prepare`], which knows
    /// the view's device-pixel size.
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        GpuMap {
            renderer: GpuMapRenderer::new(device, format),
            target: None,
            scene: None,
            size: [size.0.clamp(1, 16_384), size.1.clamp(1, 16_384)],
            format,
            overlay: None,
        }
    }

    /// The target's physical size.
    pub fn size(&self) -> (u32, u32) {
        (self.size[0], self.size[1])
    }

    /// The format [`GpuMap::render`] returns.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Recreate the target at a new physical size (the next `prepare` does this too).
    pub fn resize(&mut self, _device: &wgpu::Device, size: (u32, u32)) {
        let size = [size.0.clamp(1, 16_384), size.1.clamp(1, 16_384)];
        if size != self.size {
            self.size = size;
            self.target = None;
        }
    }

    /// Upload `scene` for `view` and make sure the offscreen target has the view's physical size
    /// (`view.width * dpi` × `view.height * dpi`). `dpi` is the device-pixel ratio: 1.25 at 125 %
    /// scaling, so a 1200×800 logical map draws 1500×1000 device pixels — the vector result.
    ///
    /// `map` is accepted for API parity with the map view (live tile layers were placed from it
    /// in the egui host); this host's scenes carry their tiles already, so it is not read.
    ///
    /// Returns `false` when the scene exceeds the device's limits (the caller keeps its CPU
    /// frame instead).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _map: &mut rustgis_map::Map,
        view: &rustgis_render::View,
        scene: &GpuScene,
        dpi: f32,
    ) -> bool {
        let ppp = device_scale(dpi, view);
        let physical = physical_size(view, ppp);
        if physical != self.size {
            self.size = physical;
            self.target = None;
        }
        if self.target.is_none() {
            self.target = SceneTarget::new(device, &self.renderer, physical);
        }
        self.scene = SceneBuffers::from_scene(
            device,
            queue,
            &self.renderer,
            scene,
            view.camera,
            [view.width, view.height],
            ppp,
        );
        self.scene.is_some() && self.target.is_some()
    }

    /// Re-use the uploaded scene for a moved camera at the same size, re-projecting its vertices
    /// in the vertex shader (`SceneBuffers::set_camera`): a pan or zoom without re-tessellation.
    /// Returns `false` when the scene must be drawn again (a CPU image run, a turn or tilt, a
    /// zoom past what f32 holds, a size change).
    ///
    /// Labels are placed for the camera the scene was drawn for; a caller re-projecting them
    /// (or hiding them for the gesture) does so itself, as the egui map view did.
    pub fn prepare_reusing(&mut self, queue: &wgpu::Queue, view: &rustgis_render::View, dpi: f32) -> bool {
        let ppp = device_scale(dpi, view);
        if physical_size(view, ppp) != self.size {
            return false;
        }
        let Some(scene) = &self.scene else {
            return false;
        };
        // Raster tiles re-project with their vertices too; the field is kept for a caller that
        // wants to know whether the scene holds any (as the egui map view did for live tiles).
        let _has_tiles = scene.has_tiles;
        if scene.has_images || !scene.fits(&view.camera) {
            return false;
        }
        scene.set_camera(queue, &view.camera);
        true
    }

    /// The scene's per-draw uniforms rewritten for a moved camera (see
    /// [`GpuMap::prepare_reusing`]).
    pub fn set_camera(&self, queue: &wgpu::Queue, view: &rustgis_render::View) {
        if let Some(scene) = &self.scene {
            scene.set_camera(queue, &view.camera);
        }
    }

    /// Whether the uploaded scene has no CPU image runs and fits a camera near `camera`.
    pub fn fits(&self, camera: &rustgis_map::Camera) -> bool {
        self.scene.as_ref().is_some_and(|s| !s.has_images && s.fits(camera))
    }

    /// Draw the prepared scene into a caller's render pass (painter's algorithm; the pass must
    /// have the same attachments as the offscreen target: 4× MSAA colour + depth/stencil).
    pub fn paint(&self, rp: &mut wgpu::RenderPass<'_>) {
        if let Some(scene) = &self.scene {
            paint_draws(&self.renderer, &scene.draws, rp);
            if let Some(overlay) = &self.overlay {
                rp.set_pipeline(&self.renderer.label_pipeline);
                rp.set_bind_group(0, &overlay.bind, &[]);
                rp.set_bind_group(1, &overlay.quad_bind, &[]);
                rp.draw(0..6, 0..1);
            }
        }
    }

    /// Upload the label layer: premultiplied RGBA at the target's device resolution, drawn over
    /// the map in the scene's pass, covering the whole frame. `None`-sized or mismatched data is
    /// ignored (the previous layer stays).
    pub fn set_overlay(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            log::warn!("[rustgis] the label layer is {} bytes for {width}x{height}", rgba.len());
            return;
        }
        let size = [width, height];
        let whole = [0.0, 0.0, width as f32, height as f32];
        if let Some(overlay) = &self.overlay
            && overlay.size == size
        {
            queue.write_texture(
                overlay.texture.as_image_copy(),
                rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            overlay.set_rect(queue, whole, size);
            return;
        }
        let texture = owned_texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("mapgpu_labels"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mapgpu_labels_bind"),
            layout: &self.renderer.image_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.renderer.sampler) },
            ],
        });
        let quad = owned_buffer(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mapgpu_labels_quad"),
                contents: bytemuck::cast_slice(&[0.0f32, 0.0, 1.0, 1.0]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let quad_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mapgpu_labels_quad_bind"),
            layout: &self.renderer.blit_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: quad.as_entire_binding() }],
        });
        let overlay = Overlay { texture, bind, size, quad, quad_bind };
        overlay.set_rect(queue, whole, size);
        self.overlay = Some(overlay);
    }

    /// Move the label layer: `rect` (device pixels, the layer's whole texture in it) is where it
    /// stands in the frame being drawn. A gesture keeps the rasterised layer and only moves it
    /// from the camera it was placed for (see `GpuMap::prepare_reusing`), so the labels follow
    /// the map without being drawn again.
    pub fn set_overlay_rect(&mut self, queue: &wgpu::Queue, rect: [f32; 4]) {
        if let Some(overlay) = &self.overlay {
            overlay.set_rect(queue, rect, overlay.size);
        }
    }

    /// Drop the label layer (a gesture is moving the map: the labels would be stale).
    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    /// Whether a label layer is on.
    pub fn has_overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// Record the scene into `encoder` (the offscreen pass only).
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        if let (Some(target), Some(scene)) = (&self.target, &self.scene) {
            record_scene(&self.renderer, &scene.draws, scene.clear, target, encoder, self.overlay.as_ref());
        }
    }

    /// Draw the prepared scene, resolve it, and blit it into `target` (a surface texture) whose
    /// format must be the one passed to [`GpuMap::new`].
    pub fn present(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        if let (Some(tgt), Some(scene)) = (&self.target, &self.scene) {
            record_scene(&self.renderer, &scene.draws, scene.clear, tgt, encoder, self.overlay.as_ref());
            blit(&self.renderer, tgt, target, encoder);
        }
    }

    /// Draw the prepared scene and return its pixels in [`GpuMap::format`] (the scene pass, the
    /// resolve, the optional format blit and the read-back happen in one submission).
    ///
    /// Empty when nothing was prepared or the read-back failed; the caller falls back.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let (Some(target), Some(scene)) = (&self.target, &self.scene) else {
            return Vec::new();
        };
        let [w, h] = target.size;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mapgpu_frame"),
        });
        record_scene(&self.renderer, &scene.draws, scene.clear, target, &mut encoder, self.overlay.as_ref());
        // A read-back needs the host format: blit first when it is not the resolve's own.
        let present = (self.format != wgpu::TextureFormat::Rgba8Unorm).then(|| {
            let texture = owned_texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some("mapgpu_present"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            blit(&self.renderer, target, &view, &mut encoder);
            texture
        });
        let source = present.as_ref().map_or_else(|| target.resolve_tex.as_image_copy(), |texture| texture.as_image_copy());
        let row_bytes = (w * 4).div_ceil(256) * 256;
        let Some(buffer) = staging_buffer(device, row_bytes, h) else {
            return Vec::new();
        };
        encoder.copy_texture_to_buffer(
            source,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        read_staging(device, &buffer, w, h, row_bytes)
    }

    /// Copy the resolved target into host bytes. Prefer [`GpuMap::render`], which does the copy in
    /// the same submission as the scene.
    pub fn read_back(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let Some(target) = &self.target else {
            return Vec::new();
        };
        let [w, h] = target.size;
        let row_bytes = (w * 4).div_ceil(256) * 256;
        let Some(buffer) = staging_buffer(device, row_bytes, h) else {
            return Vec::new();
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mapgpu_read_back"),
        });
        encoder.copy_texture_to_buffer(
            target.resolve_tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        read_staging(device, &buffer, w, h, row_bytes)
    }
}

/// The device-pixel ratio a frame is drawn at: `dpi` when sane, else the view's own.
fn device_scale(dpi: f32, view: &rustgis_render::View) -> f32 {
    if dpi.is_finite() && dpi > 0.0 {
        dpi.clamp(0.1, 8.0)
    } else {
        (view.pixels_per_point as f32).clamp(0.1, 8.0)
    }
}

/// The offscreen target's physical (device-pixel) size for `view` at `ppp`.
fn physical_size(view: &rustgis_render::View, ppp: f32) -> [u32; 2] {
    let w = (view.width * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32;
    let h = (view.height * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32;
    [w, h]
}

/// A map-read staging buffer for a `row_bytes × h` copy, `None` when the device can't hold it.
fn staging_buffer(device: &wgpu::Device, row_bytes: u32, h: u32) -> Option<wgpu::Buffer> {
    let size = u64::from(row_bytes) * u64::from(h);
    if size == 0 || size > device.limits().max_buffer_size {
        log::warn!("[rustgis] a {row_bytes}x{h} read-back exceeds the device's buffer limit");
        return None;
    }
    Some(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mapgpu_read_back"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    }))
}

/// Wait for `buffer`'s copy and copy its 256-aligned rows into a tightly packed RGBA buffer.
fn read_staging(device: &wgpu::Device, buffer: &wgpu::Buffer, w: u32, h: u32, row_bytes: u32) -> Vec<u8> {
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    if let Err(error) = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }) {
        log::warn!("[rustgis] the GPU read-back could not be waited for: {error}");
        return Vec::new();
    }
    match rx.recv() {
        Ok(Ok(())) => {}
        _ => {
            log::warn!("[rustgis] the GPU read-back buffer could not be mapped");
            return Vec::new();
        }
    }
    let data = match slice.get_mapped_range() {
        Ok(data) => data,
        Err(error) => {
            log::warn!("[rustgis] the GPU read-back buffer could not be read: {error}");
            return Vec::new();
        }
    };
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h as usize {
        let src = y * row_bytes as usize;
        let dst = y * w as usize * 4;
        let Some(row) = data.get(src..src + w as usize * 4) else {
            return Vec::new();
        };
        out[dst..dst + row.len()].copy_from_slice(row);
    }
    drop(data);
    buffer.unmap();
    out
}



#[cfg(test)]
#[path = "tests.rs"]
mod tests;
