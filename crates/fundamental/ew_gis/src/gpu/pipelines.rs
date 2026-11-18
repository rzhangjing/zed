//! Render pipelines and bind-group layouts. Six pipelines, at most five draw
//! calls per frame: tiles/globe, the three vector-plot batches (fill, line,
//! point) and icons.

use bytemuck::{Pod, Zeroable};

use crate::gpu::frame::DEPTH_FORMAT;
use crate::scene::icons::IconInstance;
use crate::scene::plane::TileInstance;
use crate::scene::plot::{PlotLineInstance, PlotPointInstance, PlotVertex};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
}

impl CameraUniform {
    pub fn new(view_proj: glam::Mat4, eye: [f32; 3]) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            eye: [eye[0], eye[1], eye[2], 1.0],
        }
    }
}

/// Camera + viewport + depth-bias uniform shared by the three plot pipelines.
///
/// Kept separate from [`CameraUniform`] on purpose: the tile, globe and icon
/// shaders keep their exact 80-byte layout, so screen-relative point sizes and
/// line widths cannot regress them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PlotUniform {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    /// `xy` = viewport size in physical pixels, `zw` = its reciprocal.
    pub screen: [f32; 4],
    /// Clip-space depth bias (fraction of `w`) for fill / line / point.
    pub depth_bias: [f32; 4],
}

impl PlotUniform {
    pub fn new(
        view_proj: glam::Mat4,
        eye: [f32; 3],
        screen: [f32; 2],
        depth_bias: [f32; 3],
    ) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            eye: [eye[0], eye[1], eye[2], 1.0],
            screen: [
                screen[0],
                screen[1],
                1.0 / screen[0].max(1.0),
                1.0 / screen[1].max(1.0),
            ],
            depth_bias: [depth_bias[0], depth_bias[1], depth_bias[2], 0.0],
        }
    }
}

const TILE_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x4,
    3 => Uint32,
];

const ICON_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32x4,
];

/// Globe tile vertices: ECEF-relative position, ellipsoid normal, tile-local UV,
/// and the texture-array layer index.
pub const GLOBE_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Uint32,
];

/// Fill vertices: world position + colour.
const PLOT_FILL_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x4,
];

/// Line instances: `a` (xyz + half width px), `b` (xyz), colour.
const PLOT_LINE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
];

/// Point instances: centre (xyz + diameter px), colour.
const PLOT_POINT_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
];

pub fn tile_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TileInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &TILE_ATTRIBUTES,
    }
}

pub fn icon_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<IconInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ICON_ATTRIBUTES,
    }
}

pub fn globe_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<crate::scene::globe::GlobeVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &GLOBE_ATTRIBUTES,
    }
}

pub fn plot_fill_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PlotVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &PLOT_FILL_ATTRIBUTES,
    }
}

pub fn plot_line_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PlotLineInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PLOT_LINE_ATTRIBUTES,
    }
}

pub fn plot_point_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PlotPointInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PLOT_POINT_ATTRIBUTES,
    }
}

pub fn tile_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ew_gis tile bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

pub fn icon_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ew_gis icon bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    })
}

/// One camera uniform for all three plot pipelines, no textures.
pub fn plot_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ew_gis plot bind group layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub fn create_tile_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ew_gis tile shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/tile.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ew_gis tile pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[tile_instance_layout()],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub fn create_icon_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ew_gis icon shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/icon.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ew_gis icon pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[icon_instance_layout()],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub fn create_globe_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ew_gis globe shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/globe.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ew_gis globe pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[globe_vertex_layout()],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Skirts are one-sided by construction; no culling keeps the
            // silhouette and any reversed winding safe. The globe is a closed
            // surface, so the depth test still removes the far side.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub fn make_pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    pipeline_layout(device, label, bind_group_layout)
}

/// Shared descriptor for the three plot pipelines: alpha blended, depth tested
/// with `LessEqual` (so coplanar plot geometry wins over the tile plane once
/// the vertex shader applies the plot depth bias) and optionally depth writing.
fn plot_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    label: &str,
    source: &'static str,
    buffers: &[wgpu::VertexBufferLayout<'static>],
    depth_write: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Shape outlines are revolved rings, so winding is not guaranteed
            // to stay consistent; culling is off like the rest of the app.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Polygon / circle / sector triangles. Depth writes on: the fill is the base
/// layer the lines and markers of the same overlay sit on.
pub fn create_plot_fill_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    plot_pipeline(
        device,
        format,
        layout,
        "ew_gis plot fill pipeline",
        include_str!("shaders/plot_fill.wgsl"),
        &[plot_fill_layout()],
        true,
    )
}

/// Instanced screen-space segments. No depth write: ribbons overlap at joins
/// and at crossings, and blending them in draw order is the intended look.
pub fn create_plot_line_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    plot_pipeline(
        device,
        format,
        layout,
        "ew_gis plot line pipeline",
        include_str!("shaders/plot_line.wgsl"),
        &[plot_line_layout()],
        false,
    )
}

/// Instanced SDF point markers. No depth write either: the quad is much larger
/// than the drawn disc, so writing the quad's depth would punch holes.
pub fn create_plot_point_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    plot_pipeline(
        device,
        format,
        layout,
        "ew_gis plot point pipeline",
        include_str!("shaders/plot_point.wgsl"),
        &[plot_point_layout()],
        false,
    )
}
